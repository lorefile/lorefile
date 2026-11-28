//! People (specs/2026-11-27-people-areas-graph.md, Part 1), end to end with the real binary: the
//! list worked out from the space, `set_owner` and the grades it keeps, rename and merge with one
//! commit and Undo, `people.yaml` and its warnings, and what agents see.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Json = serde_json::Value;
use serde_json::json;

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("lorefile-people-it-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.0.join(rel)).unwrap_or_default()
    }
}
impl Drop for Dir {
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

fn serve(dir: &Path, port: u16, history: bool) -> Server {
    let mut args = vec!["serve", "--dir", dir.to_str().unwrap(), "--no-auth", "--no-examples", "--allow-edit"];
    if !history {
        args.push("--no-commit-edits");
    }
    let port_s = port.to_string();
    args.extend(["--port", &port_s]);
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile")).args(&args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(100));
    }
    Server(child)
}

fn http(port: u16, method: &str, path: &str, body: &str) -> Json {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let req = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nX-Lorefile: write\r\nContent-Length: {}\r\n\r\n{}", method, path, port, body.len(), body);
    s.write_all(req.as_bytes()).unwrap();
    // bytes first: a chunk boundary can split a UTF-8 character, and read_to_string would then drop the whole reply
    let mut bytes = Vec::new();
    let _ = s.read_to_end(&mut bytes);
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let (mut out, mut rest) = (String::new(), body);
        while let Some((size, after)) = rest.split_once("\r\n") {
            let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            if n == 0 {
                break;
            }
            out.push_str(&after[..n]);
            rest = after[n..].trim_start_matches("\r\n");
        }
        out
    } else {
        body.to_string()
    };
    serde_json::from_str(&body).unwrap_or_else(|_| Json::String(body))
}
fn get(port: u16, path: &str) -> Json {
    http(port, "GET", path, "")
}
fn post(port: u16, path: &str, body: Json) -> Json {
    http(port, "POST", path, &body.to_string())
}
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
fn has_git() -> bool {
    Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}
/// (grade, by, at) of one metric, from GET /api/trust.
fn grade(port: u16, prefix: &str, name: &str) -> (String, String, String) {
    let t = get(port, &format!("{}/api/trust", prefix));
    let it = t["items"].as_array().unwrap().iter().find(|i| i["kind"] == "metric" && i["name"] == name).cloned().unwrap_or(Json::Null);
    let changed = it.get("changed_since").map(|c| c.as_str().unwrap_or("").to_string());
    match changed {
        Some(c) => ("changed since".into(), it["was_by"].as_str().unwrap_or("").into(), c),
        None => (it["grade"].as_str().unwrap_or("").into(), it["by"].as_str().unwrap_or("").into(), it["at"].as_str().unwrap_or("").into()),
    }
}

const ONT: &str = "name: brand\nversion: 0.1.0\n";
const SOURCES: &str = "sources:\n  - connection: shop\n    kind: csv\n    description: The shop's exports.\n";
const ORDER: &str = "type: Order\ndescription: One checkout.\nowner: Vinay\nproperties:\n  order_id: { type: string, key: true, description: The id. }\nsource: { connection: shop, table: orders, key: order_id }\n";
const METRICS: &str = "metrics:\n  - metric: m_checked\n    description: Checked one.\n    sql: select 1\n    owner: vinay # who decides\n  - metric: m_accepted\n    description: Accepted one.\n    sql: select 2\n    owner: Vinay\n  - metric: m_changed\n    description: Changed one.\n    sql: select 3\n    owner: Vinay\n  - metric: m_ungraded\n    description: Ungraded one.\n    sql: select 4\n    owner: Vinay\n";
const ACTION: &str = "action: refund_order\ndescription: Refund an order.\ninputs: { order: Order }\nrequires_approval_from: Vinay above $10k; Finance otherwise\n";

/// Grades in the states the spec names: checked and accepted by Priya, an agent's unverified
/// metric, one changed since it was accepted, and one never graded.
fn grade_them(port: u16, p: &str, dir: &Path) {
    for n in ["m_checked", "m_accepted", "m_changed"] {
        assert_eq!(post(port, &format!("{}/api/trust", p), json!({ "op": "accept", "kind": "metric", "name": n, "by": "Priya" }))["written"], true);
    }
    let c = post(port, &format!("{}/api/trust", p), json!({ "op": "check", "kind": "metric", "name": "m_checked", "against": "shop", "note": "412 paid orders, the same as Shopify", "by": "Priya" }));
    assert_eq!(c["written"], true, "{}", c);
    let a = post(port, &format!("{}/api/tool", p), json!({ "name": "propose_patch", "arguments": { "op": "add_metric", "metric": "m_proposed", "description": "An agent's.", "sql": "select 5", "owner": "Vinay", "reason": "the user said so" } }));
    assert!(a.to_string().contains("written") || a.to_string().contains("m_proposed"), "{}", a);
    // changed since: the file edited by hand after Priya accepted it
    let f = dir.join("logic/metrics.yaml");
    let t = fs::read_to_string(&f).unwrap().replace("Changed one.", "Changed by hand.");
    fs::write(&f, t).unwrap();
    post(port, &format!("{}/api/reload", p), json!({}));
}

#[test]
fn naming_an_owner_keeps_a_grade_a_person_gave_and_accepts_the_rest() {
    let d = Dir::new("set-owner");
    for (r, t) in [("ontology.yaml", ONT), ("sources/shop.yaml", SOURCES), ("types/order.yaml", ORDER), ("logic/metrics.yaml", METRICS), ("logic/actions.yaml", ACTION)] {
        d.write(r, t);
    }
    let port = 7310;
    let _srv = serve(&d.0, port, false);
    grade_them(port, "", &d.0);
    assert_eq!(grade(port, "", "m_proposed").0, "proposed");
    assert_eq!(grade(port, "", "m_changed").0, "changed since");
    let (_, _, checked_at) = grade(port, "", "m_checked");
    for n in ["m_checked", "m_accepted", "m_proposed", "m_changed", "m_ungraded"] {
        let r = post(port, "/api/patch", json!({ "op": "set_owner", "kind": "metric", "name": n, "owner": "Sam", "by": "Vinay" }));
        assert_eq!(r["written"], true, "{}: {}", n, r);
        assert_eq!(r["summary"], format!("made Sam the owner of metric {}", n));
    }
    let g = |n| grade(port, "", n);
    assert_eq!(g("m_checked"), ("checked".into(), "Priya via page".into(), checked_at.clone()), "checked: kept, same who and when");
    assert_eq!((g("m_accepted").0, g("m_accepted").1), ("accepted".into(), "Priya via page".into()), "accepted: kept in Priya's name, not re-accepted by Vinay");
    for n in ["m_proposed", "m_changed", "m_ungraded"] {
        assert_eq!((g(n).0, g(n).1), ("accepted".into(), "Vinay via page".into()), "{}: accepted by the person who saved", n);
    }
    assert!(d.read("logic/metrics.yaml").contains("owner: Sam # who decides"), "a line edit keeps the comment:\n{}", d.read("logic/metrics.yaml"));
    // an entity and a guardrail too; an address isn't a new name
    assert_eq!(post(port, "/api/patch", json!({ "op": "set_owner", "kind": "entity", "name": "Order", "owner": "Growth team", "by": "Vinay" }))["written"], true);
    assert!(d.read("types/order.yaml").contains("owner: Growth team"));
    let bad = post(port, "/api/patch", json!({ "op": "set_owner", "kind": "metric", "name": "m_checked", "owner": "sam@shop.example", "by": "Vinay" }));
    assert_ne!(bad["written"], true, "{}", bad);
    // an agent naming an owner on an accepted metric waits for a person
    let a = post(port, "/api/tool", json!({ "name": "propose_patch", "arguments": { "op": "set_owner", "kind": "metric", "name": "m_accepted", "owner": "Priya", "reason": "Priya runs it now" } }));
    let text = a["content"][0]["text"].as_str().unwrap_or("").to_string();
    assert!(text.contains("queued"), "{}", text);
    // its review: the grade stays (only the owner changes), so no "changed since" question for it
    let id = get(port, "/api/proposals")["proposals"].as_array().unwrap().iter().rev().find(|p| p["status"] == "pending").unwrap()["id"].as_str().unwrap().to_string();
    let rv = get(port, &format!("/api/proposals/{}/review", id));
    assert!(rv["notes"].to_string().contains("M accepted is accepted by Priya and stays so: only its owner changes."), "{}", rv["notes"]);
    assert!(!rv["questions"].to_string().contains("W113"), "{}", rv["questions"]);
    assert!(d.read("logic/metrics.yaml").contains("owner: Sam\n  - metric: m_changed") || !d.read("logic/metrics.yaml").contains("owner: Priya"), "nothing written");
    // a batch from the page: "Give them owners"
    let b = post(port, "/api/patch", json!({ "by": "Vinay", "ops": [
        { "op": "set_owner", "kind": "entity", "name": "Order", "owner": "Priya" },
        { "op": "set_owner", "kind": "metric", "name": "m_ungraded", "owner": "Priya" },
    ] }));
    assert_eq!(b["written"], true, "{}", b);
    assert_eq!(b["summaries"].as_array().unwrap().len(), 2);
    // approvers are sentences with conditions, never people
    let people = get(port, "/api/people");
    let names: Vec<&str> = people["people"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Priya") && names.contains(&"Sam"), "{:?}", names);
    assert!(!names.iter().any(|n| n.contains("above") || n.contains("Finance")), "approvers aren't people: {:?}", names);
    assert!(!names.iter().any(|n| n.to_lowercase().contains("agent")), "{:?}", names);
    assert_eq!(people["editable"], true);
}

#[test]
fn rename_rewrites_owners_keeps_every_grade_and_undoes_in_one_step() {
    if !has_git() {
        return;
    }
    let root = Dir::new("rename");
    for (r, t) in [("brand/ontology.yaml", ONT), ("brand/sources/shop.yaml", SOURCES), ("brand/types/order.yaml", ORDER), ("brand/logic/metrics.yaml", METRICS), ("brand/logic/actions.yaml", ACTION)] {
        root.write(r, t);
    }
    root.write("other/ontology.yaml", "name: other\n");
    root.write("other/types/thing.yaml", "type: Thing\ndescription: A thing.\nowner: Sam\nproperties:\n  thing_id: { type: string, key: true, description: The id. }\n");
    let port = 7311;
    let _srv = serve(&root.0, port, true);
    let p = "/s/brand";
    assert_eq!(post(port, &format!("{}/api/git/init", p), json!({}))["ok"], true);
    let brand = root.0.join("brand");
    grade_them(port, p, &brand);
    post(port, &format!("{}/api/git/commit", p), json!({ "message": "A hand edit" }));
    let before: Vec<(String, String, String)> = ["m_checked", "m_accepted", "m_proposed", "m_changed"].iter().map(|n| grade(port, p, n)).collect();
    let people = get(port, &format!("{}/api/people", p));
    assert_eq!(people["other_spaces"], json!(["Sam"]), "names from the other spaces here: {}", people["other_spaces"]);
    let vinay = people["people"].as_array().unwrap().iter().find(|x| x["name"] == "Vinay").cloned().unwrap();
    assert_eq!(vinay["kind"], "person");
    assert_eq!(vinay["owns_words"], "5 metrics · 1 entity", "{}", vinay);
    // the confirm counts what changes before anything is written
    let plan = get(port, &format!("{}/api/people/plan?from=Vinay&to=Vinay%20Karode", p));
    assert_eq!(plan["counts_words"], "5 metrics, 1 entity", "{}", plan);
    assert_eq!(plan["merge"], false);
    let r = post(port, &format!("{}/api/people", p), json!({ "op": "rename", "from": "Vinay", "to": "Vinay Karode", "by": "Priya" }));
    assert_eq!(r["written"], true, "{}", r);
    assert_eq!(r["summary"], "Priya renamed Vinay to Vinay Karode: 5 metrics, 1 entity");
    let m = fs::read_to_string(brand.join("logic/metrics.yaml")).unwrap();
    assert!(!m.contains("owner: Vinay\n") && m.contains("owner: Vinay Karode # who decides"), "every spelling rewritten, comments kept:\n{}", m);
    assert!(fs::read_to_string(brand.join("logic/actions.yaml")).unwrap().contains("requires_approval_from: Vinay above $10k"), "approvers left alone");
    let pf = fs::read_to_string(brand.join("people.yaml")).unwrap();
    assert!(pf.contains("name: Vinay Karode") && pf.contains("- Vinay"), "{}", pf);
    assert!(fs::read_to_string(brand.join("ontology.yaml")).unwrap().contains("format: 2"), "the first people.yaml writes format 2");
    // rename mode: every grade as it was, with a fresh fingerprint; unverified stays unverified
    let after: Vec<(String, String, String)> = ["m_checked", "m_accepted", "m_proposed", "m_changed"].iter().map(|n| grade(port, p, n)).collect();
    assert_eq!(after, before, "a rename is a spelling fix, not a review");
    assert_eq!(grade(port, p, "m_ungraded").0, "ungraded", "no record made");
    // one commit, with the words
    assert_eq!(git(&brand, &["log", "-1", "--format=%s"]), "Priya renamed Vinay to Vinay Karode: 5 metrics, 1 entity");
    let hash = git(&brand, &["log", "-1", "--format=%H"]);
    // describe: the main name and its kind
    let d = get(port, &format!("{}/api/describe?name=m_checked", p));
    assert_eq!(d["owner"], "Vinay Karode");
    assert_eq!(d["owner_kind"], "person");
    // merge: another spelling into the main name
    let u = post(port, &format!("{}/api/git/undo", p), json!({ "hash": hash }));
    assert_eq!(u["written"], true, "{}", u);
    assert!(!brand.join("people.yaml").exists(), "Undo puts people.yaml back as it was (none)");
    assert!(fs::read_to_string(brand.join("logic/metrics.yaml")).unwrap().contains("owner: vinay # who decides"));
    let undone: Vec<(String, String, String)> = ["m_checked", "m_accepted", "m_proposed", "m_changed"].iter().map(|n| grade(port, p, n)).collect();
    assert_eq!(undone, before, "and trust.yaml with it");
    // refusals
    assert_ne!(post(port, &format!("{}/api/people", p), json!({ "op": "rename", "from": "Vinay", "to": "Vinay", "by": "Priya" }))["written"], true);
    assert_ne!(post(port, &format!("{}/api/people", p), json!({ "op": "rename", "from": "Vinay", "to": "x@y.example", "by": "Priya" }))["written"], true);
}

#[test]
fn the_people_file_its_warnings_and_what_agents_see() {
    let d = Dir::new("file");
    let mut metrics = String::from("metrics:\n");
    for i in 0..14 {
        metrics.push_str(&format!("  - metric: m{}\n    description: Number {}.\n    sql: select {}\n    owner: Owner{}\n", i, i, i, i));
    }
    for (r, t) in [("ontology.yaml", ONT), ("sources/shop.yaml", SOURCES), ("types/order.yaml", ORDER), ("logic/metrics.yaml", metrics.as_str())] {
        d.write(r, t);
    }
    let port = 7312;
    let _srv = serve(&d.0, port, false);
    // the summary's Owners line: up to 12, then "and N more"
    let md = get(port, "/api/summary.md");
    let md = md.as_str().unwrap_or("");
    assert!(md.contains("Owners: ") && md.contains(", and 3 more"), "{}", md);
    // a People-screen action writes people.yaml, and format 2 with it
    let t = post(port, "/api/people", json!({ "op": "team", "name": "Owner1", "by": "Vinay" }));
    assert_eq!(t["written"], true, "{}", t);
    assert!(d.read("people.yaml").contains("name: Owner1") && d.read("people.yaml").contains("team: true"));
    assert!(d.read("ontology.yaml").contains("format: 2"));
    assert_eq!(get(port, "/api/describe?name=m1")["owner_kind"], "team");
    // a raw edit of people.yaml is refused, and so is an agent's
    let raw = post(port, "/api/edit", json!({ "file": "people.yaml", "content": "people: []\n" }));
    assert_ne!(raw["written"], true, "{}", raw);
    let a = post(port, "/api/tool", json!({ "name": "propose_change", "arguments": { "file": "people.yaml", "content": "people:\n  - name: Agent Smith\n", "reason": "x" } }));
    assert!(a.to_string().contains("written by Lorefile"), "{}", a);
    // hand-written: other spellings give W119 on the owner, contradictions give W121, no format gives W118
    d.write("ontology.yaml", ONT);
    d.write("people.yaml", "people:\n  - name: Owner Zero\n    also: [Owner0]\n  - name: Owner Two\n    also: [Owner0]\n  - name: Sam\n    left: soon\n");
    post(port, "/api/reload", json!({}));
    let ds = get(port, "/api/summary")["diagnostics"].as_array().cloned().unwrap();
    let has = |code: &str, bit: &str| ds.iter().any(|x| x["code"] == code && x["message"].as_str().unwrap_or("").contains(bit));
    assert!(has("W119", "metric m0: owner `Owner0` is another spelling of Owner Zero (people.yaml)"), "{:?}", ds);
    assert!(has("W121", "Owner0 is a spelling of both Owner Zero and Owner Two"), "{:?}", ds);
    assert!(has("W121", "isn't a date"), "{:?}", ds);
    assert!(has("W118", "people.yaml"), "{:?}", ds);
    let w119 = ds.iter().find(|x| x["code"] == "W119").unwrap();
    assert_eq!(w119["subject"]["kind"], "metric");
    // W119 lights nothing on Home
    let plane = get(port, "/api/plane");
    assert!(!plane.to_string().contains("\"W119\""), "a spelling is tidiness, not a risk");
    let dm = get(port, "/api/describe?name=m0");
    assert_eq!(dm["owner"], "Owner0");
    assert_eq!(dm["owner_is"], "Owner Zero");
    // people.yaml elsewhere, or with an email, is refused by the loader
    d.write("types/people.yaml", "people:\n  - name: Priya\n");
    d.write("people.yaml", "people:\n  - name: Priya\n    email: priya@shop.example\n");
    post(port, "/api/reload", json!({}));
    let ds = get(port, "/api/summary")["diagnostics"].as_array().cloned().unwrap();
    assert!(ds.iter().any(|x| x["code"] == "E001" && x["message"].as_str().unwrap().contains("types/people.yaml") && x["message"].as_str().unwrap().contains("people.yaml at the root")), "{:?}", ds);
    assert!(ds.iter().any(|x| x["code"] == "E001" && x["message"].as_str().unwrap().contains("unknown field `email`")), "{:?}", ds);
}
