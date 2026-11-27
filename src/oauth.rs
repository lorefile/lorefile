//! OAuth sign-in for the connectors that cannot send a header: ChatGPT's custom
//! connectors and Claude's (claude.ai, Desktop, mobile). A small authorization
//! server inside Lorefile, as `specs/2026-11-27-oauth-connectors.md` describes.
//!
//! - It answers only for requests marked HTTPS, on the public address worked out
//!   from each request (`Forwarded`, `X-Forwarded-Host`, `Host`), or only on the
//!   address `--public-url` pins. A plain `http://127.0.0.1` request sees no change.
//! - Clients register themselves (DCR), but only with a return address on a short
//!   list: ChatGPT's, Claude's, this machine's loopback, and any `--oauth-redirect`.
//! - A person approves on a page, with the login token or a browser already signed
//!   in: the token is the approver's key. Approval is never automatic.
//! - Tokens are random and opaque (`lfa_` access, `lfr_` refresh), kept only as
//!   SHA-256 hashes in `<dir>/.lorefile/oauth.json`, each tied to the one MCP
//!   address (resource) and public address (issuer) it was approved on.
//!
//! Lorefile makes no outbound request here, holds no third-party credential, and
//! never logs a token or a code.

use crate::serve::{base64url, html_escape, is_loopback, query_params, same_token, url_token};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

pub const READ: &str = "lorefile:read";
pub const PROPOSE: &str = "lorefile:propose";
/// Listed in the metadata because OpenAI asks for it; asking for it changes nothing, since
/// refresh tokens are always issued and unknown scopes are ignored.
pub const OFFLINE: &str = "offline_access";
/// OAuth stays off with a shorter login token: the approval page is on the public internet.
pub const MIN_LOGIN: usize = 32;
/// Register, authorize, token and revoke bodies are small; anything larger is refused unread.
pub const MAX_BODY: usize = 16 * 1024;

const CODE_LIFE: i64 = 60;
/// A used code is remembered this long, so a second use can revoke what the first issued.
const CODE_MEMORY: i64 = 600;
const REQUEST_LIFE: i64 = 600;
const ACCESS_LIFE: i64 = 3600;
const REFRESH_IDLE: i64 = 30 * 24 * 3600;
/// An old refresh token still works this long after it was replaced: a client that retries after a dropped reply.
const GRACE: i64 = 60;
const CLIENT_UNUSED: i64 = 24 * 3600;
const MAX_CLIENTS: usize = 200;
const MAX_REDIRECTS: usize = 10;
const MAX_NAME: usize = 100;
const MAX_STATE: usize = 1024;
const TRIES: usize = 10;
const TRY_WINDOW: i64 = 600;
const PAUSE: i64 = 600;
const MAX_SEEN: usize = 50;
/// Approval requests waiting at once, and per client: one app's flood cannot push out another's request.
const MAX_PENDING: usize = 5000;
const MAX_PENDING_PER_CLIENT: usize = 20;
const MAX_CODES: usize = 500;
const MAX_ACCESS: usize = 10;
const MAX_RETIRED: usize = 20;
/// `last_used` is written to the file at most this often; the page reads the live value.
const USE_SAVE_EVERY: i64 = 300;

const VARY: &str = "Host, X-Forwarded-Host, X-Forwarded-Proto, Forwarded";
const READ_ONLY_REFUSAL: &str = "this connection may only read; disconnect and connect again with 'suggest changes' ticked";

pub fn read_only_refusal() -> &'static str {
    READ_ONLY_REFUSAL
}

// ---------------------------------------------------------------------------
// The clock, injectable for tests
// ---------------------------------------------------------------------------

/// Unix seconds: the system clock, or (in tests) one that moves only when told to.
#[derive(Clone, Default)]
pub struct Clock(Option<Arc<AtomicI64>>);

impl Clock {
    pub fn system() -> Clock {
        Clock(None)
    }

    #[cfg(test)]
    pub fn fake(at: i64) -> (Clock, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(at));
        (Clock(Some(Arc::clone(&t))), t)
    }

    pub fn now(&self) -> i64 {
        match &self.0 {
            Some(t) => t.load(Ordering::SeqCst),
            None => chrono::Utc::now().timestamp(),
        }
    }
}

/// RFC 3339 in UTC, to the second: what `oauth.json` and the page show.
pub fn stamp(t: i64) -> String {
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default()
}

mod at {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(t: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::stamp(*t))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        let s = String::deserialize(d)?;
        chrono::DateTime::parse_from_rfc3339(&s).map(|t| t.timestamp()).map_err(serde::de::Error::custom)
    }
}

mod at_opt {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(t: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => s.serialize_str(&super::stamp(*t)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        match s {
            Some(s) => chrono::DateTime::parse_from_rfc3339(&s).map(|t| Some(t.timestamp())).map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Small pieces: randomness, hashes, PKCE, encoding
// ---------------------------------------------------------------------------

fn random_bytes(bytes: usize) -> Option<Vec<u8>> {
    let mut b = vec![0u8; bytes];
    getrandom::getrandom(&mut b).ok()?;
    Some(b)
}

fn random(bytes: usize) -> Option<String> {
    random_bytes(bytes).map(|b| base64url(&b))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// What the file keeps instead of a token: a 256-bit random token needs no slow hash.
fn hash(token: &str) -> String {
    format!("sha256:{}", hex(&Sha256::digest(token.as_bytes())))
}

/// HMAC-SHA256 (RFC 2104).
fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

/// The login token's fingerprint, so a changed token can be noticed without storing it: an
/// HMAC keyed by a random secret kept in its own file (`oauth.key`). A leaked `oauth.json`
/// alone gives nothing to test guesses against offline.
fn fingerprint(login: &str, key: &[u8]) -> String {
    format!("hmac-sha256:{}", hex(&hmac(key, login.as_bytes())))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

/// PKCE S256 (RFC 7636): BASE64URL(SHA256(verifier)) must equal the challenge.
pub fn pkce_ok(verifier: &str, challenge: &str) -> bool {
    let shape = (43..=128).contains(&verifier.len()) && verifier.bytes().all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c));
    shape && !challenge.is_empty() && same_token(&base64url(&Sha256::digest(verifier.as_bytes())), challenge)
}

/// Only S256, and a challenge of the right shape: `plain` and a missing challenge are refused.
pub fn challenge_ok(challenge: Option<&str>, method: Option<&str>) -> Result<(), &'static str> {
    let c = challenge.unwrap_or("");
    if c.is_empty() {
        return Err("code_challenge is required (PKCE with S256)");
    }
    if method != Some("S256") {
        return Err("code_challenge_method must be S256");
    }
    if !(43..=128).contains(&c.len()) || !c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("code_challenge is not a base64url S256 challenge");
    }
    Ok(())
}

/// Percent-encode everything but the unreserved characters (RFC 3986).
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Standard or URL-safe base64, padding optional: the `Authorization: Basic` header.
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.trim().bytes() {
        if c == b'=' {
            break;
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// The public address, worked out from each request (spec 2.1)
// ---------------------------------------------------------------------------

/// A plain host name with an optional port: `quiet-otter.example`, `a.example:8443`,
/// `[2001:db8::1]:443`. Anything else (a slash, a space, `@`, a path) is not one.
pub fn host_name(v: &str) -> Option<String> {
    let v = v.trim();
    if v.is_empty() || v.len() > 260 {
        return None;
    }
    let (host, port) = if let Some(rest) = v.strip_prefix('[') {
        let (ip, after) = rest.split_once(']')?;
        ip.parse::<std::net::Ipv6Addr>().ok()?;
        let port = match after {
            "" => None,
            p => Some(p.strip_prefix(':')?),
        };
        (format!("[{}]", ip), port)
    } else {
        let mut parts = v.split(':');
        let host = parts.next()?;
        let port = parts.next();
        if parts.next().is_some() {
            return None;
        }
        let ok = !host.is_empty()
            && host.len() <= 253
            && host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.' || c == b'_')
            && !host.starts_with(['.', '-'])
            && !host.ends_with(['.', '-'])
            && !host.contains("..");
        if !ok {
            return None;
        }
        (host.to_string(), port)
    };
    if let Some(p) = port {
        let n: u32 = if !p.is_empty() && p.len() <= 5 && p.bytes().all(|c| c.is_ascii_digit()) { p.parse().ok()? } else { return None };
        if n == 0 || n > 65535 {
            return None;
        }
        return Some(format!("{}:{}", host.to_ascii_lowercase(), n));
    }
    Some(host.to_ascii_lowercase())
}

/// `a.example:8443` → `a.example`; `[::1]:7070` → `[::1]`.
fn host_only(name: &str) -> &str {
    if name.starts_with('[') {
        return name.split_once(']').map(|(h, _)| &name[..h.len() + 1]).unwrap_or(name);
    }
    name.split(':').next().unwrap_or(name)
}

fn loopback_name(name: &str) -> bool {
    let h = host_only(name);
    is_loopback(h) || h.ends_with(".localhost")
}

/// The first element of the first `Forwarded` header (RFC 7239), and its `key=` pair.
fn forwarded(get: &dyn Fn(&'static str) -> Option<String>, key: &str) -> Option<String> {
    let v = get("Forwarded")?;
    let first = v.split(',').next()?;
    first.split(';').find_map(|pair| {
        let (k, val) = pair.trim().split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| val.trim().trim_matches('"').to_string())
    })
}

fn first_value(v: &str) -> &str {
    v.split(',').next().unwrap_or("").trim()
}

/// What a request says about the address it came through.
#[derive(Debug, PartialEq)]
pub struct Asked {
    pub name: Option<String>,
    pub https: bool,
}

/// The name: `Forwarded: host=`, then `X-Forwarded-Host`, then `Host`, first valid one wins.
/// HTTPS: `Forwarded: proto=`, then `X-Forwarded-Proto`, then `CF-Visitor`, else plain HTTP.
pub fn asked(get: &dyn Fn(&'static str) -> Option<String>) -> Asked {
    let name = forwarded(get, "host")
        .and_then(|v| host_name(&v))
        .or_else(|| get("X-Forwarded-Host").and_then(|v| host_name(first_value(&v))))
        .or_else(|| get("Host").and_then(|v| host_name(&v)));
    let proto = forwarded(get, "proto")
        .filter(|v| !v.is_empty())
        .or_else(|| get("X-Forwarded-Proto").map(|v| first_value(&v).to_string()).filter(|v| !v.is_empty()))
        .or_else(|| get("CF-Visitor").and_then(|v| serde_json::from_str::<Json>(&v).ok()).and_then(|j| j["scheme"].as_str().map(|s| s.to_string())));
    Asked { name, https: proto.map(|p| p.eq_ignore_ascii_case("https")).unwrap_or(false) }
}

/// `https://name`, without the default port.
fn origin_of(name: &str, https: bool) -> String {
    let (scheme, default) = if https { ("https", ":443") } else { ("http", ":80") };
    format!("{}://{}", scheme, name.strip_suffix(default).unwrap_or(name))
}

/// `--public-url`: `https://` and a host (or `http://` on loopback, for tests), no path, no query.
pub fn pin(url: &str) -> Result<String, String> {
    let u = url.trim();
    let (https, rest) = if let Some(r) = u.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = u.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("--public-url {} must start with https://", u));
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains(['/', '?', '#', '@']) {
        return Err(format!("--public-url {} must be just https:// and a host name: no path, no query (Lorefile has to sit at the root of its address)", u));
    }
    let name = host_name(rest).ok_or_else(|| format!("--public-url {}: `{}` is not a host name", u, rest))?;
    if !https && !loopback_name(&name) {
        return Err(format!("--public-url {} must be https://: ChatGPT and Claude only connect over HTTPS (http:// is allowed on 127.0.0.1, for tests)", u));
    }
    Ok(origin_of(&name, https))
}

// ---------------------------------------------------------------------------
// Return addresses (redirect URIs) and resources
// ---------------------------------------------------------------------------

const CHATGPT_STABLE: &str = "https://chatgpt.com/connector_platform_oauth_redirect";
const CHATGPT_LEGACY: &str = "https://chatgpt.com/connector/oauth/";
const CLAUDE: [&str; 2] = ["https://claude.ai/api/mcp/auth_callback", "https://claude.com/api/mcp/auth_callback"];

/// `http://localhost:<any port>/…`, `http://127.0.0.1:…`, `http://[::1]:…` (RFC 8252).
fn loopback_redirect(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("http://") else { return false };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host_ok = match host_name(authority) {
        Some(n) => n == authority.to_ascii_lowercase() && matches!(host_only(&n), "localhost" | "127.0.0.1" | "[::1]"),
        None => false,
    };
    host_ok && !path.contains('#') && path.bytes().all(|c| c.is_ascii_graphic() && c != b'\\')
}

/// Whether a client may register and use this return address: the documented
/// ChatGPT and Claude forms, loopback on any port, and `--oauth-redirect` ones, matched exactly.
pub fn redirect_allowed(uri: &str, extra: &[String]) -> bool {
    if uri.is_empty() || uri.len() > 2000 || uri.contains('#') || uri.bytes().any(|c| !c.is_ascii_graphic()) {
        return false;
    }
    if uri == CHATGPT_STABLE || CLAUDE.contains(&uri) || extra.iter().any(|e| e == uri) {
        return true;
    }
    if let Some(id) = uri.strip_prefix(CHATGPT_LEGACY) {
        return !id.is_empty() && id.len() <= 200 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_');
    }
    loopback_redirect(uri)
}

/// `--oauth-redirect`: an exact HTTPS address (or loopback HTTP), no fragment.
pub fn extra_redirect(uri: &str) -> Result<String, String> {
    let u = uri.trim();
    let shape = (u.starts_with("https://") && u.len() > "https://".len()) || loopback_redirect(u);
    if !shape || u.contains('#') || u.bytes().any(|c| !c.is_ascii_graphic()) {
        return Err(format!("--oauth-redirect {}: use an exact https:// address (or http:// on loopback), with no #fragment", u));
    }
    let host = u.trim_start_matches("https://").trim_start_matches("http://").split(['/', '?']).next().unwrap_or("");
    if host.contains('@') || host_name(host).is_none() {
        return Err(format!("--oauth-redirect {}: `{}` is not a host name", u, host));
    }
    Ok(u.to_string())
}

/// Which MCP address a token opens: `/mcp` (every space) or `/s/<id>/mcp` (one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    All,
    Space(String),
}

impl Resource {
    /// The canonical form, the one stored with a grant.
    pub fn url(&self, origin: &str) -> String {
        match self {
            Resource::All => format!("{}/mcp", origin),
            Resource::Space(id) => format!("{}/s/{}/mcp", origin, enc(id)),
        }
    }
}

/// A space id from a path segment: decoded, and nothing that could leave the segment.
fn space_segment(seg: &str) -> Option<String> {
    let id = crate::serve::percent_decode(seg);
    let ok = !id.is_empty() && id.chars().count() <= 200 && id != "." && id != ".." && !id.chars().any(|c| c == '/' || c == '\\' || c.is_control() || c.is_whitespace());
    ok.then_some(id)
}

/// A `resource` for the address this request came through: `<public>/mcp`, `<public>/s/<id>/mcp`
/// (a trailing slash ignored), or the bare origin, which means `/mcp`. Anything else is not ours.
pub fn parse_resource(origin: &str, s: &str) -> Option<Resource> {
    let s = s.trim();
    let head = s.get(..origin.len())?;
    if !head.eq_ignore_ascii_case(origin) {
        return None;
    }
    let rest = &s[origin.len()..];
    if rest.contains(['?', '#']) || !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty() || rest == "/mcp" {
        return Some(Resource::All);
    }
    let seg = rest.strip_prefix("/s/")?.strip_suffix("/mcp")?;
    if seg.contains('/') {
        return None;
    }
    space_segment(seg).map(Resource::Space)
}

/// The issuer a stored resource belongs to: everything before the path.
fn issuer_of(resource: &str) -> &str {
    let after = resource.find("://").map(|i| i + 3).unwrap_or(0);
    match resource[after..].find('/') {
        Some(i) => &resource[..after + i],
        None => resource,
    }
}

fn scope_list(scopes: &[String]) -> String {
    scopes.join(" ")
}

/// Ours, in a fixed order: read always, propose when asked for. Unknown scopes are ignored,
/// and a request that names none of ours asks for both (the page lets the person untick one).
fn requested_scopes(scope: Option<&str>) -> Vec<String> {
    let words: Vec<&str> = scope.unwrap_or("").split_whitespace().collect();
    let ours = words.iter().any(|w| *w == READ || *w == PROPOSE);
    let propose = !ours || words.contains(&PROPOSE);
    let mut out = vec![READ.to_string()];
    if propose {
        out.push(PROPOSE.to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// The store: `<dir>/.lorefile/oauth.json`, hashes only
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
struct Store {
    version: u32,
    /// Salted fingerprint of the login token these grants were approved under.
    #[serde(default)]
    login: String,
    #[serde(default)]
    clients: Vec<Client>,
    #[serde(default)]
    connections: Vec<Connection>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Client {
    id: String,
    name: String,
    redirect_uris: Vec<String>,
    /// none, client_secret_post or client_secret_basic
    auth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    #[serde(with = "at")]
    registered: i64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Connection {
    id: String,
    client: String,
    name: String,
    resource: String,
    scopes: Vec<String>,
    #[serde(with = "at")]
    approved: i64,
    approved_with: String,
    refresh: Refresh,
    #[serde(default)]
    access: Vec<Access>,
    #[serde(default, with = "at_opt", skip_serializing_if = "Option::is_none")]
    last_used: Option<i64>,
    /// What happens to what it suggests on a server with `--allow-edit`: `wait` for a person
    /// to accept (the default, also for connections approved before this setting existed), or
    /// `direct`: written straight away, graded proposed (unverified) until a person looks.
    /// Only the page changes it; the OAuth endpoints never do after approval.
    #[serde(default = "wait")]
    writes: String,
}

pub const WAIT: &str = "wait";
pub const DIRECT: &str = "direct";

fn wait() -> String {
    WAIT.to_string()
}

#[derive(Serialize, Deserialize, Clone)]
struct Refresh {
    hash: String,
    #[serde(with = "at")]
    expires: i64,
    /// Refresh tokens this one replaced: within the grace window they still work, after it they revoke.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retired: Vec<Retired>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Retired {
    hash: String,
    #[serde(with = "at")]
    replaced: i64,
}

#[derive(Serialize, Deserialize, Clone)]
struct Access {
    hash: String,
    #[serde(with = "at")]
    expires: i64,
}

impl Store {
    /// Drop what has run out: access tokens, connections idle for 30 days, clients with no
    /// connection after a day. Returns whether anything changed.
    fn prune(&mut self, now: i64) -> bool {
        let count = |s: &Store| (s.clients.len(), s.connections.len(), s.connections.iter().map(|c| c.access.len()).sum::<usize>());
        let before = count(self);
        for c in &mut self.connections {
            c.access.retain(|a| a.expires > now);
        }
        self.connections.retain(|c| c.refresh.expires > now);
        let used: HashSet<String> = self.connections.iter().map(|c| c.client.clone()).collect();
        self.clients.retain(|c| used.contains(&c.id) || now - c.registered < CLIENT_UNUSED);
        before != count(self)
    }
}

/// Written to a temporary file with mode 0600, then renamed into place.
fn write_store(file: &Path, store: &Store) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(store).map_err(std::io::Error::other)? + "\n";
    write_private(file, &text)
}

fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = file.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, file)
}

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

pub struct Config {
    pub login: String,
    /// `--public-url`, as an origin: sign-ins answer only there.
    pub pinned: Option<String>,
    /// `--oauth-redirect`: return addresses allowed on top of the list.
    pub extra_redirects: Vec<String>,
    /// `--allow-edit`: what the approval page says happens to a suggestion.
    pub edit_allowed: bool,
    /// How the `Seen` line shows the sign-in link: `None` puts the token in it (a token from
    /// the file), `Some(words)` names where the token is (from `--token` or `LOREFILE_TOKEN`).
    pub link_note: Option<String>,
}

/// One connection's rights, from a valid access token.
#[derive(Clone, Debug)]
pub struct Grant {
    pub name: String,
    pub scopes: Vec<String>,
    pub approved: i64,
    /// The person who approved it chose "go in straight away, marked unverified".
    pub direct: bool,
}

impl Grant {
    pub fn can_propose(&self) -> bool {
        self.scopes.iter().any(|s| s == PROPOSE)
    }

    /// `agent via MCP (connection "ChatGPT", approved 27 Nov)`: the author in proposals,
    /// the activity log, commits and trust.yaml.
    pub fn label(&self) -> String {
        let day = chrono::DateTime::from_timestamp(self.approved, 0).map(|d| d.format("%-d %b").to_string()).unwrap_or_default();
        format!("agent via MCP (connection \"{}\", approved {})", self.name, day)
    }
}

struct Pending {
    /// Order of arrival: many requests share a second, so this decides which is oldest.
    seq: u64,
    client: String,
    redirect: String,
    /// Whether authorize named the return address (else the client's only one was used).
    redirect_given: bool,
    challenge: String,
    resource: String,
    space: Option<String>,
    scopes: Vec<String>,
    state: Option<String>,
    origin: String,
    created: i64,
}

struct Code {
    client: String,
    redirect: String,
    redirect_given: bool,
    challenge: String,
    resource: String,
    scopes: Vec<String>,
    origin: String,
    created: i64,
    approved_with: String,
    /// The connection the first use created: a second use revokes it.
    used_by: Option<String>,
    /// `wait` or `direct`, as chosen on the approval page.
    writes: String,
}

#[derive(Default)]
struct State {
    store: Store,
    /// Clients that registered and have no connection yet: memory only, written to
    /// `oauth.json` with their first connection, so a registration costs no disk write.
    fresh: Vec<Client>,
    pending: HashMap<String, Pending>,
    seq: u64,
    codes: HashMap<String, Code>,
    tries: Vec<i64>,
    paused_until: i64,
}

impl State {
    fn client(&self, id: &str) -> Option<Client> {
        self.store.clients.iter().chain(self.fresh.iter()).find(|c| c.id == id).cloned()
    }
}

/// What has been printed once: kept apart from the store, so printing never waits on it.
#[derive(Default)]
struct Log {
    seen: Vec<String>,
    hinted: HashSet<&'static str>,
}

pub struct Server {
    file: PathBuf,
    cfg: Config,
    clock: Clock,
    state: Mutex<State>,
    log: Mutex<Log>,
    /// Connections dropped at start because the login token changed.
    pub dropped: usize,
    /// Why `oauth.json` could not be read, if it could not.
    pub load_error: Option<String>,
}

/// One OAuth request, as `serve` hands it over.
pub struct Req<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a HashMap<String, String>,
    pub header: &'a dyn Fn(&'static str) -> Option<String>,
    pub body: &'a str,
    /// The public address this request came through.
    pub origin: &'a str,
    /// How the request already proves the login: `Some("login token")` for the bearer header,
    /// `Some("signed-in browser")` for the page's cookie.
    pub approver: Option<&'static str>,
}

/// What `serve` sends back. `sign_in`: also set the page's cookie (a correct token on the approval page).
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub sign_in: bool,
}

impl Reply {
    fn new(status: u16, mime: &str, body: String) -> Reply {
        let headers = vec![("Content-Type".into(), mime.into()), ("Cache-Control".into(), "no-store".into()), ("Vary".into(), VARY.into())];
        Reply { status, headers, body, sign_in: false }
    }

    fn json(status: u16, body: &Json) -> Reply {
        Reply::new(status, "application/json; charset=utf-8", serde_json::to_string(body).unwrap_or_default())
    }

    fn error(status: u16, code: &str, description: &str) -> Reply {
        Reply::json(status, &json!({ "error": code, "error_description": description }))
    }

    fn html(status: u16, body: String) -> Reply {
        Reply::new(status, "text/html; charset=utf-8", body).with("X-Frame-Options", "DENY").with("Content-Security-Policy", "frame-ancestors 'none'").with("Referrer-Policy", "same-origin")
    }

    fn redirect(to: String) -> Reply {
        Reply::new(302, "text/plain; charset=utf-8", String::new()).with("Location", &to)
    }

    fn with(mut self, k: &str, v: &str) -> Reply {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }
}

/// The OAuth addresses Lorefile adds; `serve` routes them before the login gate.
pub fn is_route(path: &str) -> bool {
    path == "/.well-known/oauth-authorization-server" || path == "/.well-known/oauth-protected-resource" || path.starts_with("/.well-known/oauth-protected-resource/") || matches!(path, "/oauth/register" | "/oauth/authorize" | "/oauth/token" | "/oauth/revoke")
}

/// `{{name}}` → its value, in one pass: a value is never searched again, so a client's
/// name cannot pull in another field.
fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 4096);
    let mut rest = template;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find("}}") {
            Some(j) => {
                let key = &after[..j];
                match vars.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push_str("{{");
                        out.push_str(key);
                        out.push_str("}}");
                    }
                }
                rest = &after[j + 2..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The return address with `params` added to its query.
fn back_to(redirect: &str, params: &[(&str, &str)]) -> String {
    let mut out = redirect.to_string();
    let mut sep = if redirect.contains('?') { '&' } else { '?' };
    for (k, v) in params {
        out.push(sep);
        out.push_str(&format!("{}={}", k, enc(v)));
        sep = '&';
    }
    out
}

fn redirect_host(uri: &str) -> String {
    uri.split_once("://").map(|(_, r)| r).unwrap_or(uri).split(['/', '?']).next().unwrap_or("").to_string()
}

/// A client's name, as it gave it: printable, short, no quotes or angle brackets.
fn clean_name(name: &str) -> String {
    let n: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '"' | '<' | '>' | '\\' | '`')).take(MAX_NAME).collect();
    let n = n.trim().to_string();
    if n.is_empty() { "Unnamed client".to_string() } else { n }
}

impl Server {
    /// Load `<root>/.lorefile/oauth.json` (or start empty), drop grants made under another
    /// login token, prune, and save.
    pub fn open(root: &Path, cfg: Config, clock: Clock) -> Server {
        let dir = crate::spaces::lorefile_dir(root);
        let file = dir.join("oauth.json");
        // the fingerprint's key lives in its own file: without it, a stored fingerprint cannot be trusted
        let key_file = dir.join("oauth.key");
        let kept = std::fs::read_to_string(&key_file).ok().and_then(|t| hex_decode(&t)).filter(|k| k.len() == 32);
        let had_key = kept.is_some();
        let key = kept.unwrap_or_else(|| random_bytes(32).unwrap_or_default());
        if !had_key {
            let _ = write_private(&key_file, &format!("{}\n", hex(&key)));
        }
        let mut load_error = None;
        let mut store = match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Store>(&text) {
                Ok(s) => s,
                Err(e) => {
                    load_error = Some(format!("{} could not be read ({}); starting with no connections", file.display(), e));
                    Store::default()
                }
            },
            Err(_) => Store::default(),
        };
        store.version = 1;
        let mut dropped = 0;
        let mut changed = false;
        let want = fingerprint(&cfg.login, &key);
        if !had_key || !same_token(&store.login, &want) {
            if !store.login.is_empty() {
                dropped = store.connections.len();
                store.connections.clear();
            }
            store.login = want;
            changed = true;
        }
        changed |= store.prune(clock.now());
        let srv = Server { file, cfg, clock, state: Mutex::new(State { store, ..Default::default() }), log: Mutex::new(Log::default()), dropped, load_error };
        if changed {
            let st = srv.lock();
            let _ = srv.save(&st);
        }
        srv
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, st: &State) -> Result<(), String> {
        write_store(&self.file, &st.store).map_err(|e| format!("could not save {}: {}", self.file.display(), e))
    }

    pub fn pinned(&self) -> Option<&str> {
        self.cfg.pinned.as_deref()
    }

    pub fn connection_count(&self) -> usize {
        self.lock().store.connections.len()
    }

    // --- the address -------------------------------------------------------

    /// The public address OAuth answers on for this request, or `None` (plain HTTP,
    /// a hidden name, or not the pinned address). Prints the one-time hints, and the
    /// `Seen` line for the pinned address. An address read from a request is printed
    /// only once someone holding the login token approved a connection on it.
    pub fn public_origin(&self, get: &dyn Fn(&'static str) -> Option<String>) -> Option<String> {
        let a = asked(get);
        let name = a.name?;
        if let Some(pin) = &self.cfg.pinned {
            let o = origin_of(&name, a.https);
            if o == *pin {
                self.seen_pinned(&o);
                return Some(o);
            }
            return None;
        }
        if !a.https {
            if !loopback_name(&name) {
                self.hint_once("http", "OAuth   a request reached this server over plain HTTP under a public name: ChatGPT and Claude need HTTPS, so OAuth answers only on an HTTPS address (a tunnel, or a proxy that sets X-Forwarded-Proto: https)".to_string());
            }
            return None;
        }
        if loopback_name(&name) {
            self.hint_once("proxy", "OAuth   a proxy is hiding the public address (a request marked HTTPS arrived for a loopback name); have it pass Host and X-Forwarded-Proto (nginx: proxy_set_header Host $host; proxy_set_header X-Forwarded-Proto $scheme;), or start with --public-url".to_string());
            return None;
        }
        Some(origin_of(&name, true))
    }

    fn log(&self) -> std::sync::MutexGuard<'_, Log> {
        self.log.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn hint_once(&self, key: &'static str, text: String) {
        if self.log().hinted.insert(key) {
            eprintln!("{}", text);
        }
    }

    /// Whether this address gets its `Seen` line now (once each, at most `MAX_SEEN`).
    fn first_sight(&self, origin: &str) -> bool {
        let mut log = self.log();
        if log.seen.iter().any(|s| s == origin) {
            return false;
        }
        if log.seen.len() >= MAX_SEEN {
            if log.hinted.insert("seen-cap") {
                eprintln!("Seen    more than {} addresses; not printing more", MAX_SEEN);
            }
            return false;
        }
        log.seen.push(origin.to_string());
        true
    }

    /// The first request on the pinned address: the operator named it (`--public-url`), so the
    /// sign-in link follows the start-up lines' rule (the token only when it came from the file).
    fn seen_pinned(&self, origin: &str) {
        if !self.first_sight(origin) {
            return;
        }
        let link = match &self.cfg.link_note {
            None => format!("{}/?token={}", origin, url_token(&self.cfg.login)),
            Some(words) => format!("{}/   {}", origin, words),
        };
        eprintln!("Seen    {}   connectors: {}/mcp (choose OAuth, leave the client ID empty)", origin, origin);
        eprintln!("        sign in there: {}", link);
    }

    /// The first approval on an address read from requests. It took the login token, so the
    /// address is real; the line never carries the token.
    fn approved_on(&self, origin: &str) {
        if self.cfg.pinned.is_none() && self.first_sight(origin) {
            eprintln!("Seen    {}   a connection was approved on this address; its connector URL is {}/mcp", origin, origin);
        }
    }

    /// The `WWW-Authenticate` value for a 401 from `/mcp` on an OAuth address.
    pub fn challenge(&self, origin: &str, mcp_path: &str, had_token: bool) -> String {
        format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource{}\", scope=\"{} {}\"{}",
            origin,
            mcp_path,
            READ,
            PROPOSE,
            if had_token { ", error=\"invalid_token\"" } else { "" }
        )
    }

    // --- checking a token on /mcp -----------------------------------------

    /// The grant behind an access token, if it is ours, alive, and was issued for exactly
    /// this MCP address on this public address.
    pub fn check(&self, bearer: &str, origin: &str, resource: &Resource) -> Option<Grant> {
        if !bearer.starts_with("lfa_") {
            return None;
        }
        let now = self.clock.now();
        let (h, want) = (hash(bearer), resource.url(origin));
        let mut st = self.lock();
        let c = st.store.connections.iter_mut().find(|c| c.access.iter().any(|a| a.hash == h && a.expires > now))?;
        if c.resource != want || c.refresh.expires <= now {
            return None;
        }
        let stale = c.last_used.map(|t| now - t >= USE_SAVE_EVERY).unwrap_or(true);
        c.last_used = Some(now);
        let grant = Grant { name: c.name.clone(), scopes: c.scopes.clone(), approved: c.approved, direct: c.writes == DIRECT };
        if stale {
            let _ = self.save(&st);
        }
        Some(grant)
    }

    // --- Settings › Connections -------------------------------------------

    pub fn connections_json(&self) -> Json {
        let st = self.lock();
        let list: Vec<Json> = st
            .store
            .connections
            .iter()
            .map(|c| {
                let issuer = issuer_of(&c.resource);
                let space = parse_resource(issuer, &c.resource).and_then(|r| match r {
                    Resource::Space(id) => Some(id),
                    Resource::All => None,
                });
                json!({
                    "id": c.id, "name": c.name, "address": issuer, "resource": c.resource, "space": space,
                    "scopes": c.scopes, "approved": stamp(c.approved), "approved_with": c.approved_with,
                    "last_used": c.last_used.map(stamp), "writes": c.writes,
                })
            })
            .collect();
        json!({ "on": true, "pinned": self.cfg.pinned, "edit_allowed": self.cfg.edit_allowed, "connections": list })
    }

    /// Settings › Connections: what happens to one connection's suggestions, `wait` or `direct`.
    pub fn set_writes(&self, id: &str, writes: &str) -> Result<(), String> {
        if writes != WAIT && writes != DIRECT {
            return Err(format!("writes is wait or direct, not `{}`", writes));
        }
        let mut st = self.lock();
        let c = st.store.connections.iter_mut().find(|c| c.id == id).ok_or_else(|| format!("no connection {}", id))?;
        if writes == DIRECT && !c.scopes.iter().any(|s| s == PROPOSE) {
            return Err("this connection may only read, so it suggests nothing".into());
        }
        c.writes = writes.to_string();
        self.save(&st)
    }

    /// Disconnect one connection, or `all`. Returns how many went.
    pub fn disconnect(&self, id: &str) -> Result<usize, String> {
        let mut st = self.lock();
        let before = st.store.connections.len();
        if id == "all" {
            st.store.connections.clear();
        } else {
            st.store.connections.retain(|c| c.id != id);
        }
        let gone = before - st.store.connections.len();
        if gone == 0 && id != "all" {
            return Err(format!("no connection {}", id));
        }
        self.save(&st)?;
        Ok(gone)
    }

    // --- the OAuth routes --------------------------------------------------

    pub fn handle(&self, r: &Req) -> Reply {
        let p = r.path;
        match (r.method, p) {
            ("GET", "/.well-known/oauth-authorization-server") => self.server_metadata(r.origin),
            ("GET", _) if p == "/.well-known/oauth-protected-resource" || p == "/.well-known/oauth-protected-resource/mcp" => self.resource_metadata(r.origin, &format!("{}/mcp", r.origin)),
            ("GET", _) if p.starts_with("/.well-known/oauth-protected-resource/s/") => {
                let tail = &p["/.well-known/oauth-protected-resource/s/".len()..];
                match tail.strip_suffix("/mcp").filter(|seg| !seg.contains('/') && space_segment(seg).is_some()) {
                    // the same shape whether or not the space exists: the metadata reveals no names
                    Some(seg) => self.resource_metadata(r.origin, &format!("{}/s/{}/mcp", r.origin, seg)),
                    None => Reply::json(404, &json!({ "error": "not found" })),
                }
            }
            ("POST", "/oauth/register") => self.register(r),
            ("GET", "/oauth/authorize") => self.authorize_page(r),
            ("POST", "/oauth/authorize") => self.authorize_decision(r),
            ("POST", "/oauth/token") => self.token(r),
            ("POST", "/oauth/revoke") => self.revoke(r),
            (_, "/oauth/authorize") => Reply::json(405, &json!({ "error": "method not allowed" })).with("Allow", "GET, POST"),
            (_, "/oauth/register" | "/oauth/token" | "/oauth/revoke") => Reply::json(405, &json!({ "error": "method not allowed" })).with("Allow", "POST"),
            ("GET", _) => Reply::json(404, &json!({ "error": "not found" })),
            _ => Reply::json(405, &json!({ "error": "method not allowed" })).with("Allow", "GET"),
        }
    }

    fn resource_metadata(&self, origin: &str, resource: &str) -> Reply {
        Reply::json(200, &json!({
            "resource": resource,
            "authorization_servers": [origin],
            "scopes_supported": [READ, PROPOSE, OFFLINE],
            "bearer_methods_supported": ["header"],
            "resource_name": "Lorefile",
        }))
        .with("Access-Control-Allow-Origin", "*")
    }

    fn server_metadata(&self, origin: &str) -> Reply {
        Reply::json(200, &json!({
            "issuer": origin,
            "authorization_endpoint": format!("{}/oauth/authorize", origin),
            "token_endpoint": format!("{}/oauth/token", origin),
            "registration_endpoint": format!("{}/oauth/register", origin),
            "revocation_endpoint": format!("{}/oauth/revoke", origin),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
            "revocation_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
            "scopes_supported": [READ, PROPOSE, OFFLINE],
            "authorization_response_iss_parameter_supported": true,
        }))
        .with("Access-Control-Allow-Origin", "*")
    }

    /// Dynamic client registration (RFC 7591): anyone may register, with a return address on the list.
    fn register(&self, r: &Req) -> Reply {
        let ct = (r.header)("Content-Type").unwrap_or_default().to_ascii_lowercase();
        if !ct.contains("application/json") {
            return Reply::error(400, "invalid_client_metadata", "send the registration as application/json");
        }
        let Ok(body) = serde_json::from_str::<Json>(r.body) else {
            return Reply::error(400, "invalid_client_metadata", "the body is not JSON");
        };
        let uris: Vec<String> = match body.get("redirect_uris").and_then(|v| v.as_array()) {
            Some(a) if !a.is_empty() && a.len() <= MAX_REDIRECTS => a.iter().map(|u| u.as_str().unwrap_or("").to_string()).collect(),
            Some(a) if a.len() > MAX_REDIRECTS => return Reply::error(400, "invalid_redirect_uri", "at most 10 redirect_uris"),
            _ => return Reply::error(400, "invalid_redirect_uri", "redirect_uris is required"),
        };
        if let Some(bad) = uris.iter().find(|u| !redirect_allowed(u, &self.cfg.extra_redirects)) {
            return Reply::error(400, "invalid_redirect_uri", &format!("{} is not an allowed return address: Lorefile accepts ChatGPT's, Claude's and this machine's loopback addresses (start the server with --oauth-redirect to add another)", bad));
        }
        let auth = body.get("token_endpoint_auth_method").and_then(|v| v.as_str()).unwrap_or("none").to_string();
        if !matches!(auth.as_str(), "none" | "client_secret_post" | "client_secret_basic") {
            return Reply::error(400, "invalid_client_metadata", "token_endpoint_auth_method is none, client_secret_post or client_secret_basic");
        }
        if let Some(g) = body.get("grant_types").and_then(|v| v.as_array()) {
            let words: Vec<&str> = g.iter().filter_map(|x| x.as_str()).collect();
            if !words.contains(&"authorization_code") || words.iter().any(|w| !matches!(*w, "authorization_code" | "refresh_token")) {
                return Reply::error(400, "invalid_client_metadata", "grant_types are authorization_code and refresh_token");
            }
        }
        if let Some(rt) = body.get("response_types").and_then(|v| v.as_array()) {
            if rt.iter().any(|x| x.as_str() != Some("code")) {
                return Reply::error(400, "invalid_client_metadata", "response_types is code");
            }
        }
        let name = clean_name(body.get("client_name").and_then(|v| v.as_str()).unwrap_or(""));
        let now = self.clock.now();
        let (Some(id), Some(secret)) = (random(16), random(32)) else { return Reply::error(500, "server_error", "no random bytes from the operating system") };
        let id = format!("lfc_{}", id);
        let secret = (auth != "none").then(|| format!("lfs_{}", secret));
        let mut st = self.lock();
        st.fresh.retain(|c| now - c.registered < CLIENT_UNUSED);
        if st.fresh.len() >= MAX_CLIENTS {
            // only a client past the 10-minute sign-in window makes room; one still signing in is never pushed out
            match st.fresh.iter().position(|c| now - c.registered >= REQUEST_LIFE) {
                Some(i) => {
                    let gone = st.fresh.remove(i);
                    st.pending.retain(|_, p| p.client != gone.id);
                }
                None => return Reply::error(429, "temporarily_unavailable", "too many apps are signing in to this server right now; try again in a few minutes"),
            }
        }
        // memory only: written to oauth.json with the client's first connection
        st.fresh.push(Client { id: id.clone(), name: name.clone(), redirect_uris: uris.clone(), auth: auth.clone(), secret: secret.as_deref().map(hash), registered: now });
        let mut out = json!({
            "client_id": id, "client_id_issued_at": now, "client_name": name, "redirect_uris": uris,
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"],
            "token_endpoint_auth_method": auth, "scope": format!("{} {}", READ, PROPOSE),
        });
        if let Some(s) = secret {
            out["client_secret"] = json!(s);
            out["client_secret_expires_at"] = json!(0);
        }
        Reply::json(201, &out)
    }

    /// `GET /oauth/authorize`: the client and its return address are checked before
    /// anything else, and a bad one never redirects. Then the rest, then the page.
    fn authorize_page(&self, r: &Req) -> Reply {
        let q = |k: &str| r.query.get(k).map(|s| s.as_str());
        let now = self.clock.now();
        let mut st = self.lock();
        let Some(client) = q("client_id").and_then(|id| st.client(id)) else {
            return error_page(400, "This sign-in link names an app this server does not know (its client_id is unknown or was removed). Nothing was sent back. Add the connector again in ChatGPT or Claude.");
        };
        let redirect_given = q("redirect_uri").is_some();
        let redirect = match q("redirect_uri") {
            Some(u) => u.to_string(),
            None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
            None => return error_page(400, "This sign-in link has no return address (redirect_uri). Nothing was sent back."),
        };
        if !client.redirect_uris.contains(&redirect) || !redirect_allowed(&redirect, &self.cfg.extra_redirects) {
            return error_page(400, "This sign-in link asks to send you back to an address the app did not register, or one this server does not allow. Nothing was sent back.");
        }
        let iss = r.origin;
        let state = q("state").map(|s| s.to_string());
        let fail = |code: &str, why: &str| {
            let mut p = vec![("error", code), ("error_description", why)];
            if let Some(s) = &state {
                p.push(("state", s.as_str()));
            }
            p.push(("iss", iss));
            Reply::redirect(back_to(&redirect, &p))
        };
        if q("response_type") != Some("code") {
            return fail("unsupported_response_type", "response_type must be code");
        }
        if let Err(why) = challenge_ok(q("code_challenge"), q("code_challenge_method")) {
            return fail("invalid_request", why);
        }
        if state.as_deref().map(|s| s.len() > MAX_STATE).unwrap_or(false) {
            return fail("invalid_request", "state is too long");
        }
        let resource = match q("resource").filter(|s| !s.trim().is_empty()) {
            None => Resource::All,
            Some(s) => match parse_resource(r.origin, s) {
                Some(res) => res,
                None => return fail("invalid_request", "resource is not an MCP address of this server"),
            },
        };
        let Some(id) = random(32) else { return error_page(500, "The operating system gave no random bytes. Try again.") };
        st.pending.retain(|_, p| now - p.created < REQUEST_LIFE);
        // one client keeps at most 20 waiting, its own oldest dropped first: a flood under one
        // client_id never pushes out another client's request
        let mine: Vec<(u64, String)> = st.pending.iter().filter(|(_, p)| p.client == client.id).map(|(k, p)| (p.seq, k.clone())).collect();
        if mine.len() >= MAX_PENDING_PER_CLIENT {
            if let Some((_, oldest)) = mine.into_iter().min() {
                st.pending.remove(&oldest);
            }
        }
        if st.pending.len() >= MAX_PENDING {
            return error_page(429, "Too many sign-ins are waiting on this server right now. Wait a few minutes, then press Connect again.");
        }
        st.seq += 1;
        let seq = st.seq;
        let space = match &resource {
            Resource::Space(s) => Some(s.clone()),
            Resource::All => None,
        };
        let pending = Pending {
            seq,
            client: client.id.clone(),
            redirect,
            redirect_given,
            challenge: q("code_challenge").unwrap_or("").to_string(),
            resource: resource.url(r.origin),
            space,
            scopes: requested_scopes(q("scope")),
            state,
            origin: r.origin.to_string(),
            created: now,
        };
        let page = self.approval_page(&pending, &id, &client.name, None, now);
        st.pending.insert(id, pending);
        page
    }

    fn approval_page(&self, p: &Pending, id: &str, client: &str, error: Option<&str>, now: i64) -> Reply {
        let what = match &p.space {
            None => "read every space on this server".to_string(),
            Some(s) => format!("read only the space <b>{}</b>", html_escape(s)),
        };
        let propose = if p.scopes.iter().any(|s| s == PROPOSE) {
            if self.cfg.edit_allowed {
                // the person approving decides how far they trust it; waiting is the default
                "<li style=\"flex-direction:column;gap:4px\"><label><input type=\"checkbox\" name=\"propose\" value=\"on\" checked> suggest changes</label>\
<div style=\"display:flex;flex-direction:column;gap:4px;padding-left:24px\">\
<label><input type=\"radio\" name=\"writes\" value=\"wait\" checked> wait for me to accept them</label>\
<label><input type=\"radio\" name=\"writes\" value=\"direct\"> go in straight away, marked unverified until a person looks</label></div></li>".to_string()
            } else {
                "<li><label><input type=\"checkbox\" name=\"propose\" value=\"on\" checked> suggest changes (they wait for a person to accept)</label></li>".to_string()
            }
        } else {
            String::new()
        };
        let err = error.map(|e| format!("<div class=\"err\">{}</div>", html_escape(e))).unwrap_or_default();
        let html = fill(
            APPROVE_HTML,
            &[
                ("style", crate::serve::PAGE_STYLE),
                ("logo", LOGO),
                ("error", &err),
                ("client", &html_escape(client)),
                ("what", &what),
                ("propose", &propose),
                ("host", &html_escape(&redirect_host(&p.redirect))),
                ("age", &(now - p.created).max(0).to_string()),
                ("request", &html_escape(id)),
            ],
        );
        Reply::html(if error.is_some() { 401 } else { 200 }, html)
    }

    /// `POST /oauth/authorize`: Allow or Deny, same-origin, with the one-time request ID.
    fn authorize_decision(&self, r: &Req) -> Reply {
        if let Some(o) = (r.header)("Origin") {
            if !o.trim().eq_ignore_ascii_case(r.origin) {
                return error_page(403, "This approval came from another site, so it was refused. Open the sign-in window from ChatGPT or Claude again.");
            }
        }
        let form = query_params(&format!("?{}", r.body)).1;
        let f = |k: &str| form.get(k).map(|s| s.as_str()).unwrap_or("");
        let now = self.clock.now();
        let mut st = self.lock();
        st.pending.retain(|_, p| now - p.created < REQUEST_LIFE);
        let id = f("request").to_string();
        let ok = st.pending.get(&id).map(|p| p.origin == r.origin).unwrap_or(false);
        if id.is_empty() || !ok {
            return error_page(403, "This approval request is unknown or expired (requests last 10 minutes). Press Connect again in ChatGPT or Claude.");
        }
        let back = |p: &Pending, params: &mut Vec<(&str, String)>| {
            if let Some(s) = &p.state {
                params.push(("state", s.clone()));
            }
            params.push(("iss", r.origin.to_string()));
            let refs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
            back_to(&p.redirect, &refs)
        };
        if f("decision") != "allow" {
            let p = st.pending.remove(&id).expect("checked above");
            return Reply::redirect(back(&p, &mut vec![("error", "access_denied".into()), ("error_description", "the person denied the request".into())]));
        }
        let client_name = st.pending.get(&id).and_then(|p| st.client(&p.client)).map(|c| c.name).unwrap_or_default();
        let given = f("token").trim().to_string();
        let by_token = !given.is_empty() && same_token(&given, &self.cfg.login);
        // the login token (typed, or in a bearer header) or a signed-in browser always gets through
        let approved_with = if by_token { Some("login token") } else { r.approver };
        if approved_with.is_none() && st.paused_until > now {
            // the failed-try pause: wrong tokens are refused without being counted. The 32-character
            // minimum is what stops guessing; this only slows the noise down and says so in the log.
            let mins = (st.paused_until - now + 59) / 60;
            let msg = format!("Too many wrong tokens: approvals with a typed token are paused for {} more minute{}. A browser already signed in to this server can still approve.", mins, if mins == 1 { "" } else { "s" });
            return self.approval_page(&st.pending[&id], &id, &client_name, Some(&msg), now).with_status(429);
        }
        let Some(approved_with) = approved_with else {
            let msg = if given.is_empty() {
                "Paste the login token to allow this (or deny it)."
            } else {
                st.tries.retain(|t| now - t < TRY_WINDOW);
                st.tries.push(now);
                if st.tries.len() >= TRIES {
                    st.tries.clear();
                    st.paused_until = now + PAUSE;
                    eprintln!("OAuth   {} wrong login tokens on the approval page in 10 minutes: approvals are paused for 10 minutes", TRIES);
                }
                crate::serve::WRONG_TOKEN
            };
            return self.approval_page(&st.pending[&id], &id, &client_name, Some(msg), now);
        };
        let p = st.pending.remove(&id).expect("checked above");
        let mut scopes = vec![READ.to_string()];
        if p.scopes.iter().any(|s| s == PROPOSE) && f("propose") == "on" {
            scopes.push(PROPOSE.to_string());
        }
        // straight away only when it may suggest at all, and the server takes writes
        let writes = if scopes.iter().any(|s| s == PROPOSE) && self.cfg.edit_allowed && f("writes") == DIRECT { DIRECT } else { WAIT };
        let Some(code) = random(32) else { return error_page(500, "The operating system gave no random bytes. Try again.") };
        st.codes.retain(|_, c| now - c.created < CODE_MEMORY);
        if st.codes.len() >= MAX_CODES {
            if let Some(oldest) = st.codes.iter().min_by_key(|(_, c)| c.created).map(|(k, _)| k.clone()) {
                st.codes.remove(&oldest);
            }
        }
        st.codes.insert(hash(&code), Code { client: p.client.clone(), redirect: p.redirect.clone(), redirect_given: p.redirect_given, challenge: p.challenge.clone(), resource: p.resource.clone(), scopes, origin: p.origin.clone(), created: now, approved_with: approved_with.into(), used_by: None, writes: writes.to_string() });
        let mut reply = Reply::redirect(back(&p, &mut vec![("code", code)]));
        reply.sign_in = by_token;
        drop(st);
        self.approved_on(r.origin);
        reply
    }

    /// Client authentication at the token and revoke endpoints: a public client names
    /// itself; a confidential one also proves its secret (form field or Basic header).
    fn client_of(&self, st: &State, form: &HashMap<String, String>, r: &Req) -> Result<Option<Client>, Reply> {
        let basic = (r.header)("Authorization").and_then(|v| {
            let (scheme, rest) = v.trim().split_once(' ')?;
            if !scheme.eq_ignore_ascii_case("basic") {
                return None;
            }
            let raw = String::from_utf8(b64_decode(rest)?).ok()?;
            let (id, secret) = raw.split_once(':')?;
            Some((crate::serve::percent_decode(id), crate::serve::percent_decode(secret)))
        });
        let id = basic.as_ref().map(|(i, _)| i.clone()).or_else(|| form.get("client_id").cloned());
        let Some(id) = id.filter(|i| !i.is_empty()) else { return Ok(None) };
        let unknown = || Reply::error(401, "invalid_client", "unknown client; add the connector again");
        let client = st.client(&id).ok_or_else(unknown)?;
        if let Some(want) = &client.secret {
            let given = basic.as_ref().map(|(_, s)| s.clone()).or_else(|| form.get("client_secret").cloned()).unwrap_or_default();
            if given.is_empty() || !same_token(&hash(&given), want) {
                let mut rep = Reply::error(401, "invalid_client", "the client secret is missing or wrong");
                if basic.is_some() {
                    rep = rep.with("WWW-Authenticate", "Basic realm=\"lorefile\"");
                }
                return Err(rep);
            }
        }
        Ok(Some(client))
    }

    fn form_of(r: &Req) -> Result<HashMap<String, String>, Reply> {
        let ct = (r.header)("Content-Type").unwrap_or_default().to_ascii_lowercase();
        if !ct.contains("application/x-www-form-urlencoded") {
            return Err(Reply::error(415, "invalid_request", "send the request form-encoded (application/x-www-form-urlencoded)"));
        }
        Ok(query_params(&format!("?{}", r.body)).1)
    }

    fn tokens_reply(access: &str, refresh: &str, scopes: &[String]) -> Reply {
        Reply::json(200, &json!({ "access_token": access, "token_type": "Bearer", "expires_in": ACCESS_LIFE, "refresh_token": refresh, "scope": scope_list(scopes) })).with("Pragma", "no-cache")
    }

    /// `POST /oauth/token`: a code for tokens, or a refresh token for new ones.
    fn token(&self, r: &Req) -> Reply {
        let form = match Self::form_of(r) {
            Ok(f) => f,
            Err(rep) => return rep,
        };
        let f = |k: &str| form.get(k).map(|s| s.as_str()).unwrap_or("");
        let now = self.clock.now();
        let mut st = self.lock();
        let client = match self.client_of(&st, &form, r) {
            Ok(Some(c)) => c,
            Ok(None) => return Reply::error(401, "invalid_client", "client_id is required"),
            Err(rep) => return rep,
        };
        let grant = || Reply::error(400, "invalid_grant", "the code or refresh token is invalid, expired, already used, or was issued to another client or address");
        match f("grant_type") {
            "authorization_code" => {
                let h = hash(f("code"));
                let Some(code) = st.codes.get(&h) else { return grant() };
                if let Some(conn) = code.used_by.clone() {
                    // a second use revokes what the first issued, but only when it is the same client
                    // with the right verifier: anyone else replaying a code just gets invalid_grant
                    if code.client == client.id && pkce_ok(f("code_verifier"), &code.challenge) {
                        st.store.connections.retain(|c| c.id != conn);
                        let _ = self.save(&st);
                    }
                    return grant();
                }
                let resource_ok = match form.get("resource").map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    None => true,
                    Some(s) => parse_resource(r.origin, s).map(|res| res.url(r.origin) == code.resource).unwrap_or(false),
                };
                // redirect_uri must match when authorize named it; when it was left out there, it may be left out here
                let redirect_ok = match form.get("redirect_uri") {
                    Some(u) => *u == code.redirect,
                    None => !code.redirect_given,
                };
                let fresh = now - code.created < CODE_LIFE;
                if !fresh || code.client != client.id || !redirect_ok || code.origin != r.origin || !resource_ok || !pkce_ok(f("code_verifier"), &code.challenge) {
                    return grant();
                }
                let (Some(cid), Some(a), Some(rt)) = (random(6), random(32), random(32)) else { return Reply::error(500, "server_error", "no random bytes") };
                let (access, refresh) = (format!("lfa_{}", a), format!("lfr_{}", rt));
                let conn = Connection {
                    id: format!("c{}", cid),
                    client: client.id.clone(),
                    name: client.name.clone(),
                    resource: code.resource.clone(),
                    scopes: code.scopes.clone(),
                    approved: code.created,
                    approved_with: code.approved_with.clone(),
                    refresh: Refresh { hash: hash(&refresh), expires: now + REFRESH_IDLE, retired: vec![] },
                    access: vec![Access { hash: hash(&access), expires: now + ACCESS_LIFE }],
                    last_used: None,
                    writes: code.writes.clone(),
                };
                if let Some(c) = st.codes.get_mut(&h) {
                    c.used_by = Some(conn.id.clone());
                }
                // the client's first connection: now it is written to oauth.json
                if let Some(i) = st.fresh.iter().position(|c| c.id == client.id) {
                    let c = st.fresh.remove(i);
                    st.store.clients.push(c);
                }
                let scopes = conn.scopes.clone();
                st.store.connections.push(conn);
                if let Err(e) = self.save(&st) {
                    return Reply::error(500, "server_error", &e);
                }
                Self::tokens_reply(&access, &refresh, &scopes)
            }
            "refresh_token" => {
                let h = hash(f("refresh_token"));
                let Some(i) = st.store.connections.iter().position(|c| c.refresh.hash == h || c.refresh.retired.iter().any(|x| x.hash == h)) else { return grant() };
                let c = &st.store.connections[i];
                let resource_ok = match form.get("resource").map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    None => true,
                    Some(s) => parse_resource(r.origin, s).map(|res| res.url(r.origin) == c.resource).unwrap_or(false),
                };
                if c.client != client.id || issuer_of(&c.resource) != r.origin || !resource_ok {
                    return grant();
                }
                let current = c.refresh.hash == h;
                let dead = if current { c.refresh.expires <= now } else { c.refresh.retired.iter().find(|x| x.hash == h).map(|x| now - x.replaced > GRACE).unwrap_or(true) };
                if dead {
                    // idle too long, or an old refresh token used again: the connection goes
                    st.store.connections.remove(i);
                    let _ = self.save(&st);
                    return grant();
                }
                let (Some(a), Some(rt)) = (random(32), random(32)) else { return Reply::error(500, "server_error", "no random bytes") };
                let (access, refresh) = (format!("lfa_{}", a), format!("lfr_{}", rt));
                let c = &mut st.store.connections[i];
                let old = std::mem::replace(&mut c.refresh.hash, hash(&refresh));
                c.refresh.retired.push(Retired { hash: old, replaced: now });
                if c.refresh.retired.len() > MAX_RETIRED {
                    let extra = c.refresh.retired.len() - MAX_RETIRED;
                    c.refresh.retired.drain(..extra);
                }
                c.refresh.expires = now + REFRESH_IDLE;
                c.access.retain(|x| x.expires > now);
                c.access.push(Access { hash: hash(&access), expires: now + ACCESS_LIFE });
                if c.access.len() > MAX_ACCESS {
                    let extra = c.access.len() - MAX_ACCESS;
                    c.access.drain(..extra);
                }
                c.last_used = Some(now);
                let scopes = c.scopes.clone();
                if let Err(e) = self.save(&st) {
                    return Reply::error(500, "server_error", &e);
                }
                Self::tokens_reply(&access, &refresh, &scopes)
            }
            "" => Reply::error(400, "invalid_request", "grant_type is required"),
            _ => Reply::error(400, "unsupported_grant_type", "grant_type is authorization_code or refresh_token"),
        }
    }

    /// `POST /oauth/revoke` (RFC 7009): a client gives a token back. A refresh token ends
    /// the connection, an access token just itself. Always 200, known token or not.
    fn revoke(&self, r: &Req) -> Reply {
        let form = match Self::form_of(r) {
            Ok(f) => f,
            Err(rep) => return rep,
        };
        let mut st = self.lock();
        let client = match self.client_of(&st, &form, r) {
            Ok(c) => c,
            Err(rep) => return rep,
        };
        let h = hash(form.get("token").map(|s| s.as_str()).unwrap_or(""));
        let mine = |c: &Connection| client.as_ref().map(|cl| cl.id == c.client).unwrap_or(true) && issuer_of(&c.resource) == r.origin;
        let before = serde_json::to_string(&st.store.connections).unwrap_or_default();
        st.store.connections.retain(|c| !(c.refresh.hash == h && mine(c)));
        for c in st.store.connections.iter_mut() {
            if mine(c) {
                c.access.retain(|a| a.hash != h);
            }
        }
        if serde_json::to_string(&st.store.connections).unwrap_or_default() != before {
            let _ = self.save(&st);
        }
        Reply::new(200, "application/json; charset=utf-8", String::new())
    }
}

impl Reply {
    fn with_status(mut self, status: u16) -> Reply {
        self.status = status;
        self
    }
}

fn error_page(status: u16, message: &str) -> Reply {
    let html = fill(ERROR_HTML, &[("style", crate::serve::PAGE_STYLE), ("logo", LOGO), ("message", &html_escape(message))]);
    Reply::html(status, html)
}

const LOGO: &str = r##"<div class="brand"><div class="logo"><svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><circle cx="5" cy="6" r="2"/><circle cx="19" cy="6" r="2"/><circle cx="12" cy="20" r="2"/><path d="M7 7l3 3M17 7l-3 3M12 15v3"/></svg></div>Lorefile</div>"##;

const ERROR_HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light">
<title>Lorefile · sign-in problem</title>
<style>{{style}}</style></head>
<body><main>
{{logo}}
<div class="err">{{message}}</div>
<p>You can close this window.</p>
</main></body></html>
"##;

/// The approval page: Lorefile's login screen with the question on top.
const APPROVE_HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light">
<title>Lorefile · connect {{client}}</title>
<style>{{style}}
.ask{font-size:16px;font-weight:700;line-height:1.35}
ul{margin:4px 0 0;padding:0;list-style:none;display:flex;flex-direction:column;gap:6px}
li{font-size:13.5px;display:flex;gap:8px;align-items:flex-start}li label{font-weight:400;display:flex;gap:8px;align-items:flex-start;cursor:pointer}
li input{flex:none;margin:3px 0 0;width:auto}
.tick{color:#15803d;font-weight:700;width:13px;flex:none;text-align:center}
.warn{background:#fff7e6;color:#8a5a00;border-radius:10px;padding:8px 12px;font-size:13px}
.actions{display:flex;flex-direction:row-reverse;gap:8px}
button.ghost{background:var(--panel);color:var(--ink);border-color:var(--line2)}
.ok{font-size:13px;color:#15803d}
</style></head>
<body><main>
{{logo}}
{{error}}
<div class="ask">Connect “{{client}}” to Lorefile?</div>
<form method="post" action="/oauth/authorize" style="display:flex;flex-direction:column;gap:12px">
<input type="hidden" name="request" value="{{request}}">
<div><b>It asks to:</b>
<ul>
<li><span class="tick">✓</span><span>{{what}}</span></li>
{{propose}}
</ul></div>
<p>After you allow it, you go back to <b>{{host}}</b>.</p>
<div class="warn"><span id="age" data-age="{{age}}">Asked just now.</span> Only allow it if you pressed Connect yourself just now.</div>
<div id="tokenbox"><label for="token">Login token</label>
<div class="row"><input id="token" name="token" type="password" autocomplete="current-password" spellcheck="false" autofocus></div>
<p style="margin-top:6px">The token the server printed in the terminal; also in <code>.lorefile/auth.toml</code>, or in <code>LOREFILE_TOKEN</code> on a server. Not needed when this browser is already signed in.</p></div>
<div id="signedin" class="ok" hidden>This browser is signed in to Lorefile, so no token is needed.</div>
<div class="actions"><button type="submit" name="decision" value="allow">Allow</button><button type="submit" name="decision" value="deny" class="ghost">Deny</button></div>
</form>
<p>The app's name is its own claim. You can disconnect it at any time in Lorefile's Settings › Connections.</p>
</main>
<script>
(function () {
  var el = document.getElementById('age'), base = +el.getAttribute('data-age') || 0, t0 = Date.now();
  function show() { var s = base + Math.round((Date.now() - t0) / 1000); el.textContent = s < 5 ? 'Asked just now.' : 'Asked ' + (s < 90 ? s + ' seconds' : Math.round(s / 60) + ' minutes') + ' ago.'; }
  show(); setInterval(show, 5000);
})();
// the popup arrives from another site without the SameSite=Strict cookie; ask from this page, where it is sent
(async function () { try {
  var r = await fetch('/api/auth', { credentials: 'same-origin' }); var j = await r.json();
  if (j.signed_in) { document.getElementById('tokenbox').hidden = true; document.getElementById('signedin').hidden = false; }
} catch (e) {} })();
</script>
</body></html>
"##;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN: &str = "a-login-token-that-is-long-enough-0123456789";
    const ORIGIN: &str = "https://quiet-otter.example";
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const LOOPBACK: &str = "http://127.0.0.1:33418/callback";

    fn headers(pairs: &[(&'static str, &str)]) -> impl Fn(&'static str) -> Option<String> {
        let owned: Vec<(&'static str, String)> = pairs.iter().map(|(k, v)| (*k, v.to_string())).collect();
        move |name: &'static str| owned.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }

    #[test]
    fn pkce_takes_the_rfc_7636_vector_and_refuses_plain_and_a_missing_challenge() {
        assert!(pkce_ok(VERIFIER, CHALLENGE));
        assert!(!pkce_ok("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXK", CHALLENGE), "one character off");
        assert!(!pkce_ok(VERIFIER, ""), "no challenge");
        assert!(!pkce_ok("short", CHALLENGE));
        assert!(challenge_ok(Some(CHALLENGE), Some("S256")).is_ok());
        assert!(challenge_ok(Some(VERIFIER), Some("plain")).is_err(), "plain is refused");
        assert!(challenge_ok(Some(CHALLENGE), None).is_err(), "the method must be named");
        assert!(challenge_ok(None, Some("S256")).is_err(), "a missing challenge is refused");
        assert!(challenge_ok(Some("has space and is long enough to pass the length check ok"), Some("S256")).is_err());
    }

    #[test]
    fn return_addresses_pass_only_in_their_exact_documented_forms() {
        let none: Vec<String> = vec![];
        for ok in [
            "https://chatgpt.com/connector_platform_oauth_redirect",
            "https://chatgpt.com/connector/oauth/abc_DEF-123",
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.com/api/mcp/auth_callback",
            "http://localhost:33418/callback",
            "http://127.0.0.1:1/callback",
            "http://[::1]:65535/callback",
            "http://localhost/callback",
        ] {
            assert!(redirect_allowed(ok, &none), "{} should pass", ok);
        }
        for bad in [
            "https://chatgpt.com.evil.example/connector_platform_oauth_redirect",
            "https://chatgpt.com@evil.example/connector_platform_oauth_redirect",
            "http://chatgpt.com/connector_platform_oauth_redirect",
            "https://chatgpt.com/connector_platform_oauth_redirect#fragment",
            "https://evil.example/?u=https://chatgpt.com",
            "http://192.168.1.5:3000/callback",
            "https://chatgpt.com/connector/oauth/",
            "https://chatgpt.com/connector/oauth/a/b",
            "https://claude.ai/api/mcp/auth_callback?x=1",
            "http://127.0.0.1@evil.example/callback",
            "http://127.0.0.1:99999/callback",
            "http://localhost.evil.example/callback",
            "https://localhost:3000/callback",
            "",
        ] {
            assert!(!redirect_allowed(bad, &none), "{} should be refused", bad);
        }
        let extra = vec!["https://agents.acme.example/oauth/cb".to_string()];
        assert!(redirect_allowed("https://agents.acme.example/oauth/cb", &extra));
        assert!(!redirect_allowed("https://agents.acme.example/oauth/cb2", &extra), "exact matching");
        assert!(extra_redirect("https://agents.acme.example/cb").is_ok());
        assert!(extra_redirect("http://evil.example/cb").is_err() && extra_redirect("https://a.example/cb#x").is_err() && extra_redirect("https://u@a.example/cb").is_err());
    }

    #[test]
    fn resources_are_mapped_or_refused() {
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/mcp"), Some(Resource::All));
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/mcp/"), Some(Resource::All));
        assert_eq!(parse_resource(ORIGIN, "https://Quiet-Otter.example/mcp"), Some(Resource::All), "the host ignores case");
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/s/brand-two/mcp"), Some(Resource::Space("brand-two".into())));
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/s/%40ecommerce/mcp"), Some(Resource::Space("@ecommerce".into())));
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example"), Some(Resource::All), "a bare origin means /mcp");
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/"), Some(Resource::All));
        assert_eq!(parse_resource(ORIGIN, "https://other.example/mcp"), None, "another host");
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example.evil/mcp"), None);
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/s/../mcp"), None);
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/s/%2e%2e/mcp"), None);
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/s/a/b/mcp"), None);
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/api"), None);
        assert_eq!(parse_resource(ORIGIN, "https://quiet-otter.example/mcp?x=1"), None);
        assert_eq!(Resource::Space("@ecommerce".into()).url(ORIGIN), "https://quiet-otter.example/s/%40ecommerce/mcp");
        assert_eq!(issuer_of("https://quiet-otter.example/s/a/mcp"), ORIGIN);
    }

    #[test]
    fn the_address_comes_from_the_request_by_the_stated_order() {
        let a = |h: &[(&'static str, &str)]| asked(&headers(h));
        assert_eq!(a(&[("Host", "127.0.0.1:7070")]), Asked { name: Some("127.0.0.1:7070".into()), https: false });
        assert_eq!(a(&[("Host", "127.0.0.1"), ("X-Forwarded-Host", "q.example"), ("X-Forwarded-Proto", "https")]).name.as_deref(), Some("q.example"), "X-Forwarded-Host beats Host");
        let f = a(&[("Host", "h.example"), ("X-Forwarded-Host", "x.example"), ("X-Forwarded-Proto", "http"), ("Forwarded", "for=1.2.3.4;host=f.example;proto=https")]);
        assert_eq!(f, Asked { name: Some("f.example".into()), https: true }, "Forwarded beats both");
        assert!(a(&[("Host", "c.example"), ("CF-Visitor", "{\"scheme\":\"https\"}")]).https, "CF-Visitor counts");
        assert_eq!(a(&[("X-Forwarded-Host", "one.example, two.example"), ("X-Forwarded-Proto", "https, http")]), Asked { name: Some("one.example".into()), https: true }, "first values only");
        assert_eq!(a(&[("Forwarded", "host=a.example;proto=https, host=b.example;proto=http")]).name.as_deref(), Some("a.example"));
        assert_eq!(origin_of("q.example:443", true), "https://q.example", ":443 dropped");
        assert_eq!(origin_of("q.example:8443", true), "https://q.example:8443", "other ports kept");
        for bad in ["a.example/path", "a b.example", "user@a.example", "a.example:x", "a.example:0", "/", "a..example", ""] {
            assert_eq!(host_name(bad), None, "{:?} is not a host name", bad);
        }
        assert_eq!(a(&[("Host", "h.example"), ("X-Forwarded-Host", "evil.example/x")]).name.as_deref(), Some("h.example"), "an invalid value is ignored");
        assert_eq!(host_name("[2001:db8::1]:443").as_deref(), Some("[2001:db8::1]:443"));
        assert_eq!(host_name("Quiet-Otter.Example").as_deref(), Some("quiet-otter.example"));
        assert!(loopback_name("127.0.0.1:7070") && loopback_name("localhost") && loopback_name("[::1]:1") && loopback_name("app.localhost"));
        assert!(!loopback_name("q.example"));
        assert_eq!(pin("https://acme.fly.dev/").unwrap(), "https://acme.fly.dev");
        assert_eq!(pin("http://127.0.0.1:7123").unwrap(), "http://127.0.0.1:7123");
        assert!(pin("http://acme.fly.dev").is_err() && pin("https://acme.fly.dev/lorefile").is_err() && pin("https://acme.fly.dev?x=1").is_err() && pin("acme.fly.dev").is_err());
    }

    fn server(dir: &Path, login: &str, clock: Clock) -> Server {
        Server::open(dir, Config { login: login.into(), pinned: None, extra_redirects: vec![], edit_allowed: false, link_note: Some("test".into()) }, clock)
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lorefile-oauth-unit-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    struct Call<'a> {
        srv: &'a Server,
    }

    impl Call<'_> {
        fn run(&self, method: &str, path: &str, query: &[(&str, &str)], hdrs: &[(&'static str, &str)], body: &str, signed_in: bool) -> Reply {
            let q: HashMap<String, String> = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let get = headers(hdrs);
            self.srv.handle(&Req { method, path, query: &q, header: &get, body, origin: ORIGIN, approver: if signed_in { Some("signed-in browser") } else { None } })
        }

        fn register(&self) -> String {
            let r = self.run("POST", "/oauth/register", &[], &[("Content-Type", "application/json")], &json!({ "client_name": "ChatGPT", "redirect_uris": [LOOPBACK] }).to_string(), false);
            assert_eq!(r.status, 201, "{}", r.body);
            serde_json::from_str::<Json>(&r.body).unwrap()["client_id"].as_str().unwrap().to_string()
        }

        /// GET the page, POST Allow with the token: the code.
        fn approve(&self, client: &str, resource: &str) -> String {
            let page = self.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", client), ("redirect_uri", LOOPBACK), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256"), ("state", "xyz"), ("resource", resource)], &[], "", false);
            assert_eq!(page.status, 200, "{}", page.body);
            let id = page.body.split("name=\"request\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
            let r = self.run("POST", "/oauth/authorize", &[], &[("Origin", ORIGIN)], &format!("request={}&decision=allow&propose=on&token={}", id, LOGIN), false);
            assert_eq!(r.status, 302, "{}", r.body);
            let loc = r.headers.iter().find(|(k, _)| k == "Location").unwrap().1.clone();
            assert!(loc.contains("state=xyz") && loc.contains("iss=https%3A%2F%2Fquiet-otter.example"), "{}", loc);
            loc.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string()
        }

        fn exchange(&self, client: &str, code: &str) -> Reply {
            self.run("POST", "/oauth/token", &[], &[("Content-Type", "application/x-www-form-urlencoded")], &format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}", code, enc(LOOPBACK), client, VERIFIER), false)
        }

        fn refresh(&self, client: &str, rt: &str) -> Reply {
            self.run("POST", "/oauth/token", &[], &[("Content-Type", "application/x-www-form-urlencoded")], &format!("grant_type=refresh_token&refresh_token={}&client_id={}", rt, client), false)
        }
    }

    fn tokens(r: &Reply) -> (String, String) {
        let j: Json = serde_json::from_str(&r.body).unwrap();
        (j["access_token"].as_str().unwrap().to_string(), j["refresh_token"].as_str().unwrap().to_string())
    }

    #[test]
    fn the_store_keeps_hashes_only_saves_atomically_prunes_and_forgets_another_login() {
        let dir = scratch("store");
        let (clock, t) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock.clone());
        let call = Call { srv: &srv };
        let client = call.register();
        let code = call.approve(&client, &format!("{}/mcp", ORIGIN));
        let (access, refresh) = tokens(&call.exchange(&client, &code));
        assert!(access.starts_with("lfa_") && refresh.starts_with("lfr_"));
        let file = dir.join(".lorefile/oauth.json");
        let text = std::fs::read_to_string(&file).unwrap();
        for secret in [&access, &refresh, &code, &LOGIN.to_string()] {
            assert!(!text.contains(secret.as_str()), "{} is in the file", secret);
        }
        assert!(text.contains(&hash(&access)) && text.contains("\"approved_with\": \"login token\""), "{}", text);
        assert!(!dir.join(".lorefile/oauth.json.tmp").exists(), "the temporary file was renamed into place");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(srv.check(&access, ORIGIN, &Resource::All).is_some());
        drop(srv);

        // the same login: tokens survive a restart
        let srv = server(&dir, LOGIN, clock.clone());
        assert_eq!(srv.dropped, 0);
        assert!(srv.check(&access, ORIGIN, &Resource::All).is_some(), "survives a restart");
        // a client with no connection lives in memory only, and goes after a day
        let call = Call { srv: &srv };
        let before = std::fs::read_to_string(&file).unwrap();
        let idle = call.register();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "a registration writes nothing to disk");
        assert!(srv.lock().fresh.iter().any(|c| c.id == idle));
        t.fetch_add(CLIENT_UNUSED + 1, Ordering::SeqCst);
        call.register();
        assert!(!srv.lock().fresh.iter().any(|c| c.id == idle), "an unused client is gone");
        assert!(srv.lock().store.clients.iter().any(|c| c.id == client), "a connected one was saved and stays");
        drop(srv);

        // another login token: every grant goes
        let srv = server(&dir, "another-login-token-that-is-long-enough-xyz", clock.clone());
        assert_eq!(srv.dropped, 1);
        assert_eq!(srv.connection_count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_login_fingerprint_is_keyed_by_a_secret_in_its_own_file() {
        // RFC 4231, test case 2
        assert_eq!(hex(&hmac(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        let dir = scratch("key");
        let (clock, _) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock.clone());
        let call = Call { srv: &srv };
        let client = call.register();
        let code = call.approve(&client, ORIGIN);
        assert_eq!(call.exchange(&client, &code).status, 200);
        drop(srv);
        let key_file = dir.join(".lorefile/oauth.key");
        let key = hex_decode(&std::fs::read_to_string(&key_file).unwrap()).unwrap();
        assert_eq!(key.len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&key_file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let store: Json = serde_json::from_str(&std::fs::read_to_string(dir.join(".lorefile/oauth.json")).unwrap()).unwrap();
        let stored = store["login"].as_str().unwrap();
        assert_eq!(stored, fingerprint(LOGIN, &key), "an HMAC under the key");
        assert_ne!(stored, fingerprint(LOGIN, &[0u8; 32]), "without the key the fingerprint cannot be reproduced");
        assert!(!stored.contains(&hex(&Sha256::digest(LOGIN.as_bytes()))), "not a plain hash of the token");
        // the same token and key: connections stay
        let srv = server(&dir, LOGIN, clock.clone());
        assert_eq!((srv.dropped, srv.connection_count()), (0, 1));
        drop(srv);
        // the key file gone: the fingerprint cannot be trusted, so connections go (the safe side)
        std::fs::remove_file(&key_file).unwrap();
        let srv = server(&dir, LOGIN, clock);
        assert_eq!((srv.dropped, srv.connection_count()), (1, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn templates_are_filled_in_one_pass() {
        assert_eq!(fill("a {{x}} b {{y}} c", &[("x", "{{y}}"), ("y", "Y")]), "a {{y}} b Y c", "a value is never searched again");
        assert_eq!(fill("{{unknown}} {{", &[]), "{{unknown}} {{");
        let dir = scratch("tpl");
        let (clock, _) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock);
        let call = Call { srv: &srv };
        let r = call.run("POST", "/oauth/register", &[], &[("Content-Type", "application/json")], &json!({ "client_name": "{{host}} {{request}} {{propose}}", "redirect_uris": [LOOPBACK] }).to_string(), false);
        let client = serde_json::from_str::<Json>(&r.body).unwrap()["client_id"].as_str().unwrap().to_string();
        let page = call.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", &client), ("redirect_uri", LOOPBACK), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256")], &[], "", false);
        assert!(page.body.contains("Connect “{{host}} {{request}} {{propose}}” to Lorefile?"), "the name is shown as given: {}", page.body);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_replayed_code_revokes_only_for_its_own_client_with_the_right_verifier() {
        let dir = scratch("replay");
        let (clock, _) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock);
        let call = Call { srv: &srv };
        let client = call.register();
        let code = call.approve(&client, ORIGIN);
        let (access, _) = tokens(&call.exchange(&client, &code));
        // someone else replays it: another client, or the right client with a wrong verifier
        let other = call.register();
        assert_eq!(call.exchange(&other, &code).status, 400);
        let wrong = call.run("POST", "/oauth/token", &[], &[("Content-Type", "application/x-www-form-urlencoded")], &format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}", code, enc(LOOPBACK), client, "x".repeat(43)), false);
        assert_eq!(wrong.status, 400);
        assert!(srv.check(&access, ORIGIN, &Resource::All).is_some(), "the connection survives both");
        // the same client with the right verifier: the code leaked from its own exchange, so what it issued goes
        assert_eq!(call.exchange(&client, &code).status, 400);
        assert!(srv.check(&access, ORIGIN, &Resource::All).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn floods_never_push_out_a_client_or_a_request_that_is_signing_in() {
        let dir = scratch("flood");
        let (clock, t) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock);
        let call = Call { srv: &srv };
        let victim = call.register();
        let page = |client: &str| call.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", client), ("redirect_uri", LOOPBACK), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256")], &[], "", false);
        let request_id = |r: &Reply| r.body.split("name=\"request\" value=\"").nth(1).map(|s| s.split('"').next().unwrap().to_string()).unwrap_or_default();
        let waiting = request_id(&page(&victim));

        // pending requests: a flood under another client_id, and its own oldest go first
        let attacker = call.register();
        let first = request_id(&page(&attacker));
        for _ in 0..MAX_PENDING_PER_CLIENT {
            page(&attacker);
        }
        assert!(!srv.lock().pending.contains_key(&first), "the flooding client's own oldest request went");
        assert_eq!(srv.lock().pending.values().filter(|p| p.client == attacker).count(), MAX_PENDING_PER_CLIENT);
        let ok = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow&token={}", waiting, LOGIN), false);
        assert_eq!(ok.status, 302, "the other client's request survived the flood: {}", ok.body);

        // clients: 200 in ten minutes fill the room, and none of them is pushed out
        let victim = call.register();
        let waiting = request_id(&page(&victim));
        while srv.lock().fresh.len() < MAX_CLIENTS {
            call.register();
        }
        let refused = call.run("POST", "/oauth/register", &[], &[("Content-Type", "application/json")], &json!({ "client_name": "x", "redirect_uris": [LOOPBACK] }).to_string(), false);
        assert_eq!(refused.status, 429, "refused at the cap, nobody evicted: {}", refused.body);
        let ok = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow&token={}", waiting, LOGIN), false);
        assert_eq!(ok.status, 302, "the client signing in was kept");
        // past the 10-minute sign-in window, the oldest unconnected client makes room
        t.fetch_add(REQUEST_LIFE, Ordering::SeqCst);
        let oldest = srv.lock().fresh[0].id.clone();
        call.register();
        assert!(srv.lock().client(&oldest).is_none(), "the oldest went");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn redirect_uri_may_be_left_out_at_the_token_endpoint_when_authorize_left_it_out() {
        let dir = scratch("redirect");
        let (clock, _) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock);
        let call = Call { srv: &srv };
        let client = call.register();
        let form = |code: &str, redirect: Option<&str>| {
            let ru = redirect.map(|u| format!("&redirect_uri={}", enc(u))).unwrap_or_default();
            call.run("POST", "/oauth/token", &[], &[("Content-Type", "application/x-www-form-urlencoded")], &format!("grant_type=authorization_code&code={}&client_id={}&code_verifier={}{}", code, client, VERIFIER, ru), false)
        };
        let approve_without = || {
            let page = call.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", &client), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256")], &[], "", false);
            let id = page.body.split("name=\"request\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
            let r = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow", id), true);
            let loc = r.headers.iter().find(|(k, _)| k == "Location").unwrap().1.clone();
            loc.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string()
        };
        assert_eq!(form(&approve_without(), None).status, 200, "left out at both");
        assert_eq!(form(&approve_without(), Some(LOOPBACK)).status, 200, "named only here, and it matches");
        assert_eq!(form(&approve_without(), Some("http://127.0.0.1:1/other")).status, 400, "named here, and wrong");
        // named at authorize: it must come back
        let code = call.approve(&client, ORIGIN);
        assert_eq!(form(&code, None).status, 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lifetimes_follow_a_fake_clock() {
        let dir = scratch("life");
        let (clock, t) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock);
        let call = Call { srv: &srv };
        let client = call.register();

        // a code is good for 60 seconds
        let code = call.approve(&client, ORIGIN);
        t.fetch_add(CODE_LIFE, Ordering::SeqCst);
        assert_eq!(call.exchange(&client, &code).status, 400, "a 60-second-old code is refused");

        // access lasts an hour
        let code = call.approve(&client, ORIGIN);
        let (access, refresh) = tokens(&call.exchange(&client, &code));
        t.fetch_add(ACCESS_LIFE - 1, Ordering::SeqCst);
        assert!(srv.check(&access, ORIGIN, &Resource::All).is_some());
        t.fetch_add(1, Ordering::SeqCst);
        assert!(srv.check(&access, ORIGIN, &Resource::All).is_none(), "an hour later the access token is dead");

        // refresh: replaced on use, the old one works for 60 seconds, then revokes the connection
        let r1 = call.refresh(&client, &refresh);
        assert_eq!(r1.status, 200, "{}", r1.body);
        let (a2, _) = tokens(&r1);
        t.fetch_add(GRACE, Ordering::SeqCst);
        let retry = call.refresh(&client, &refresh);
        assert_eq!(retry.status, 200, "within the grace window an old refresh token still works: {}", retry.body);
        let (_, rt3) = tokens(&retry);
        assert!(srv.check(&a2, ORIGIN, &Resource::All).is_some(), "earlier access tokens live out their hour");
        t.fetch_add(1, Ordering::SeqCst);
        assert_eq!(call.refresh(&client, &refresh).status, 400, "61 seconds after it was replaced, the first refresh token is refused");
        assert_eq!(srv.connection_count(), 0, "and the connection is revoked");
        assert_eq!(call.refresh(&client, &rt3).status, 400, "so even the newest refresh token is dead");
        assert!(srv.check(&a2, ORIGIN, &Resource::All).is_none());

        // 30 idle days end a connection
        let code = call.approve(&client, ORIGIN);
        let (_, refresh) = tokens(&call.exchange(&client, &code));
        t.fetch_add(REFRESH_IDLE, Ordering::SeqCst);
        assert_eq!(call.refresh(&client, &refresh).status, 400, "30 days without use");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tokens_open_only_their_own_address_and_scopes_follow_the_tick_box() {
        let dir = scratch("aud");
        let (clock, _) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock);
        let call = Call { srv: &srv };
        let client = call.register();
        let code = call.approve(&client, &format!("{}/s/brand-two/mcp", ORIGIN));
        let (access, _) = tokens(&call.exchange(&client, &code));
        assert!(srv.check(&access, ORIGIN, &Resource::Space("brand-two".into())).is_some());
        assert!(srv.check(&access, ORIGIN, &Resource::All).is_none(), "not every space");
        assert!(srv.check(&access, ORIGIN, &Resource::Space("other".into())).is_none(), "not another space");
        assert!(srv.check(&access, "https://b.example", &Resource::Space("brand-two".into())).is_none(), "not through another address");
        let g = srv.check(&access, ORIGIN, &Resource::Space("brand-two".into())).unwrap();
        assert!(g.can_propose() && g.label().starts_with("agent via MCP (connection \"ChatGPT\", approved "), "{}", g.label());

        // unticking "suggest changes" leaves read only
        let page = call.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", &client), ("redirect_uri", LOOPBACK), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256")], &[], "", false);
        let id = page.body.split("name=\"request\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        let r = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow", id), true);
        assert_eq!(r.status, 302, "a signed-in browser approves without the token");
        let loc = &r.headers.iter().find(|(k, _)| k == "Location").unwrap().1;
        let code = loc.split("code=").nth(1).unwrap().split('&').next().unwrap();
        let ok = call.exchange(&client, code);
        assert!(ok.body.contains("\"scope\":\"lorefile:read\""), "{}", ok.body);
        let (access, _) = tokens(&ok);
        assert!(!srv.check(&access, ORIGIN, &Resource::All).unwrap().can_propose());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_tokens_pause_typed_tokens_but_not_the_login_itself() {
        let dir = scratch("tries");
        let (clock, t) = Clock::fake(1_800_000_000);
        let srv = server(&dir, LOGIN, clock);
        let call = Call { srv: &srv };
        let client = call.register();
        let page = call.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", &client), ("redirect_uri", LOOPBACK), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256")], &[], "", false);
        let id = page.body.split("name=\"request\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        for _ in 0..TRIES {
            let r = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow&token=wrong", id), false);
            assert_eq!(r.status, 401);
        }
        let r = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow&token=still-wrong", id), false);
        assert_eq!(r.status, 429, "paused for wrong tokens: {}", r.body);
        assert!(r.body.contains("paused") && r.body.contains("signed in to this server can still approve"));
        assert_eq!(srv.lock().tries.len(), 0, "wrong tokens during the pause are not counted");
        // a signed-in browser, and the login token itself, still get through
        let r = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow", id), true);
        assert_eq!(r.status, 302, "a signed-in browser approves during the pause");
        let page = call.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", &client), ("redirect_uri", LOOPBACK), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256")], &[], "", false);
        let id = page.body.split("name=\"request\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        let q: HashMap<String, String> = HashMap::new();
        let get = headers(&[]);
        let r = srv.handle(&Req { method: "POST", path: "/oauth/authorize", query: &q, header: &get, body: &format!("request={}&decision=allow", id), origin: ORIGIN, approver: Some("login token") });
        assert_eq!(r.status, 302, "the bearer header approves during the pause");
        let code = r.headers.iter().find(|(k, _)| k == "Location").unwrap().1.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
        assert_eq!(call.exchange(&client, &code).status, 200);
        assert!(srv.lock().store.connections.iter().any(|c| c.approved_with == "login token"), "recorded as the login token, not a browser");
        t.fetch_add(PAUSE, Ordering::SeqCst);
        let page = call.run("GET", "/oauth/authorize", &[("response_type", "code"), ("client_id", &client), ("redirect_uri", LOOPBACK), ("code_challenge", CHALLENGE), ("code_challenge_method", "S256")], &[], "", false);
        let id = page.body.split("name=\"request\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        let r = call.run("POST", "/oauth/authorize", &[], &[], &format!("request={}&decision=allow&token={}", id, LOGIN), false);
        assert_eq!(r.status, 302, "ten minutes later it works again");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
