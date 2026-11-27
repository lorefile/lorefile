//! The HTTP side, against the real server on a scratch copy: no wildcard
//! CORS on reads, SQL only by POST, writes only with the page's header and a
//! matching Origin, oversized bodies refused, a stale proposal not accepted,
//! and history: a new space is a repository, every change a commit, Undo
//! through the gate, Share / Push / Pull with a bare repository (no network).
//! Then the login: no token, no API (401) and a login screen for the page; the
//! bearer header and the cookie from `/api/login` open it; `--no-auth` is the
//! old behaviour; the first run makes, saves and prints a token.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

struct Server { child: Child, port: u16, dir: PathBuf }
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The old behaviour, no login: what these tests were written against.
fn start(port: u16) -> Server { start_with(port, &["--no-examples", "--no-auth"]) }

fn start_with(port: u16, extra: &[&str]) -> Server {
    let dir = std::env::temp_dir().join(format!("lorefile-http-{}-{}", port, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/supply-chain"), &dir);
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string(), "--allow-edit"])
        .args(extra)
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() { break; }
        std::thread::sleep(Duration::from_millis(100));
    }
    Server { child, port, dir }
}

/// One raw HTTP/1.1 request; returns (status, headers in lower case, body).
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String, String) {
    let (status, head, body) = http_raw(port, method, path, headers, body);
    (status, head.to_lowercase(), body)
}

/// The same, with the headers as sent (a cookie's value keeps its case).
fn http_raw(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Length: {}\r\n", method, path, port, body.len());
    for (k, v) in headers { req.push_str(&format!("{}: {}\r\n", k, v)); }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, head.to_string(), body.to_string())
}

const JSON: (&str, &str) = ("Content-Type", "application/json");
const PAGE: (&str, &str) = ("X-Lorefile", "write");

#[test]
fn reads_are_not_cors_open_and_sql_is_post_only() {
    let srv = start(7097);
    let (status, head, body) = http(srv.port, "GET", "/api/summary", &[("Origin", "https://evil.example")], "");
    assert_eq!(status, 200);
    assert!(!head.contains("access-control-allow-origin"), "no wildcard CORS on reads:\n{}", head);
    assert!(body.contains("\"manifest\""));
    let (status, _, _) = http(srv.port, "GET", "/api/query?sql=select%201", &[], "");
    assert_ne!(status, 200, "SQL over GET is gone");
    if cfg!(feature = "duckdb") {
        let (status, _, body) = http(srv.port, "POST", "/api/query", &[JSON], r#"{"sql":"select name from Supplier limit 1"}"#);
        assert_eq!(status, 200, "{}", body);
        let (status, _, body) = http(srv.port, "POST", "/api/query", &[JSON], r#"{"sql":"select * from read_text('/etc/passwd')"}"#);
        assert_eq!(status, 400, "the fence holds over HTTP too: {}", body);
    }
}

#[test]
fn writes_need_the_page_header_and_a_matching_origin() {
    let srv = start(7098);
    let edit = r#"{"op":"set_type_description","type":"Supplier","text":"A company we buy from."}"#;
    let (status, _, _) = http(srv.port, "POST", "/api/patch", &[JSON], edit);
    assert_eq!(status, 403, "no header, no write");
    let (status, _, _) = http(srv.port, "POST", "/api/patch", &[JSON, PAGE, ("Origin", "https://evil.example")], edit);
    assert_eq!(status, 403, "a foreign Origin is refused even with the header");
    let (status, _, body) = http(srv.port, "POST", "/api/patch", &[JSON, PAGE, ("Origin", &format!("http://127.0.0.1:{}", srv.port))], edit);
    assert_eq!(status, 200, "{}", body);
    assert!(body.contains("\"written\":true") || body.contains("\"written\": true"), "{}", body);
    let big = format!("{{\"sql\":\"{}\"}}", "x".repeat(5 * 1024 * 1024));
    let (status, _, _) = http(srv.port, "POST", "/api/query", &[JSON], &big);
    assert_eq!(status, 413, "a 5 MB body is refused");
}

#[test]
fn a_stale_proposal_is_not_accepted_over_newer_changes() {
    let srv = start(7099);
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let o: (&str, &str) = ("Origin", &origin);
    // an agent proposes a glossary word (read-only path: propose through the tool with edit off is not available here,
    // so queue one by hand the way the server would)
    let queued = http(srv.port, "POST", "/api/tool", &[JSON, PAGE, o], r#"{"name":"propose_change","arguments":{"file":"glossary.yaml","content":"terms:\n  - term: seller\n    means: Supplier\n","reason":"t"}}"#);
    assert_eq!(queued.0, 200);
    // in edit mode that was written straight away; make a proposal file that is now stale
    let pdir = srv.dir.join(".lorefile/proposals");
    fs::create_dir_all(&pdir).unwrap();
    let before = "terms: []\n";
    let proposal = serde_json::json!({ "id": "p-stale", "file": "glossary.yaml", "content": "terms:\n  - term: maker\n    means: Supplier\n", "reason": "", "via": "test", "created_at": "2026-11-26T00:00:00Z", "status": "pending", "ok": true, "before": before });
    fs::write(pdir.join("p-stale.json"), proposal.to_string()).unwrap();
    let (status, _, body) = http(srv.port, "POST", "/api/proposals/accept", &[JSON, PAGE, o], r#"{"id":"p-stale"}"#);
    assert_eq!(status, 409, "{}", body);
    assert!(fs::read_to_string(srv.dir.join("glossary.yaml")).unwrap().contains("seller"), "the newer change survived");
}

#[test]
fn a_shipped_example_shows_its_yaml_but_takes_no_writes() {
    // with the examples in the picker: the page's View YAML reads a shipped file, a write to it is still refused
    let srv = start_with(7100, &["--no-auth"]);
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let (status, _, body) = http(srv.port, "GET", "/s/%40ecommerce/api/yaml?file=logic%2Fmeasurement.yaml", &[], "");
    assert_eq!(status, 200, "{}", body);
    assert!(body.contains("metric: success"), "the file itself comes back: {}", body);
    let edit = serde_json::json!({ "file": "logic/measurement.yaml", "content": "metrics: []\n" }).to_string();
    let (status, _, body) = http(srv.port, "POST", "/s/%40ecommerce/api/edit", &[JSON, PAGE, ("Origin", &origin)], &edit);
    assert_eq!(status, 403, "a shipped example stays read-only even with --allow-edit: {}", body);
    assert!(body.contains("read-only"), "{}", body);
    let (_, _, body) = http(srv.port, "GET", "/s/%40ecommerce/api/yaml?file=logic%2Fmeasurement.yaml", &[], "");
    assert!(body.contains("metric: success"), "nothing was written: {}", body);
    // and it has no history to read or undo
    let (status, _, body) = http(srv.port, "GET", "/s/%40ecommerce/api/git/log", &[], "");
    assert_eq!(status, 403, "{}", body);
    assert!(body.contains("read-only"), "{}", body);
    let (status, _, body) = http(srv.port, "POST", "/s/%40ecommerce/api/git/undo", &[JSON, PAGE, ("Origin", &origin)], r#"{"hash":"abcdef1"}"#);
    assert_eq!(status, 403, "{}", body);
}

// ---------------------------------------------------------------------------
// History: local git from the first moment, shared only when asked
// ---------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {:?} in {}: {}", args, dir.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn has_git() -> bool {
    Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// A server on an empty folder of spaces (no shipped examples), edits on.
fn start_spaces(port: u16, extra: &[&str]) -> Server {
    let dir = std::env::temp_dir().join(format!("lorefile-http-spaces-{}-{}", port, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string(), "--allow-edit", "--no-examples", "--no-auth"])
        .args(extra)
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() { break; }
        std::thread::sleep(Duration::from_millis(100));
    }
    Server { child, port, dir }
}

fn j(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("not JSON ({}): {}", e, body))
}

#[test]
fn history_starts_with_the_space_and_undo_and_share_round_trip() {
    if !has_git() { return; }
    let srv = start_spaces(7101, &[]);
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let w = [JSON, PAGE, ("Origin", origin.as_str())];
    let post = |path: &str, body: serde_json::Value| { let (s, _, b) = http(srv.port, "POST", path, &w, &body.to_string()); (s, j(&b)) };
    let get = |path: &str| { let (s, _, b) = http(srv.port, "GET", path, &[], ""); (s, j(&b)) };

    // a new space is a repository with one commit, no flag given
    let (status, created) = post("/api/spaces", serde_json::json!({ "name": "Brand Two", "from": "starter" }));
    assert_eq!(status, 200, "{}", created);
    assert_eq!(created["history"], "on");
    let space = srv.dir.join("brand-two");
    assert_eq!(git(&space, &["log", "--format=%an|%s"]), "person via page|Start the space");

    // describe a field: the second commit, that sentence as its message
    let (status, saved) = post("/s/brand-two/api/patch", serde_json::json!({ "op": "set_type_description", "type": "Customer", "text": "Someone who buys from us." }));
    assert_eq!(status, 200, "{}", saved);
    assert!(saved["commit"].is_string(), "{}", saved);
    assert_eq!(git(&space, &["log", "-1", "--format=%an|%s"]), "person via page|Described Customer");
    let described = git(&space, &["rev-parse", "--short", "HEAD"]);

    // an agent's batch accepted from the inbox: one commit, the agent the author, the person the committer
    let glossary = fs::read_to_string(space.join("glossary.yaml")).unwrap();
    let with_patron = format!("{}  - term: patron\n    means: Customer\n", glossary);
    let widget = "type: Widget\ndescription: A thing we sell.\nproperties:\n  widget_id: { type: string, key: true }\n";
    let pdir = space.join(".lorefile/proposals");
    fs::create_dir_all(&pdir).unwrap();
    let proposal = serde_json::json!({ "id": "p-agent", "file": "glossary.yaml", "content": with_patron, "before": glossary, "reason": "The team says patron.", "via": "agent via MCP", "created_at": "2026-11-26T00:00:00Z", "status": "pending", "ok": true, "kind": "batch", "name": "2 files",
        "summary": "2 changes: added the word \"patron\" for Customer; created Widget",
        "files": [ { "file": "glossary.yaml", "content": with_patron, "before": glossary }, { "file": "types/widget.yaml", "content": widget, "before": "" } ] });
    fs::write(pdir.join("p-agent.json"), proposal.to_string()).unwrap();
    let (status, accepted) = post("/s/brand-two/api/proposals/accept", serde_json::json!({ "id": "p-agent" }));
    assert_eq!(status, 200, "{}", accepted);
    assert_eq!(git(&space, &["log", "-1", "--format=%an <%ae>|%cn|%s"]), "agent via MCP <agent-via-mcp@localhost>|person via page|An agent suggests 2 changes: Widget, patron", "the commit says the review's headline");
    assert_eq!(git(&space, &["show", "--format=", "--name-only", "HEAD"]), "glossary.yaml\ntrust.yaml\ntypes/widget.yaml", "both files in one commit, and the grade with them");
    let trust = fs::read_to_string(space.join("trust.yaml")).unwrap();
    assert!(trust.contains("patron: { grade: accepted, by: \"person via page\""), "the person who accepted stands behind the word, not the agent: {}", trust);

    // History lists the three, newest first, agent and person told apart
    let (status, log) = get("/s/brand-two/api/git/log");
    assert_eq!(status, 200, "{}", log);
    assert_eq!(log["history"], "on");
    let commits = log["commits"].as_array().unwrap();
    let rows: Vec<(String, String)> = commits.iter().map(|c| (c["kind"].as_str().unwrap().to_string(), c["message"].as_str().unwrap().to_string())).collect();
    assert_eq!(rows.len(), 3, "{:?}", rows);
    assert_eq!(rows[0].0, "agent");
    assert_eq!(rows[1], ("person".to_string(), "Described Customer".to_string()));
    assert_eq!(rows[2], ("person".to_string(), "Start the space".to_string()));
    assert_eq!(commits[0]["files"], serde_json::json!(["glossary.yaml", "trust.yaml", "types/widget.yaml"]));
    assert_eq!(commits[2]["first"], true);

    // Show gives the diff of one change, paths inside the space
    let (status, shown) = get(&format!("/s/brand-two/api/git/show?hash={}", described));
    assert_eq!(status, 200, "{}", shown);
    let diff = shown["diff"].as_str().unwrap();
    assert!(diff.contains("+++ b/types/customer.yaml") && diff.contains("+") && diff.contains("Someone who buys from us."), "{}", diff);
    assert_eq!(get("/s/brand-two/api/git/show?hash=nothex!").0, 404);

    // Undo the description: through the gate, as a new commit, and the page sees the old sentence again
    let (status, undone) = post("/s/brand-two/api/git/undo", serde_json::json!({ "hash": described }));
    assert_eq!(status, 200, "{}", undone);
    assert_eq!(undone["written"], true);
    assert_eq!(git(&space, &["log", "-1", "--format=%an|%s"]), "person via page|Undo: Described Customer");
    let (_, customer) = get("/s/brand-two/api/describe?name=Customer");
    assert!(customer.to_string().contains("signed up with us"), "the old description is back: {}", customer);
    assert!(!customer.to_string().contains("Someone who buys"), "{}", customer);
    assert!(fs::read_to_string(space.join("glossary.yaml")).unwrap().contains("patron"), "the agent's later change stays");
    let (status, _, refused) = http(srv.port, "POST", "/s/brand-two/api/git/undo", &[JSON], &serde_json::json!({ "hash": described }).to_string());
    assert_eq!(status, 403, "undo is a write like any other: {}", refused);

    // Share with a bare repository on this machine, push, then pull a teammate's change back
    let shared = srv.dir.join("shared.git");
    git(&srv.dir, &["init", "-q", "--bare", "-b", "main", shared.to_str().unwrap()]);
    let (status, r) = post("/s/brand-two/api/git/remote", serde_json::json!({ "url": shared.to_str().unwrap() }));
    assert_eq!(status, 200, "{}", r);
    let (_, info) = get("/s/brand-two/api/git");
    assert_eq!(info["remote_host"], "this computer");
    assert_eq!(info["commits"], 4);
    assert_eq!(info["pushed"], false);
    let (status, r) = post("/s/brand-two/api/git/push", serde_json::json!({}));
    assert_eq!(status, 200, "{}", r);
    let (_, info) = get("/s/brand-two/api/git");
    assert_eq!((info["ahead"].as_u64(), info["behind"].as_u64(), info["pushed"].as_bool()), (Some(0), Some(0), Some(true)), "{}", info);

    let teammate = srv.dir.join("teammate");
    git(&srv.dir, &["clone", "-q", shared.to_str().unwrap(), teammate.to_str().unwrap()]);
    let g = fs::read_to_string(teammate.join("glossary.yaml")).unwrap();
    fs::write(teammate.join("glossary.yaml"), format!("{}  - term: buyer\n    means: Customer\n", g)).unwrap();
    git(&teammate, &["-c", "user.name=Teammate", "-c", "user.email=t@example.com", "commit", "-qam", "Add the word buyer"]);
    git(&teammate, &["push", "-q", "origin", "HEAD"]);
    let (status, r) = post("/s/brand-two/api/git/pull", serde_json::json!({}));
    assert_eq!(status, 200, "{}", r);
    assert!(r["summary"].as_str().unwrap().contains("Pulled 1 change"), "{}", r);
    let (_, summary) = get("/s/brand-two/api/summary");
    assert!(summary["glossary"].to_string().contains("buyer"), "the page sees the pulled word: {}", summary["glossary"]);

    // an address with nothing behind it: an error in words, not a hang
    let (_, _) = post("/s/brand-two/api/git/remote", serde_json::json!({ "url": srv.dir.join("nowhere.git").to_str().unwrap() }));
    let (status, r) = post("/s/brand-two/api/git/push", serde_json::json!({}));
    assert_eq!(status, 400);
    assert!(r["error"].as_str().unwrap().contains("no repository at that address"), "{}", r);
    let (status, r) = post("/s/brand-two/api/git/remote", serde_json::json!({ "url": "--upload-pack=touch /tmp/x" }));
    assert_eq!(status, 400, "{}", r);
}

#[test]
fn a_space_inside_another_repository_is_left_uncommitted_as_before() {
    if !has_git() { return; }
    let outer = std::env::temp_dir().join(format!("lorefile-http-nested-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outer);
    fs::create_dir_all(&outer).unwrap();
    git(&outer, &["init", "-q"]);
    fs::write(outer.join("README.md"), "a project\n").unwrap();
    git(&outer, &["add", "-A"]);
    git(&outer, &["-c", "user.name=Dev", "-c", "user.email=dev@example.com", "commit", "-qm", "project"]);
    let onto = outer.join("examples/shop");
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/supply-chain"), &onto);
    git(&outer, &["add", "-A"]);
    git(&outer, &["-c", "user.name=Dev", "-c", "user.email=dev@example.com", "commit", "-qm", "add the ontology"]);
    let port = 7102;
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", onto.to_str().unwrap(), "--port", &port.to_string(), "--allow-edit", "--no-examples", "--no-auth"])
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let srv = Server { child, port, dir: outer.clone() };
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() { std::thread::sleep(Duration::from_millis(100)); }
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let w = [JSON, PAGE, ("Origin", origin.as_str())];

    let (status, _, body) = http(srv.port, "POST", "/api/patch", &w, r#"{"op":"set_type_description","type":"Supplier","text":"A company we buy from."}"#);
    assert_eq!(status, 200, "{}", body);
    let saved = j(&body);
    assert!(saved["written"] == true && saved.get("commit").is_none(), "no automatic commit in someone else's repository: {}", saved);
    assert_eq!(git(&outer, &["rev-list", "--count", "HEAD"]), "2");
    let (_, _, body) = http(srv.port, "GET", "/api/git", &[], "");
    let info = j(&body);
    assert_eq!(info["history"], "off (inside another repository)");
    assert_eq!(info["nested"], true);
    assert!(info["uncommitted"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().starts_with("types/")), "{}", info);
    let (_, _, body) = http(srv.port, "GET", "/api/summary", &[], "");
    assert_eq!(j(&body)["history"], "off (inside another repository)");
    // sharing is done from that repository, not from here
    let (status, _, body) = http(srv.port, "POST", "/api/git/remote", &w, r#"{"url":"git@example.com:x/y.git"}"#);
    assert_eq!(status, 400, "{}", body);
    assert!(body.contains("inside the repository"), "{}", body);
    // the Commit button still works
    let (status, _, body) = http(srv.port, "POST", "/api/git/commit", &w, r#"{"message":"Describe suppliers"}"#);
    assert_eq!(status, 200, "{}", body);
    assert_eq!(git(&outer, &["log", "-1", "--format=%s"]), "Describe suppliers");
}

// ---------------------------------------------------------------------------
// Login: one token, kept as a cookie by the page, sent as a bearer header by clients
// ---------------------------------------------------------------------------

const TOKEN: &str = "test-token-0123456789abcdef";

/// `Set-Cookie: lorefile-<port>=<value>; …` → `lorefile-<port>=<value>`, ready to send back.
fn cookie_from(head: &str) -> String {
    let line = head.lines().find(|l| l.to_ascii_lowercase().starts_with("set-cookie:")).unwrap_or_else(|| panic!("no Set-Cookie in:\n{}", head));
    line.split_once(':').unwrap().1.trim().split(';').next().unwrap().trim().to_string()
}

#[test]
fn without_the_token_the_api_says_401_and_the_page_asks_for_it() {
    let srv = start_with(7103, &["--no-examples", "--token", TOKEN]);
    let (status, head, body) = http(srv.port, "GET", "/api/summary", &[], "");
    assert_eq!(status, 401, "{}", body);
    assert!(head.contains("content-type: application/json"), "{}", head);
    assert!(j(&body)["error"].as_str().unwrap().contains("sign in first"), "a one-line JSON error: {}", body);
    let (status, _, body) = http(srv.port, "GET", "/s/supply-chain/api/describe?name=Supplier", &[], "");
    assert_eq!(status, 401, "every space's API is behind it: {}", body);
    let (status, head, body) = http(srv.port, "GET", "/", &[], "");
    assert_eq!(status, 401);
    assert!(head.contains("text/html"), "{}", head);
    assert!(body.contains("Paste the token printed in the terminal") && body.contains("LOREFILE_TOKEN"), "the login screen: {}", body);
    assert!(!body.contains("renderSettings"), "the app itself waits until the token is given");
    // a wrong token is the same as none
    let (status, _, _) = http(srv.port, "GET", "/api/summary", &[("Authorization", "Bearer test-token-0123456789abcdeX")], "");
    assert_eq!(status, 401);
    let (status, _, body) = http(srv.port, "POST", "/api/login", &[JSON], r#"{"token":"nope"}"#);
    assert_eq!(status, 401, "{}", body);
    let (status, _, body) = http(srv.port, "GET", "/?token=nope", &[], "");
    assert_eq!(status, 401);
    assert!(body.contains("That is not this server") && body.contains("class=\"err\""), "the login screen says so: {}", body);
    // the health check and the sign-in state answer without it, and give nothing away
    assert_eq!(http(srv.port, "GET", "/healthz", &[], "").0, 200);
    let (_, _, body) = http(srv.port, "GET", "/api/auth", &[], "");
    let a = j(&body);
    assert_eq!((a["required"].as_bool(), a["signed_in"].as_bool()), (Some(true), Some(false)), "{}", a);
    assert!(a.get("token").is_none(), "no token for someone without it: {}", a);
}

#[test]
fn the_bearer_header_and_the_login_cookie_both_open_it() {
    let srv = start_with(7104, &["--no-examples", "--token", TOKEN]);
    let bearer = format!("Bearer {}", TOKEN);
    let (status, _, body) = http(srv.port, "GET", "/api/summary", &[("Authorization", &bearer)], "");
    assert_eq!(status, 200, "{}", body);
    assert!(body.contains("\"manifest\""));

    // a script signs in with JSON and gets the cookie
    let (status, head, body) = http_raw(srv.port, "POST", "/api/login", &[JSON], &serde_json::json!({ "token": TOKEN }).to_string());
    assert_eq!(status, 200, "{}", body);
    let set = head.to_lowercase();
    assert!(set.contains("httponly") && set.contains("samesite=strict") && set.contains("path=/"), "{}", head);
    let cookie = cookie_from(&head);
    assert_eq!(cookie, format!("lorefile-{}={}", srv.port, TOKEN), "one cookie name per port");
    let (status, _, body) = http(srv.port, "GET", "/api/summary", &[("Cookie", &cookie)], "");
    assert_eq!(status, 200, "{}", body);
    let (status, _, body) = http(srv.port, "GET", "/", &[("Cookie", &format!("other=1; {}", cookie))], "");
    assert_eq!(status, 200, "the page itself, once signed in");
    assert!(body.contains("renderSettings"));
    let (_, _, body) = http(srv.port, "GET", "/api/auth", &[("Cookie", &cookie)], "");
    assert_eq!(j(&body)["token"], TOKEN, "the page shows it masked in Settings, with a copy button");

    // the link the terminal printed: cookie set, token out of the address bar
    let (status, head, _) = http_raw(srv.port, "GET", &format!("/s/supply-chain/?token={}", TOKEN), &[], "");
    assert_eq!(status, 303);
    assert!(head.contains("Location: /s/supply-chain/"), "{}", head);
    assert_eq!(cookie_from(&head), cookie);
    // the login screen's form: back to where the visitor was going
    let (status, head, _) = http_raw(srv.port, "POST", "/api/login", &[("Content-Type", "application/x-www-form-urlencoded")], &format!("token={}&next=%2Fs%2Fsupply-chain%2F", TOKEN));
    assert_eq!(status, 303);
    assert!(head.contains("Location: /s/supply-chain/"), "{}", head);
    let (_, head, _) = http_raw(srv.port, "POST", "/api/login", &[("Content-Type", "application/x-www-form-urlencoded")], &format!("token={}&next=https%3A%2F%2Fevil.example", TOKEN));
    assert!(head.lines().any(|l| l.trim_end() == "Location: /"), "never sent off to another site: {}", head);

    // signing out clears the cookie
    let (status, head, _) = http_raw(srv.port, "POST", "/api/logout", &[JSON, ("Cookie", &cookie)], "{}");
    assert_eq!(status, 200);
    assert!(head.contains("Max-Age=0") && cookie_from(&head) == format!("lorefile-{}=", srv.port), "{}", head);
}

#[test]
fn signed_in_writes_still_need_the_page_header_and_a_matching_origin() {
    let srv = start_with(7105, &["--no-examples", "--token", TOKEN]);
    let bearer = format!("Bearer {}", TOKEN);
    let cookie = format!("lorefile-{}={}", srv.port, TOKEN);
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let edit = r#"{"op":"set_type_description","type":"Supplier","text":"A company we buy from."}"#;
    let (status, _, _) = http(srv.port, "POST", "/api/patch", &[JSON, PAGE, ("Origin", &origin)], edit);
    assert_eq!(status, 401, "the header alone is not a login");
    let (status, _, _) = http(srv.port, "POST", "/api/patch", &[JSON, ("Authorization", &bearer)], edit);
    assert_eq!(status, 403, "the token alone is not the write header");
    let (status, _, _) = http(srv.port, "POST", "/api/patch", &[JSON, PAGE, ("Cookie", &cookie), ("Origin", "https://evil.example")], edit);
    assert_eq!(status, 403, "a foreign Origin is refused even signed in");
    let (status, _, body) = http(srv.port, "POST", "/api/patch", &[JSON, PAGE, ("Cookie", &cookie), ("Origin", &origin)], edit);
    assert_eq!(status, 200, "the page, signed in: {}", body);
    assert_eq!(j(&body)["written"], true, "{}", body);
    let (status, _, body) = http(srv.port, "POST", "/api/patch", &[JSON, PAGE, ("Authorization", &bearer)], r#"{"op":"set_type_description","type":"Part","text":"Something we buy."}"#);
    assert_eq!(status, 200, "a script with the token and the header: {}", body);
}

#[test]
fn no_auth_keeps_the_old_behaviour_and_is_refused_on_an_open_address() {
    let srv = start(7106);
    let (status, _, body) = http(srv.port, "GET", "/", &[], "");
    assert_eq!(status, 200);
    assert!(body.contains("renderSettings"), "the page, no login");
    let (_, _, body) = http(srv.port, "GET", "/api/auth", &[], "");
    assert_eq!(j(&body)["required"], false);
    let (status, _, _) = http(srv.port, "GET", "/api/summary", &[], "");
    assert_eq!(status, 200);
    // and not on 0.0.0.0 without --i-know: the process refuses before it binds
    let out = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", srv.dir.to_str().unwrap(), "--port", "7107", "--no-examples", "--no-auth", "--host", "0.0.0.0"])
        .output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--i-know"), "{}", String::from_utf8_lossy(&out.stderr));
}

/// Start a server with stderr captured, and return it with the lines it printed up to `Edit`.
fn start_printing(port: u16, dir: &Path, env: &[(&str, &str)]) -> (Child, Vec<String>) {
    use std::io::BufRead;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lorefile"));
    cmd.args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string(), "--no-examples"]).env_remove("LOREFILE_TOKEN");
    for (k, v) in env { cmd.env(k, v); }
    let mut child = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    let err = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || for l in std::io::BufReader::new(err).lines().map_while(Result::ok) { if tx.send(l).is_err() { break; } });
    let mut lines = Vec::new();
    while let Ok(l) = rx.recv_timeout(Duration::from_secs(30)) {
        let done = l.starts_with("Edit");
        lines.push(l);
        if done { break; }
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() { std::thread::sleep(Duration::from_millis(100)); }
    (child, lines)
}

#[test]
fn the_first_run_makes_a_token_saves_it_and_prints_the_link_and_the_mcp_command() {
    let dir = std::env::temp_dir().join(format!("lorefile-http-first-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/supply-chain"), &dir);
    let port = 7108;
    let (child, lines) = start_printing(port, &dir, &[]);
    let srv = Server { child, port, dir: dir.clone() };
    let text = lines.join("\n");
    let saved = fs::read_to_string(dir.join(".lorefile/auth.toml")).unwrap();
    let token = saved.lines().find(|l| l.starts_with("token")).unwrap().split('"').nth(1).unwrap().to_string();
    assert_eq!(token.len(), 43, "32 random bytes as base64url: {}", token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(dir.join(".lorefile/auth.toml")).unwrap().permissions().mode() & 0o777, 0o600, "only this user can read it");
    }
    assert!(text.contains(&format!("http://127.0.0.1:{}/?token={}", port, token)) && text.contains("first run: token saved to"), "{}", text);
    assert!(text.contains(&format!("claude mcp add --transport http lorefile http://127.0.0.1:{}/mcp --header \"Authorization: Bearer {}\"", port, token)), "{}", text);
    let (status, _, _) = http(port, "GET", "/api/summary", &[("Authorization", &format!("Bearer {}", token))], "");
    assert_eq!(status, 200);
    drop(srv); // removes the folder: copy it again and keep the token file this time
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/supply-chain"), &dir);
    fs::create_dir_all(dir.join(".lorefile")).unwrap();
    fs::write(dir.join(".lorefile/auth.toml"), &saved).unwrap();

    // the next start reads the same token back; LOREFILE_TOKEN comes before the file
    let (child, lines) = start_printing(port, &dir, &[]);
    assert!(lines.join("\n").contains("token from"), "{:?}", lines);
    let srv = Server { child, port, dir: dir.clone() };
    assert_eq!(http(port, "GET", "/api/summary", &[("Authorization", &format!("Bearer {}", token))], "").0, 200);
    drop(srv);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/supply-chain"), &dir);
    fs::create_dir_all(dir.join(".lorefile")).unwrap();
    fs::write(dir.join(".lorefile/auth.toml"), &saved).unwrap();
    let (child, lines) = start_printing(port, &dir, &[("LOREFILE_TOKEN", "from-the-environment-0123")]);
    let srv = Server { child, port, dir };
    let text = lines.join("\n");
    assert!(text.contains("sign in with the token in LOREFILE_TOKEN") && !text.contains("from-the-environment"), "the operator's token is not echoed: {}", text);
    assert_eq!(http(port, "GET", "/api/summary", &[("Authorization", "Bearer from-the-environment-0123")], "").0, 200);
    assert_eq!(http(port, "GET", "/api/summary", &[("Authorization", &format!("Bearer {}", token))], "").0, 401, "the file's token is not used then");
    drop(srv);
}

// ---------------------------------------------------------------------------
// Grades: who stands behind each definition
// ---------------------------------------------------------------------------

#[test]
fn grades_are_set_by_a_persons_click_behind_the_write_gate() {
    if !has_git() {
        return;
    }
    let srv = start_spaces(7130, &[]);
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let w = [JSON, PAGE, ("Origin", origin.as_str())];
    let post = |path: &str, body: serde_json::Value| { let (s, _, b) = http(srv.port, "POST", path, &w, &body.to_string()); (s, j(&b)) };
    let get = |path: &str| { let (s, _, b) = http(srv.port, "GET", path, &[], ""); (s, j(&b)) };
    let (status, _) = post("/api/spaces", serde_json::json!({ "name": "Brand Two", "from": "starter" }));
    assert_eq!(status, 200);
    let space = srv.dir.join("brand-two");

    // no trust.yaml yet: every definition ungraded, and the summary says so
    let (status, t) = get("/s/brand-two/api/trust");
    assert_eq!(status, 200, "{}", t);
    assert_eq!(t["file"], false);
    assert_eq!(t["counts"]["ungraded"], 5, "three words and two guardrails: {}", t);
    let (_, summary) = get("/s/brand-two/api/summary");
    assert_eq!(summary["trust"]["counts"]["ungraded"], 5);

    // without the page's header: refused
    let (status, _, _) = http(srv.port, "POST", "/s/brand-two/api/trust", &[JSON], r#"{"op":"vouch_all"}"#);
    assert_eq!(status, 403);
    // the raw editor cannot write trust.yaml
    let (status, r) = post("/s/brand-two/api/edit", serde_json::json!({ "file": "trust.yaml", "content": "trust: {}\n" }));
    assert_eq!(status, 400, "{}", r);
    assert!(r["error"].as_str().unwrap().contains("trust.yaml is written by Lorefile"), "{}", r);

    // Vouch for all: one commit, in the person's name
    let (status, r) = post("/s/brand-two/api/trust", serde_json::json!({ "op": "vouch_all", "by": "Priya" }));
    assert_eq!(status, 200, "{}", r);
    assert_eq!(git(&space, &["log", "-1", "--format=%an|%s"]), "Priya via page|Vouched for 5 definitions");
    assert_eq!(git(&space, &["show", "--format=", "--name-only", "HEAD"]), "trust.yaml");
    let (_, t) = get("/s/brand-two/api/trust");
    assert_eq!(t["counts"]["accepted"], 5, "{}", t);
    let item = t["items"].as_array().unwrap().iter().find(|x| x["name"] == "ACC-001").unwrap().clone();
    assert_eq!(item["grade"], "accepted");
    assert_eq!(item["by"], "Priya via page");

    // an agent reports a check; the person's "Yes" makes it checked, in their name
    let (status, r) = post("/s/brand-two/api/create", serde_json::json!({ "kind": "source", "name": "crm", "content": "connection: crm\nkind: postgres\ndescription: The CRM database.\n" }));
    assert_eq!(status, 200, "{}", r);
    let (_, r) = post("/s/brand-two/api/tool", serde_json::json!({ "name": "propose_patch", "arguments": { "op": "record_check", "kind": "guardrail", "name": "ACC-001", "against": "crm", "note": "no closed account had a balance in 900 rows" } }));
    let reply = r["content"][0]["text"].as_str().unwrap().to_string();
    assert!(reply.contains("queued as p-"), "{}", reply);
    let (_, props) = get("/s/brand-two/api/proposals");
    let p = props["proposals"].as_array().unwrap().iter().find(|p| p["check"].is_object()).unwrap().clone();
    let (status, r) = post("/s/brand-two/api/proposals/accept", serde_json::json!({ "id": p["id"], "by": "Priya" }));
    assert_eq!(status, 200, "{}", r);
    assert_eq!(git(&space, &["log", "-1", "--format=%an|%s"]), "Priya via page|Checked guardrail ACC-001 against crm");
    let trust = fs::read_to_string(space.join("trust.yaml")).unwrap();
    assert!(trust.contains("ACC-001: { grade: checked, by: \"Priya via page\"") && trust.contains("(reported by agent via API)"), "{}", trust);
    // Mark as checked from the page: a source and a note, or nothing
    let (status, r) = post("/s/brand-two/api/trust", serde_json::json!({ "op": "check", "kind": "guardrail", "name": "ACC-002", "against": "crm" }));
    assert_eq!(status, 400, "no note: {}", r);
    let (status, _) = post("/s/brand-two/api/trust", serde_json::json!({ "op": "check", "kind": "word", "name": "client", "against": "crm", "note": "x" }));
    assert_eq!(status, 400, "a word cannot be checked");
    let (status, r) = post("/s/brand-two/api/trust", serde_json::json!({ "op": "check", "name": "ACC-002", "against": "crm", "note": "every closed account was open the day before" }));
    assert_eq!(status, 200, "{}", r);
    // and History shows each grade as a commit
    let (_, log) = get("/s/brand-two/api/git/log");
    let messages: Vec<&str> = log["commits"].as_array().unwrap().iter().map(|c| c["message"].as_str().unwrap()).collect();
    assert_eq!(&messages[..3], &["Checked guardrail ACC-002 against crm", "Checked guardrail ACC-001 against crm", "Added the source crm"], "{:?}", messages);

    // a reported check is about the definition the agent compared: if a person changes the rule before clicking, the click is refused
    let (_, r) = post("/s/brand-two/api/tool", serde_json::json!({ "name": "propose_patch", "arguments": { "op": "record_check", "name": "ACC-002", "against": "crm", "note": "checked again" } }));
    assert!(r["content"][0]["text"].as_str().unwrap().contains("queued as p-"), "{}", r);
    let rules = fs::read_to_string(space.join("constraints/basics.yaml")).unwrap();
    let (status, r) = post("/s/brand-two/api/edit", serde_json::json!({ "file": "constraints/basics.yaml", "content": rules.replace("rule: open == true", "rule: open != false"), "by": "Priya" }));
    assert_eq!(status, 200, "{}", r);
    assert_eq!(r["by"], "Priya via page", "the reply says the name that was used: {}", r);
    let (_, props) = get("/s/brand-two/api/proposals");
    let p = props["proposals"].as_array().unwrap().iter().find(|p| p["check"].is_object() && p["status"] == "pending").unwrap().clone();
    assert!(!p["check"]["of"].as_str().unwrap().is_empty(), "the claim remembers what was compared: {}", p);
    let (status, r) = post("/s/brand-two/api/proposals/accept", serde_json::json!({ "id": p["id"], "by": "Priya" }));
    assert_eq!(status, 400, "{}", r);
    assert!(r["error"].as_str().unwrap().contains("has changed since the agent compared it"), "{}", r);
    let (_, t) = get("/s/brand-two/api/trust");
    let item = t["items"].as_array().unwrap().iter().find(|x| x["name"] == "ACC-002").unwrap().clone();
    assert_eq!(item["grade"], "accepted", "the person's edit of the rule made it accepted, and the old comparison did not make it checked: {}", item);
    // a name the server cannot use is said, not silently swapped; an unknown kind is an error, not ignored
    let (_, r) = post("/s/brand-two/api/trust", serde_json::json!({ "op": "check", "name": "ACC-002", "against": "crm", "note": "open accounts only, 900 rows", "by": "priya@acme.example" }));
    assert_eq!(r["by"], "person via page", "{}", r);
    let (status, r) = post("/s/brand-two/api/trust", serde_json::json!({ "op": "accept", "kind": "banana", "name": "client" }));
    assert_eq!(status, 400, "{}", r);
    let (status, r) = post("/s/brand-two/api/trust", serde_json::json!({ "op": "accept", "kind": "metric", "name": "client" }));
    assert_eq!(status, 400);
    assert!(r["error"].as_str().unwrap().contains("there is no metric named `client`"), "{}", r);

    // a copy of the space leaves trust.yaml behind: a copy is a new place
    let (status, _) = post("/api/spaces", serde_json::json!({ "name": "Brand Three", "from": "brand-two" }));
    assert_eq!(status, 200);
    assert!(!srv.dir.join("brand-three/trust.yaml").exists());
}

#[test]
fn a_shipped_example_takes_no_grade() {
    let srv = start_with(7131, &["--no-auth"]);
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let (status, _, body) = http(srv.port, "POST", "/s/%40ecommerce/api/trust", &[JSON, PAGE, ("Origin", &origin)], r#"{"op":"accept","name":"success"}"#);
    assert_eq!(status, 403, "{}", body);
    let (status, _, body) = http(srv.port, "GET", "/s/%40ecommerce/api/trust", &[], "");
    assert_eq!(status, 200);
    assert!(body.contains("\"grade\":\"ungraded\""), "a shipped example is ungraded: {}", body);
}
