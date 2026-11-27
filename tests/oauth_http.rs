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
    spawn(dir, port, extra, env, env.is_empty() && !extra.contains(&"--no-auth"))
}

/// `serve` with the token it makes and saves in `.lorefile/auth.toml`: the laptop default.
fn start_with_file_token(dir: &Path, port: u16, extra: &[&str]) -> (Server, String) {
    let srv = spawn(dir, port, extra, &[], false);
    let saved = fs::read_to_string(dir.join(".lorefile/auth.toml")).unwrap();
    let token = saved.lines().find(|l| l.starts_with("token")).unwrap().split('"').nth(1).unwrap().to_string();
    (srv, token)
}

fn spawn(dir: &Path, port: u16, extra: &[&str], env: &[(&str, &str)], flag_token: bool) -> Server {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lorefile"));
    cmd.args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string()]).env_remove("LOREFILE_TOKEN").env_remove("LOREFILE_PUBLIC_URL");
    if flag_token {
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
    approve_with(port, name, resource, propose, TOKEN)
}

fn approve_with(port: u16, name: &str, resource: &str, propose: bool, token: &str) -> (String, String) {
    let client = register(port, name, "Test client");
    let (page, id) = authorize(port, name, &client, resource, "lorefile:read lorefile:propose");
    assert_eq!(page.status, 200, "{}", page.body);
    let r = decide(port, name, &id, &format!("decision=allow&token={}{}", enc(token), if propose { "&propose=on" } else { "" }));
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
    // plain http://127.0.0.1: the OAuth addresses are not served here, so they are not found,
    // before the login and with no sign-in challenge (a 401 on a discovery address misleads clients)
    for path in ["/.well-known/oauth-authorization-server", "/.well-known/oauth-protected-resource/mcp", "/.well-known/openid-configuration"] {
        let r = http(srv.port, "GET", path, &[], "");
        assert_eq!(r.status, 404, "not served on plain http: {}", r.body);
        assert!(r.header("WWW-Authenticate").is_none(), "{}", r.head);
        assert_eq!(http(srv.port, "GET", path, &[("Authorization", &login)], "").status, 404, "signed in: not found either");
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
    assert_eq!(r.status, 404, "not served with OAuth off: {}", r.body);
    assert!(r.header("WWW-Authenticate").is_none(), "{}", r.head);
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
    assert_eq!(j["scopes_supported"], json!(["lorefile:read", "lorefile:propose", "offline_access"]), "offline_access is listed, as OpenAI asks");
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
    assert_eq!(am["scopes_supported"], json!(["lorefile:read", "lorefile:propose", "offline_access"]));
    assert!(am.get("client_id_metadata_document_supported").is_none(), "no CIMD in phase 1");
    let r = http(srv.port, "POST", "/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), "{}");
    assert_eq!(r.status, 401);
    assert_eq!(
        r.header("WWW-Authenticate").as_deref(),
        Some(format!("Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\", scope=\"lorefile:read lorefile:propose\"", o).as_str())
    );
    let r = http(srv.port, "POST", "/s/brand-two/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), "{}");
    assert!(r.header("WWW-Authenticate").unwrap().contains("/.well-known/oauth-protected-resource/s/brand-two/mcp\""), "{}", r.head);
    // an address read from requests is not printed until someone approves a connection on it
    let text = srv.text();
    assert!(!text.contains("Seen"), "{}", text);
    assert!(!text.contains(TOKEN), "{}", text);
    // Lorefile is not an identity provider: other discovery addresses are not found, before the
    // login and with no challenge (ChatGPT asks for openid-configuration three times)
    for path in ["/.well-known/openid-configuration", "/.well-known/anything", "/s/brand-two/.well-known/openid-configuration"] {
        let r = get(srv.port, OTTER, path);
        assert_eq!(r.status, 404, "{}: {}", path, r.body);
        assert!(r.header("WWW-Authenticate").is_none(), "{}: {}", path, r.head);
    }
}

#[test]
fn offline_access_is_listed_and_asking_for_it_changes_nothing() {
    let folder = Folder::new(7163);
    let port = 7163;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let resource = "https://quiet-otter.example/mcp";
    let client = register(port, OTTER, "ChatGPT");
    let (page, id) = authorize(port, OTTER, &client, resource, "lorefile:read lorefile:propose offline_access");
    assert_eq!(page.status, 200, "{}", page.body);
    let r = decide(port, OTTER, &id, &format!("decision=allow&propose=on&token={}", enc(TOKEN)));
    assert_eq!(r.status, 302, "{}", r.body);
    let code = param(&r.header("Location").unwrap(), "code").unwrap();
    let t = exchange(port, OTTER, &client, &code, resource);
    assert_eq!(t.status, 200, "{}", t.body);
    let j = t.json();
    assert_eq!(j["scope"], "lorefile:read lorefile:propose", "the token carries our two scopes only");
    assert!(j["refresh_token"].as_str().unwrap_or("").starts_with("lfr_"), "and a refresh token, as always: {}", j);
    // the challenge on /mcp still asks for our two scopes only
    let r = http(port, "POST", "/mcp", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), "{}");
    assert!(r.header("WWW-Authenticate").unwrap().ends_with("scope=\"lorefile:read lorefile:propose\""), "{}", r.head);
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
    assert_eq!(get(srv.port, b, "/.well-known/oauth-authorization-server").status, 404, "another name: not served there, so not found");
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
    let r = decide(port, OTTER, &id, "decision=allow&token=wrong-again");
    assert_eq!(r.status, 429, "paused for typed tokens: {}", r.body);
    assert!(r.body.contains("paused"));
    // a signed-in browser still approves during the pause
    let cookie = format!("lorefile-{}={}", port, TOKEN);
    let origin = format!("https://{}", OTTER);
    let r = http(port, "POST", "/oauth/authorize", &with(&tunnel(OTTER), &[("Content-Type", "application/x-www-form-urlencoded"), ("Origin", &origin), ("Cookie", &cookie)]), &format!("request={}&decision=allow", id));
    assert_eq!(r.status, 302, "the cookie gets through: {}", r.body);
    // and so does the login token itself
    let (_, id) = authorize(port, OTTER, &client, resource, "lorefile:read");
    assert_eq!(decide(port, OTTER, &id, &format!("decision=allow&token={}", enc(TOKEN))).status, 302);

    // a replayed code from another client changes nothing (the review's case)
    let (client, code) = approve(port, OTTER, resource, true);
    let access = exchange(port, OTTER, &client, &code, resource).json()["access_token"].as_str().unwrap().to_string();
    let intruder = register(port, OTTER, "Intruder");
    let r = form_post(port, OTTER, "/oauth/token", &format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}", code, enc(LOOPBACK), intruder, "y".repeat(43)));
    assert_eq!((r.status, r.json()["error"].as_str()), (400, Some("invalid_grant")));
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 200, "the real connection survives");
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
    assert!(srv.text().contains("Seen    https://quiet-otter.example   a connection was approved on this address; its connector URL is https://quiet-otter.example/mcp"), "{}", srv.text());
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
    assert_eq!(get(srv.port, OTTER, "/.well-known/oauth-authorization-server").status, 404, "not served with OAuth off");
    let r = http(srv.port, "GET", "/api/oauth/connections", &[("Authorization", "Bearer twenty-chars-0123456")], "").json();
    assert_eq!(r["reason"], "short-token");
}

/// Standard base64, for an `Authorization: Basic` header.
fn b64(input: &str) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = input.as_bytes();
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= chunk.len() { A[((n >> (18 - 6 * i)) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn rpc_result(r: &Resp) -> Json {
    assert_eq!(r.status, 200, "{}", r.body);
    r.json()
}

#[test]
fn a_one_space_token_stays_in_its_space_and_is_not_told_where_files_live() {
    let folder = Folder::new(7155);
    let port = 7155;
    let _srv = start(&folder.0, port, &[], &[]);
    let one = "https://quiet-otter.example/s/%40ecommerce/mcp";
    let (_, access, _) = connect(port, OTTER, one, false);
    let read = |uri: &str| rpc_result(&mcp(port, OTTER, "/s/%40ecommerce/mcp", &access, &json!({"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri": uri}})));
    assert_eq!(read("ontology://@supply-chain/summary")["error"]["code"], -32002, "another space's resource is not found");
    let named = read("ontology://@ecommerce/summary")["result"]["contents"][0]["text"].as_str().unwrap().to_string();
    assert!(named.contains("lumen"), "its own space, named: {}", &named[..named.len().min(200)]);
    assert_eq!(read("ontology://summary")["result"]["contents"][0]["text"].as_str().unwrap(), named, "the flat form means its own space");
    let spaces = rpc_result(&mcp(port, OTTER, "/s/%40ecommerce/mcp", &access, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"spaces","arguments":{}}})));
    let text = spaces["result"]["content"][0]["text"].as_str().unwrap();
    let j: Json = serde_json::from_str(text).unwrap();
    let ids: Vec<&str> = j["spaces"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["@ecommerce"], "only its own space: {}", text);
    assert!(j.get("root").is_none() && !text.contains("\"dir\"") && !text.contains(folder.0.to_str().unwrap()), "no folders: {}", text);

    // an all-spaces connection sees every space, still without folders
    let (_, all, _) = connect(port, OTTER, "https://quiet-otter.example/mcp", false);
    let text = rpc_result(&mcp(port, OTTER, "/mcp", &all, &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"spaces","arguments":{}}})))["result"]["content"][0]["text"].as_str().unwrap().to_string();
    assert!(text.contains("@supply-chain") && !text.contains("\"dir\"") && !text.contains("\"root\""), "{}", text);
    // the owner's header still gets the folders
    let text = rpc_result(&mcp(port, OTTER, "/mcp", TOKEN, &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"spaces","arguments":{}}})))["result"]["content"][0]["text"].as_str().unwrap().to_string();
    assert!(text.contains("\"root\"") && text.contains("\"dir\""), "{}", text);

    // a connection to a space that does not exist gets a 404 that names no other space
    let (_, nowhere, _) = connect(port, OTTER, "https://quiet-otter.example/s/nowhere/mcp", false);
    let r = mcp(port, OTTER, "/s/nowhere/mcp", &nowhere, &tools_list());
    assert_eq!(r.status, 404);
    assert!(!r.body.contains("@supply-chain") && !r.body.contains("@ecommerce"), "{}", r.body);
    let r = mcp(port, OTTER, "/s/nowhere/mcp", TOKEN, &tools_list());
    assert!(r.body.contains("@supply-chain"), "the owner is still told which spaces there are: {}", r.body);
}

#[test]
fn seen_never_puts_the_token_next_to_a_name_read_from_a_request() {
    let folder = Folder::new(7156);
    let (srv, token) = start_with_file_token(&folder.0, 7156, &["--no-examples"]);
    let port = srv.port;
    // sixty forged names, the review's attack: nothing is printed for them
    for i in 0..60 {
        let name = format!("lorefile-signin-{}.evil.example", i);
        get(port, &name, "/.well-known/oauth-authorization-server");
    }
    let text = srv.text();
    assert!(!text.contains("Seen") && !text.contains("evil.example"), "{}", text);
    // the token appears only on the start-up lines, next to 127.0.0.1, as before
    assert!(text.lines().filter(|l| l.contains(&token)).all(|l| l.contains("127.0.0.1")), "{}", text);
    // the first approval on the real address prints it, without the token
    let (client, code) = approve_with(port, OTTER, "https://quiet-otter.example/mcp", true, &token);
    assert_eq!(exchange(port, OTTER, &client, &code, "https://quiet-otter.example/mcp").status, 200);
    let text = srv.text();
    let seen: Vec<&str> = text.lines().filter(|l| l.starts_with("Seen")).collect();
    assert_eq!(seen, ["Seen    https://quiet-otter.example   a connection was approved on this address; its connector URL is https://quiet-otter.example/mcp"], "{}", text);
    assert!(!seen[0].contains(&token));
    drop(srv);

    // pinned: the operator named the address, so its first request prints the sign-in link there,
    // following the start-up rule (a file token is shown); a forged name prints nothing
    let (srv, token) = start_with_file_token(&folder.0, 7157, &["--no-examples", "--public-url", "https://a.example"]);
    get(srv.port, "evil.example", "/.well-known/oauth-authorization-server");
    get(srv.port, "a.example", "/.well-known/oauth-authorization-server");
    let text = srv.text();
    assert!(text.contains(&format!("sign in there: https://a.example/?token={}", token)) && !text.contains("evil.example"), "{}", text);
}

#[test]
fn confidential_clients_prove_their_secret() {
    let folder = Folder::new(7158);
    let port = 7158;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let resource = "https://quiet-otter.example/mcp";
    let reg = |method: &str| -> (String, String) {
        let body = json!({ "client_name": "Confidential", "redirect_uris": [LOOPBACK], "token_endpoint_auth_method": method }).to_string();
        let r = http(port, "POST", "/oauth/register", &with(&tunnel(OTTER), &[("Content-Type", "application/json")]), &body);
        assert_eq!(r.status, 201, "{}", r.body);
        let j = r.json();
        assert_eq!(j["token_endpoint_auth_method"], method);
        (j["client_id"].as_str().unwrap().to_string(), j["client_secret"].as_str().unwrap().to_string())
    };
    let code_for = |client: &str| {
        let (_, id) = authorize(port, OTTER, client, resource, "lorefile:read");
        let loc = decide(port, OTTER, &id, &format!("decision=allow&token={}", enc(TOKEN))).header("Location").unwrap();
        param(&loc, "code").unwrap()
    };
    let exchange_form = |client: &str, code: &str| format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}", code, enc(LOOPBACK), client, VERIFIER);

    // client_secret_post
    let (client, secret) = reg("client_secret_post");
    assert!(secret.starts_with("lfs_"));
    let r = form_post(port, OTTER, "/oauth/token", &exchange_form(&client, &code_for(&client)));
    assert_eq!((r.status, r.json()["error"].as_str()), (401, Some("invalid_client")), "no secret");
    let r = form_post(port, OTTER, "/oauth/token", &format!("{}&client_secret=lfs_wrong", exchange_form(&client, &code_for(&client))));
    assert_eq!(r.status, 401, "a wrong secret");
    let r = form_post(port, OTTER, "/oauth/token", &format!("{}&client_secret={}", exchange_form(&client, &code_for(&client)), secret));
    assert_eq!(r.status, 200, "{}", r.body);
    let rt = r.json()["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(refresh(port, OTTER, &client, &rt).status, 401, "a refresh needs the secret too");
    assert_eq!(form_post(port, OTTER, "/oauth/token", &format!("grant_type=refresh_token&refresh_token={}&client_id={}&client_secret={}", rt, client, secret)).status, 200);

    // client_secret_basic
    let (client, secret) = reg("client_secret_basic");
    let basic = |s: &str| format!("Basic {}", b64(&format!("{}:{}", client, s)));
    let wrong = basic("lfs_wrong");
    let r = http(port, "POST", "/oauth/token", &with(&tunnel(OTTER), &[("Content-Type", "application/x-www-form-urlencoded"), ("Authorization", &wrong)]), &exchange_form(&client, &code_for(&client)));
    assert_eq!(r.status, 401);
    assert!(r.header("WWW-Authenticate").unwrap_or_default().starts_with("Basic"), "{}", r.head);
    let right = basic(&secret);
    let code = code_for(&client);
    let r = http(port, "POST", "/oauth/token", &with(&tunnel(OTTER), &[("Content-Type", "application/x-www-form-urlencoded"), ("Authorization", &right)]), &format!("grant_type=authorization_code&code={}&redirect_uri={}&code_verifier={}", code, enc(LOOPBACK), VERIFIER));
    assert_eq!(r.status, 200, "{}", r.body);
}

#[test]
fn revoking_an_access_token_keeps_the_connection() {
    let folder = Folder::new(7159);
    let port = 7159;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let (client, access, rt) = connect(port, OTTER, "https://quiet-otter.example/mcp", true);
    let r = form_post(port, OTTER, "/oauth/revoke", &format!("token={}&token_type_hint=access_token&client_id={}", access, client));
    assert_eq!(r.status, 200);
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 401, "that access token is gone");
    let r = refresh(port, OTTER, &client, &rt);
    assert_eq!(r.status, 200, "the connection is not: {}", r.body);
    let fresh = r.json()["access_token"].as_str().unwrap().to_string();
    assert_eq!(mcp(port, OTTER, "/mcp", &fresh, &tools_list()).status, 200);
    // an unknown token is a 200 too, and changes nothing
    assert_eq!(form_post(port, OTTER, "/oauth/revoke", "token=lfa_nothing").status, 200);
    assert_eq!(mcp(port, OTTER, "/mcp", &fresh, &tools_list()).status, 200);
}

#[test]
fn the_approval_post_takes_no_origin_but_refuses_origin_null() {
    let folder = Folder::new(7160);
    let port = 7160;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let client = register(port, OTTER, "Test client");
    let form = |id: &str| format!("request={}&decision=allow&token={}", id, enc(TOKEN));
    let (_, id) = authorize(port, OTTER, &client, "https://quiet-otter.example/mcp", "lorefile:read");
    let r = http(port, "POST", "/oauth/authorize", &with(&tunnel(OTTER), &[("Content-Type", "application/x-www-form-urlencoded"), ("Origin", "null")]), &form(&id));
    assert_eq!(r.status, 403, "Origin: null is not this address");
    let r = http(port, "POST", "/oauth/authorize", &with(&tunnel(OTTER), &[("Content-Type", "application/x-www-form-urlencoded")]), &form(&id));
    assert_eq!(r.status, 302, "no Origin at all (not a browser, or an old one): the request ID and the token decide: {}", r.body);
    // an approval sent with the bearer header is recorded as the login token
    let (_, id) = authorize(port, OTTER, &client, "https://quiet-otter.example/mcp", "lorefile:read");
    let bearer = format!("Bearer {}", TOKEN);
    let r = http(port, "POST", "/oauth/authorize", &with(&tunnel(OTTER), &[("Content-Type", "application/x-www-form-urlencoded"), ("Authorization", &bearer)]), &format!("request={}&decision=allow", id));
    assert_eq!(r.status, 302, "{}", r.body);
    let code = param(&r.header("Location").unwrap(), "code").unwrap();
    assert_eq!(exchange(port, OTTER, &client, &code, "https://quiet-otter.example/mcp").status, 200);
    let list = http(port, "GET", "/api/oauth/connections", &[("Authorization", &bearer)], "").json();
    let ways: Vec<&str> = list["connections"].as_array().unwrap().iter().map(|c| c["approved_with"].as_str().unwrap()).collect();
    assert_eq!(ways, ["login token"], "{}", list);
}

#[test]
fn concurrent_refreshes_keep_the_connection() {
    let folder = Folder::new(7161);
    let port = 7161;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let (client, _, rt) = connect(port, OTTER, "https://quiet-otter.example/mcp", true);
    let both: Vec<std::thread::JoinHandle<(u16, String, String)>> = (0..2)
        .map(|_| {
            let (client, rt) = (client.clone(), rt.clone());
            std::thread::spawn(move || {
                let r = refresh(port, OTTER, &client, &rt);
                let j: Json = serde_json::from_str(&r.body).unwrap_or(Json::Null);
                (r.status, j["access_token"].as_str().unwrap_or("").to_string(), j["refresh_token"].as_str().unwrap_or("").to_string())
            })
        })
        .collect();
    let results: Vec<(u16, String, String)> = both.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(results.iter().all(|(s, _, _)| *s == 200), "both refreshes succeed (one inside the grace window): {:?}", results.iter().map(|r| r.0).collect::<Vec<_>>());
    for (_, access, _) in &results {
        assert_eq!(mcp(port, OTTER, "/mcp", access, &tools_list()).status, 200);
    }
    // whichever reply the client kept, its refresh token works now
    for (_, _, rt) in &results {
        assert_eq!(refresh(port, OTTER, &client, rt).status, 200);
    }
    let bearer = format!("Bearer {}", TOKEN);
    assert_eq!(http(port, "GET", "/api/oauth/connections", &[("Authorization", &bearer)], "").json()["connections"].as_array().unwrap().len(), 1);
}

#[test]
fn disconnect_needs_a_real_id() {
    let folder = Folder::new(7162);
    let port = 7162;
    let _srv = start(&folder.0, port, &["--no-examples"], &[]);
    let bearer = format!("Bearer {}", TOKEN);
    let write = |path: &str| http(port, "POST", path, &[("Authorization", &bearer), ("X-Lorefile", "write"), ("Content-Type", "application/json")], "{}");
    for path in ["/api/oauth/connections/revoke", "/api/oauth/connections//revoke", "/api/oauth/connections/no-such/revoke", "/api/oauth/connections/a/b/revoke"] {
        let r = write(path);
        assert_eq!(r.status, 400, "{}: {}", path, r.body);
    }
    let (_, access, _) = connect(port, OTTER, "https://quiet-otter.example/mcp", true);
    let id = http(port, "GET", "/api/oauth/connections", &[("Authorization", &bearer)], "").json()["connections"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(http(port, "POST", &format!("/api/oauth/connections/{}/revoke", id), &[("Authorization", &bearer)], "{}").status, 403, "the write header is still needed");
    let r = write(&format!("/api/oauth/connections/{}/revoke", id));
    assert_eq!((r.status, r.json()["disconnected"].as_u64()), (200, Some(1)));
    assert_eq!(mcp(port, OTTER, "/mcp", &access, &tools_list()).status, 401);
    assert_eq!(write("/api/oauth/connections/all/revoke").status, 200, "all, with nothing left, is fine");
}

// ---------------------------------------------------------------------------
// A file from the user, over a connector (specs/2026-11-27-learn-file.md)
// ---------------------------------------------------------------------------

/// A blank space like Branda's: one domain, no types yet.
struct Blank(PathBuf);
impl Blank {
    fn new(port: u16) -> Blank {
        let dir = std::env::temp_dir().join(format!("lorefile-oauth-learn-{}-{}", port, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("types")).unwrap();
        fs::write(dir.join("ontology.yaml"), "name: branda\ndomains:\n  - name: Core\n    types: []\n").unwrap();
        fs::write(dir.join("glossary.yaml"), "terms: []\n").unwrap();
        Blank(dir)
    }
}
impl Drop for Blank {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn file_args(file: &str, file_name: &str, described: bool) -> Json {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/learn").join(file);
    let mut rdr = csv::Reader::from_path(path).unwrap();
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(|s| s.to_string()).collect();
    let rows: Vec<Vec<String>> = rdr.records().map(|r| r.unwrap().iter().map(|s| s.to_string()).collect()).collect();
    let columns: Vec<Json> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let vals: Vec<&String> = rows.iter().map(|r| &r[i]).collect();
            let mut c = json!({ "name": h, "examples": vals.iter().take(5).collect::<Vec<_>>(), "distinct": vals.iter().collect::<std::collections::HashSet<_>>().len(), "empty": 0 });
            if described {
                c["description"] = json!(if h == "refunded" { "True when the line was refunded." } else { "A column of the promo-sales export." });
            }
            c
        })
        .collect();
    json!({ "file_name": file_name, "row_count": rows.len(), "columns": columns })
}

fn tool(port: u16, bearer: &str, name: &str, args: Json) -> Json {
    let r = mcp(port, OTTER, "/mcp", bearer, &json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":name,"arguments":args}}));
    assert_eq!(r.status, 200, "{}", r.body);
    r.json()["result"].clone()
}

fn tool_json(port: u16, bearer: &str, name: &str, args: Json) -> Json {
    let r = tool(port, bearer, name, args);
    let t = r["content"][0]["text"].as_str().unwrap_or("").to_string();
    serde_json::from_str(&t).unwrap_or_else(|e| panic!("{} did not answer JSON ({}): {}", name, e, t))
}

#[test]
fn a_file_is_learned_over_a_connector_accepted_on_the_page_and_next_month_is_read_the_same() {
    let folder = Blank::new(7164);
    let port = 7164;
    let srv = start(&folder.0, port, &["--no-examples"], &[]);
    let resource = "https://quiet-otter.example/mcp";

    // a read-only connection does not see learn_file, is refused if it calls it, and can still match
    let (_, read_only, _) = connect(port, OTTER, resource, false);
    let names = tool_names(&mcp(port, OTTER, "/mcp", &read_only, &tools_list()));
    assert!(names.contains(&"match_file".to_string()) && !names.contains(&"learn_file".to_string()), "{:?}", names);
    let refused = tool(port, &read_only, "learn_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv", true));
    assert_eq!(refused["isError"], true);
    assert!(refused["content"][0]["text"].as_str().unwrap().contains("may only read"), "{}", refused);

    // ChatGPT, with both scopes: match → questions → one proposal
    let (_, agent, _) = connect(port, OTTER, resource, true);
    let m = tool_json(port, &agent, "match_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv", false));
    assert_eq!(m["match"], "new", "{}", m);
    let dry = tool_json(port, &agent, "learn_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv", true));
    assert_eq!(dry["saved"], false);
    assert!(dry["questions"].as_array().unwrap().len() >= 2, "{}", dry);
    let mut learn = file_args("promo_sales_2026-11.csv", "promo_sales.csv", true);
    learn["answers"] = json!({ "success": { "column": "revenue", "how": "sum" }, "exclude": [{ "column": "refunded", "equals": true }], "exclude_words": "Refunded lines never count.", "currency": "USD" });
    let saved = tool_json(port, &agent, "learn_file", learn);
    assert_eq!((saved["saved"].as_bool(), saved["ok"].as_bool(), saved["errors"].as_u64()), (Some(true), Some(true), Some(0)), "{}", saved);
    let id = saved["queued"].as_str().unwrap().to_string();
    let link = saved["review_at"].as_str().unwrap_or("");
    assert!(link.starts_with("https://quiet-otter.example/s/") && link.ends_with(&format!("/?review={}", id)), "a link to this proposal's review on the public address: {}", saved);
    assert_eq!(saved["accept_at"], saved["review_at"], "the old name stays as an alias");
    assert!(saved["say"].as_str().unwrap().contains(link));
    let p: Json = serde_json::from_str(&fs::read_to_string(folder.0.join(format!(".lorefile/proposals/{}.json", id))).unwrap()).unwrap();
    assert!(p["via"].as_str().unwrap().starts_with("agent via MCP (connection \"Test client\""), "{}", p["via"]);
    assert_eq!(p["learned"]["entity"], "PromoSale");
    drop(srv);

    // the person opens the page on a server that may write, and accepts it (the tokens survive the restart)
    let srv = start(&folder.0, port, &["--no-examples", "--allow-edit"], &[]);
    let login = format!("Bearer {}", TOKEN);
    let r = http(port, "POST", "/api/proposals/accept", &[("Authorization", &login), ("Content-Type", "application/json"), ("X-Lorefile", "write")], &json!({ "id": id, "by": "Vinay" }).to_string());
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["written"], true, "{}", r.body);

    // next month, a new chat: the hint, then December is the same file, accepted by Vinay
    let w = tool_json(port, &agent, "where_is", json!({ "type": "PromoSale" }));
    assert_eq!((w["kind"].as_str(), w["readable_here"].as_bool()), (Some("from_user"), Some(false)), "{}", w);
    assert!(w["hint"].as_str().unwrap().contains("arrive as a file a person gives you"));
    let dec = tool_json(port, &agent, "match_file", file_args("promo_sales_2026-12.csv", "promo_sales_2026-12.csv", false));
    assert_eq!(dec["match"], "same", "{}", dec);
    assert_eq!(dec["entity"], "PromoSale");
    let read = &dec["read_it"][0];
    assert_eq!((read["measure"].as_str(), read["filter"].as_str()), (Some("sum(revenue)"), Some("refunded = false")));
    assert_eq!(read["trust"]["grade"], "accepted");
    assert_eq!(read["trust"]["by"], "Vinay via page");
    assert!(dec["say"].as_str().unwrap().contains("(accepted by Vinay on "), "{}", dec);
    drop(srv);
}

#[test]
fn a_connection_waits_for_a_person_unless_its_approver_chose_straight_away() {
    let folder = Blank::new(7165);
    let port = 7165;
    let srv = start(&folder.0, port, &["--no-examples", "--allow-edit"], &[]);
    let resource = "https://quiet-otter.example/mcp";
    // the approval page offers the choice on a server that takes writes, waiting first
    let client = register(port, OTTER, "Test client");
    let (page, _) = authorize(port, OTTER, &client, resource, "lorefile:read lorefile:propose");
    assert!(page.body.contains("value=\"wait\" checked") && page.body.contains("go in straight away, marked unverified"), "{}", page.body);
    // the default is wait, even with --allow-edit: ChatGPT's save is queued, and Accept works on this one server
    let (_, agent, _) = connect(port, OTTER, resource, true);
    let one = json!({ "op": "add_source", "connection": "shop_warehouse", "kind": "bigquery", "project": "acme", "dataset": "shop", "description": "The warehouse." });
    let saved = tool_json(port, &agent, "propose_patch", one);
    assert_eq!(saved["written"], false, "{}", saved);
    let id = saved["queued"].as_str().unwrap().to_string();
    assert!(saved["review_at"].as_str().unwrap().ends_with(&format!("?review={}", id)), "{}", saved);
    assert!(!folder.0.join("sources/shop_warehouse.yaml").exists());
    let login = format!("Bearer {}", TOKEN);
    let page_headers = [("Authorization", login.as_str()), ("Content-Type", "application/json"), ("X-Lorefile", "write")];
    let r = http(port, "POST", "/api/proposals/accept", &page_headers, &json!({ "id": id, "by": "Vinay" }).to_string());
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(folder.0.join("sources/shop_warehouse.yaml").exists(), "one server: the agent waited, the page accepted");

    // Settings › Connections: the owner sets it to straight away, and its next save is written
    let list = http(port, "GET", "/api/oauth/connections", &[("Authorization", &login)], "");
    let conns = list.json()["connections"].as_array().unwrap().clone();
    let conn = conns.iter().find(|c| c["scopes"].as_array().unwrap().iter().any(|s| s == "lorefile:propose")).unwrap();
    assert_eq!(conn["writes"], "wait");
    let cid = conn["id"].as_str().unwrap().to_string();
    let r = http(port, "POST", &format!("/api/oauth/connections/{}/writes", cid), &page_headers, &json!({ "writes": "direct" }).to_string());
    assert_eq!(r.status, 200, "{}", r.body);
    let two = json!({ "op": "add_source", "connection": "ads", "kind": "bigquery", "project": "acme", "dataset": "ads", "description": "Ad spend." });
    let saved = tool_json(port, &agent, "propose_patch", two);
    assert_eq!(saved["written"], true, "{}", saved);
    // the page header is needed to change it: an agent's token cannot
    let r = http(port, "POST", &format!("/api/oauth/connections/{}/writes", cid), &[("Authorization", &format!("Bearer {}", agent)), ("Content-Type", "application/json")], &json!({ "writes": "wait" }).to_string());
    assert!(r.status == 401 || r.status == 403, "{} {}", r.status, r.body);
    drop(srv);

    // a connection stored before this setting existed reads as wait
    let file = folder.0.join(".lorefile/oauth.json");
    let mut store: Json = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    for c in store["connections"].as_array_mut().unwrap() {
        c.as_object_mut().unwrap().remove("writes");
    }
    fs::write(&file, serde_json::to_string_pretty(&store).unwrap()).unwrap();
    let _srv = start(&folder.0, port, &["--no-examples", "--allow-edit"], &[]);
    let list = http(port, "GET", "/api/oauth/connections", &[("Authorization", &login)], "");
    assert!(list.json()["connections"].as_array().unwrap().iter().all(|c| c["writes"] == "wait"), "{}", list.body);
}
