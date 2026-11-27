//! Areas (specs/2026-11-27-people-areas-graph.md, Part 3), end to end with the real binary:
//! deleting an entity cleans its area's list, the six area ops, moves with kinds, colours,
//! W120, and the review words.

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
        let dir = std::env::temp_dir().join(format!("lorefile-areas-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let s = Space(dir);
        for (rel, text) in files {
            s.write(rel, text);
        }
        s
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

fn serve(dir: &Path, port: u16) -> Server {
    let child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["serve", "--dir", dir.to_str().unwrap(), "--port", &port.to_string(), "--no-auth", "--no-examples", "--allow-edit", "--no-commit-edits"])
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

fn http(port: u16, method: &str, path: &str, body: &str) -> Json {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let req = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nX-Lorefile: write\r\nContent-Length: {}\r\n\r\n{}", method, path, port, body.len(), body);
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
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
    serde_json::from_str(&body).unwrap_or(Json::Null)
}

fn get(port: u16, path: &str) -> Json {
    http(port, "GET", path, "")
}
fn post(port: u16, path: &str, body: Json) -> Json {
    http(port, "POST", path, &body.to_string())
}
fn diags(port: u16) -> Vec<Json> {
    get(port, "/api/summary")["diagnostics"].as_array().cloned().unwrap_or_default()
}
fn has(ds: &[Json], code: &str, bit: &str) -> bool {
    ds.iter().any(|d| d["code"] == code && d["message"].as_str().unwrap_or("").contains(bit))
}

const MANIFEST: &str = "name: shop\nversion: 0.1.0\ndomains:\n  - name: Core\n    description: The main things.\n    types: [Order, Customer]\n";
const ORDER: &str = "type: Order\ndescription: One checkout.\nproperties:\n  order_id: { type: string, key: true, description: The id. }\n";
const CUSTOMER: &str = "type: Customer\ndescription: Someone who buys.\nproperties:\n  customer_id: { type: string, key: true, description: The id. }\n";

#[test]
fn deleting_an_entity_listed_in_an_area_also_takes_it_off_the_list() {
    // written first (spec Part 3): before the fix, the list kept the name, which is E010
    // ("domain lists unknown type"), and the save gate refused the delete
    let sp = Space::new("delete", &[("ontology.yaml", MANIFEST), ("types/order.yaml", ORDER), ("types/customer.yaml", CUSTOMER)]);
    let port = 7300;
    let _srv = serve(&sp.0, port);
    assert!(!diags(port).iter().any(|d| d["level"] == "error"), "{:?}", diags(port));
    let r = http(port, "DELETE", "/api/file?file=types/customer.yaml", "");
    assert_eq!(r["written"], true, "the delete goes through: {}", r);
    assert!(!sp.0.join("types/customer.yaml").exists());
    let m = sp.read("ontology.yaml");
    assert!(!m.contains("Customer"), "the area's list no longer names it:\n{}", m);
    assert!(m.contains("Order"), "{}", m);
    assert!(!has(&diags(port), "E010", ""), "{:?}", diags(port));
}

const SHOP: &str = "# the shop's areas\nname: shop\nversion: 0.1.0\nformat: 2\ndomains:\n  - name: Core\n    description: The main things.\n    types: [Order, Customer]\n  - name: Pipeline\n    description: Deals.\n    types: [Deal]\n  - name: Channel\n    types: []\n# said last\nprefixes: {}\n";
const DEAL: &str = "type: Deal\ndescription: A deal.\nproperties:\n  deal_id: { type: string, key: true, description: The id. }\n";
const LEAD: &str = "type: Lead\ndomain: Pipeline # page-made\ndescription: A lead.\nproperties:\n  lead_id: { type: string, key: true, description: The id. }\n";
const SALE: &str = "type: Sale\ndomain: Core\ndescription: A sale.\nproperties:\n  sale_id: { type: string, key: true, description: The id. }\n  kind: { type: string, description: promo or not. }\n";
const PROMO: &str = "type: PromoSale\nextends: Sale\ndescription: A promo sale.\nwhere: { kind: promo }\n";
const PARTNER: &str = "type: Partner\ndescription: A partner.\nproperties:\n  partner_id: { type: string, key: true, description: The id. }\n";

fn shop(tag: &str) -> Space {
    Space::new(tag, &[("ontology.yaml", SHOP), ("types/order.yaml", ORDER), ("types/customer.yaml", CUSTOMER), ("types/deal.yaml", DEAL), ("types/lead.yaml", LEAD), ("types/sale.yaml", SALE), ("types/promo_sale.yaml", PROMO), ("types/partner.yaml", PARTNER)])
}
fn area_of(port: u16, name: &str) -> String {
    get(port, &format!("/api/describe?name={}", name))["domain"].as_str().unwrap_or("Other").to_string()
}
fn patch(port: u16, op: Json) -> Json {
    let mut op = op;
    op["by"] = json!("Vinay");
    post(port, "/api/patch", op)
}

#[test]
fn areas_are_added_renamed_described_and_reordered_from_the_page() {
    let sp = shop("ops");
    let port = 7301;
    let _srv = serve(&sp.0, port);
    assert!(!diags(port).iter().any(|d| d["level"] == "error"), "{:?}", diags(port));
    // add: the name rule, Other, a taken name, an IRI that would collide
    let r = patch(port, json!({ "op": "add_area", "name": "Sales & Ops", "description": "Selling." }));
    assert_eq!(r["written"], true, "{}", r);
    assert_eq!(r["summary"], "added the area Sales & Ops");
    let m = sp.read("ontology.yaml");
    assert!(m.starts_with("# the shop's areas\n") && m.contains("# said last\nprefixes: {}"), "comments outside the list stay:\n{}", m);
    assert!(m.contains("  - name: Sales & Ops\n    description: Selling.\n    types: []\n"), "{}", m);
    assert!(!m.contains("color:"), "appending recolours nothing");
    for (bad, why) in [("other", "Other is where"), ("core", "already an area called Core"), ("Sales Ops", "domain.sales-ops"), ("Bad<name>", "letters, digits")] {
        let e = patch(port, json!({ "op": "add_area", "name": bad }));
        assert_ne!(e["written"], true, "{}", bad);
        assert!(e.to_string().contains(why), "{}: {}", bad, e);
    }
    // rename: the area, and every `domain:` line that says it; capitals alone are allowed
    let r = patch(port, json!({ "op": "rename_area", "area": "Pipeline", "to": "Sales" }));
    assert_eq!(r["written"], true, "{}", r);
    assert_eq!(r["summary"], "renamed the area Pipeline to Sales");
    assert!(sp.read("types/lead.yaml").contains("domain: Sales # page-made"), "line edit, comment kept: {}", sp.read("types/lead.yaml"));
    assert!(sp.read("ontology.yaml").contains("- name: Sales\n    description: Deals.\n    types: [Deal]"));
    assert_eq!(area_of(port, "Lead"), "Sales");
    assert_eq!(area_of(port, "Deal"), "Sales");
    assert_eq!(patch(port, json!({ "op": "rename_area", "area": "Channel", "to": "channel" }))["written"], true, "capitals only");
    assert_ne!(patch(port, json!({ "op": "rename_area", "area": "Core", "to": "Sales" }))["written"], true, "taken");
    // describe
    assert_eq!(patch(port, json!({ "op": "describe_area", "area": "channel", "text": "Partners that bring deals." }))["written"], true);
    assert!(sp.read("ontology.yaml").contains("- name: channel\n    description: Partners that bring deals.\n"));
    // order: every name once; colours kept where a position colour would change
    assert!(patch(port, json!({ "op": "order_areas", "order": ["Sales", "Core"] })).to_string().contains("say where"), "missing names refused");
    let r = patch(port, json!({ "op": "order_areas", "order": ["Sales", "Core", "channel", "Sales & Ops"] }));
    assert_eq!(r["written"], true, "{}", r);
    let m = sp.read("ontology.yaml");
    let core = m.find("- name: Core").unwrap();
    let sales = m.find("- name: Sales\n").unwrap();
    assert!(sales < core, "{}", m);
    assert!(m.contains("- name: Sales\n    description: Deals.\n    types: [Deal]\n    color: teal\n"), "Sales was second (teal) and keeps it:\n{}", m);
    assert!(m.contains("types: [Order, Customer]\n    color: blue\n"), "Core keeps blue: {}", m);
    assert!(!m[m.find("- name: channel").unwrap()..].split("- name: Sales & Ops").next().unwrap().contains("color:"), "channel stayed third: {}", m);
    assert!(!diags(port).iter().any(|d| d["code"] == "W118"), "format 2 is said");
    let plane = get(port, "/api/plane");
    let a: Vec<&Json> = plane["areas"].as_array().unwrap().iter().collect();
    assert_eq!(a[0]["name"], "Sales");
    assert_eq!(a[0]["color"], "teal");
    let ops = a.iter().find(|x| x["name"] == "Sales & Ops").unwrap();
    assert_eq!(ops["entities"], 0, "an empty area is drawn: {}", ops);
}

#[test]
fn moving_an_entity_writes_where_its_area_is_said_and_kinds_follow() {
    let sp = shop("move");
    let port = 7302;
    let _srv = serve(&sp.0, port);
    // listed under Core: it leaves Core's list and joins Pipeline's
    let r = patch(port, json!({ "op": "move_to_area", "type": "Order", "area": "Pipeline" }));
    assert_eq!(r["written"], true, "{}", r);
    assert_eq!(r["summary"], "moved Order from Core to Pipeline");
    let m = sp.read("ontology.yaml");
    assert!(m.contains("types: [Customer]") && m.contains("types: [Deal, Order]"), "{}", m);
    assert!(!sp.read("types/order.yaml").contains("domain:"), "its file says nothing, as before");
    // says `domain:` in its file: that line changes
    assert_eq!(patch(port, json!({ "op": "move_to_area", "type": "Lead", "area": "Channel" }))["written"], true);
    assert!(sp.read("types/lead.yaml").contains("domain: Channel # page-made"));
    // in no area (Other): `domain:` is added to its file, after `type:`
    assert_eq!(area_of(port, "Partner"), "Other");
    assert_eq!(patch(port, json!({ "op": "move_to_area", "type": "Partner", "area": "Channel" }))["written"], true);
    assert!(sp.read("types/partner.yaml").starts_with("type: Partner\ndomain: Channel\n"), "{}", sp.read("types/partner.yaml"));
    // both: listed under one area while saying another (W120); a move writes both places
    sp.write("types/deal.yaml", &DEAL.replace("type: Deal\n", "type: Deal\ndomain: Core\n"));
    post(port, "/api/reload", json!({}));
    assert!(has(&diags(port), "W120", "Deal is listed under Pipeline but says Core"), "{:?}", diags(port));
    assert_eq!(patch(port, json!({ "op": "move_to_area", "type": "Deal", "area": "Channel" }))["written"], true);
    assert!(sp.read("types/deal.yaml").contains("domain: Channel") && sp.read("ontology.yaml").contains("types: [Deal]"), "{}", sp.read("ontology.yaml"));
    assert!(!has(&diags(port), "W120", "Deal"), "the move clears it");
    // kinds: PromoSale follows Sale, and moves with it
    assert_eq!(area_of(port, "PromoSale"), "Core");
    let r = patch(port, json!({ "op": "move_to_area", "type": "Sale", "area": "Pipeline" }));
    assert_eq!(r["summary"], "moved Sale from Core to Pipeline (PromoSale follows Sale and moves too)");
    assert_eq!(area_of(port, "PromoSale"), "Pipeline", "a kind with no area of its own follows its parent");
    // moving the kind alone gives it its own area: it stops following
    let r = patch(port, json!({ "op": "move_to_area", "type": "PromoSale", "area": "Core" }));
    assert_eq!(r["summary"], "moved PromoSale from Pipeline to Core (PromoSale stops following Sale's area)");
    assert!(sp.read("types/promo_sale.yaml").contains("domain: Core"));
    assert_eq!(patch(port, json!({ "op": "move_to_area", "type": "Sale", "area": "Channel" }))["written"], true);
    assert_eq!(area_of(port, "PromoSale"), "Core", "it stays");
    // into its own area: nothing written
    let same = patch(port, json!({ "op": "move_to_area", "type": "Sale", "area": "Channel" }));
    assert_ne!(same["written"], true);
    assert!(same.to_string().contains("Sale is already in Channel"), "{}", same);
    // several at once, as "Move here…" sends them
    let b = post(port, "/api/patch", json!({ "by": "Vinay", "ops": [{ "op": "move_to_area", "type": "Customer", "area": "Channel" }, { "op": "move_to_area", "type": "Order", "area": "Channel" }] }));
    assert_eq!(b["written"], true, "{}", b);
    assert_eq!(area_of(port, "Order"), "Channel");
    assert_eq!(area_of(port, "Customer"), "Channel");
}

#[test]
fn an_area_is_deleted_only_when_empty_and_later_areas_keep_their_colours() {
    let sp = shop("delete-area");
    let port = 7303;
    let _srv = serve(&sp.0, port);
    let e = patch(port, json!({ "op": "delete_area", "area": "Core" }));
    assert_ne!(e["written"], true);
    assert!(e.to_string().contains("Core still holds Order, Customer and 2 more"), "{}", e);
    // a middle area emptied, then deleted: Channel was third (violet) and stays violet
    assert_eq!(patch(port, json!({ "op": "move_to_area", "type": "Deal", "area": "Core" }))["written"], true);
    assert_eq!(patch(port, json!({ "op": "move_to_area", "type": "Lead", "area": "Core" }))["written"], true);
    let r = patch(port, json!({ "op": "delete_area", "area": "Pipeline" }));
    assert_eq!(r["written"], true, "{}", r);
    assert_eq!(r["summary"], "deleted the empty area Pipeline");
    let m = sp.read("ontology.yaml");
    assert!(!m.contains("Pipeline") && m.contains("- name: Channel\n    types: []\n    color: violet\n"), "{}", m);
    // a colour that isn't one of the eight
    sp.write("ontology.yaml", &m.replace("color: violet", "color: purple"));
    post(port, "/api/reload", json!({}));
    assert!(has(&diags(port), "W120", "area Channel: colour `purple` isn't one of blue, teal, violet, pink, slate, sky, green, rose"), "{:?}", diags(port));
    // an area an entity names that ontology.yaml doesn't list
    sp.write("types/partner.yaml", &PARTNER.replace("type: Partner\n", "type: Partner\ndomain: Pipline\n"));
    post(port, "/api/reload", json!({}));
    assert!(has(&diags(port), "W120", "Partner says area `Pipline`, which ontology.yaml doesn't list"), "{:?}", diags(port));
    // W120 lights nothing on Home
    assert!(!get(port, "/api/plane").to_string().contains("\"W120\""));
    // the last area can go once it is empty, and nothing after it is recoloured
    assert_eq!(patch(port, json!({ "op": "add_area", "name": "Spare" }))["written"], true);
    let r = patch(port, json!({ "op": "delete_area", "area": "Spare" }));
    assert_eq!(r["written"], true, "{}", r);
    assert!(!sp.read("ontology.yaml").contains("Spare"));
}

#[test]
fn agents_can_propose_area_ops_and_the_review_says_them_in_words() {
    let sp = shop("agents");
    let port = 7304;
    let _srv = serve(&sp.0, port);
    let r = post(port, "/api/tool", json!({ "name": "propose_patch", "arguments": { "op": "add_area", "name": "Finance", "description": "Money.", "reason": "the user keeps talking about invoices" } }));
    assert!(r["isError"] != true, "{}", r);
    assert!(sp.read("ontology.yaml").contains("- name: Finance"), "written: edits are on and it is clean");
    let r = post(port, "/api/tool", json!({ "name": "propose_patch", "arguments": { "op": "rename_area", "area": "Finance", "to": "Money", "reason": "shorter" } }));
    assert!(r["isError"] != true, "a multi-file op from an agent: {}", r);
    assert!(sp.read("ontology.yaml").contains("- name: Money"));
    let r = post(port, "/api/tool", json!({ "name": "propose_patch", "arguments": { "ops": [{ "op": "move_to_area", "type": "Partner", "area": "Money" }], "reason": "partners are paid" } }));
    assert!(r["isError"] != true, "{}", r);
    assert_eq!(area_of(port, "Partner"), "Money");
    let bad = post(port, "/api/tool", json!({ "name": "propose_patch", "arguments": { "op": "_write", "file": "x.yaml", "reason": "x" } }));
    assert!(bad["isError"] == true, "internal steps are not ops: {}", bad);
}
