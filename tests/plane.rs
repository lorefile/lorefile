//! The home page's picture (specs/2026-11-27-home-plane.md), end to end with the real binary:
//! `GET /api/plane` on spaces written for each case. Traffic lights and their reasons, the
//! roll-up onto entities, importance, which metric counts which entity and the fields it uses,
//! the version check, waiting proposals as ghosts, never a row value, and every warning that
//! names an item carrying it as its subject.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Json = serde_json::Value;
use serde_json::json;

struct Space(PathBuf);
impl Space {
    fn new(tag: &str, files: &[(&str, &str)]) -> Space {
        let dir = std::env::temp_dir().join(format!("lorefile-plane-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (rel, text) in files {
            let p = dir.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        Space(dir)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}
impl Drop for Space {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(dir: &Path, port: u16, extra: &[&str]) -> Server {
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string(), "--no-auth", "--allow-edit"])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(100));
    }
    Server(child)
}

fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let req = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nX-Lorefile: write\r\nContent-Length: {}\r\n\r\n{}", method, path, port, body.len(), body);
    s.write_all(req.as_bytes()).unwrap();
    // bytes first: a chunk boundary can split a UTF-8 character, and read_to_string would then drop the whole reply
    let mut bytes = Vec::new();
    let _ = s.read_to_end(&mut bytes);
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    if !head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        return (status, body.to_string());
    }
    // a big answer comes in chunks: size in hex, CRLF, the bytes, CRLF
    let (mut out, mut rest) = (String::new(), body);
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    (status, out)
}

fn plane(port: u16, path: &str) -> Json {
    let (status, body) = http(port, "GET", path, "");
    assert_eq!(status, 200, "{}", body);
    serde_json::from_str(&body).unwrap()
}

fn post(port: u16, path: &str, body: Json) -> Json {
    serde_json::from_str(&http(port, "POST", path, &body.to_string()).1).unwrap_or(Json::Null)
}

fn ent<'a>(p: &'a Json, name: &str) -> &'a Json {
    p["entities"].as_array().unwrap().iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("no {} in {}", name, p["entities"]))
}

const MANIFEST: &str = "name: branda\nversion: 0.1.0\ndomains:\n  - name: Core\n    description: The main things.\n    types: []\n";
const SOURCE: &str = "connection: user_files\nkind: from_user\ndescription: Files people hand to an agent.\n";
const PROMO: &str = "type: PromoSale\ndescription: One order line sold with a promo code.\nproperties:\n  line_id: { type: string, key: true, description: One line. }\n  revenue: { type: number, unit: USD, description: What was paid. }\n  refunded: { type: boolean, description: True when refunded. }\nsource: { connection: user_files, table: promo_sales, key: line_id }\n";
const METRIC: &str = "metrics:\n  - metric: promo_sale_revenue\n    description: Revenue from promo lines, refunded ones left out.\n    source: { connection: user_files, table: promo_sales, filter: refunded = false, measure: sum(revenue) }\n    basis: transactional\n";

fn branda(tag: &str) -> Space {
    Space::new(tag, &[("ontology.yaml", MANIFEST), ("glossary.yaml", "terms: []\n"), ("sources/user_files.yaml", SOURCE), ("types/promo_sale.yaml", PROMO), ("logic/metrics.yaml", METRIC)])
}

#[test]
fn branda_reads_amber_for_its_metric_with_no_owner_and_green_once_named() {
    let sp = branda("branda");
    let port = 7240;
    let _srv = serve(&sp.0, port, &["--no-examples"]);
    // the grades of the live test: the source accepted and the metric checked, both with no name
    assert_eq!(post(port, "/api/trust", json!({ "op": "accept", "kind": "binding", "name": "PromoSale" }))["written"], true);
    assert_eq!(post(port, "/api/trust", json!({ "op": "accept", "kind": "metric", "name": "promo_sale_revenue" }))["written"], true);
    assert_eq!(post(port, "/api/trust", json!({ "op": "check", "kind": "metric", "name": "promo_sale_revenue", "against": "user_files", "note": "looked at other tables" }))["written"], true);
    let p = plane(port, "/api/plane");
    let e = ent(&p, "PromoSale");
    assert_eq!(e["light"], "amber", "{}", e);
    let reasons = e["reasons"].as_array().unwrap();
    assert_eq!(reasons.len(), 1, "exactly one reason, from the metric: {:?}", reasons);
    assert_eq!((reasons[0]["code"].as_str(), reasons[0]["from"].as_str()), (Some("W109"), Some("metric promo_sale_revenue")));
    assert!(reasons[0]["text"].as_str().unwrap().contains("no owner. Who decides what it means?"));
    assert_eq!(e["icon"], "receipt");
    assert_eq!(e["icon_why"], "name: sale");
    assert_eq!(e["source"]["grade"], "accepted");
    let m = &p["metrics"][0];
    assert_eq!((m["light"].as_str(), m["grade"].as_str()), (Some("amber"), Some("checked")));
    assert_eq!(m["entities"], json!(["PromoSale"]));
    assert_eq!(m["uses"], json!(["revenue", "refunded"]));
    assert_eq!(m["uses_how"], json!([{ "field": "revenue", "how": "adds up" }, { "field": "refunded", "how": "leaves out" }]));
    assert_eq!(m["sentence"], "Add up revenue, leaving out lines where refunded is true");
    assert_eq!(p["sources"][0]["light"], "none");
    assert_eq!(p["sources"][0]["icon"], "paperclip");
    // importance: a metric, a look, data, changed today (a grade given today, history off)
    assert_eq!(e["importance"], 10, "{}", e["importance_why"]);
    assert_eq!(e["importance_why"], json!(["counted by a metric (+4)", "needs a look (+3)", "has data (+1)", "changed this week (+2)"]));
    assert_eq!(p["counts"]["lights"]["amber"], 1);
    // never a value: the plane has names and words only
    assert!(!p.to_string().contains("looked at other tables"), "a check's note is for the card's grade line, not the picture");

    // the owner named: both green, and the bar counts one green
    assert_eq!(post(port, "/api/patch", json!({ "op": "set_metric", "metric": "promo_sale_revenue", "owner": "Vinay", "by": "Vinay" }))["written"], true);
    let p = plane(port, "/api/plane");
    assert_eq!((ent(&p, "PromoSale")["light"].as_str(), p["metrics"][0]["light"].as_str()), (Some("green"), Some("green")));
    assert_eq!(p["counts"]["lights"]["green"], 1);
}

#[test]
fn each_warning_lights_its_entity_amber_and_errors_red() {
    let sp = Space::new("lights", &[
        ("ontology.yaml", MANIFEST),
        ("glossary.yaml", "terms: []\n"),
        ("sources/files.yaml", "connection: files\nkind: csv\ndescription: CSV files here.\n"),
        ("sources/wh.yaml", "connection: wh\nkind: bigquery\ndescription: The warehouse.\n"),
        // W101: no description
        ("types/plain.yaml", "type: Plain\nproperties:\n  id: { type: string, key: true, description: The id. }\n"),
        // W103: bound, no key
        ("types/nokey.yaml", "type: NoKey\ndescription: Bound with no key.\nproperties:\n  id: { type: string, description: The id. }\nsource: { connection: wh, table: nokey }\n"),
        // W106: its file is not found
        ("types/lost.yaml", "type: Lost\ndescription: Its CSV is missing.\nproperties:\n  id: { type: string, key: true, description: The id. }\nsource: { connection: files, table: lost }\n"),
        // W105: one-of with no values
        ("types/oneof.yaml", "type: OneOf\ndescription: A one-of with no values.\nproperties:\n  id: { type: string, key: true, description: The id. }\n  state: { type: enum, description: The state. }\n"),
        // red: a link to an entity that does not exist
        ("types/broken.yaml", "type: Broken\ndescription: Links to nothing.\nproperties:\n  id: { type: string, key: true, description: The id. }\nrelationships:\n  ships_with: { to: Carrier, cardinality: many-to-one, description: The carrier. }\n"),
        // no badge: ungraded and clean
        ("types/clean.yaml", "type: Clean\ndescription: Nothing to fix.\nproperties:\n  id: { type: string, key: true, description: The id. }\n"),
        // a field without words
        ("types/quiet.yaml", "type: Quiet\ndescription: One field has no words.\nproperties:\n  id: { type: string, key: true, description: The id. }\n  note: string\n"),
    ]);
    let port = 7241;
    let _srv = serve(&sp.0, port, &["--no-examples"]);
    let p = plane(port, "/api/plane");
    let has = |name: &str, code: &str| ent(&p, name)["reasons"].as_array().map(|a| a.iter().any(|r| r["code"] == code)).unwrap_or(false);
    for (name, code) in [("Plain", "W101"), ("NoKey", "W103"), ("Lost", "W106"), ("OneOf", "W105"), ("Quiet", "fields")] {
        assert_eq!(ent(&p, name)["light"], "amber", "{}: {}", name, ent(&p, name));
        assert!(has(name, code), "{} has {}: {}", name, code, ent(&p, name)["reasons"]);
    }
    assert_eq!(ent(&p, "Lost")["reasons"][0]["text"], "Lorefile can't find lost.csv");
    assert_eq!(ent(&p, "Plain")["reasons"][0]["fix"]["op"], "set_type_description", "the card's one-field fix");
    assert_eq!(ent(&p, "Broken")["light"], "red");
    assert!(has("Broken", "E003"));
    assert_eq!(ent(&p, "Clean")["light"], "none", "ungraded and clean: no badge");
    // a source is never green; W106 for an entity bound to it turns it amber
    let files = p["sources"].as_array().unwrap().iter().find(|s| s["name"] == "files").unwrap();
    assert_eq!(files["light"], "amber", "{}", files);
    assert!(p["sources"].as_array().unwrap().iter().all(|s| s["light"] != "green"));
    // areas show the worst of their entities
    assert_eq!(p["areas"].as_array().unwrap().iter().find(|a| a["name"] == "Other").map(|a| a["light"].clone()), Some(json!("red")));
}

#[test]
fn proposed_changed_and_waiting_are_amber_and_graded_is_green() {
    let sp = branda("grades");
    sp.write("logic/metrics.yaml", &METRIC.replace("    basis: transactional\n", "    owner: Vinay\n    basis: transactional\n"));
    let port = 7242;
    let _srv = serve(&sp.0, port, &["--no-examples"]);
    // proposed by an agent: the page's tool runner writes straight in (edit mode on), marked proposed
    let r = post(port, "/api/tool", json!({ "name": "propose_patch", "arguments": { "ops": [{ "op": "add_term", "term": "promo line", "means": "PromoSale" }], "reason": "the team says it" } }));
    assert!(r["written"] == true || r.to_string().contains("written"), "{}", r);
    assert_eq!(post(port, "/api/trust", json!({ "op": "accept", "kind": "metric", "name": "promo_sale_revenue", "by": "Vinay" }))["written"], true);
    let p = plane(port, "/api/plane");
    assert_eq!(p["metrics"][0]["light"], "green", "accepted and clean: {}", p["metrics"][0]);
    assert_eq!(ent(&p, "PromoSale")["light"], "none", "its source is ungraded: no badge, not green");
    // changed since a person accepted it
    sp.write("logic/metrics.yaml", &METRIC.replace("measure: sum(revenue)", "measure: count(*)").replace("    basis: transactional\n", "    owner: Vinay\n    basis: transactional\n"));
    post(port, "/api/reload", json!({}));
    let p = plane(port, "/api/plane");
    let m = &p["metrics"][0];
    assert_eq!(m["light"], "amber", "{}", m);
    assert!(m["reasons"].as_array().unwrap().iter().any(|r| r["code"] == "W113" && r["text"].as_str().unwrap().starts_with("changed since Vinay accepted it on")), "{}", m["reasons"]);
    assert_eq!(ent(&p, "PromoSale")["light"], "amber", "the entity shows the worst of its metrics");
    // a waiting proposal that changes an entity: amber, with a Review id, and the agent's chip counts it
    let wait = Command::new(env!("CARGO_BIN_EXE_lorefile")).args(["mcp", "--dir", sp.0.to_str().unwrap(), "--no-examples"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut wait = wait;
    {
        let mut stdin = wait.stdin.take().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"propose_patch","arguments":{"ops":[{"op":"set_type_description","type":"PromoSale","description":"One promo order line."}],"reason":"clearer"}}})).unwrap();
    }
    let _ = wait.wait_with_output();
    let p = plane(port, "/api/plane");
    let e = ent(&p, "PromoSale");
    let w = e["reasons"].as_array().unwrap().iter().find(|r| r["code"] == "waiting").unwrap_or_else(|| panic!("{}", e));
    assert!(w["review"].as_str().unwrap().starts_with("p-") && w["text"].as_str().unwrap().contains("wants to change it"), "{}", w);
    assert_eq!(p["waiting"][0]["drawn"], true, "{}", p["waiting"]);
    assert_eq!(p["counts"]["waiting"], 1);
    let agent = p["agents"].as_array().unwrap().iter().find(|a| a["seen"] == "activity").unwrap_or_else(|| panic!("{}", p["agents"]));
    assert_eq!((agent["waiting"].as_u64(), agent["light"].as_str()), (Some(1), Some("amber")), "{}", agent);
    assert!(agent["recent"].as_array().unwrap().iter().any(|r| r["entity"] == "PromoSale" && r["how"] == "proposed"), "{}", agent);
}

#[test]
fn the_version_is_the_same_until_something_changes() {
    let sp = branda("version");
    let port = 7243;
    let _srv = serve(&sp.0, port, &["--no-examples"]);
    let v = plane(port, "/api/plane")["version"].as_str().unwrap().to_string();
    assert_eq!(plane(port, "/api/plane")["version"], v.as_str(), "unchanged between two calls");
    let same = plane(port, &format!("/api/plane?version={}", v));
    assert_eq!(same["same"], true, "{}", same);
    assert!(same.get("entities").is_none(), "nothing redrawn");
    assert_eq!(post(port, "/api/patch", json!({ "op": "set_metric", "metric": "promo_sale_revenue", "owner": "Vinay" }))["written"], true);
    let v2 = plane(port, "/api/plane")["version"].as_str().unwrap().to_string();
    assert_ne!(v, v2, "a patch changes it");
    // an activity line alone changes it too
    let log = sp.0.join(".lorefile/activity.jsonl");
    let mut text = fs::read_to_string(&log).unwrap_or_default();
    text.push_str("{\"at\":\"2026-11-27T10:00:00Z\",\"who\":\"agent via MCP\",\"text\":\"read\",\"file\":\"\"}\n");
    fs::write(&log, text).unwrap();
    assert_ne!(plane(port, "/api/plane")["version"].as_str().unwrap(), v2);
}

#[test]
fn at_most_five_waiting_proposals_are_drawn() {
    let sp = branda("waiting");
    let port = 7244;
    let calls: Vec<String> = (0..7).map(|i| json!({"jsonrpc":"2.0","id":i + 1,"method":"tools/call","params":{"name":"propose_patch","arguments":{"ops":[{"op":"add_type","name":format!("Thing{}", i),"description":"A thing.","properties":{"id":"string"},"key":"id"}],"reason":"more"}}}).to_string()).collect();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile")).args(["mcp", "--dir", sp.0.to_str().unwrap(), "--no-examples"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for c in &calls {
            writeln!(stdin, "{}", c).unwrap();
        }
    }
    let _ = child.wait_with_output();
    // one more whose files no longer load: its file is a broken type. Stamped an hour ahead of
    // now, so it is the newest on any date (the seven above carry the time they were made).
    let later = chrono::Utc::now() + chrono::Duration::hours(1);
    let late_id = format!("p-{}-9", later.format("%Y%m%d-%H%M%S"));
    sp.write(&format!(".lorefile/proposals/{}.json", late_id), &json!({ "id": late_id, "file": "types/bad.yaml", "content": "type: [nope", "via": "agent via MCP", "created_at": later.format("%Y-%m-%dT%H:%M:%SZ").to_string(), "status": "pending", "ok": false }).to_string());
    let _srv = serve(&sp.0, port, &["--no-examples"]);
    let p = plane(port, "/api/plane");
    let w = p["waiting"].as_array().unwrap();
    assert_eq!(w.len(), 5, "{}", p["waiting"]);
    assert_eq!(p["waiting_more"], 3);
    assert_eq!(w[0]["id"], late_id.as_str(), "newest first");
    assert_eq!(w[0]["drawn"], false, "a proposal that no longer loads can't be drawn: {}", w[0]);
    let drawn = w.iter().find(|x| x["drawn"] == true).unwrap();
    assert!(drawn["nodes"].as_array().unwrap().iter().any(|n| n["status"] == "new"), "{}", drawn);
    // the second call is served from the cache and says the same
    assert_eq!(plane(port, "/api/plane")["waiting"], p["waiting"]);
}

#[test]
fn the_picture_never_holds_a_row_value() {
    let sp = Space::new("rows", &[("ontology.yaml", MANIFEST), ("types/x.yaml", "type: X\ndescription: x.\nproperties: { id: { type: string, key: true, description: id } }\n")]);
    let port = 7245;
    let _srv = serve(&sp.0, port, &[]);
    let (status, body) = http(port, "GET", "/s/@supply-chain/api/plane", "");
    assert_eq!(status, 200, "{}", body);
    assert!(body.contains("\"Supplier\""), "the readable example is drawn");
    for value in ["Acme Corp", "INC-2301", "42000000", "Gamma Inc"] {
        assert!(!body.contains(value), "{} is a row value and must not be on the plane", value);
    }
    let p: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(p["space"]["builtin"], true);
    assert_eq!(p["space"]["editable"], false, "a shipped example has no inline fixes");
}

#[test]
fn every_warning_that_names_an_item_carries_it_as_its_subject() {
    let port = 7246;
    let sp = Space::new("subjects", &[
        ("ontology.yaml", "name: s\nversion: 0.1.0\ndomains:\n  - name: Core\n    description: x\n    types: [A, Ghost]\n"),
        ("glossary.yaml", "terms:\n  - term: thing\n    means: Nowhere\n"),
        ("sources/files.yaml", "connection: files\nkind: csv\ndescription: CSV.\n"),
        ("types/a.yaml", "type: A\nproperties:\n  id: { type: string }\n  s: { type: enum }\nsource: { connection: files, table: gone }\n"),
        ("constraints/c.yaml", "constraints:\n  - { id: C-1, name: c, applies_to: A, rule: nothere == 1, severity: block }\n"),
        ("actions/act.yaml", "action: act\ninputs: { a: A }\n"),
        ("logic/m.yaml", "metrics:\n  - metric: m\n    sql: select 1\n"),
        ("logic/f.yaml", "functions:\n  - function: f\n    inputs: {}\n    outputs: {}\n"),
    ]);
    let _srv = serve(&sp.0, port, &[]);
    let (_, spaces) = http(port, "GET", "/api/spaces", "");
    let spaces: Json = serde_json::from_str(&spaces).unwrap();
    let mine = format!("/s/{}/api/summary", spaces["spaces"].as_array().unwrap().iter().find(|s| s["builtin"] == false).unwrap()["id"].as_str().unwrap());
    let mut checked = 0;
    for path in [mine.as_str(), "/s/@supply-chain/api/summary", "/s/@ecommerce/api/summary", "/s/@saas-pricing/api/summary", "/s/@b2b-sales/api/summary"] {
        let (_, body) = http(port, "GET", path, "");
        let sum: Json = serde_json::from_str(&body).unwrap();
        for d in sum["diagnostics"].as_array().unwrap() {
            let code = d["code"].as_str().unwrap();
            if code.starts_with('W') && code != "W112" {
                assert!(d.get("subject").map(|s| s["kind"].is_string() && s["name"].is_string()).unwrap_or(false), "{} has no subject: {}", path, d);
                checked += 1;
            }
        }
    }
    assert!(checked >= 8, "the crafted space raises warnings of many kinds: {}", checked);
    let (_, body) = http(port, "GET", &mine, "");
    let sum: Json = serde_json::from_str(&body).unwrap();
    let subject = |code: &str| sum["diagnostics"].as_array().unwrap().iter().find(|d| d["code"] == code).map(|d| d["subject"].clone()).unwrap_or(Json::Null);
    assert_eq!(subject("W105"), json!({ "kind": "entity", "name": "A" }), "a field's subject is its entity");
    assert_eq!(subject("W102"), json!({ "kind": "guardrail", "name": "C-1" }));
    assert_eq!(subject("W108"), json!({ "kind": "term", "name": "thing" }));
    assert_eq!(subject("W109"), json!({ "kind": "metric", "name": "m" }));
    assert_eq!(subject("W111"), json!({ "kind": "function", "name": "f" }));
}

/// The size and speed budget for a big space: release builds only (`cargo test --release -- --ignored`).
#[test]
#[ignore]
fn a_space_of_two_thousand_entities_is_drawn_quickly() {
    let mut files: Vec<(String, String)> = vec![("sources/wh.yaml".into(), "connection: wh\nkind: bigquery\ndescription: The warehouse.\n".into())];
    let names: Vec<String> = (0..2000).map(|i| format!("Thing{}", i)).collect();
    for (i, n) in names.iter().enumerate() {
        let rels: String = (0..8).map(|j| format!("  r{}: {{ to: {}, cardinality: many-to-one, description: link }}\n", j, names[(i * 7 + j * 13 + 1) % 2000])).collect();
        files.push((format!("types/t{}.yaml", i), format!("type: {}\ndescription: One thing.\nproperties:\n  id: {{ type: string, key: true, description: The id. }}\nsource: {{ connection: wh, table: t{}, key: id }}\nrelationships:\n{}", n, i, rels)));
    }
    let metrics: String = (0..50).map(|k| format!("  - metric: m{}\n    description: M.\n    owner: team\n    source: {{ connection: wh, table: t{}, measure: count(*) }}\n", k, k * 3)).collect();
    files.push(("logic/metrics.yaml".into(), format!("metrics:\n{}", metrics)));
    files.push(("ontology.yaml".into(), "name: big\nversion: 0.1.0\n".into()));
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let sp = Space::new("big", &refs);
    let port = 7247;
    let _srv = serve(&sp.0, port, &["--no-examples"]);
    let _ = http(port, "GET", "/api/plane", "");
    let t = Instant::now();
    let (status, body) = http(port, "GET", "/api/plane", "");
    let took = t.elapsed();
    assert_eq!(status, 200);
    eprintln!("2,000 entities: {} bytes in {:?}", body.len(), took);
    assert!(took < Duration::from_millis(400), "{:?}", took);
}
