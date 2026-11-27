//! Reviewing what agents propose (specs/2026-11-27-review-flow.md), end to end with the real
//! binary: agents queue proposals over stdio with edit mode off, and a person reviews and decides
//! on a page server started with `--allow-edit`. The review screen's answer, Accept with its
//! order of checks (fresh, replayed, refused in words, the fingerprint, the fixes), two Accepts at
//! once, Not now with a why, Look again, Take it back with history off and on, a read-only server,
//! errors that are no longer queued, and `learn_file` asking first.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Json = serde_json::Value;
use serde_json::json;

/// A blank space like the one `lorefile new` makes for Branda: one domain, no types yet.
struct Space(PathBuf);
impl Space {
    fn new(tag: &str) -> Space {
        let dir = std::env::temp_dir().join(format!("lorefile-review-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("types")).unwrap();
        fs::write(dir.join("ontology.yaml"), "name: branda\nversion: 0.1.0\ndomains:\n  - name: Core\n    description: The main things this business works with.\n    types: []\n").unwrap();
        fs::write(dir.join("glossary.yaml"), "# Business words and what they mean here.\nterms: []\n").unwrap();
        Space(dir)
    }
    fn read(&self, rel: &str) -> Option<String> {
        fs::read_to_string(self.0.join(rel)).ok()
    }
    fn proposal(&self, id: &str) -> Json {
        serde_json::from_str(&fs::read_to_string(self.0.join(format!(".lorefile/proposals/{}.json", id))).unwrap()).unwrap()
    }
    fn proposals(&self) -> Vec<Json> {
        fs::read_dir(self.0.join(".lorefile/proposals")).map(|rd| rd.map(|e| serde_json::from_str(&fs::read_to_string(e.unwrap().path()).unwrap()).unwrap()).collect()).unwrap_or_default()
    }
    fn activity(&self) -> Vec<String> {
        self.read(".lorefile/activity.jsonl").unwrap_or_default().lines().filter_map(|l| serde_json::from_str::<Json>(l).ok()).map(|j| j["text"].as_str().unwrap_or("").to_string()).collect()
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
        .args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string(), "--no-auth", "--no-examples"])
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

fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, Json) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let req = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nX-Lorefile: write\r\nContent-Length: {}\r\n\r\n{}",
        method, path, port, body.len(), body
    );
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, serde_json::from_str(body).unwrap_or(Json::Null))
}

fn get(port: u16, path: &str) -> (u16, Json) {
    http(port, "GET", path, "")
}

fn post(port: u16, path: &str, body: Json) -> (u16, Json) {
    http(port, "POST", path, &body.to_string())
}

fn review(port: u16, id: &str) -> Json {
    let (status, r) = get(port, &format!("/api/proposals/{}/review", id));
    assert_eq!(status, 200, "{}", r);
    r
}

/// An agent over stdio, edit mode off: its saves wait for a person.
fn agent(dir: &Path, calls: &[(&str, Json)]) -> Vec<Json> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile")).args(["mcp", "--dir", dir.to_str().unwrap(), "--no-examples"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for (i, (name, args)) in calls.iter().enumerate() {
            writeln!(stdin, "{}", json!({"jsonrpc":"2.0","id":i + 1,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| {
            let r: Json = serde_json::from_str(l).unwrap();
            let t = r["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
            let mut j = serde_json::from_str::<Json>(&t).unwrap_or(json!({ "text": t }));
            j["is_error"] = json!(r["result"]["isError"].as_bool().unwrap_or(false));
            j
        })
        .collect()
}

fn file_args(file: &str, file_name: &str) -> Json {
    let mut rdr = csv::Reader::from_path(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/learn").join(file)).unwrap();
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(|s| s.to_string()).collect();
    let rows: Vec<Vec<String>> = rdr.records().map(|r| r.unwrap().iter().map(|s| s.to_string()).collect()).collect();
    let words = |h: &str| match h {
        "line_id" => "Identifier of one order line.",
        "revenue" => "What the customer paid for the line.",
        "refunded" => "True when the line was refunded.",
        _ => "A column of the monthly promo-sales export.",
    };
    let columns: Vec<Json> = headers.iter().enumerate().map(|(i, h)| json!({ "name": h, "examples": rows.iter().take(5).map(|r| r[i].clone()).collect::<Vec<_>>(), "description": words(h) })).collect();
    json!({ "file_name": file_name, "columns": columns, "entity": "PromoSale", "description": "One order line sold with a promo code." })
}

fn learn() -> Json {
    let mut a = file_args("promo_sales_2026-11.csv", "promo_sales.csv");
    a["answers"] = json!({ "success": { "column": "revenue", "how": "sum" }, "exclude": [{ "column": "refunded", "equals": true }], "exclude_words": "Refunded lines never count.", "currency": "USD", "also_called": ["promo report"] });
    a["reason"] = json!("The user said: revenue counts, refunded lines don't.");
    a
}

fn queued(r: &Json) -> String {
    r["queued"].as_str().unwrap_or_else(|| panic!("not queued: {}", r)).to_string()
}

#[test]
fn a_learned_file_reads_by_meaning_and_accept_takes_the_owner_fix() {
    let sp = Space::new("meaning");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let p = sp.proposal(&id);
    // the batch headings: each file carries its own ops' sentences, not one by position
    let heading = |f: &str| p["files"].as_array().unwrap().iter().find(|x| x["file"] == f).map(|x| x["summary"].as_str().unwrap().to_string()).unwrap_or_default();
    assert_eq!(heading("types/promo_sale.yaml"), "created PromoSale; bound PromoSale to user_files.promo_sales");
    assert!(heading("logic/metrics.yaml").starts_with("added metric promo_sale_revenue"), "{}", heading("logic/metrics.yaml"));
    assert!(heading("glossary.yaml").starts_with("added the word \"promo report\""), "{}", heading("glossary.yaml"));
    assert_eq!(p["replay"]["kind"], "learn", "a learned file keeps how to make it again");
    assert!(p["replay"]["input"]["columns"][0].get("examples").is_none(), "never its examples: {}", p["replay"]);

    let port = 7190;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let r = review(port, &id);
    assert_eq!(r["headline"], "An agent wants Lorefile to remember a file");
    assert_eq!(r["counts"]["adds"][0], "1 entity (Promo sale, 14 fields)");
    let metric = r["changes"].as_array().unwrap().iter().find(|c| c["kind"] == "metric").unwrap();
    assert_eq!(metric["sentence"], "add up revenue, leaving out lines where refunded is true", "it says lines: the key is line_id");
    assert_eq!(metric["preview"], "Lorefile can't preview this metric: the rows stay with the agent.");
    let q = &r["questions"][0];
    assert_eq!((q["code"].as_str(), q["fix"]["op"].as_str()), (Some("W109"), Some("set_metric")), "{}", r["questions"]);
    assert_eq!(r["can"]["accept"], true);
    let fp = r["fingerprint"].as_str().unwrap().to_string();
    assert!(r["notes"].as_array().unwrap().iter().any(|n| n["kind"] == "undo" && n["text"].as_str().unwrap().contains("take this back")));

    // a fix the screen did not offer is refused; nothing is written
    let (status, bad) = post(port, "/api/proposals/accept", json!({ "id": id, "by": "Vinay", "seen": fp, "fixes": [{ "op": "set_metric", "metric": "promo_sale_revenue", "caveats": "x" }] }));
    assert_eq!(status, 400, "{}", bad);
    assert!(sp.read("types/promo_sale.yaml").is_none());
    // Accept with the owner: one change, credited to the person
    let (status, ok) = post(port, "/api/proposals/accept", json!({ "id": id, "by": "Vinay", "seen": fp, "fixes": [{ "op": "set_metric", "metric": "promo_sale_revenue", "owner": "Vinay" }] }));
    assert_eq!(status, 200, "{}", ok);
    assert!(sp.read("logic/metrics.yaml").unwrap().contains("owner: Vinay"));
    assert!(!ok["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "W109"), "W109 is gone: {}", ok);
    assert!(sp.read("trust.yaml").unwrap().contains("promo_sale_revenue: { grade: accepted, by: \"Vinay via page\""));
    let p = sp.proposal(&id);
    assert_eq!((p["status"].as_str(), p["accepted_by"].as_str()), (Some("accepted"), Some("Vinay via page")));
    assert_eq!(p["fixes"][0], "owner of promo_sale_revenue set to Vinay");
    let last = sp.activity().pop().unwrap();
    assert_eq!(last, "Vinay accepted an agent's promo_sales.csv file: Promo sale, Promo sale revenue and promo report (owner of promo_sale_revenue set to Vinay)");
    // a second Accept is told what happened, in words
    let (status, again) = post(port, "/api/proposals/accept", json!({ "id": id, "by": "Priya" }));
    assert_eq!(status, 409);
    assert!(again["error"].as_str().unwrap().contains("accepted by Vinay a moment ago"), "{}", again);
    // the linked states: accepted shows who, and a proposal that does not exist says so
    assert_eq!(review(port, &id)["decided"]["accepted_by"], "Vinay");
    let (status, missing) = get(port, "/api/proposals/p-nope/review");
    assert_eq!((status, missing["missing"].as_bool()), (404, Some(true)));
}

#[test]
fn a_space_that_moved_on_replays_op_made_proposals_and_refuses_whole_file_ones() {
    let sp = Space::new("moved");
    let source = json!({ "op": "add_source", "connection": "shop_warehouse", "kind": "bigquery", "project": "acme", "dataset": "shop", "description": "The warehouse." });
    let voucher = json!({ "op": "add_type", "name": "Voucher", "description": "A gift voucher.", "properties": { "code": "string" }, "key": "code" });
    let r = agent(&sp.0, &[("learn_file", learn()), ("propose_patch", json!({ "ops": [source, voucher], "reason": "two ops" }))]);
    let (learned, ops) = (queued(&r[0]), queued(&r[1]));
    assert_eq!(sp.proposal(&ops)["replay"]["kind"], "ops");
    let port = 7191;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);

    // Vinay adds a metric of his own: logic/metrics.yaml changed since the agent proposed
    fs::create_dir_all(sp.0.join("logic")).unwrap();
    fs::write(sp.0.join("logic/metrics.yaml"), "metrics:\n  - metric: gross_margin\n    description: Revenue minus cost.\n    owner: finance\n    sql: select 1\n").unwrap();
    let rv = review(port, &learned);
    let note = rv["notes"].as_array().unwrap().iter().find(|n| n["kind"] == "replayed").unwrap_or_else(|| panic!("{}", rv["notes"]));
    assert!(note["text"].as_str().unwrap().contains("logic/metrics.yaml changed"), "{}", note);
    assert_eq!(rv["can"]["accept"], true, "{}", rv["can"]);
    // an Accept from an older screen is refused; with the screen's fingerprint it writes both metrics
    let (status, old) = post(port, "/api/proposals/accept", json!({ "id": learned, "by": "Vinay" }));
    assert_eq!(status, 409, "{}", old);
    let (status, ok) = post(port, "/api/proposals/accept", json!({ "id": learned, "by": "Vinay", "seen": rv["fingerprint"] }));
    assert_eq!(status, 200, "{}", ok);
    let metrics = sp.read("logic/metrics.yaml").unwrap();
    assert!(metrics.contains("gross_margin") && metrics.contains("promo_sale_revenue"), "nothing lost: {}", metrics);

    // Voucher now exists another way: the ops no longer plan, refused in words
    fs::write(sp.0.join("types/voucher.yaml"), "type: Voucher\ndescription: A voucher.\nproperties:\n  code: { type: string, key: true }\n").unwrap();
    let rv = review(port, &ops);
    assert_eq!(rv["can"]["accept"], false);
    let stale = rv["notes"].as_array().unwrap().iter().find(|n| n["kind"] == "stale").unwrap_or_else(|| panic!("{}", rv["notes"]));
    assert!(stale["text"].as_str().unwrap().contains("already exists"), "{}", stale);
    let (status, refused) = post(port, "/api/proposals/accept", json!({ "id": ops, "by": "Vinay", "seen": "x" }));
    assert_eq!(status, 409, "{}", refused);
    assert!(sp.read("sources/shop_warehouse.yaml").is_none(), "nothing written");
}

#[test]
fn a_whole_file_proposal_on_a_changed_file_is_refused_in_words() {
    let sp = Space::new("whole");
    fs::write(sp.0.join("types/customer.yaml"), "type: Customer\ndescription: Someone who buys.\nproperties:\n  customer_id: { type: string, key: true }\n").unwrap();
    let id = queued(&agent(&sp.0, &[("propose_change", json!({ "file": "glossary.yaml", "content": "terms:\n  - term: patron\n    means: Customer\n", "reason": "the team says patron" }))])[0]);
    let port = 7192;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    assert_eq!(review(port, &id)["headline"], "An agent suggests an also-called word: \"patron\"");
    fs::write(sp.0.join("glossary.yaml"), "terms:\n  - term: client\n    means: Customer\n").unwrap();
    let rv = review(port, &id);
    assert_eq!(rv["can"]["accept"], false);
    let stale = rv["notes"].as_array().unwrap().iter().find(|n| n["kind"] == "stale").unwrap();
    assert!(stale["text"].as_str().unwrap().contains("replaces the whole file"), "{}", stale);
    assert!(stale["copy"].as_str().unwrap().contains("propose it again"), "a sentence to copy for the agent");
    let (status, r) = post(port, "/api/proposals/accept", json!({ "id": id }));
    assert_eq!(status, 409, "{}", r);
    assert!(sp.read("glossary.yaml").unwrap().contains("client"), "the newer change survived");
}

#[test]
fn two_accepts_at_once_one_writes_and_the_other_is_told() {
    let sp = Space::new("race");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let port = 7193;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let fp = review(port, &id)["fingerprint"].clone();
    let go = |who: &'static str| {
        let (id, fp) = (id.clone(), fp.clone());
        std::thread::spawn(move || post(port, "/api/proposals/accept", json!({ "id": id, "by": who, "seen": fp })))
    };
    let (a, b) = (go("Vinay"), go("Priya"));
    let mut results = vec![a.join().unwrap(), b.join().unwrap()];
    results.sort_by_key(|r| r.0);
    assert_eq!((results[0].0, results[1].0), (200, 409), "{:?}", results);
    assert!(results[1].1["error"].as_str().unwrap().contains("accepted by"), "{}", results[1].1);
    assert_eq!(sp.proposals().iter().filter(|p| p["status"] == "accepted").count(), 1);
}

#[test]
fn not_now_look_again_and_take_it_back_with_history_off() {
    let sp = Space::new("decide");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let port = 7194;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    // Not now, with a why the agent will see
    let (status, r) = post(port, "/api/proposals/dismiss", json!({ "id": id, "by": "Vinay", "why": "not until finance agrees on refunds" }));
    assert_eq!(status, 200, "{}", r);
    let p = sp.proposal(&id);
    assert_eq!((p["status"].as_str(), p["decided_by"].as_str(), p["why"].as_str()), (Some("set_aside"), Some("Vinay via page"), Some("not until finance agrees on refunds")));
    assert!(sp.activity().last().unwrap().starts_with("Vinay set aside an agent's promo_sales.csv file"), "who set it aside is recorded: {:?}", sp.activity());
    let m = &agent(&sp.0, &[("match_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv"))])[0];
    assert_eq!(m["set_aside"]["why"], "not until finance agrees on refunds", "{}", m);
    assert!(m["say"].as_str().unwrap().contains("set aside by Vinay"), "{}", m);
    // Look again: it waits again
    let (status, r) = post(port, "/api/proposals/reopen", json!({ "id": id, "by": "Vinay" }));
    assert_eq!(status, 200, "{}", r);
    assert_eq!(sp.proposal(&id)["status"], "pending");

    // Accept, then Take it back: the files and trust.yaml as they were, byte for byte, and it waits again
    let glossary = sp.read("glossary.yaml").unwrap();
    let fp = review(port, &id)["fingerprint"].clone();
    assert_eq!(post(port, "/api/proposals/accept", json!({ "id": id, "by": "Vinay", "seen": fp })).0, 200);
    assert!(sp.read("trust.yaml").is_some());
    let rv = review(port, &id);
    assert_eq!(rv["take_back"]["possible"], true, "{}", rv["take_back"]);
    let (status, r) = post(port, "/api/proposals/take_back", json!({ "id": id, "by": "Vinay" }));
    assert_eq!(status, 200, "{}", r);
    assert_eq!(sp.read("glossary.yaml").unwrap(), glossary);
    for f in ["types/promo_sale.yaml", "logic/metrics.yaml", "sources/user_files.yaml", "trust.yaml"] {
        assert!(sp.read(f).is_none(), "{} is gone again", f);
    }
    let p = sp.proposal(&id);
    assert_eq!((p["status"].as_str(), p["taken_back_by"].as_str()), (Some("pending"), Some("Vinay via page")));

    // after another change to one of its files, there is no taking back, and the reason is given
    let fp = review(port, &id)["fingerprint"].clone();
    assert_eq!(post(port, "/api/proposals/accept", json!({ "id": id, "by": "Vinay", "seen": fp })).0, 200);
    let mut g = sp.read("glossary.yaml").unwrap();
    g.push_str("  - term: promo export\n    means: PromoSale\n");
    fs::write(sp.0.join("glossary.yaml"), g).unwrap();
    let rv = review(port, &id);
    assert_eq!(rv["take_back"]["possible"], false);
    assert!(rv["take_back"]["why_not"].as_str().unwrap().contains("glossary.yaml has changed since"), "{}", rv["take_back"]);
    assert_eq!(post(port, "/api/proposals/take_back", json!({ "id": id })).0, 409);
}

#[test]
fn a_replaced_proposal_cannot_be_looked_at_again_or_accepted() {
    let sp = Space::new("replaced");
    let mut more = learn();
    more["answers"]["also_called"] = json!(["promo report", "promo export"]);
    let r = agent(&sp.0, &[("learn_file", learn()), ("learn_file", more)]);
    let (old, new) = (queued(&r[0]), queued(&r[1]));
    assert_eq!(r[1]["replaced"], old.as_str());
    let port = 7195;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let note = review(port, &new)["notes"].as_array().unwrap().iter().find(|n| n["kind"] == "replaces").cloned().unwrap();
    assert!(note["text"].as_str().unwrap().contains("promo export"), "says what the new one does differently: {}", note);
    let (status, r) = post(port, "/api/proposals/accept", json!({ "id": old }));
    assert_eq!(status, 409);
    assert!(r["error"].as_str().unwrap().contains("replaced by a newer version"), "{}", r);
    let (status, r) = post(port, "/api/proposals/reopen", json!({ "id": old }));
    assert_eq!(status, 409);
    assert!(r["error"].as_str().unwrap().contains(&new), "{}", r);
}

#[test]
fn a_read_only_server_shows_the_review_and_refuses_every_decision() {
    let sp = Space::new("readonly");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let port = 7196;
    let _srv = serve(&sp.0, port, &[]);
    let rv = review(port, &id);
    assert_eq!(rv["can"]["accept"], false);
    assert!(rv["can"]["why_not"].as_str().unwrap().contains("started without --allow-edit"), "{}", rv["can"]);
    for what in ["accept", "dismiss", "reopen", "take_back"] {
        let (status, r) = post(port, &format!("/api/proposals/{}", what), json!({ "id": id }));
        assert_eq!(status, 403, "{}: {}", what, r);
    }
    assert_eq!(sp.proposal(&id)["status"], "pending");
}

#[test]
fn errors_are_not_queued_and_a_waiting_name_is_pointed_at() {
    let sp = Space::new("errors");
    let r = agent(&sp.0, &[
        ("learn_file", learn()),
        ("propose_change", json!({ "file": "types/order.yaml", "content": "type: Order\ndescription: One checkout.\nproperties:\n  order_id: { type: string, key: true }\nsource:\n  connection: nowhere\n  table: orders\n", "reason": "x" })),
        ("propose_patch", json!({ "op": "add_term", "term": "promo report two", "means": "PromoSale" })),
    ]);
    let waiting = queued(&r[0]);
    assert!(r[1]["queued"].is_null() && r[1]["errors"].as_u64().unwrap_or(0) > 0, "not queued, and the errors come back: {}", r[1]);
    assert!(r[1]["note"].as_str().unwrap().contains("nothing was written or queued"), "{}", r[1]);
    assert!(r[2]["is_error"].as_bool().unwrap(), "{}", r[2]);
    assert!(r[2]["text"].as_str().unwrap().contains(&format!("PromoSale is waiting for review in {}", waiting)), "{}", r[2]);
    assert_eq!(sp.proposals().len(), 1, "only the clean one waits");
}

#[test]
fn learn_file_asks_first_and_saves_without_a_number_only_in_the_users_words() {
    let sp = Space::new("asks");
    let mut shape = file_args("promo_sales_2026-11.csv", "promo_sales.csv");
    shape["answers"] = json!({});
    let mut said = shape.clone();
    said["skip_because"] = json!("just save it, I'll say which number later");
    let r = agent(&sp.0, &[("learn_file", shape), ("learn_file", said)]);
    assert_eq!(r[0]["saved"], false, "{}", r[0]);
    assert!(r[0]["next"].as_str().unwrap().starts_with("Ask the user these questions now"), "{}", r[0]);
    assert!(r[0]["questions"].as_array().map(|q| !q.is_empty()).unwrap_or(false));
    assert_eq!(r[1]["saved"], true, "{}", r[1]);
    let p = sp.proposal(&queued(&r[1]));
    assert!(p["reason"].as_str().unwrap().contains("just save it, I'll say which number later"), "{}", p["reason"]);
    // the review screen asks which number counts, and its answer is written as learn_file would
    let port = 7197;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let rv = review(port, p["id"].as_str().unwrap());
    let q = rv["questions"].as_array().unwrap().iter().find(|q| q["code"] == "which_number").cloned().unwrap_or_else(|| panic!("{}", rv["questions"]));
    assert!(q["fix"]["numbers"].as_array().unwrap().iter().any(|n| n["name"] == "revenue"));
    let fix = json!({ "op": "add_metric", "entity": "PromoSale", "how": "sum", "column": "revenue", "leave_out": "refunded" });
    let (status, ok) = post(port, "/api/proposals/accept", json!({ "id": p["id"], "by": "Vinay", "seen": rv["fingerprint"], "fixes": [fix] }));
    assert_eq!(status, 200, "{}", ok);
    let m = sp.read("logic/metrics.yaml").unwrap();
    assert!(m.contains("metric: promo_sale_revenue") && m.contains("filter: refunded = false") && m.contains("measure: sum(revenue)"), "{}", m);
}

#[test]
fn the_map_shows_the_space_as_it_would_be() {
    let sp = Space::new("map");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let port = 7198;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let (status, g) = get(port, &format!("/api/graph?proposal={}", id));
    assert_eq!(status, 200, "{}", g);
    assert_eq!(g["nodes"][0]["name"], "PromoSale");
    assert_eq!(g["nodes"][0]["status"], "new");
    assert_eq!(review(port, &id)["map"], true);
}

#[test]
fn a_review_link_survives_the_sign_in() {
    let sp = Space::new("link");
    let port = 7199;
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", sp.0.to_str().unwrap(), "--port", &port.to_string(), "--no-examples", "--token", "review-link-token-0123456789abcdef"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _srv = Server(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let raw = |req: String| {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    };
    let page = raw(format!("GET /s/branda/?review=p-20261127-080758-6 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", port));
    assert!(page.contains("name=\"next\" value=\"/s/branda/?review=p-20261127-080758-6\""), "the form carries the review link: {}", page);
    assert!(page.contains("location.pathname + location.search + location.hash"), "the cookie retry keeps it too");
    assert!(page.contains("id=\"me\"") && page.contains("Your name") && page.contains("localStorage.setItem('lorefile-name'"), "an optional name on the sign-in page, kept in this browser");
    // a form that sends a name too still signs in: the server takes the token and ignores the rest
    let form = "token=review-link-token-0123456789abcdef&me=Vinay&next=%2Fs%2Fbranda%2F%3Freview%3Dp-20261127-080758-6";
    let back = raw(format!("POST /api/login HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{}", port, form.len(), form));
    assert!(back.to_lowercase().contains("location: /s/branda/?review=p-20261127-080758-6"), "{}", back);
}

#[test]
fn a_learned_file_whose_entity_came_another_way_replays_to_just_the_difference() {
    let sp = Space::new("difference");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    // PromoSale and its source arrive another way (accepted from another proposal) before this one
    let p = sp.proposal(&id);
    for f in p["files"].as_array().unwrap() {
        let file = f["file"].as_str().unwrap();
        if file == "types/promo_sale.yaml" || file == "sources/user_files.yaml" {
            fs::create_dir_all(sp.0.join(file).parent().unwrap()).unwrap();
            fs::write(sp.0.join(file), f["content"].as_str().unwrap()).unwrap();
        }
    }
    let port = 7189;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let rv = review(port, &id);
    assert!(rv["notes"].as_array().unwrap().iter().any(|n| n["kind"] == "replayed"), "{}", rv["notes"]);
    let kinds: Vec<(String, String)> = rv["changes"].as_array().unwrap().iter().map(|c| (c["kind"].as_str().unwrap().to_string(), c["op"].as_str().unwrap().to_string())).collect();
    assert!(!kinds.iter().any(|(k, _)| k == "entity" || k == "source"), "only what is still missing: {:?}", kinds);
    assert!(kinds.contains(&("metric".to_string(), "add".to_string())), "{:?}", kinds);
    let (status, ok) = post(port, "/api/proposals/accept", json!({ "id": id, "by": "Vinay", "seen": rv["fingerprint"] }));
    assert_eq!(status, 200, "{}", ok);
    assert!(sp.read("logic/metrics.yaml").unwrap().contains("promo_sale_revenue"));
}

#[test]
fn with_history_on_take_it_back_is_that_commits_undo() {
    if Command::new("git").arg("--version").output().map(|o| !o.status.success()).unwrap_or(true) {
        return;
    }
    let sp = Space::new("history");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let port = 7188;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let (status, r) = post(port, "/api/git/init", json!({}));
    assert_eq!(status, 200, "{}", r);
    let rv = review(port, &id);
    assert!(rv["notes"].as_array().unwrap().iter().any(|n| n["kind"] == "undo" && n["text"].as_str().unwrap().contains("History is on")), "{}", rv["notes"]);
    let (status, ok) = post(port, "/api/proposals/accept", json!({ "id": id, "by": "Vinay", "seen": rv["fingerprint"] }));
    assert_eq!(status, 200, "{}", ok);
    let p = sp.proposal(&id);
    assert!(p["commit"].is_string() && p.get("snapshot").is_none(), "a commit, no snapshot: {}", p);
    let log = Command::new("git").args(["log", "-1", "--format=%an|%cn|%s"]).current_dir(&sp.0).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "agent via MCP|Vinay via page|An agent wants Lorefile to remember a file", "the agent the author, the person the committer, the headline the message");
    assert_eq!(review(port, &id)["take_back"]["undo"], p["commit"]);
    let (status, r) = post(port, "/api/proposals/take_back", json!({ "id": id, "by": "Vinay" }));
    assert_eq!(status, 200, "{}", r);
    assert!(sp.read("types/promo_sale.yaml").is_none());
    assert_eq!(sp.proposal(&id)["status"], "pending");
}

// ---------------------------------------------------------------------------
// specs/2026-11-27-ui-fixes.md: one set of words, metric sentences, names on decisions
// ---------------------------------------------------------------------------

/// Learn the promo file and accept it the way Vinay did in the live test: no name, no owner.
fn accepted_with_no_name(sp: &Space, port: u16) -> String {
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let fp = review(port, &id)["fingerprint"].as_str().unwrap().to_string();
    let (status, ok) = post(port, "/api/proposals/accept", json!({ "id": id, "seen": fp, "fixes": [] }));
    assert_eq!(status, 200, "{}", ok);
    id
}

fn trust_line(sp: &Space, name: &str) -> String {
    sp.read("trust.yaml").unwrap_or_default().lines().find(|l| l.trim_start().starts_with(&format!("{}:", name))).unwrap_or("").trim().to_string()
}

#[test]
fn the_summary_says_metrics_in_words_and_a_blank_space_reads_empty() {
    let sp = Space::new("words");
    let port = 7230;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let (_, spaces) = get(port, "/api/spaces");
    let me = &spaces["spaces"][0];
    assert_eq!((me["empty"].as_bool(), me["ok"].as_bool()), (Some(true), Some(false)), "a blank space is empty, not broken: {}", me);

    accepted_with_no_name(&sp, port);
    let (_, sum) = get(port, "/api/summary");
    let m = sum["metrics"].as_array().unwrap().iter().find(|m| m["metric"] == "promo_sale_revenue").unwrap();
    assert_eq!(m["sentence"], "Add up revenue, leaving out lines where refunded is true");
    assert_eq!(m["label"], "Promo sale revenue");
    let (_, spaces) = get(port, "/api/spaces");
    assert_eq!((spaces["spaces"][0]["empty"].as_bool(), spaces["spaces"][0]["types"].as_u64()), (Some(false), Some(1)), "the picker is fresh after Accept");

    // a metric in SQL has no sentence and keeps its SQL, for Details
    fs::write(sp.0.join("logic/extra.yaml"), "metrics:\n  - metric: gross_margin\n    description: Revenue minus cost.\n    owner: finance\n    sql: select sum(revenue) - sum(cost) from orders\n").unwrap();
    post(port, "/api/reload", json!({}));
    let (_, sum) = get(port, "/api/summary");
    let g = sum["metrics"].as_array().unwrap().iter().find(|m| m["metric"] == "gross_margin").unwrap();
    assert!(g["sentence"].is_null() && g["sql"].as_str().unwrap().contains("sum(cost)"), "{}", g);
}

#[test]
fn naming_an_owner_keeps_a_check_and_who_gave_it() {
    let sp = Space::new("owner");
    let port = 7231;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    accepted_with_no_name(&sp, port);
    let (status, ck) = post(port, "/api/trust", json!({ "op": "check", "kind": "metric", "name": "promo_sale_revenue", "against": "user_files", "note": "looked at other tables" }));
    assert_eq!(status, 200, "{}", ck);
    let say = |port: u16| { let (_, t) = get(port, "/api/summary"); t["trust"]["items"].as_array().unwrap().iter().find(|x| x["name"] == "promo_sale_revenue").unwrap()["say"].clone() };
    let said = say(port);
    let (status, ok) = post(port, "/api/patch", json!({ "op": "set_metric", "metric": "promo_sale_revenue", "owner": "Vinay", "by": "Vinay" }));
    assert_eq!(status, 200, "the page sends its name as by, and set_metric takes it: {}", ok);
    assert!(sp.read("logic/metrics.yaml").unwrap().contains("owner: Vinay"));
    let line = trust_line(&sp, "promo_sale_revenue");
    assert!(line.contains("grade: checked") && line.contains("by: \"person via page\"") && line.contains("note: \"looked at other tables\""), "{}", line);
    let (_, sum) = get(port, "/api/summary");
    assert!(!sum["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "W113" || d["code"] == "W109"), "{}", sum["diagnostics"]);
    assert_eq!(say(port), said, "agents say the same as before");
    // the binding is not touched by a metric's owner
    let binding = trust_line(&sp, "PromoSale");
    assert!(binding.contains("grade: accepted") && binding.contains("person via page"), "{}", binding);
}

#[test]
fn a_name_goes_only_on_a_grade_saved_without_one() {
    let sp = Space::new("sign");
    let port = 7232;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    accepted_with_no_name(&sp, port);
    post(port, "/api/trust", json!({ "op": "check", "kind": "metric", "name": "promo_sale_revenue", "against": "user_files", "note": "looked at other tables" }));
    let before = trust_line(&sp, "PromoSale");
    // no name, or one that reads as an agent: refused, nothing written
    for by in [json!(null), json!("agent smith")] {
        let (status, r) = post(port, "/api/trust", json!({ "op": "sign", "kind": "binding", "name": "PromoSale", "by": by }));
        assert!(status != 200 || r["written"] != true, "{}", r);
        assert!(r["error"].as_str().or(r["refused"].as_str()).unwrap_or("").contains("type your name first"), "{}", r);
    }
    assert_eq!(trust_line(&sp, "PromoSale"), before);
    // the binding: only `by` changes
    let (status, r) = post(port, "/api/trust", json!({ "op": "sign", "kind": "binding", "name": "PromoSale", "by": "Priya" }));
    assert_eq!((status, r["written"].as_bool()), (200, Some(true)), "{}", r);
    assert_eq!(trust_line(&sp, "PromoSale"), before.replace("person via page", "Priya via page"));
    assert!(r["summary"].as_str().unwrap().starts_with("Priya put their name on source of PromoSale (accepted "), "{}", r);
    assert!(sp.activity().last().unwrap().contains("with no name"));
    // the checked metric keeps its source and note
    let m = trust_line(&sp, "promo_sale_revenue");
    let (_, r) = post(port, "/api/trust", json!({ "op": "sign", "kind": "metric", "name": "promo_sale_revenue", "by": "Priya" }));
    assert_eq!(r["written"], true, "{}", r);
    assert_eq!(trust_line(&sp, "promo_sale_revenue"), m.replace("person via page", "Priya via page"));
    // a named record is never renamed
    let (_, r) = post(port, "/api/trust", json!({ "op": "sign", "kind": "binding", "name": "PromoSale", "by": "Vinay" }));
    assert!(r["error"].as_str().or(r["refused"].as_str()).unwrap_or("").contains("already recorded as Priya's"), "{}", r);
    assert!(trust_line(&sp, "PromoSale").contains("Priya via page"));
    // changed since it was graded: look again instead; the warning says "source of"
    let t = sp.read("types/promo_sale.yaml").unwrap();
    fs::write(sp.0.join("types/promo_sale.yaml"), t.replace("table: promo_sales", "table: promo_sales_v2")).unwrap();
    post(port, "/api/reload", json!({}));
    let (_, sum) = get(port, "/api/summary");
    let w113 = sum["diagnostics"].as_array().unwrap().iter().find(|d| d["code"] == "W113").unwrap_or_else(|| panic!("{}", sum["diagnostics"]));
    assert!(w113["message"].as_str().unwrap().starts_with("source of PromoSale changed since it was accepted"), "{}", w113);
}

#[test]
fn a_decision_made_without_a_name_takes_one_with_its_grades() {
    let sp = Space::new("signp");
    let port = 7233;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let id = accepted_with_no_name(&sp, port);
    assert_eq!(review(port, &id)["can"]["sign"], true);
    post(port, "/api/trust", json!({ "op": "check", "kind": "metric", "name": "promo_sale_revenue", "against": "user_files", "note": "looked at other tables" }));
    let why = review(port, &id)["take_back"]["why_not"].clone();
    let (status, r) = post(port, "/api/proposals/sign", json!({ "id": id, "by": "Vinay" }));
    assert_eq!((status, r["written"].as_bool()), (200, Some(true)), "{}", r);
    assert_eq!(sp.proposal(&id)["accepted_by"], "Vinay via page");
    assert!(trust_line(&sp, "PromoSale").contains("Vinay via page") && trust_line(&sp, "promo_sale_revenue").contains("Vinay via page"), "one save names both records");
    assert!(trust_line(&sp, "promo_sale_revenue").contains("grade: checked"));
    let last = sp.activity().pop().unwrap();
    assert!(last.starts_with("Vinay put their name on an agent's promo_sales.csv file") && last.contains("(accepted ") && last.ends_with("with no name)"), "{}", last);
    let rv = review(port, &id);
    assert_eq!((rv["decided"]["accepted_by"].as_str(), rv["can"]["sign"].as_bool()), (Some("Vinay"), Some(false)));
    assert_eq!(rv["take_back"]["why_not"], why, "Take it back stays refused for the same reason");
    let (status, again) = post(port, "/api/proposals/sign", json!({ "id": id, "by": "Priya" }));
    assert_eq!(status, 409, "{}", again);

    // a fresh Accept with no name: a name alone never blocks Take it back
    let sp2 = Space::new("signp2");
    let port2 = 7234;
    let _srv2 = serve(&sp2.0, port2, &["--allow-edit"]);
    let id2 = accepted_with_no_name(&sp2, port2);
    let (_, r) = post(port2, "/api/proposals/sign", json!({ "id": id2, "by": "Vinay" }));
    assert_eq!(r["written"], true, "{}", r);
    assert_eq!(review(port2, &id2)["take_back"]["possible"], true);
    let (_, back) = post(port2, "/api/proposals/take_back", json!({ "id": id2, "by": "Vinay" }));
    assert_eq!(back["written"], true, "{}", back);
}

#[test]
fn review_words_name_entities_metrics_and_sources() {
    let sp = Space::new("kinds");
    let id = queued(&agent(&sp.0, &[("learn_file", learn())])[0]);
    let port = 7235;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    let r = review(port, &id);
    let adds = r["counts"]["adds"].to_string();
    assert!(adds.contains("entity") && adds.contains("metric") && adds.contains("source"), "{}", adds);
    for old in ["thing", "number", "place rows", "column"] {
        assert!(!adds.contains(old), "{} still says {}", adds, old);
    }
    let metric = r["changes"].as_array().unwrap().iter().find(|c| c["kind"] == "metric").unwrap();
    assert_eq!(metric["label"], "Promo sale revenue", "the metric's title is its label");
}

#[test]
fn naming_an_owner_keeps_an_accepted_grade_and_accepts_unverified_or_changed() {
    let sp = Space::new("ownergrades");
    let w = |rel: &str, text: &str| { let p = sp.0.join(rel); fs::create_dir_all(p.parent().unwrap()).unwrap(); fs::write(p, text).unwrap(); };
    w("sources/user_files.yaml", "connection: user_files\nkind: from_user\ndescription: Files people hand to an agent.\n");
    w("types/promo_sale.yaml", "type: PromoSale\ndescription: One line.\nproperties:\n  line_id: { type: string, key: true, description: One line. }\n  revenue: { type: number, description: Paid. }\nsource: { connection: user_files, table: promo_sales, key: line_id }\n");
    let metrics = |c_measure: &str| format!("metrics:\n  - metric: kept\n    description: k\n    source: {{ connection: user_files, table: promo_sales, measure: sum(revenue) }}\n  - metric: moved\n    description: m\n    source: {{ connection: user_files, table: promo_sales, measure: {} }}\n", c_measure);
    w("logic/metrics.yaml", &metrics("sum(revenue)"));
    let port = 7236;
    let _srv = serve(&sp.0, port, &["--allow-edit"]);
    for m in ["kept", "moved"] {
        assert_eq!(post(port, "/api/trust", json!({ "op": "accept", "kind": "metric", "name": m, "by": "Priya" })).1["written"], true);
    }
    // changed since: edited by hand after Priya accepted it
    w("logic/metrics.yaml", &metrics("count(*)"));
    // unverified: written by the page's tool runner (an agent, edits on)
    let (_, r) = post(port, "/api/tool", json!({ "name": "propose_patch", "arguments": { "ops": [{ "op": "add_metric", "metric": "fresh", "description": "f", "connection": "user_files", "table": "promo_sales", "measure": "count(*)" }], "reason": "r" } }));
    assert!(r["isError"] != true, "{}", r);
    post(port, "/api/reload", json!({}));
    assert!(trust_line(&sp, "moved").is_empty() || get(port, "/api/summary").1["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "W113"), "moved reads changed since");
    assert!(trust_line(&sp, "fresh").contains("grade: proposed"), "{}", trust_line(&sp, "fresh"));
    for m in ["kept", "moved", "fresh"] {
        let (status, r) = post(port, "/api/patch", json!({ "op": "set_metric", "metric": m, "owner": "Vinay", "by": "Vinay" }));
        assert_eq!(status, 200, "{}", r);
        let line = trust_line(&sp, m);
        // specs/2026-11-27-people-areas-graph.md decision 9: who stands behind it stays when only the owner changes
        let by = if m == "kept" { "Priya via page" } else { "Vinay via page" };
        assert!(line.contains("grade: accepted") && line.contains(&format!("by: \"{}\"", by)), "{}: {}", m, line);
    }
    let (_, sum) = get(port, "/api/summary");
    assert!(!sum["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "W113"), "{}", sum["diagnostics"]);
    // an owner written into the file by hand is a change behind the grade's back
    let text = sp.read("logic/metrics.yaml").unwrap().replace("owner: Vinay", "owner: Priya");
    w("logic/metrics.yaml", &text);
    post(port, "/api/reload", json!({}));
    let (_, sum) = get(port, "/api/summary");
    assert!(sum["diagnostics"].as_array().unwrap().iter().any(|d| d["code"] == "W113"), "{}", sum["diagnostics"]);
}
