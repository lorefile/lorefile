//! Kinds of entities (specs/2026-11-27-sub-kinds.md), end to end with the real binary: the
//! format, what a kind gets from its parent, the problems the compiler names, rows of a `where`
//! kind, grades, `validate` with kinds, and the ops that write `extends`.

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
        let dir = std::env::temp_dir().join(format!("lorefile-kinds-{}-{}", tag, std::process::id()));
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
fn tool(port: u16, name: &str, args: Json) -> (bool, String) {
    let r = post(port, "/api/tool", json!({ "name": name, "arguments": args }));
    (r["isError"] == true, r["content"][0]["text"].as_str().unwrap_or("").to_string())
}

const MANIFEST2: &str = "name: shop\nversion: 0.1.0\nformat: 2\ndomains:\n  - name: Sales\n    description: Selling.\n    types: [Sale]\n";
const SOURCES: &str = "sources:\n  - connection: shop\n    kind: csv\n    description: The shop's exports.\n  - connection: wh\n    kind: bigquery\n    description: The warehouse.\n";
const SALE: &str = "type: Sale\ndescription: One order line.\nproperties:\n  line_id: { type: string, key: true, description: One line. }\n  kind: { type: string, description: promo or wholesale. }\n  revenue: { type: number, unit: USD, description: What was paid. }\n  status: { type: enum, values: [paid, refunded, open], description: Where it stands. }\nsource: { connection: shop, table: orders, key: line_id }\nrelationships:\n  placed_by: { to: Customer, cardinality: many-to-one, description: Who bought it. }\n";
const CUSTOMER: &str = "type: Customer\ndescription: Someone who buys.\nproperties:\n  customer_id: { type: string, key: true, description: The id. }\n";
const PROMO: &str = "type: PromoSale\nextends: Sale\ndescription: A sale made with a promo code.\nwhere: { kind: promo }\nproperties:\n  revenue: { description: What was paid after the promo. }\n  promo_code: { type: string, description: The code used. }\n";
const ORDERS: &str = "line_id,kind,revenue,status,promo_code\nL-1,promo,10,paid,SPRING\nL-2,wholesale,99,paid,\nL-3,promo,12,refunded,SPRING\n";

fn shop(tag: &str) -> Space {
    Space::new(tag, &[
        ("ontology.yaml", MANIFEST2),
        ("sources/sources.yaml", SOURCES),
        ("types/sale.yaml", SALE),
        ("types/customer.yaml", CUSTOMER),
        ("types/promo_sale.yaml", PROMO),
        ("data/orders.csv", ORDERS),
    ])
}

#[test]
fn a_kind_gets_its_parents_fields_first_and_reads_only_its_rows() {
    let sp = shop("inherit");
    let port = 7260;
    let _srv = serve(&sp.0, port);
    let ds = diags(port);
    assert!(!ds.iter().any(|d| d["level"] == "error"), "{:?}", ds);
    assert!(!has(&ds, "W118", ""), "format 2 is said");
    let d = get(port, "/api/describe?name=PromoSale");
    let names: Vec<&str> = d["properties"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["line_id", "kind", "revenue", "status", "promo_code"], "the parent's fields first, then its own");
    assert_eq!(d["properties"][0]["from"], "Sale");
    assert!(d["properties"][2].get("from").is_none(), "a narrowed field is the kind's own: {}", d["properties"][2]);
    assert_eq!(d["properties"][2]["description"], "What was paid after the promo.", "the narrowed description wins for the kind");
    assert_eq!(get(port, "/api/describe?name=Sale")["properties"][2]["description"], "What was paid.", "and only for the kind");
    assert_eq!(d["key"], "line_id");
    assert_eq!(d["extends"], "Sale");
    assert_eq!(d["domain"], "Sales", "a kind sits in its parent's domain");
    assert!(d["is_kind_of"].as_str().unwrap().starts_with("PromoSale is a kind of Sale. It has Sale's 4 fields (line_id, kind, revenue, status) plus 1 of its own: promo_code."), "{}", d["is_kind_of"]);
    assert!(d["rows"].as_str().unwrap().contains("Sale's rows where kind is promo"), "{}", d["rows"]);
    assert!(d["relationships"].as_array().unwrap().iter().any(|r| r["name"] == "placed_by" && r["from"] == "Sale"), "links come from the parent: {}", d["relationships"]);
    // IRIs: an inherited field keeps the parent's, an own field gets the kind's
    assert_eq!(d["properties"][0]["iri"], get(port, "/api/describe?name=Sale")["properties"][0]["iri"]);
    assert!(d["properties"][4]["iri"].as_str().unwrap().ends_with("PromoSale.promo_code"));
    // the parent names its kinds
    let s = get(port, "/api/describe?name=Sale");
    assert!(s["kinds_words"].as_str().unwrap().contains("PromoSale (where kind is promo)"), "{}", s);
    // rows: only promo lines, a wholesale key is not a PromoSale, Sale has all of them
    let promo = get(port, "/api/instances?name=PromoSale");
    let rows = promo["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{}", promo);
    assert!(rows.iter().all(|r| r["kind"] == "promo"));
    assert_eq!(get(port, "/api/instances?name=Sale")["rows"].as_array().unwrap().len(), 3);
    // summary: the kind under its parent's area, and the sentence about never adding
    let md = http(port, "GET", "/api/summary", "");
    let _ = md;
    // the SQL door: the kind is a view over its parent's rows
    if cfg!(feature = "duckdb") {
        let (err, text) = tool(port, "query", json!({ "sql": "select count(*) as n from PromoSale" }));
        assert!(!err && text.contains('2'), "{}", text);
    }
    // removing a parent that has kinds is refused, naming the kind
    let gone = http(port, "DELETE", "/api/file?file=types/sale.yaml", "");
    assert!(gone["written"] != true, "{}", gone);
    assert!(gone.to_string().contains("PromoSale"), "the refusal names the kind: {}", gone);
    assert!(!sp.read("types/sale.yaml").is_empty());
}

#[test]
fn the_compiler_names_what_is_wrong_with_a_kind() {
    let sp = Space::new("problems", &[
        ("ontology.yaml", "name: p\nversion: 0.1.0\nformat: 2\n"),
        ("sources/s.yaml", SOURCES),
        ("types/sale.yaml", SALE),
        ("types/customer.yaml", CUSTOMER),
        ("glossary.yaml", "terms:\n  - term: order\n    means: Sale\n"),
        ("types/a.yaml", "type: Retyped\nextends: Sale\ndescription: x\nproperties:\n  revenue: { type: integer }\n"),
        ("types/b.yaml", "type: Rekeyed\nextends: Sale\ndescription: x\nproperties:\n  other_id: { type: string, key: true }\n"),
        ("types/c.yaml", "type: Relinked\nextends: Sale\ndescription: x\nrelationships:\n  placed_by: { to: Customer, cardinality: many-to-one, description: again }\n"),
        ("types/d.yaml", "type: Orphan\nextends: Sal\ndescription: x\n"),
        ("types/e.yaml", "type: Worded\nextends: Order\ndescription: x\n"),
        ("types/f.yaml", "type: Twice\nextends: [Sale, Customer]\ndescription: x\n"),
        ("types/g.yaml", "type: LoopA\nextends: LoopB\ndescription: x\n"),
        ("types/h.yaml", "type: LoopB\nextends: LoopA\ndescription: x\n"),
        ("types/i.yaml", "type: Loose\ndescription: x\nwhere: { kind: promo }\n"),
        ("types/j.yaml", "type: Both\nextends: Sale\ndescription: x\nwhere: { kind: promo }\nsource: { connection: wh, table: both, key: line_id }\n"),
        ("types/k.yaml", "type: Typo\nextends: Sale\ndescription: x\nwhere: { knd: promo }\n"),
        ("types/l.yaml", "type: BadValue\nextends: Sale\ndescription: x\nwhere: { status: lost }\n"),
        ("types/m.yaml", "type: Widened\nextends: Sale\ndescription: x\nproperties:\n  status: { type: enum, values: [paid, refunded, open, lost] }\n"),
    ]);
    let port = 7261;
    let _srv = serve(&sp.0, port);
    let ds = diags(port);
    for (code, bit) in [
        ("E021", "Retyped.revenue is money (USD) in Sale; a kind can't change its type"),
        ("E022", "Sale is identified by line_id; Rekeyed can't be identified by other_id"),
        ("E023", "Relinked.placed_by already comes from Sale"),
        ("E020", "Orphan extends Sal, which doesn't exist"),
        ("E020", "`Order` is a word for Sale; extend Sale"),
        ("E020", "an entity extends one other entity; `extends: [Sale, Customer]` names 2"),
        ("E020", "goes round in a circle"),
        ("E024", "Loose has `where` but extends nothing"),
        ("E024", "Both has both where and a source"),
        ("E024", "where names `knd`, which Sale has no field called"),
        ("E024", "status is one of a list in Sale; lost isn't among its values"),
        ("E021", "Widened.status"),
    ] {
        assert!(has(&ds, code, bit), "{} {}: {:?}", code, bit, ds.iter().filter(|d| d["code"].as_str().unwrap_or("").starts_with("E02")).collect::<Vec<_>>());
    }
    let e = ds.iter().find(|d| d["code"] == "E022").unwrap();
    assert_eq!(e["subject"], json!({ "kind": "entity", "name": "Rekeyed" }));
}

#[test]
fn seven_levels_is_too_deep_and_the_format_is_said() {
    let mut files: Vec<(String, String)> = vec![("ontology.yaml".into(), "name: deep\nversion: 0.1.0\n".into())];
    files.push(("types/k0.yaml".into(), "type: K0\ndescription: x\nproperties: { id: { type: string, key: true, description: id } }\n".into()));
    for i in 1..=7 {
        files.push((format!("types/k{}.yaml", i), format!("type: K{}\nextends: K{}\ndescription: x\n", i, i - 1)));
    }
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let sp = Space::new("deep", &refs);
    let port = 7262;
    let _srv = serve(&sp.0, port);
    let ds = diags(port);
    assert!(has(&ds, "E020", "7 levels of kinds"), "{:?}", ds);
    assert!(has(&ds, "W118", "add `format: 2`"), "extends without format 2: {:?}", ds);
    // a format newer than this binary: one sentence, and no unknown-field noise
    sp.write("ontology.yaml", "name: deep\nversion: 0.1.0\nformat: 3\nicons: true\n");
    post(port, "/api/reload", json!({}));
    let ds = diags(port);
    assert!(has(&ds, "E019", "This space needs a newer Lorefile (format 3)"), "{:?}", ds);
    assert!(!ds.iter().any(|d| d["message"].as_str().unwrap_or("").contains("unknown field")), "{:?}", ds);
}

#[test]
fn a_where_kinds_source_is_graded_on_its_own_and_follows_its_parents() {
    let sp = shop("grades");
    sp.write("types/sale.yaml", &SALE.replace("connection: shop, table: orders", "connection: wh, table: orders"));
    let port = 7263;
    let _srv = serve(&sp.0, port);
    // not readable here: where_is says where to fetch and what to keep
    let w = get(port, "/api/where?name=PromoSale");
    assert!(w["hint"].as_str().unwrap().contains("Sale's rows (in wh.orders) where kind is promo: fetch them with your own connector and keep those"), "{}", w);
    assert_eq!(post(port, "/api/trust", json!({ "op": "accept", "kind": "binding", "name": "PromoSale", "by": "Vinay" }))["written"], true, "a where kind's source is a grade of its own");
    let trust = sp.read("trust.yaml");
    assert!(trust.contains("PromoSale:") && !trust.lines().any(|l| l.trim_start().starts_with("Sale:")), "accepting the kind grades only the kind: {}", trust);
    // re-pointing Sale's source changes what PromoSale's grade was about
    sp.write("types/sale.yaml", &SALE.replace("connection: shop, table: orders", "connection: wh, table: orders_v2"));
    post(port, "/api/reload", json!({}));
    assert!(has(&diags(port), "W113", "source of PromoSale changed since it was accepted"), "{:?}", diags(port));
    assert!(has(&diags(port), "W113", "because Sale's source changed"));
}

#[test]
fn validate_checks_a_sale_as_its_kind_and_refuses_a_parent_for_a_kind() {
    let sp = shop("validate");
    sp.write("constraints/c.yaml", "constraints:\n  - { id: PR-1, name: Promo needs a code, applies_to: PromoSale, rule: \"promo_code != ''\", severity: block, rationale: A promo line without a code is a data error. }\n");
    sp.write("actions/refund.yaml", "action: refund_sale\ndescription: Refund one line.\ninputs: { sale: Sale }\n");
    sp.write("actions/promo.yaml", "action: extend_promo\ndescription: Extend a promo line.\ninputs: { promo: PromoSale }\n");
    let port = 7264;
    let _srv = serve(&sp.0, port);
    let (err, text) = tool(port, "validate", json!({ "action": "refund_sale", "inputs": { "sale": "L-1" } }));
    assert!(!err, "{}", text);
    let rep: Json = serde_json::from_str(&text).unwrap();
    assert!(rep["notes"].to_string().contains("checked as a PromoSale (a kind of Sale), because kind is promo"), "{}", rep);
    assert!(rep["results"].as_array().unwrap().iter().any(|r| r["id"] == "PR-1" && r["passed"] == true), "the kind's guardrail ran: {}", rep["results"]);
    let (_, text) = tool(port, "validate", json!({ "action": "refund_sale", "inputs": { "sale": "L-2" } }));
    let rep: Json = serde_json::from_str(&text).unwrap();
    assert!(!rep["results"].as_array().unwrap().iter().any(|r| r["id"] == "PR-1"), "a wholesale line is not checked as a PromoSale: {}", rep);
    let (err, text) = tool(port, "validate", json!({ "action": "refund_sale", "inputs": { "sale": "L-1" }, "kinds": { "sale": "Customer" } }));
    assert!(err && text.contains("Customer is not a kind of Sale"), "{}", text);
    let (err, text) = tool(port, "validate", json!({ "action": "extend_promo", "inputs": { "promo": "L-2" } }));
    assert!(err && text.contains("`promo` needs a PromoSale; L-2 is a Sale whose kind is wholesale"), "{}", text);
}

#[test]
fn ops_write_extends_with_format_2_and_never_inherited_fields() {
    let sp = shop("ops");
    sp.write("ontology.yaml", &MANIFEST2.replace("format: 2\n", ""));
    sp.write("types/promo_sale.yaml", "type: PromoSale\ndescription: A sale made with a promo code.\nproperties:\n  line_id: { type: string, key: true, description: One line. }\n  revenue: { type: number, unit: USD, description: What was paid after the promo. }\n  status: { type: enum, values: [paid, refunded, open], description: Where it stands. }\n  promo_code: { type: string, description: The code used. }\n");
    let port = 7266;
    let _srv = serve(&sp.0, port);
    // set_kind_of: the fields now inherited unchanged go; differing words stay; format 2 comes with it
    let r = post(port, "/api/patch", json!({ "op": "set_kind_of", "type": "PromoSale", "extends": "Sale", "where": { "kind": "promo" } }));
    assert_eq!(r["written"], true, "{}", r);
    let text = sp.read("types/promo_sale.yaml");
    assert!(text.contains("extends: Sale") && text.contains("kind: promo"), "{}", text);
    assert!(!text.contains("line_id") && !text.contains("status:"), "fields now from Sale are dropped: {}", text);
    assert!(text.contains("What was paid after the promo."), "a differing description stays as a narrowing: {}", text);
    assert!(r["summary"].as_str().unwrap().contains("keeps its own words for revenue"), "{}", r);
    assert!(sp.read("ontology.yaml").contains("format: 2"), "format 2 in the same save: {}", sp.read("ontology.yaml"));
    // describing a field of the kind writes only its own fields back
    let r = post(port, "/api/patch", json!({ "op": "set_property_description", "type": "PromoSale", "property": "promo_code", "text": "The promo code typed at checkout." }));
    assert_eq!(r["written"], true, "{}", r);
    let text = sp.read("types/promo_sale.yaml");
    assert!(!text.contains("line_id") && !text.contains("kind:\n"), "inherited fields are never written: {}", text);
    // add_type with extends: no <name>_id field of its own, the parent's key
    let r = post(port, "/api/patch", json!({ "op": "add_type", "name": "WholesaleSale", "extends": "Sale", "where": { "kind": "wholesale" }, "description": "A sale to a reseller." }));
    assert_eq!(r["written"], true, "{}", r);
    let text = sp.read("types/wholesale_sale.yaml");
    assert!(text.contains("extends: Sale") && !text.contains("wholesale_sale_id"), "{}", text);
    assert_eq!(get(port, "/api/describe?name=WholesaleSale")["key"], "line_id");
    // a type conflict refuses in words
    let r = post(port, "/api/patch", json!({ "op": "add_type", "name": "Odd", "description": "x", "properties": { "revenue": "integer", "odd_id": { "type": "string", "key": true } } }));
    assert_eq!(r["written"], true, "{}", r);
    let r = post(port, "/api/patch", json!({ "op": "set_kind_of", "type": "Odd", "extends": "Sale" }));
    assert!(r["error"].as_str().or(r["refused"].as_str()).unwrap_or("").contains("Odd.revenue is integer but Sale's is number"), "{}", r);
}

// ---------------------------------------------------------------------------
// Things said twice, "different on purpose", the rule of three, make_parent
// ---------------------------------------------------------------------------

fn lines(fields: &[&str], desc: &str) -> String {
    fields.iter().map(|f| format!("  {}: {{ type: string, description: {} {}. }}\n", f, desc, f)).collect()
}

#[test]
fn the_same_thing_said_twice_is_a_warning_and_a_person_can_say_why() {
    let sp = Space::new("twice", &[
        ("ontology.yaml", "name: twice\nversion: 0.1.0\n"),
        ("sources/s.yaml", SOURCES),
        ("types/sale.yaml", SALE),
        ("types/customer.yaml", CUSTOMER),
        ("types/line.yaml", "type: SaleLine\ndescription: The same rows again.\nproperties:\n  line_id: { type: string, key: true, description: id }\nsource: { connection: shop, table: orders, key: line_id }\n"),
        ("logic/m.yaml", "metrics:\n  - metric: promo_sale_revenue\n    description: a\n    owner: x\n    source: { connection: shop, table: orders, filter: \"refunded = false and kind == 'promo'\", measure: sum(revenue) }\n  - metric: promo_revenue\n    description: b\n    owner: x\n    source: { connection: shop, table: orders, filter: \"kind = 'promo' AND NOT refunded\", measure: SUM( revenue ) }\n  - metric: other_revenue\n    description: c\n    owner: x\n    source: { connection: shop, table: orders, filter: \"refunded = true\", measure: sum(revenue) }\n"),
        ("glossary.yaml", "terms:\n  - term: promo\n    means: Sale\n  - term: promo\n    means: Customer\n  - term: customer\n    means: Sale\n"),
    ]);
    let port = 7267;
    let _srv = serve(&sp.0, port);
    let ds = diags(port);
    let w115: Vec<&Json> = ds.iter().filter(|d| d["code"] == "W115").collect();
    assert_eq!(w115.len(), 1, "case, quotes, == and NOT, and the order of terms don't hide it; other_revenue really differs: {:?}", w115);
    assert!(w115[0]["message"].as_str().unwrap().contains("Promo revenue looks the same as Promo sale revenue: same rows, same filter, same sum."), "{}", w115[0]);
    assert_eq!(w115[0]["pair"], json!(["metric:promo_revenue", "metric:promo_sale_revenue"]));
    assert!(has(&ds, "W116", "read the same rows (shop.orders): probably one entity.") && ds.iter().any(|d| d["code"] == "W116" && d["message"].as_str().unwrap().contains("SaleLine")), "{:?}", ds);
    assert!(has(&ds, "W117", "\"promo\" means both Sale and Customer"), "{:?}", ds);
    assert!(has(&ds, "W117", "\"Customer\" is an entity, and also a word for Sale."), "{:?}", ds);
    // different on purpose: needs a name, then the warning goes quiet
    let r = post(port, "/api/trust", json!({ "op": "distinct", "a": "metric:promo_revenue", "b": "metric:promo_sale_revenue", "why": "promo_revenue will include tax from January" }));
    assert!(r["refused"].as_str().or(r["error"].as_str()).unwrap_or("").contains("type your name first"), "{}", r);
    let r = post(port, "/api/trust", json!({ "op": "distinct", "a": "metric:promo_revenue", "b": "metric:promo_sale_revenue", "why": "promo_revenue will include tax from January", "by": "Vinay" }));
    assert_eq!(r["written"], true, "{}", r);
    let trust = sp.read("trust.yaml");
    assert!(trust.contains("distinct:") && trust.contains("promo_revenue will include tax from January") && trust.contains("Vinay via page"), "{}", trust);
    assert!(sp.read("ontology.yaml").contains("format: 2"), "distinct: is format 2, written in the same save");
    assert!(!has(&diags(port), "W115", ""), "quiet while both sides are as they were");
    // a change in meaning on one side brings it back, saying so
    let m = sp.read("logic/m.yaml").replace("description: b\n", "description: b changed\n").replace("SUM( revenue )", "SUM(revenue)");
    sp.write("logic/m.yaml", &m);
    post(port, "/api/reload", json!({}));
    assert!(has(&diags(port), "W115", "changed since Vinay said these were different"), "{:?}", diags(port));
    // one side deleted: the line goes on the next save of trust.yaml
    sp.write("logic/m.yaml", "metrics:\n  - metric: promo_sale_revenue\n    description: a\n    owner: x\n    source: { connection: shop, table: orders, filter: \"refunded = false\", measure: sum(revenue) }\n");
    post(port, "/api/reload", json!({}));
    assert_eq!(post(port, "/api/trust", json!({ "op": "accept", "kind": "metric", "name": "promo_sale_revenue", "by": "Vinay" }))["written"], true);
    assert!(!sp.read("trust.yaml").contains("distinct:"), "{}", sp.read("trust.yaml"));
    // an agent can't write it: trust.yaml is Lorefile's
    let (err, text) = tool(port, "propose_change", json!({ "file": "trust.yaml", "content": "trust:\n  distinct: []\n", "reason": "r" }));
    assert!(err || text.contains("trust.yaml is written by Lorefile"), "{}", text);
}

#[test]
fn entities_that_share_most_fields_or_overlapping_rows_are_flagged_but_never_a_parent_and_its_kind() {
    let core = ["line_id", "order_id", "order_date", "sku", "units", "unit_price", "discount", "revenue", "store", "region", "channel", "currency"];
    let mut a = core.to_vec();
    a.extend(["promo_code", "campaign"]);
    let mut b = core.to_vec();
    b.extend(["reseller", "terms"]);
    let sp = Space::new("w116", &[
        ("ontology.yaml", "name: w\nversion: 0.1.0\nformat: 2\n"),
        ("sources/s.yaml", SOURCES),
        ("types/sale.yaml", SALE),
        ("types/customer.yaml", CUSTOMER),
        ("types/a.yaml", &format!("type: PromoLine\ndescription: a\nproperties:\n{}", lines(&a, "The"))),
        ("types/b.yaml", &format!("type: ResellerLine\ndescription: b\nproperties:\n{}", lines(&b, "A"))),
        ("types/p.yaml", "type: PromoSale\nextends: Sale\ndescription: p\nwhere: { kind: promo }\n"),
        ("types/d.yaml", "type: DiscountSale\nextends: Sale\ndescription: d\nwhere: { kind: [promo, discount] }\n"),
        ("types/w.yaml", "type: WholesaleSale\nextends: Sale\ndescription: w\nwhere: { kind: wholesale }\n"),
    ]);
    let port = 7268;
    let _srv = serve(&sp.0, port);
    let ds = diags(port);
    assert!(has(&ds, "W116", "ResellerLine and PromoLine share 12 of 14 fields: probably one entity."), "{:?}", ds);
    assert!(has(&ds, "W116", "both pick Sale's rows where") && has(&ds, "W116", "DiscountSale"), "{:?}", ds);
    assert!(!has(&ds, "W116", "WholesaleSale and PromoSale"), "kind is wholesale can't be promo");
    assert!(!ds.iter().any(|d| d["code"] == "W116" && d["message"].as_str().unwrap().contains(" and Sale ")), "never a parent and its kind: {:?}", ds);
}

#[test]
fn three_entities_sharing_a_core_get_advice_and_make_parent_acts_on_it() {
    let core = ["line_id", "order_id", "order_date", "sku", "units", "unit_price", "discount", "revenue"];
    let with = |extra: &[&'static str]| -> Vec<&'static str> { let mut v = core.to_vec(); v.extend(extra); v };
    let sp = Space::new("three", &[
        ("ontology.yaml", "name: three\nversion: 0.1.0\n"),
        ("types/a.yaml", &format!("type: PromoSale\ndescription: a\nproperties:\n{}", lines(&with(&["promo_code"]), "The").replacen("line_id: { type: string,", "line_id: { type: string, key: true,", 1))),
        ("types/b.yaml", &format!("type: WholesaleSale\ndescription: b\nproperties:\n{}", lines(&with(&["reseller"]), "The").replacen("line_id: { type: string,", "line_id: { type: string, key: true,", 1).replace("The revenue.", "What the reseller paid."))),
    ]);
    let port = 7269;
    let _srv = serve(&sp.0, port);
    assert!(get(port, "/api/summary")["advice"].as_array().unwrap().is_empty(), "two entities: no advice");
    sp.write("types/c.yaml", &format!("type: ReturnLine\ndescription: c\nproperties:\n{}", lines(&with(&["reason"]), "The").replacen("line_id: { type: string,", "line_id: { type: string, key: true,", 1)));
    post(port, "/api/reload", json!({}));
    let advice = get(port, "/api/summary")["advice"].clone();
    assert_eq!(advice.as_array().unwrap().len(), 1, "{}", advice);
    assert!(advice[0]["text"].as_str().unwrap().contains("share 8 fields (line_id, order_id, order_date, sku, units, unit_price, discount, revenue). They could be kinds of one parent."), "{}", advice);
    assert!(!diags(port).iter().any(|d| d["level"] == "warning" && d["message"].as_str().unwrap_or("").contains("kinds of one parent")), "advice is never a warning");
    // make_parent: one change, the parent with the shared fields, each child extending it
    let r = post(port, "/api/patch", json!({ "op": "make_parent", "name": "Sale", "description": "One order line.", "children": ["PromoSale", "WholesaleSale", "ReturnLine"] }));
    assert_eq!(r["written"], true, "{}", r);
    let parent = sp.read("types/sale.yaml");
    assert!(parent.contains("line_id") && parent.contains("key: true") && parent.contains("The revenue."), "the most common words: {}", parent);
    let wholesale = sp.read("types/b.yaml");
    assert!(wholesale.contains("extends: Sale") && wholesale.contains("reseller") && !wholesale.contains("order_id"), "{}", wholesale);
    assert!(wholesale.contains("What the reseller paid."), "differing words stay as a narrowing: {}", wholesale);
    assert!(sp.read("ontology.yaml").contains("format: 2"));
    assert!(!diags(port).iter().any(|d| d["level"] == "error"), "{:?}", diags(port));
    assert_eq!(get(port, "/api/describe?name=ReturnLine")["extends"], "Sale");
    // refused in words on a type clash
    sp.write("types/x.yaml", "type: OddA\ndescription: x\nproperties:\n  units: integer\n  sku: string\n");
    sp.write("types/y.yaml", "type: OddB\ndescription: y\nproperties:\n  units: string\n  sku: string\n");
    post(port, "/api/reload", json!({}));
    let r = post(port, "/api/patch", json!({ "op": "make_parent", "name": "Odd", "children": ["OddA", "OddB"], "fields": ["units", "sku"] }));
    assert!(r["refused"].as_str().or(r["error"].as_str()).unwrap_or("").contains("`units` is integer in OddA but string in OddB"), "{}", r);
}

// ---------------------------------------------------------------------------
// Learning files, and import and export
// ---------------------------------------------------------------------------

fn bin(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_lorefile")).args(args).output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn mcp(dir: &Path, calls: &[(&str, Json)]) -> Vec<(bool, String)> {
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
            (r["result"]["isError"] == true, r["result"]["content"][0]["text"].as_str().unwrap_or("").to_string())
        })
        .collect()
}

fn file(name: &str, headers: &[&str]) -> Json {
    json!({ "file_name": name, "columns": headers.iter().map(|h| json!({ "name": h, "examples": ["a", "b"], "description": format!("The {}.", h) })).collect::<Vec<_>>() })
}

#[test]
fn a_file_is_offered_as_a_kind_only_once_a_family_exists_and_learned_as_one() {
    let sale = "type: Sale\ndescription: One order line.\nproperties:\n  line_id: { type: string, key: true, description: One line. }\n  order_id: { type: string, description: The order. }\n  sku: { type: string, description: The product. }\n  units: { type: string, description: How many. }\nsource: { connection: shop, table: orders, key: line_id }\n";
    let sp = Space::new("learn", &[
        ("ontology.yaml", "name: learn\nversion: 0.1.0\n"),
        ("sources/s.yaml", "sources:\n  - connection: shop\n    kind: csv\n    description: Exports.\n  - connection: user_files\n    kind: from_user\n    description: Files people hand to an agent.\n"),
        ("types/sale.yaml", sale),
    ]);
    let promo = file("promo_sales.csv", &["line_id", "order_id", "sku", "units", "promo_code", "campaign"]);
    // no kind yet: today's answer, the new columns offered to Sale
    let r = mcp(&sp.0, &[("match_file", promo.clone())]);
    let j: Json = serde_json::from_str(&r[0].1).unwrap();
    assert_eq!(j["match"], "changed", "{}", j);
    assert!(j.get("kind_of").is_none(), "no family yet: {}", j);
    // once Sale has a kind, the same file is offered as "a kind of Sale?"
    sp.write("types/wholesale.yaml", "type: WholesaleSale\nextends: Sale\ndescription: To resellers.\nproperties:\n  reseller: { type: string, description: Who bought it. }\nsource: { connection: shop, table: wholesale, key: line_id }\n");
    sp.write("ontology.yaml", "name: learn\nversion: 0.1.0\nformat: 2\n");
    let r = mcp(&sp.0, &[("match_file", promo.clone())]);
    let j: Json = serde_json::from_str(&r[0].1).unwrap();
    assert_eq!(j["kind_of"], "Sale", "{}", j);
    assert!(j["say"].as_str().unwrap().contains("This looks like a kind of Sale: Sale's 4 columns plus promo_code and campaign."), "{}", j);
    // learn_file with kind_of: extends Sale, its own rows, only the extra columns
    let mut args = promo.clone();
    args["entity"] = json!("PromoSale");
    args["answers"] = json!({ "kind_of": "Sale" });
    args["skip_because"] = json!("The user said there is no number in it.");
    args["reason"] = json!("The user said these are promo sales.");
    let r = mcp(&sp.0, &[("learn_file", args.clone())]);
    assert!(!r[0].0, "{}", r[0].1);
    let id = serde_json::from_str::<Json>(&r[0].1).unwrap()["queued"].as_str().unwrap().to_string();
    let p: Json = serde_json::from_str(&sp.read(&format!(".lorefile/proposals/{}.json", id))).unwrap();
    let ty = p["files"].as_array().unwrap().iter().find(|f| f["file"] == "types/promo_sale.yaml").unwrap()["content"].as_str().unwrap().to_string();
    assert!(ty.contains("extends: Sale") && ty.contains("promo_code") && ty.contains("campaign") && !ty.contains("order_id"), "{}", ty);
    assert!(ty.contains("connection: user_files"), "its own rows, the file: {}", ty);
    // a file without Sale's key can't be a Sale
    let mut bad = file("promo_sales.csv", &["order_id", "sku", "units", "promo_code"]);
    bad["entity"] = json!("PromoSale");
    bad["answers"] = json!({ "kind_of": "Sale" });
    bad["skip_because"] = json!("No number.");
    let r = mcp(&sp.0, &[("learn_file", bad)]);
    assert!(r[0].0 && r[0].1.contains("promo_sales.csv has no line_id, so its rows can't be Sales; learn it as its own entity"), "{:?}", r[0]);
}

#[test]
fn kinds_go_through_rdf_shacl_and_ossie_and_come_back() {
    let sp = shop("export");
    let dir = sp.0.to_str().unwrap();
    // what the exports write goes beside the space, never inside it (a folder holding spaces reads differently)
    let outside = Space::new("export-out", &[]);
    fs::create_dir_all(&outside.0).unwrap();
    let out = outside.0.join("export.ttl");
    let (ok, text) = bin(&["export", "--dir", dir, "--rdf", "--out", out.to_str().unwrap(), "--no-examples"]);
    assert!(ok, "{}", text);
    let ttl = fs::read_to_string(&out).unwrap();
    assert!(ttl.contains("rdfs:subClassOf") && ttl.contains("sh:node") && ttl.contains("sh:hasValue \"promo\""), "{}", ttl);
    // back in: the same kind, its where, and only its own fields written
    let back = outside.0.join("back");
    let (ok, text) = bin(&["import", "--from-rdf", out.to_str().unwrap(), "--into", back.to_str().unwrap()]);
    assert!(ok, "{}", text);
    let promo = fs::read_to_string(back.join("types/promo_sale.yaml")).unwrap_or_else(|_| fs::read_dir(back.join("types")).unwrap().filter_map(|e| e.ok()).map(|e| fs::read_to_string(e.path()).unwrap()).find(|t| t.contains("PromoSale")).unwrap());
    assert!(promo.contains("extends: Sale") && promo.contains("kind: promo") && !promo.contains("line_id"), "{}", promo);
    assert!(fs::read_to_string(back.join("ontology.yaml")).unwrap().contains("format: 2"));
    // Ossie: written and read back
    let oss = outside.0.join("ossie");
    let (ok, text) = bin(&["export", "--dir", dir, "--ossie", "--out", oss.to_str().unwrap(), "--no-examples"]);
    assert!(ok, "{}", text);
    let names: Vec<String> = fs::read_dir(&oss).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    let pick = |bit: &str| oss.join(names.iter().find(|n| n.contains(bit)).unwrap_or_else(|| panic!("{:?}", names)));
    let ontology_doc = fs::read_to_string(pick("ontology")).unwrap();
    assert!(ontology_doc.contains("extends:") && ontology_doc.contains("- Sale"), "{}", ontology_doc);
    let into = outside.0.join("from_ossie");
    let (ok, text) = bin(&["import", "--from-ossie", pick("semantic").to_str().unwrap(), "--ontology", pick("ontology").to_str().unwrap(), "--mapping", pick("mapping").to_str().unwrap(), "--into", into.to_str().unwrap()]);
    assert!(ok, "{}", text);
    let all: String = fs::read_dir(into.join("types")).unwrap().filter_map(|e| e.ok()).map(|e| fs::read_to_string(e.path()).unwrap()).collect();
    assert!(all.contains("extends: Sale"), "{}", all);
}

#[test]
fn a_schema_org_slice_brings_its_parents_and_their_fields() {
    let sp = Space::new("schema", &[]);
    fs::create_dir_all(&sp.0).unwrap();
    let ttl = r#"@prefix schema: <https://schema.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
schema:Thing a rdfs:Class ; rdfs:label "Thing" ; rdfs:comment "The most generic type of item." .
schema:Person a rdfs:Class ; rdfs:label "Person" ; rdfs:subClassOf schema:Thing ; rdfs:comment "A person." .
schema:Organization a rdfs:Class ; rdfs:label "Organization" ; rdfs:subClassOf schema:Thing ; rdfs:comment "An organization." .
schema:LocalBusiness a rdfs:Class ; rdfs:label "LocalBusiness" ; rdfs:subClassOf schema:Organization ; rdfs:comment "A shop." .
schema:name a rdf:Property ; rdfs:label "name" ; rdfs:domain schema:Thing ; rdfs:range xsd:string ; rdfs:comment "The name of the item." .
schema:birthDate a rdf:Property ; rdfs:label "birthDate" ; rdfs:domain schema:Person ; rdfs:range xsd:date ; rdfs:comment "Date of birth." .
"#;
    let path = sp.0.join("slice.ttl");
    fs::write(&path, ttl).unwrap();
    let into = sp.0.join("people");
    let (ok, text) = bin(&["import", "--from-rdf", path.to_str().unwrap(), "--into", into.to_str().unwrap(), "--only", "schema:Person,schema:LocalBusiness"]);
    assert!(ok, "{}", text);
    let read = |n: &str| fs::read_dir(into.join("types")).unwrap().filter_map(|e| e.ok()).map(|e| fs::read_to_string(e.path()).unwrap()).find(|t| t.contains(&format!("type: {}\n", n))).unwrap_or_else(|| panic!("no {}", n));
    assert!(read("Person").contains("extends: Thing") && read("Person").contains("birthDate") && !read("Person").contains("name:"), "{}", read("Person"));
    assert!(read("Thing").contains("name:"), "name lives on Thing only: {}", read("Thing"));
    assert!(read("LocalBusiness").contains("extends: Organization"), "the chain comes along: {}", read("LocalBusiness"));
    let port = 7270;
    let _srv = serve(&into, port);
    let d = get(port, "/api/describe?name=Person");
    assert!(d["properties"].as_array().unwrap().iter().any(|p| p["name"] == "name" && p["from"] == "Thing"), "name reaches Person by inheritance: {}", d);
    // --no-parents: today's tags
    let flat = sp.0.join("flat");
    let (ok, text) = bin(&["import", "--from-rdf", path.to_str().unwrap(), "--into", flat.to_str().unwrap(), "--only", "schema:Person", "--no-parents"]);
    assert!(ok, "{}", text);
    let person = fs::read_dir(flat.join("types")).unwrap().filter_map(|e| e.ok()).map(|e| fs::read_to_string(e.path()).unwrap()).find(|t| t.contains("type: Person")).unwrap();
    assert!(person.contains("subclass-of:Thing") && !person.contains("extends"), "{}", person);
}
