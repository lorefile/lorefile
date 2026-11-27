//! OAuth for connectors, end to end through the real binary, with no network: requests
//! carry the headers a tunnel or proxy adds (`Host: quiet-otter.example`,
//! `X-Forwarded-Proto: https`), so the server works out a public HTTPS address from each
//! one, exactly as it would behind ngrok or Cloudflare. Plain `http://127.0.0.1`
//! requests must see no change at all. (specs/2026-11-27-oauth-connectors.md, "Tests")

use serde_json::{json, Value as Json};
use std::fs;
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TOKEN: &str = "oauth-test-login-token-0123456789abcdefghijk";
/// RFC 7636, Appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const LOOPBACK: &str = "http://127.0.0.1:33418/callback";
const OTTER: &str = "quiet-otter.example";

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if e.path().is_dir() { copy_dir(&e.path(), &dst.join(&name)); } else { fs::copy(e.path(), dst.join(&name)).unwrap(); }
    }
}

/// A scratch copy of the supply-chain example, removed when dropped.
struct Folder(PathBuf);
impl Folder {
    fn new(port: u16) -> Folder {
        let dir = std::env::temp_dir().join(format!("lorefile-oauth-http-{}-{}", port, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/supply-chain"), &dir);
        Folder(dir)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A running server; its stderr lines are collected for the start-up and `Seen` lines.
struct Server {
    child: Child,
    port: u16,
    lines: Arc<Mutex<Vec<String>>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn text(&self) -> String {
        // stderr arrives on its own thread: give a line printed just now a moment to land
        std::thread::sleep(Duration::from_millis(150));
        self.lines.lock().unwrap().join("\n")
    }
}

/// `serve` on `dir` with `--token TOKEN` unless `env` sets LOREFILE_TOKEN or `extra` says --no-auth.
fn start(dir: &Path, port: u16, extra: &[&str], env: &[(&str, &str)]) -> Server {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lorefile"));
    cmd.args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string()]).env_remove("LOREFILE_TOKEN").env_remove("LOREFILE_PUBLIC_URL");
    if env.is_empty() && !extra.contains(&"--no-auth") {
        cmd.args(["--token", TOKEN]);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.args(extra).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let (err, sink) = (child.stderr.take().unwrap(), Arc::clone(&lines));
    std::thread::spawn(move || {
        for l in std::io::BufReader::new(err).lines().map_while(Result::ok) {
            sink.lock().unwrap().push(l);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(100));
    }
    Server { child, port, lines }
}

struct Resp {
    status: u16,
    head: String,
    body: String,
}
impl Resp {
    /// A header's value, the name matched without case.
    fn header(&self, name: &str) -> Option<String> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    }
    fn json(&self) -> Json {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({}): {} {}", e, self.status, self.body))
    }
}

/// One raw HTTP/1.1 request. `Host` defaults to 127.0.0.1:<port> unless a header sets it.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Resp {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut req = format!("{} {} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n", method, path, body.len());
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("Host")) {
        req.push_str(&format!("Host: 127.0.0.1:{}\r\n", port));
    }
    for (k, v) in headers {
        req.push_str(&format!("{}: {}\r\n", k, v));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Resp { status, head: head.to_string(), body: body.to_string() }
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect()
}

/// What a tunnel in front adds: the public name, and the HTTPS mark.
fn tunnel(name: &str) -> Vec<(&'static str, String)> {
    vec![("Host", name.to_string()), ("X-Forwarded-Proto", "https".into())]
}

fn with<'a>(base: &'a [(&'static str, String)], more: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v: Vec<(&str, &str)> = base.iter().map(|(k, v)| (*k, v.as_str())).collect();
    v.extend_from_slice(more);
    v
}

fn get(port: u16, name: &str, path: &str) -> Resp {
    http(port, "GET", path, &with(&tunnel(name), &[]), "")
}

fn register(port: u16, name: &str, client_name: &str) -> String {
    let body = json!({ "client_name": client_name, "redirect_uris": [LOOPBACK], "grant_types": ["authorization_code", "refresh_token"], "token_endpoint_auth_method": "none" }).to_string();
    let r = http(port, "POST", "/oauth/register", &with(&tunnel(name), &[("Content-Type", "application/json")]), &body);
    assert_eq!(r.status, 201, "{}", r.body);
    r.json()["client_id"].as_str().unwrap().to_string()
}

/// GET the approval page; returns it and the one-time request ID in its form.
fn authorize(port: u16, name: &str, client: &str, resource: &str, scope: &str) -> (Resp, String) {
    let q = format!(
        "/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state=st-123&scope={}&resource={}",
        client, enc(LOOPBACK), CHALLENGE, enc(scope), enc(resource)
    );
    let r = get(port, name, &q);
    let id = r.body.split("name=\"request\" value=\"").nth(1).map(|s| s.split('"').next().unwrap().to_string()).unwrap_or_default();
    (r, id)
}

fn decide(port: u16, name: &str, id: &str, form: &str) -> Resp {
    let origin = format!("https://{}", name);
    http(port, "POST", "/oauth/authorize", &with(&tunnel(name), &[("Content-Type", "application/x-www-form-urlencoded"), ("Origin", &origin)]), &format!("request={}&{}", id, form))
}

fn param(location: &str, key: &str) -> Option<String> {
    let q = location.split_once('?')?.1;
    q.split('&').find_map(|p| p.strip_prefix(&format!("{}=", key)).map(|v| v.to_string()))
}

/// Register, approve with the token, return (client, code).
fn approve(port: u16, name: &str, resource: &str, propose: bool) -> (String, String) {
    let client = register(port, name, "Test client");
    let (page, id) = authorize(port, name, &client, resource, "lorefile:read lorefile:propose");
    assert_eq!(page.status, 200, "{}", page.body);
    let r = decide(port, name, &id, &format!("decision=allow&token={}{}", enc(TOKEN), if propose { "&propose=on" } else { "" }));
    assert_eq!(r.status, 302, "{}", r.body);
    let loc = r.header("Location").unwrap();
    (client, param(&loc, "code").unwrap())
}

fn form_post(port: u16, name: &str, path: &str, form: &str) -> Resp {
    http(port, "POST", path, &with(&tunnel(name), &[("Content-Type", "application/x-www-form-urlencoded")]), form)
}

fn exchange(port: u16, name: &str, client: &str, code: &str, resource: &str) -> Resp {
    form_post(port, name, "/oauth/token", &format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}&resource={}", code, enc(LOOPBACK), client, VERIFIER, enc(resource)))
}

fn refresh(port: u16, name: &str, client: &str, rt: &str) -> Resp {
    form_post(port, name, "/oauth/token", &format!("grant_type=refresh_token&refresh_token={}&client_id={}", rt, client))
}

/// The whole dance, for tests that need a working access token.
fn connect(port: u16, name: &str, resource: &str, propose: bool) -> (String, String, String) {
    let (client, code) = approve(port, name, resource, propose);
    let t = exchange(port, name, &client, &code, resource);
    assert_eq!(t.status, 200, "{}", t.body);
    let j = t.json();
    (client, j["access_token"].as_str().unwrap().to_string(), j["refresh_token"].as_str().unwrap().to_string())
}

fn mcp(port: u16, name: &str, path: &str, bearer: &str, msg: &Json) -> Resp {
    let auth = format!("Bearer {}", bearer);
    http(port, "POST", path, &with(&tunnel(name), &[("Authorization", &auth), ("Content-Type", "application/json"), ("Accept", "application/json, text/event-stream")]), &msg.to_string())
}

fn tools_list() -> Json {
    json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
}

fn tool_names(r: &Resp) -> Vec<String> {
    r.json()["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

/// Rewrite part of `oauth.json` while the server is stopped (to make time pass).
fn edit_store(dir: &Path, f: impl Fn(&mut Json)) {
    let p = dir.join(".lorefile/oauth.json");
    let mut j: Json = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
    f(&mut j);
    fs::write(&p, serde_json::to_string_pretty(&j).unwrap()).unwrap();
}

const LONG_AGO: &str = "2000-01-01T00:00:00Z";

// ---------------------------------------------------------------------------

#[test]
fn plain_http_and_no_oauth_see_no_change() {
    let folder = Folder::new(7140);
    let srv = start(&folder.0, 7140, &["--no-examples"], &[]);
    let login = format!("Bearer {}", TOKEN);
    // plain http://127.0.0.1: the OAuth addresses are unknown routes, exactly as before
    for path in ["/.well-known/oauth-authorization-server", "/.well-known/oauth-protected-resource/mcp"] {
        let r = http(srv.port, "GET", path, &[], "");
        assert_eq!(r.status, 401, "behind the login gate as before: {}", r.body);
        assert_eq!(r.header("WWW-Authenticate").as_deref(), Some("Bearer"));
        assert_eq!(http(srv.port, "GET", path, &[("Authorization", &login)], "").status, 404, "signed in: not found, as before");
    }
    assert_eq!(http(srv.port, "POST", "/oauth/register", &[("Authorization", &login), ("Content-Type", "application/json")], "{}").status, 404);
    let r = http(srv.port, "POST", "/mcp", &[("Content-Type", "application/json")], "{}");
    assert_eq!(r.status, 401);
    assert_eq!(r.header("WWW-Authenticate").as_deref(), Some("Bearer"), "the same bare challenge as before:\n{}", r.head);
    assert!(srv.text().contains("OAuth   on for any HTTPS address that reaches this server"), "{}", srv.text());
    assert!(!srv.text().contains("Seen"), "no address seen yet: {}", srv.text());
    drop(srv);

    // --no-oauth: the same on every address, HTTPS included
    let srv = start(&folder.0, 7141, &["--no-examples", "--no-oauth"], &[]);
    let r = get(srv.port, OTTER, "/.well-known/oauth-authorization-server");
    assert_eq!(r.status, 401, "{}", r.body);
    let r = http(srv.port, "POST", "/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), "{}");
    assert_eq!(r.header("WWW-Authenticate").as_deref(), Some("Bearer"));
    assert!(srv.text().contains("OAuth   off (--no-oauth)"), "{}", srv.text());
}

#[test]
fn https_marked_requests_get_metadata_for_their_own_address() {
    let folder = Folder::new(7142);
    let srv = start(&folder.0, 7142, &["--no-examples"], &[]);
    let o = format!("https://{}", OTTER);
    let pr = get(srv.port, OTTER, "/.well-known/oauth-protected-resource/mcp");
    assert_eq!(pr.status, 200, "{}", pr.body);
    let j = pr.json();
    assert_eq!(j["resource"], format!("{}/mcp", o));
    assert_eq!(j["authorization_servers"], json!([o]));
    assert_eq!(j["scopes_supported"], json!(["lorefile:read", "lorefile:propose"]));
    assert_eq!(j["bearer_methods_supported"], json!(["header"]));
    assert_eq!(pr.header("Access-Control-Allow-Origin").as_deref(), Some("*"));
    assert_eq!(pr.header("Cache-Control").as_deref(), Some("no-store"));
    assert!(pr.header("Vary").unwrap_or_default().contains("X-Forwarded-Host"));
    assert_eq!(get(srv.port, OTTER, "/.well-known/oauth-protected-resource").json()["resource"], format!("{}/mcp", o));
    // a space that does not exist gets the same shape: the metadata reveals no names
    assert_eq!(get(srv.port, OTTER, "/.well-known/oauth-protected-resource/s/brand-two/mcp").json()["resource"], format!("{}/s/brand-two/mcp", o));
    let am = get(srv.port, OTTER, "/.well-known/oauth-authorization-server").json();
    assert_eq!(am["issuer"], o, "the issuer is the same everywhere");
    assert_eq!(am["authorization_endpoint"], format!("{}/oauth/authorize", o));
    assert_eq!(am["token_endpoint"], format!("{}/oauth/token", o));
    assert_eq!(am["registration_endpoint"], format!("{}/oauth/register", o));
    assert_eq!(am["revocation_endpoint"], format!("{}/oauth/revoke", o));
    assert_eq!(am["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(am["authorization_response_iss_parameter_supported"], json!(true));
    assert!(am.get("client_id_metadata_document_supported").is_none(), "no CIMD in phase 1");
    let r = http(srv.port, "POST", "/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), "{}");
    assert_eq!(r.status, 401);
    assert_eq!(
        r.header("WWW-Authenticate").as_deref(),
        Some(format!("Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\", scope=\"lorefile:read lorefile:propose\"", o).as_str())
    );
    let r = http(srv.port, "POST", "/s/brand-two/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), "{}");
    assert!(r.header("WWW-Authenticate").unwrap().contains("/.well-known/oauth-protected-resource/s/brand-two/mcp\""), "{}", r.head);
    // the one-time Seen line, with the connector URL; a --token token is not echoed
    let text = srv.text();
    assert_eq!(text.matches("Seen    https://quiet-otter.example").count(), 1, "{}", text);
    assert!(text.contains("connectors: https://quiet-otter.example/mcp (choose OAuth") && text.contains("sign in there: https://quiet-otter.example/   sign in with the token given to --token"), "{}", text);
    assert!(!text.contains(TOKEN), "{}", text);
    // the page is unchanged: /.well-known/openid-configuration is not served
    assert_eq!(get(srv.port, OTTER, "/.well-known/openid-configuration").status, 401);
}

#[test]
fn the_address_rule_one_case_each() {
    let folder = Folder::new(7143);
    let srv = start(&folder.0, 7143, &["--no-examples"], &[]);
    let issuer = |hs: &[(&str, &str)]| -> Json {
        let r = http(srv.port, "GET", "/.well-known/oauth-authorization-server", hs, "");
        if r.status == 200 { r.json()["issuer"].clone() } else { Json::Null }
    };
    assert_eq!(issuer(&[("Host", "127.0.0.1"), ("X-Forwarded-Host", "x.example"), ("X-Forwarded-Proto", "https")]), "https://x.example", "X-Forwarded-Host beats Host");
    assert_eq!(issuer(&[("Host", "h.example"), ("X-Forwarded-Host", "x.example"), ("X-Forwarded-Proto", "http"), ("Forwarded", "for=192.0.2.1;host=f.example;proto=https")]), "https://f.example", "Forwarded beats both");
    assert_eq!(issuer(&[("Host", "c.example"), ("CF-Visitor", "{\"scheme\":\"https\"}")]), "https://c.example", "CF-Visitor counts as HTTPS");
    assert_eq!(issuer(&[("Host", "h.example"), ("X-Forwarded-Host", "one.example, two.example"), ("X-Forwarded-Proto", "https, http")]), "https://one.example", "only the first of several values");
    assert_eq!(issuer(&[("Host", "p.example:443"), ("X-Forwarded-Proto", "https")]), "https://p.example", ":443 is dropped");
    assert_eq!(issuer(&[("Host", "p.example:8443"), ("X-Forwarded-Proto", "https")]), "https://p.example:8443", "other ports are kept");
    for bad in ["evil.example/path", "evil example", "user@evil.example"] {
        assert_eq!(issuer(&[("Host", "h.example"), ("X-Forwarded-Host", bad), ("X-Forwarded-Proto", "https")]), "https://h.example", "{:?} is ignored", bad);
    }
    assert_eq!(issuer(&[("Host", "h.example")]), Json::Null, "no HTTPS mark, no OAuth");
    // a proxy that hides the name: HTTPS-marked, loopback Host
    let hidden = [("Host", "127.0.0.1:7070"), ("X-Forwarded-Proto", "https")];
    assert_eq!(issuer(&hidden), Json::Null);
    assert_eq!(issuer(&hidden), Json::Null);
    let text = srv.text();
    assert_eq!(text.matches("a proxy is hiding the public address").count(), 1, "the hint is printed once: {}", text);
    assert!(text.contains("proxy_set_header Host $host; proxy_set_header X-Forwarded-Proto $scheme;"), "{}", text);
}

#[test]
fn two_addresses_at_once_and_a_pinned_one() {
    let folder = Folder::new(7144);
    let srv = start(&folder.0, 7144, &["--no-examples"], &[]);
    let (a, b) = ("a.example", "b.example");
    let (_, access, _) = connect(srv.port, a, "https://a.example/mcp", true);
    assert_eq!(mcp(srv.port, a, "/mcp", &access, &tools_list()).status, 200);
    let r = mcp(srv.port, b, "/mcp", &access, &tools_list());
    assert_eq!(r.status, 401, "a connection approved through a.example does not open b.example");
    assert!(r.header("WWW-Authenticate").unwrap().contains("https://b.example/.well-known/oauth-protected-resource/mcp"));
    // a code issued on a.example is refused at b.example's token endpoint
    let (client, code) = approve(srv.port, a, "https://a.example/mcp", true);
    let r = exchange(srv.port, b, &client, &code, "https://b.example/mcp");
    assert_eq!((r.status, r.json()["error"].as_str()), (400, Some("invalid_grant")), "{}", r.body);
    drop(srv);

    // pinned: sign-ins answer on that one address only
    let srv = start(&folder.0, 7145, &["--no-examples", "--public-url", "https://a.example/"], &[]);
    assert!(srv.text().contains("OAuth   on at https://a.example only (--public-url)"), "{}", srv.text());
    assert_eq!(get(srv.port, a, "/.well-known/oauth-authorization-server").json()["issuer"], "https://a.example");
    assert_eq!(get(srv.port, b, "/.well-known/oauth-authorization-server").status, 401, "another name gets today's login gate");
    let r = http(srv.port, "POST", "/mcp", &with(&tunnel(b), &[("Content-Type", "application/json")]), "{}");
    assert_eq!(r.header("WWW-Authenticate").as_deref(), Some("Bearer"), "and today's plain 401");
    let login = format!("Bearer {}", TOKEN);
    assert_eq!(http(srv.port, "GET", "/oauth/authorize", &with(&tunnel(b), &[("Authorization", &login)]), "").status, 404, "signed in: today's 404");
    drop(srv);
    // a bad --public-url stops the start with a reason
    let out = Command::new(env!("CARGO_BIN_EXE_lorefile")).args(["serve", "--dir", folder.0.to_str().unwrap(), "--port", "7146", "--token", TOKEN, "--public-url", "https://a.example/lorefile"]).output().unwrap();
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("no path"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn the_whole_flow_register_approve_token_tools_refresh_revoke() {
    let folder = Folder::new(7147);
    let port = 7147;
    let srv = start(&folder.0, port, &["--no-examples"], &[]);
    let resource = "https://quiet-otter.example/mcp";
    let client = register(port, OTTER, "Test client");
    assert!(client.starts_with("lfc_"));

    // the approval page: no redirect, cannot be framed, names the client and where you go back to
    let (page, id) = authorize(port, OTTER, &client, resource, "lorefile:read lorefile:propose");
    assert_eq!(page.status, 200, "{}", page.body);
    assert!(page.header("Location").is_none());
    assert_eq!(page.header("X-Frame-Options").as_deref(), Some("DENY"));
    assert!(page.header("Content-Security-Policy").unwrap().contains("frame-ancestors 'none'"));
    assert!(page.body.contains("Connect “Test client” to Lorefile?") && page.body.contains("read every space on this server") && page.body.contains("<b>127.0.0.1:33418</b>"), "{}", page.body);
    assert!(page.body.contains("suggest changes (they wait for a person to accept)") && page.body.contains("Only allow it if you pressed Connect yourself"));
    assert!(!id.is_empty());

    // Allow with the token: back to the client with code, state and iss, and this browser is signed in
    let r = decide(port, OTTER, &id, &format!("decision=allow&propose=on&token={}", enc(TOKEN)));
    assert_eq!(r.status, 302, "{}", r.body);
    let loc = r.header("Location").unwrap();
    assert!(loc.starts_with(LOOPBACK), "{}", loc);
    assert_eq!(param(&loc, "state").as_deref(), Some("st-123"));
    assert_eq!(param(&loc, "iss").as_deref(), Some("https%3A%2F%2Fquiet-otter.example"));
    let code = param(&loc, "code").unwrap();
    let cookie = r.header("Set-Cookie").unwrap();
    assert!(cookie.starts_with(&format!("lorefile-{}=", port)) && cookie.contains("Secure") && cookie.contains("SameSite=Strict"), "{}", cookie);

    // the code for tokens
    let t = exchange(port, OTTER, &client, &code, resource);
    assert_eq!(t.status, 200, "{}", t.body);
    assert_eq!(t.header("Cache-Control").as_deref(), Some("no-store"));
    let j = t.json();
    assert_eq!((j["token_type"].as_str(), j["expires_in"].as_i64(), j["scope"].as_str()), (Some("Bearer"), Some(3600), Some("lorefile:read lorefile:propose")));
    let (access, rt1) = (j["access_token"].as_str().unwrap().to_string(), j["refresh_token"].as_str().unwrap().to_string());
    assert!(access.starts_with("lfa_") && rt1.starts_with("lfr_"));

    // initialize and tools/list give the same bytes as with the login token
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}});
    for msg in [&init, &tools_list()] {
        let with_oauth = mcp(port, OTTER, "/mcp", &access, msg);
        let with_login = mcp(port, OTTER, "/mcp", TOKEN, msg);
        assert_eq!(with_oauth.status, 200, "{}", with_oauth.body);
        assert_eq!(with_oauth.body, with_login.body, "the same answer either way");
    }

    // refresh: a new pair, the old refresh token replaced
    let r = refresh(port, OTTER, &client, &rt1);
    assert_eq!(r.status, 200, "{}", r.body);
    let (access2, rt2) = (r.json()["access_token"].as_str().unwrap().to_string(), r.json()["refresh_token"].as_str().unwrap().to_string());
    assert_ne!(rt1, rt2);
    assert_eq!(mcp(port, OTTER, "/mcp", &access2, &tools_list()).status, 200);

    // oauth.json: mode 0600, hashes only
    let file = folder.0.join(".lorefile/oauth.json");
    let stored = fs::read_to_string(&file).unwrap();
    for secret in [&access, &access2, &rt1, &rt2, &code, &TOKEN.to_string()] {
        assert!(!stored.contains(secret.as_str()), "{} must not be in oauth.json", secret);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // nothing secret in the log either
    let text = srv.text();
    for secret in [&access, &access2, &rt1, &rt2, &code] {
        assert!(!text.contains(secret.as_str()), "{} must not be logged", secret);
    }

    // Settings › Connections lists it (with the login), and the page's write gate guards Disconnect
    let login = format!("Bearer {}", TOKEN);
    let list = http(port, "GET", "/api/oauth/connections", &[("Authorization", &login)], "").json();
    assert_eq!(list["on"], true);
    let conn = &list["connections"][0];
    assert_eq!((conn["name"].as_str(), conn["address"].as_str(), conn["approved_with"].as_str()), (Some("Test client"), Some("https://quiet-otter.example"), Some("login token")));
    assert!(conn["last_used"].is_string(), "{}", conn);
    assert_eq!(http(port, "GET", "/api/oauth/connections", &[], "").status, 401, "the list needs the login");
    drop(srv);

    // after the 60-second grace window, the first refresh token revokes the connection
    edit_store(&folder.0, |j| {
        for r in j["connections"][0]["refresh"]["retired"].as_array_mut().unwrap() {
            r["replaced"] = json!(LONG_AGO);
        }
    });
    let srv = start(&folder.0, port, &["--no-examples"], &[]);
    let r = refresh(srv.port, OTTER, &client, &rt1);
    assert_eq!((r.status, r.json()["error"].as_str()), (400, Some("invalid_grant")));
    assert_eq!(mcp(port, OTTER, "/mcp", &access2, &tools_list()).status, 401, "the connection is gone");
    assert_eq!(refresh(port, OTTER, &client, &rt2).status, 400, "even its newest refresh token");

    // connect again, then the client gives its refresh token back (RFC 7009)
    let (client, access, rt) = connect(port, OTTER, resource, true);
    let r = form_post(port, OTTER, "/oauth/revoke", &format!("token={}&token_type_hint=refresh_token&client_id={}", rt, client));
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 401, "revoked");
    assert_eq!(refresh(port, OTTER, &client, &rt).status, 400);
}

#[test]
fn refusals() {
    let folder = Folder::new(7148);
    let port = 7148;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let resource = "https://quiet-otter.example/mcp";

    // a wrong verifier
    let (client, code) = approve(port, OTTER, resource, true);
    let r = form_post(port, OTTER, "/oauth/token", &format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}", code, enc(LOOPBACK), client, "x".repeat(43)));
    assert_eq!((r.status, r.json()["error"].as_str()), (400, Some("invalid_grant")));

    // a code used twice: refused, and what the first use issued stops working
    let (client, code) = approve(port, OTTER, resource, true);
    let first = exchange(port, OTTER, &client, &code, resource);
    let access = first.json()["access_token"].as_str().unwrap().to_string();
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 200);
    let second = exchange(port, OTTER, &client, &code, resource);
    assert_eq!((second.status, second.json()["error"].as_str()), (400, Some("invalid_grant")));
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 401, "the first use's token is revoked");

    // a return address not on the list, at registration
    for bad in ["https://chatgpt.com.evil.example/connector_platform_oauth_redirect", "https://chatgpt.com@evil.example/x", "http://chatgpt.com/connector_platform_oauth_redirect", "http://192.168.1.5:3000/callback"] {
        let r = http(port, "POST", "/oauth/register", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), &json!({ "client_name": "x", "redirect_uris": [bad] }).to_string());
        assert_eq!((r.status, r.json()["error"].as_str()), (400, Some("invalid_redirect_uri")), "{}", bad);
    }
    // an unregistered return address at approval: an error page, never a redirect
    let client = register(port, OTTER, "Test client");
    let r = get(port, OTTER, &format!("/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state=s", client, enc("http://127.0.0.1:9/other"), CHALLENGE));
    assert_eq!(r.status, 400);
    assert!(r.header("Location").is_none() && r.body.contains("Nothing was sent back"), "{}", r.body);
    let r = get(port, OTTER, &format!("/oauth/authorize?response_type=code&client_id=lfc_nobody&redirect_uri={}&code_challenge={}&code_challenge_method=S256", enc(LOOPBACK), CHALLENGE));
    assert_eq!(r.status, 400);
    assert!(r.header("Location").is_none());
    // the rest goes back to the client as an error, with iss
    let r = get(port, OTTER, &format!("/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=plain&state=s", client, enc(LOOPBACK), VERIFIER));
    let loc = r.header("Location").unwrap();
    assert_eq!((param(&loc, "error").as_deref(), param(&loc, "iss").is_some()), (Some("invalid_request"), true), "{}", loc);
    let (_, id) = authorize(port, OTTER, &client, "https://elsewhere.example/mcp", "lorefile:read");
    assert!(id.is_empty(), "a resource that is not ours goes back as an error");

    // Deny: access_denied, with iss
    let (_, id) = authorize(port, OTTER, &client, resource, "lorefile:read");
    let r = decide(port, OTTER, &id, "decision=deny");
    let loc = r.header("Location").unwrap();
    assert_eq!((r.status, param(&loc, "error").as_deref(), param(&loc, "iss").as_deref(), param(&loc, "state").as_deref()), (302, Some("access_denied"), Some("https%3A%2F%2Fquiet-otter.example"), Some("st-123")));

    // Allow with no request ID, or from another site: 403
    assert_eq!(decide(port, OTTER, "", &format!("decision=allow&token={}", enc(TOKEN))).status, 403);
    let (_, id) = authorize(port, OTTER, &client, resource, "lorefile:read");
    let foreign = http(port, "POST", "/oauth/authorize", &with(&tunnel(OTTER), &[("Content-Type", "application/x-www-form-urlencoded"), ("Origin", "https://evil.example")]), &format!("request={}&decision=allow&token={}", id, enc(TOKEN)));
    assert_eq!(foreign.status, 403);
    assert!(foreign.header("Location").is_none());

    // a token form sent as JSON at /oauth/token
    let r = http(port, "POST", "/oauth/token", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), &json!({ "grant_type": "authorization_code" }).to_string());
    assert_eq!(r.status, 415);
    // a body over 16 KB is refused unread
    let big = http(port, "POST", "/oauth/register", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), &"x".repeat(17 * 1024));
    assert_eq!(big.status, 413);

    // a wrong token: the page again; ten in ten minutes pause the form for everyone
    let r = decide(port, OTTER, &id, "decision=allow&token=wrong-token");
    assert_eq!(r.status, 401);
    assert!(r.body.contains("That is not this server&#39;s token") && r.body.contains(&id), "the same request, with the error: {}", r.body);
    for _ in 0..9 {
        decide(port, OTTER, &id, "decision=allow&token=wrong-token");
    }
    let r = decide(port, OTTER, &id, &format!("decision=allow&token={}", enc(TOKEN)));
    assert_eq!(r.status, 429, "paused, even for the right token: {}", r.body);
    assert!(r.body.contains("paused for everyone"));
}

#[test]
fn a_token_opens_only_its_own_mcp_address_and_expires() {
    let folder = Folder::new(7149);
    let port = 7149;
    // with the shipped examples, so a one-space address exists: @ecommerce
    let srv = start(&folder.0, port, &[], &[]);
    let one = "https://quiet-otter.example/s/%40ecommerce/mcp";
    let (_, access, _) = connect(port, OTTER, one, true);
    let r = mcp(port, OTTER, "/s/%40ecommerce/mcp", &access, &tools_list());
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(!tool_names(&r).contains(&"spaces".to_string()), "one space");
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 401, "not every space");
    assert_eq!(mcp(port, OTTER, "/s/%40supply-chain/mcp", &access, &tools_list()).status, 401, "not another space");
    let page = authorize(port, OTTER, &register(port, OTTER, "x"), one, "lorefile:read").0;
    assert!(page.body.contains("read only the space <b>@ecommerce</b>"), "{}", page.body);
    drop(srv);

    // an hour later (made to pass in the file): 401, with where to sign in again
    edit_store(&folder.0, |j| {
        for a in j["connections"][0]["access"].as_array_mut().unwrap() {
            a["expires"] = json!(LONG_AGO);
        }
    });
    let _srv = start(&folder.0, port, &[], &[]);
    let r = mcp(port, OTTER, "/s/%40ecommerce/mcp", &access, &tools_list());
    assert_eq!(r.status, 401);
    let www = r.header("WWW-Authenticate").unwrap();
    assert!(www.contains("resource_metadata=\"https://quiet-otter.example/.well-known/oauth-protected-resource/s/%40ecommerce/mcp\"") && www.contains("error=\"invalid_token\""), "{}", www);
}

#[test]
fn scopes_decide_the_propose_tools_and_the_connection_is_named_in_proposals() {
    let folder = Folder::new(7150);
    let port = 7150;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let resource = "https://quiet-otter.example/mcp";
    let propose = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_term","term":"seller","means":"Supplier","reason":"Finance says seller."}}});

    // read only: no propose tools, and a call to one is a tool error
    let (_, read_only, _) = connect(port, OTTER, resource, false);
    let names = tool_names(&mcp(port, OTTER, "/mcp", &read_only, &tools_list()));
    assert!(!names.contains(&"propose_patch".to_string()) && !names.contains(&"propose_change".to_string()) && names.contains(&"summary".to_string()), "{:?}", names);
    let r = mcp(port, OTTER, "/mcp", &read_only, &propose).json();
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("this connection may only read; disconnect and connect again with 'suggest changes' ticked"), "{}", r);
    let init = mcp(port, OTTER, "/mcp", &read_only, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})).json();
    assert!(init["result"]["instructions"].as_str().unwrap().contains("This connection may only read"), "{}", init);

    // both scopes, no --allow-edit: queued for a person, as over stdio, in the connection's name
    let (_, both, _) = connect(port, OTTER, resource, true);
    let r = mcp(port, OTTER, "/mcp", &both, &propose).json();
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("queued for review"), "{}", text);
    let proposals: Vec<PathBuf> = fs::read_dir(folder.0.join(".lorefile/proposals")).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(proposals.len(), 1);
    let p: Json = serde_json::from_str(&fs::read_to_string(&proposals[0]).unwrap()).unwrap();
    let via = p["via"].as_str().unwrap();
    assert!(via.starts_with("agent via MCP (connection \"Test client\", approved ") && via.ends_with(')'), "{}", via);
    let activity = fs::read_to_string(folder.0.join(".lorefile/activity.jsonl")).unwrap();
    assert!(activity.contains("agent via MCP (connection \\\"Test client\\\", approved "), "{}", activity);
}

#[test]
fn restarts_keep_tokens_and_a_new_login_token_disconnects_everything() {
    let folder = Folder::new(7151);
    let port = 7151;
    let env = [("LOREFILE_TOKEN", TOKEN)];
    let srv = start(&folder.0, port, &["--no-examples"], &env);
    let resource = "https://quiet-otter.example/mcp";
    let (client, access, rt) = connect(port, OTTER, resource, true);
    assert!(srv.text().contains("sign in there: https://quiet-otter.example/   sign in with the token in LOREFILE_TOKEN"), "{}", srv.text());
    drop(srv);

    // the same token: access and refresh still work
    let srv = start(&folder.0, port, &["--no-examples"], &env);
    assert!(srv.text().contains("· 1 connection"), "{}", srv.text());
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 200);
    let r = refresh(port, OTTER, &client, &rt);
    assert_eq!(r.status, 200, "{}", r.body);
    let rt2 = r.json()["refresh_token"].as_str().unwrap().to_string();
    drop(srv);

    // another LOREFILE_TOKEN: everything approved under the old one stops working
    let other = "a-completely-different-login-token-9876543210";
    let srv = start(&folder.0, port, &["--no-examples"], &[("LOREFILE_TOKEN", other)]);
    assert!(srv.text().contains("the login token changed: 1 connection was disconnected"), "{}", srv.text());
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 401);
    let r = refresh(port, OTTER, &client, &rt2);
    assert_eq!((r.status, r.json()["error"].as_str()), (400, Some("invalid_grant")));
}

#[test]
fn claude_codes_header_still_works_with_oauth_on() {
    let folder = Folder::new(7152);
    let port = 7152;
    let _srv = start(&folder.0, port, &["--no-examples", "--allow-edit"], &[]);
    // the tests/mcp_http.rs conversation, through HTTPS-marked headers, with the login token
    let init = mcp(port, OTTER, "/mcp", TOKEN, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}));
    assert_eq!(init.status, 200, "{}", init.body);
    assert!(init.json()["result"]["instructions"].as_str().unwrap().contains("offer to save it with `propose_patch`"));
    assert!(tool_names(&mcp(port, OTTER, "/mcp", TOKEN, &tools_list())).contains(&"propose_patch".to_string()));
    let saved = mcp(port, OTTER, "/mcp", TOKEN, &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_term","term":"seller","means":"Supplier","reason":"Finance says seller."}}})).json();
    assert!(saved["result"]["content"][0]["text"].as_str().unwrap().contains("written to glossary.yaml"), "{}", saved);
    assert!(fs::read_to_string(folder.0.join("trust.yaml")).unwrap().contains("by: \"agent via MCP\""), "the owner's header keeps the plain author");
    // the page's cookie still does not open MCP, even through the tunnel
    let cookie = format!("lorefile-{}={}", port, TOKEN);
    let r = http(port, "POST", "/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json"), ("Cookie", &cookie)]), "{}");
    assert_eq!(r.status, 401);
    // and an OAuth token does not open the page's API
    let (_, access, _) = connect(port, OTTER, "https://quiet-otter.example/mcp", true);
    assert_eq!(http(port, "GET", "/api/summary", &with(&tunnel(OTTER), &[("Authorization", &format!("Bearer {}", access))]), "").status, 401);
}

#[test]
fn no_auth_and_a_short_token_leave_oauth_off() {
    let folder = Folder::new(7153);
    let srv = start(&folder.0, 7153, &["--no-examples", "--no-auth"], &[]);
    assert_eq!(get(srv.port, OTTER, "/.well-known/oauth-authorization-server").status, 404, "no OAuth routes, even over HTTPS");
    assert_eq!(get(srv.port, OTTER, "/.well-known/oauth-protected-resource/mcp").status, 404);
    let r = http(srv.port, "POST", "/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), &json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string());
    assert_eq!(r.status, 200, "connectors use No authentication, as today");
    assert!(srv.text().contains("OAuth   off (--no-auth)"), "{}", srv.text());
    let off = http(srv.port, "GET", "/api/oauth/connections", &[], "").json();
    assert_eq!((off["on"].as_bool(), off["reason"].as_str()), (Some(false), Some("no-auth")));
    drop(srv);

    // a 20-character LOREFILE_TOKEN: the server starts, OAuth stays off, and says why
    let srv = start(&folder.0, 7154, &["--no-examples"], &[("LOREFILE_TOKEN", "twenty-chars-0123456")]);
    let text = srv.text();
    assert!(text.contains("OAuth   off: the login token is shorter than 32 characters"), "{}", text);
    assert_eq!(get(srv.port, OTTER, "/.well-known/oauth-authorization-server").status, 401, "the login gate as before");
    let r = http(srv.port, "GET", "/api/oauth/connections", &[("Authorization", "Bearer twenty-chars-0123456")], "").json();
    assert_eq!(r["reason"], "short-token");
}
