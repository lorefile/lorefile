//! MCP over Streamable HTTP, end to end: the real binary serving `POST /mcp`
//! with a bearer token. The same conversation `tests/mcp_loop.rs` has over
//! stdio (initialize, tools/list, summary, a propose_patch queued in words),
//! the same answers to `initialize` and `tools/list` as stdio gives, writes
//! under `--allow-edit`, `space`, batches and notifications, and no token, no MCP.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "mcp-test-token-0123456789abcdef";

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

fn scratch_copy(example: &str, port: u16) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorefile-mcp-http-{}-{}-{}", example, port, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(example), &dir);
    dir
}

struct Server { child: Child, port: u16, dir: PathBuf }
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn start(dir: PathBuf, port: u16, extra: &[&str]) -> Server {
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string(), "--token", TOKEN])
        .args(extra)
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(100));
    }
    Server { child, port, dir }
}

/// One raw HTTP/1.1 request; returns (status, headers in lower case, body).
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut req = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Length: {}\r\n", method, path, port, body.len());
    for (k, v) in headers { req.push_str(&format!("{}: {}\r\n", k, v)); }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    // bytes first: a chunk boundary can split a UTF-8 character, and read_to_string would then drop the whole reply
    let mut bytes = Vec::new();
    let _ = s.read_to_end(&mut bytes);
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, head.to_lowercase(), body.to_string())
}

/// What an MCP client sends with every message.
fn client_headers(bearer: &str) -> [(&'static str, String); 3] {
    [("Authorization", bearer.to_string()), ("Content-Type", "application/json".into()), ("Accept", "application/json, text/event-stream".into())]
}

/// POST one JSON-RPC message (or a batch) to `path` with the token; returns (status, reply).
fn rpc_at(port: u16, path: &str, msg: &serde_json::Value) -> (u16, serde_json::Value) {
    let h = client_headers(&format!("Bearer {}", TOKEN));
    let hs: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (status, head, body) = http(port, "POST", path, &hs, &msg.to_string());
    if status == 200 {
        assert!(head.contains("content-type: application/json"), "a plain JSON reply, no event stream:\n{}", head);
    }
    let reply = if body.trim().is_empty() { serde_json::Value::Null } else { serde_json::from_str(&body).unwrap_or_else(|e| panic!("not JSON ({}): {}", e, body)) };
    (status, reply)
}

fn rpc(port: u16, msg: serde_json::Value) -> serde_json::Value {
    let (status, reply) = rpc_at(port, "/mcp", &msg);
    assert_eq!(status, 200, "{}", reply);
    reply
}

fn text(reply: &serde_json::Value) -> String {
    reply["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()
}

/// The stdio server's replies to the same requests, to compare with.
fn stdio(dir: &Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["mcp", "--dir", dir.to_str().unwrap(), "--no-examples"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests { writeln!(stdin, "{}", r).unwrap(); }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn an_agent_over_http_reads_the_summary_and_its_saves_are_queued_in_words() {
    let srv = start(scratch_copy("ecommerce", 7110), 7110, &["--no-examples"]);
    let initialize = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}});
    let list = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
    let init = rpc(srv.port, initialize.clone());
    let tools_reply = rpc(srv.port, list.clone());
    // the client says it is ready: a notification, accepted with nothing to say
    let (status, reply) = rpc_at(srv.port, "/mcp", &serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    assert_eq!((status, reply), (202, serde_json::Value::Null));
    let summary = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"summary","arguments":{}}})));
    let saved = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_metric","metric":"trial_starts","description":"Trials on the SaaS line.","connection":"shop_bigquery","table":"analytics.ga4_events","filter":"event_name = 'trial_started'","measure":"count distinct user_pseudo_id","grain":"day","owner":"growth@lumenhome","reason":"The user said trials are the win for SaaS."}}})));
    let described = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"describe","arguments":{"name":"forecast_success"}}})));
    let refused = rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_metric","metric":"bad","connection":"meta_ads"}}}));

    // the assertions tests/mcp_loop.rs makes over stdio
    assert_eq!(init["id"], 1);
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("offer to save it with `propose_patch`"), "the offer-to-save sentence is in the instructions");
    let tools: Vec<&str> = tools_reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tools.contains(&"propose_patch") && tools.contains(&"validate") && tools.contains(&"where_is"), "{:?}", tools);
    assert!(summary.contains("## Metrics") && summary.contains("**success**"), "the summary names the success metric:\n{}", summary);
    assert!(summary.contains("## Functions") && summary.contains("forecast_success"), "and the forecasting function");
    assert!(saved.contains("queued for review"), "read-only server queues the save: {}", saved);
    assert!(saved.contains("\"summary\": \"added metric trial_starts"), "{}", saved);
    assert!(described.contains("mmm_run_budget_optimizer") && described.contains("\"kind\": \"function\""), "{}", described);
    assert!(refused["result"]["isError"].as_bool().unwrap_or(false), "an MCP source without a tool is refused");
    let proposals: Vec<PathBuf> = fs::read_dir(srv.dir.join(".lorefile/proposals")).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(proposals.len(), 1, "exactly one proposal was queued");
    let p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&proposals[0]).unwrap()).unwrap();
    assert_eq!(p["status"], "pending");
    assert_eq!(p["summary"], "added metric trial_starts (shop_bigquery.analytics.ga4_events)");
    assert_eq!(p["via"], "agent via MCP", "the same author as over stdio");

    // and stdio, on the same folder, answers initialize and tools/list identically
    let from_stdio = stdio(&srv.dir, &[initialize, list]);
    assert_eq!(from_stdio[0]["result"], init["result"], "initialize: same capabilities and instructions");
    assert_eq!(from_stdio[1]["result"], tools_reply["result"], "tools/list: same tools");

    // a batch comes back as an array, one reply per request, none for the notification
    let (status, batch) = rpc_at(srv.port, "/mcp", &serde_json::json!([
        {"jsonrpc":"2.0","method":"notifications/initialized"},
        {"jsonrpc":"2.0","id":"a","method":"ping"},
        {"jsonrpc":"2.0","id":"b","method":"tools/call","params":{"name":"describe","arguments":{"name":"success"}}}
    ]));
    assert_eq!(status, 200, "{}", batch);
    let ids: Vec<&str> = batch.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["a", "b"]);
    let (status, reply) = rpc_at(srv.port, "/mcp", &serde_json::json!({"jsonrpc":"2.0","id":7,"method":"no/such"}));
    assert_eq!(status, 200);
    assert_eq!(reply["error"]["code"], -32601, "{}", reply);
}

#[test]
fn without_the_token_there_is_no_mcp_and_the_transport_answers_by_the_rules() {
    let srv = start(scratch_copy("supply-chain", 7111), 7111, &["--no-examples"]);
    let ping = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    let (status, head, body) = http(srv.port, "POST", "/mcp", &[("Content-Type", "application/json")], &ping);
    assert_eq!(status, 401, "{}", body);
    assert!(head.contains("www-authenticate: bearer"), "{}", head);
    let (status, _, _) = http(srv.port, "POST", "/mcp", &[("Content-Type", "application/json"), ("Authorization", "Bearer wrong")], &ping);
    assert_eq!(status, 401, "a wrong token is the same as none");
    let (status, _, _) = http(srv.port, "POST", "/mcp", &[("Content-Type", "application/json"), ("Cookie", &format!("lorefile-{}={}", srv.port, TOKEN))], &ping);
    assert_eq!(status, 401, "the page's cookie does not open MCP: clients send the header");

    let bearer = format!("Bearer {}", TOKEN);
    let auth = ("Authorization", bearer.as_str());
    let (status, head, _) = http(srv.port, "GET", "/mcp", &[auth], "");
    assert_eq!(status, 405, "no server-to-client event stream");
    assert!(head.contains("allow: post, delete"), "{}", head);
    assert_eq!(http(srv.port, "DELETE", "/mcp", &[auth], "").0, 204, "no sessions to end");
    assert_eq!(http(srv.port, "POST", "/mcp", &[auth, ("Content-Type", "text/plain")], &ping).0, 415, "JSON only");
    assert_eq!(http(srv.port, "POST", "/mcp", &[auth, ("Content-Type", "application/json"), ("Origin", "https://evil.example")], &ping).0, 403, "the Origin is checked");
    let (status, _, body) = http(srv.port, "POST", "/mcp", &[auth, ("Content-Type", "application/json")], "{not json");
    assert_eq!(status, 400);
    assert!(body.contains("-32700"), "a JSON-RPC parse error: {}", body);
    let big = format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"params\":{{\"x\":\"{}\"}}}}", "x".repeat(5 * 1024 * 1024));
    assert_eq!(http(srv.port, "POST", "/mcp", &[auth, ("Content-Type", "application/json")], &big).0, 413, "the 4 MB cap holds here too");
}

#[test]
fn edit_mode_and_spaces_work_over_http_as_they_do_over_stdio() {
    // --allow-edit: a proposal is written and the next call sees it
    let srv = start(scratch_copy("supply-chain", 7112), 7112, &["--no-examples", "--allow-edit"]);
    let saved = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_term","term":"seller","means":"Supplier","reason":"Finance says seller."}}})));
    assert!(saved.contains("written to glossary.yaml"), "{}", saved);
    assert!(fs::read_to_string(srv.dir.join("glossary.yaml")).unwrap().contains("- term: seller"));
    let described = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"describe","arguments":{"name":"seller"}}})));
    assert!(described.contains("Supplier"), "the server reloaded and resolves the new word: {}", described);
    drop(srv);

    // with the shipped examples: `space` picks one, and a shipped one takes no proposals
    let srv = start(scratch_copy("supply-chain", 7113), 7113, &["--allow-edit"]);
    let tools = rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"spaces"), "{:?}", names);
    let summary = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"summary","arguments":{"space":"@ecommerce"}}})));
    assert!(summary.contains("**success**"), "the ecommerce example's summary: {}", summary);
    let refused = rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"propose_patch","arguments":{"space":"@ecommerce","op":"add_term","term":"x","means":"Order"}}}));
    assert!(refused["result"]["isError"].as_bool().unwrap_or(false) && text(&refused).contains("read-only"), "{}", refused);
    // /s/<id>/mcp serves that one space, like `lorefile mcp --space <id>`
    let (status, one) = rpc_at(srv.port, "/s/%40ecommerce/mcp", &serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}));
    assert_eq!(status, 200, "{}", one);
    let names: Vec<&str> = one["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(!names.contains(&"spaces"), "one space, no `spaces` tool: {:?}", names);
    let (status, _) = rpc_at(srv.port, "/s/nowhere/mcp", &serde_json::json!({"jsonrpc":"2.0","id":5,"method":"ping"}));
    assert_eq!(status, 404);
}

#[test]
fn over_http_trust_yaml_is_refused_and_an_agents_save_is_proposed() {
    let srv = start(scratch_copy("supply-chain", 7114), 7114, &["--no-examples", "--allow-edit"]);
    let refused = rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"propose_change","arguments":{"file":"trust.yaml","content":"trust: {}\n"}}}));
    assert!(refused["result"]["isError"].as_bool().unwrap_or(false) && text(&refused).contains("trust.yaml is written by Lorefile"), "{}", refused);
    assert!(!srv.dir.join(".lorefile/proposals").exists() || fs::read_dir(srv.dir.join(".lorefile/proposals")).unwrap().count() == 0, "nothing queued");
    let saved = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_term","term":"seller","means":"Supplier"}}})));
    assert!(saved.contains("Grade: proposed"), "{}", saved);
    assert!(fs::read_to_string(srv.dir.join("trust.yaml")).unwrap().contains("seller: { grade: proposed, by: \"agent via MCP\""));
    let described = text(&rpc(srv.port, serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"describe","arguments":{"name":"seller"}}})));
    let d: serde_json::Value = serde_json::from_str(&described).unwrap();
    assert_eq!(d["word"]["trust"]["grade"], "proposed", "{}", described);
}

#[test]
fn server_discover_is_method_not_found_and_every_tool_says_whether_it_only_reads() {
    let srv = start(scratch_copy("ecommerce", 7115), 7115, &["--no-examples"]);
    // openai-mcp sends server/discover first: a JSON-RPC error in a 200, never a 401 or 404
    let (status, reply) = rpc_at(srv.port, "/mcp", &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"server/discover"}));
    assert_eq!(status, 200, "{}", reply);
    assert_eq!(reply["error"]["code"], -32601, "{}", reply);
    assert_eq!(reply["error"]["message"], "method not found: server/discover");
    // as a notification (no id) it is accepted with nothing to say
    let h = client_headers(&format!("Bearer {}", TOKEN));
    let hs: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (status, _, body) = http(srv.port, "POST", "/mcp", &hs, &serde_json::json!({"jsonrpc":"2.0","method":"server/discover"}).to_string());
    assert_eq!((status, body.as_str()), (202, ""));

    // tools/list: every tool carries annotations; reads say read-only, the writes do not
    let list = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
    let over_http = rpc(srv.port, list.clone());
    let over_stdio = stdio(&srv.dir, &[list]).remove(0);
    for reply in [&over_http, &over_stdio] {
        let tools = reply["result"]["tools"].as_array().unwrap();
        let reads = ["search", "describe", "neighbors", "path", "constraints_for", "validate", "where_is", "instances", "summary", "query", "tables", "match_file"];
        for t in tools {
            let name = t["name"].as_str().unwrap();
            let a = &t["annotations"];
            assert!(a.is_object(), "{} has annotations", name);
            assert_eq!(a["openWorldHint"], false, "{} reaches nothing outside", name);
            if reads.contains(&name) {
                assert_eq!(a["readOnlyHint"], true, "{} only reads", name);
            } else {
                assert_eq!(a["readOnlyHint"], false, "{} is a write", name);
            }
        }
        let by_name = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap_or_else(|| panic!("{} listed", n))["annotations"].clone();
        assert_eq!(by_name("propose_patch"), serde_json::json!({"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}));
        assert_eq!(by_name("learn_file"), serde_json::json!({"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}));
        assert_eq!(by_name("propose_change"), serde_json::json!({"readOnlyHint": false, "openWorldHint": false}), "keeps the default destructiveHint: it can replace a whole file");
        assert_eq!(by_name("match_file")["readOnlyHint"], true);
    }
    assert_eq!(over_http["result"], over_stdio["result"], "the same list either way");
}

/// A blank space like Branda's, and what an agent sends for one of the promo-sales fixtures.
fn blank_branda(port: u16) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorefile-mcp-http-branda-{}-{}", port, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("types")).unwrap();
    fs::write(dir.join("ontology.yaml"), "name: branda\ndomains:\n  - name: Core\n    types: []\n").unwrap();
    fs::write(dir.join("glossary.yaml"), "terms: []\n").unwrap();
    dir
}

fn promo_args(file: &str, file_name: &str) -> serde_json::Value {
    let mut rdr = csv::Reader::from_path(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/learn").join(file)).unwrap();
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(|s| s.to_string()).collect();
    let rows: Vec<Vec<String>> = rdr.records().map(|r| r.unwrap().iter().map(|s| s.to_string()).collect()).collect();
    let columns: Vec<serde_json::Value> = headers.iter().enumerate().map(|(i, h)| serde_json::json!({ "name": h, "examples": rows.iter().take(5).map(|r| r[i].clone()).collect::<Vec<_>>() })).collect();
    serde_json::json!({ "file_name": file_name, "row_count": rows.len(), "columns": columns })
}

#[test]
fn learned_over_stdio_accepted_on_the_page_and_read_over_stdio_next_month() {
    let dir = blank_branda(7116);
    let call = |id: u64, name: &str, args: serde_json::Value| serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}});
    let mut learn = promo_args("promo_sales_2026-11.csv", "promo_sales.csv");
    learn["answers"] = serde_json::json!({ "success": { "column": "revenue" }, "exclude": [{ "column": "refunded", "equals": true }], "exclude_words": "Refunded lines never count." });
    let r = stdio(&dir, &[call(1, "match_file", promo_args("promo_sales_2026-11.csv", "promo_sales.csv")), call(2, "learn_file", learn)]);
    let saved: serde_json::Value = serde_json::from_str(&text(&r[1])).unwrap();
    assert!(text(&r[0]).contains("\"match\": \"new\""), "{}", text(&r[0]));
    let id = saved["queued"].as_str().unwrap_or_else(|| panic!("queued: {}", saved)).to_string();
    assert!(saved.get("accept_at").is_none(), "over stdio the server does not know the page's address");

    // the page, with edits allowed: Accept
    let srv = start(dir.clone(), 7116, &["--no-examples", "--allow-edit"]);
    let bearer = format!("Bearer {}", TOKEN);
    let (status, _, body) = http(srv.port, "POST", "/api/proposals/accept", &[("Authorization", &bearer), ("Content-Type", "application/json"), ("X-Lorefile", "write")], &serde_json::json!({ "id": id, "by": "Vinay" }).to_string());
    assert_eq!(status, 200, "{}", body);
    assert!(body.contains("\"written\":true"), "{}", body);
    // MCP over HTTP from this machine gets a link to the page
    let mut again = promo_args("promo_sales_2026-12.csv", "promo_sales_2026-12.csv");
    again["answers"] = serde_json::json!({});
    let over_http: serde_json::Value = serde_json::from_str(&text(&rpc(srv.port, call(3, "learn_file", again)))).unwrap();
    assert_eq!(over_http["saved"], false, "December is the same file: nothing to save: {}", over_http);

    // next month, over stdio (the server keeps running; dropping it removes the folder): where the rows are, and December read the same way
    let r = stdio(&dir, &[call(4, "where_is", serde_json::json!({ "type": "PromoSale" })), call(5, "match_file", promo_args("promo_sales_2026-12.csv", "promo_sales_2026-12.csv"))]);
    assert!(text(&r[0]).contains("arrive as a file a person gives you"), "{}", text(&r[0]));
    let dec: serde_json::Value = serde_json::from_str(&text(&r[1])).unwrap();
    assert_eq!(dec["match"], "same");
    assert_eq!(dec["read_it"][0]["filter"], "refunded = false");
    assert_eq!(dec["read_it"][0]["trust"]["grade"], "accepted");
    assert_eq!(dec["read_it"][0]["trust"]["by"], "Vinay via page");
    drop(srv);
}
