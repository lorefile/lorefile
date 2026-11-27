//! OWL, SKOS and SHACL. Every shipped example and pack survives `export --rdf`
//! then `import --from-rdf` into a fresh folder (the same summary, the same
//! counts, everything described the same, the same Turtle again), and so does
//! the ecommerce example through JSON-LD, N-Triples and RDF/XML. A slice of
//! schema.org (vendored in tests/fixtures/rdf, CC BY-SA 3.0) imports as
//! entities with fields, links and descriptions. A SHACL shape becomes a
//! guardrail that `check` enforces. One IRI on two things is an error, and an
//! entity's own `iri:` reaches both the RDF and the Ossie export.
#![cfg(feature = "rdf")]

use serde_json::Value as Json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lorefile"))
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lorefile-rdf-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

fn ok(out: Output, what: &str) -> Output {
    assert!(out.status.success(), "{} failed:\n{}{}", what, String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    out
}

fn stdout(args: &[&str], dir: &Path) -> String {
    let out = ok(bin().args(args).arg("--no-examples").arg("--dir").arg(dir).output().unwrap(), &format!("{:?} on {}", args, dir.display()));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn json(args: &[&str], dir: &Path) -> Json {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    serde_json::from_str(&stdout(&all, dir)).unwrap()
}

fn export(space: &Path, out: &Path) {
    ok(bin().args(["export", "--rdf", "--no-examples", "--dir"]).arg(space).arg("--out").arg(out).output().unwrap(), "export --rdf");
}

fn import(file: &Path, into: &Path, extra: &[&str]) -> Output {
    bin().arg("import").arg("--from-rdf").arg(file).arg("--into").arg(into).arg("--report").args(extra).output().unwrap()
}

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (rel, text) in files {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}

/// Every name in a space, by kind, read from its Ossie export (which lists them all).
fn names(space: &Path) -> Vec<String> {
    let b = json(&["export", "--ossie"], space);
    let mut out: Vec<String> = b["ontology"]["ontology"].as_array().unwrap().iter().map(|c| c["concept"].as_str().unwrap().to_string()).collect();
    let ext = b["semantic_model"]["nonportable_extensions"].as_array().unwrap().iter().find(|e| e["vendor_name"] == "lorefile").unwrap();
    for (list, key) in [("constraints", "id"), ("actions", "action"), ("functions", "function"), ("sources", "connection")] {
        for x in ext["data"][list].as_array().into_iter().flatten() {
            out.push(x[key].as_str().unwrap().to_string());
        }
    }
    out.extend(ext["data"]["metric_order"].as_array().into_iter().flatten().map(|m| m.as_str().unwrap().to_string()));
    out
}

/// What the compiler made of a space: the summary, the counts, and `describe`
/// of every entity, guardrail, action, metric, function and source.
fn compiled(space: &Path, names: &[String]) -> Json {
    let mut described = serde_json::Map::new();
    for n in names {
        described.insert(n.clone(), json(&["describe", n], space));
    }
    serde_json::json!({
        "summary": stdout(&["summary"], space),
        "stats": json(&["validate"], space)["stats"],
        "described": described,
    })
}

fn round_trip(space: &Path) {
    let name = space.file_name().unwrap().to_string_lossy().to_string();
    let work = scratch(&format!("rt-{}", name));
    let ttl = work.join("space.ttl");
    export(space, &ttl);
    let imp = work.join("imp");
    ok(import(&ttl, &imp, &[]), &format!("import of {}", name));
    let all = names(space);
    let (a, b) = (compiled(space, &all), compiled(&imp, &all));
    assert_eq!(a["summary"], b["summary"], "{}: the summary changed", name);
    assert_eq!(a["stats"], b["stats"], "{}: the counts changed", name);
    for (n, d) in a["described"].as_object().unwrap() {
        assert_eq!(d, &b["described"][n], "{}: {} changed", name, n);
    }
    let again = work.join("again.ttl");
    export(&imp, &again);
    assert_eq!(fs::read_to_string(&ttl).unwrap(), fs::read_to_string(&again).unwrap(), "{}: exporting the imported space gave different Turtle", name);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn a_format_2_space_survives_turtle_and_back() {
    // the same round trip as every shipped folder, with --keep-grades so trust.yaml (here: one
    // distinct pair) comes back too; then the files themselves
    let space = root().join("tests/fixtures/format2");
    let work = scratch("rt-format2");
    let ttl = work.join("shop.ttl");
    export(&space, &ttl);
    let turtle = fs::read_to_string(&ttl).unwrap();
    assert!(turtle.contains("lorefile:formatVersion 2") || turtle.contains("formatVersion> 2"), "a format-2 space says so in the graph:\n{}", &turtle[..turtle.len().min(1500)]);
    let imp = work.join("imp");
    ok(import(&ttl, &imp, &["--keep-grades"]), "import of format2");
    let all = names(&space);
    let (a, b) = (compiled(&space, &all), compiled(&imp, &all));
    assert_eq!(a["summary"], b["summary"], "format2: the summary changed");
    // every count but the file count: people.yaml stays behind, so the copy has one file fewer
    let (mut sa, mut sb) = (a["stats"].clone(), b["stats"].clone());
    for s in [&mut sa, &mut sb] {
        s.as_object_mut().unwrap().remove("files");
    }
    assert_eq!(sa, sb, "format2: the counts changed");
    for (n, d) in a["described"].as_object().unwrap() {
        assert_eq!(d, &b["described"][n], "format2: {} changed", n);
    }
    let again = work.join("again.ttl");
    export(&imp, &again);
    assert_eq!(turtle, fs::read_to_string(&again).unwrap(), "format2: exporting the imported space gave different Turtle");
    let promo = fs::read_to_string(imp.join("types/promo_sale.yaml")).unwrap();
    assert!(promo.contains("extends: Sale") && promo.contains("promo_code") && !promo.contains("revenue"), "{}", promo);
    let manifest = fs::read_to_string(imp.join("ontology.yaml")).unwrap();
    assert!(manifest.contains("color: teal"), "the area keeps its colour: {}", manifest);
    let trust = fs::read_to_string(imp.join("trust.yaml")).unwrap();
    assert!(trust.contains("distinct:") && trust.contains("metric:gross_sales"), "{}", trust);
    assert!(!imp.join("people.yaml").exists(), "people.yaml is this team's and does not travel");
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn every_shipped_folder_survives_turtle_and_back() {
    let spaces: Vec<PathBuf> = ["examples", "packs"]
        .iter()
        .flat_map(|f| fs::read_dir(root().join(f)).unwrap().map(|e| e.unwrap().path()))
        .filter(|d| d.join("ontology.yaml").exists())
        .collect();
    assert!(spaces.len() >= 8, "every example and pack, found {}", spaces.len());
    let failed: Vec<String> = spaces
        .into_iter()
        .map(|s| std::thread::spawn(move || round_trip(&s)))
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|h| h.join().err().map(|e| e.downcast_ref::<String>().cloned().unwrap_or_else(|| "a round trip panicked".into())))
        .collect();
    assert!(failed.is_empty(), "{}", failed.join("\n\n"));
}

#[test]
fn json_ld_n_triples_and_rdf_xml_carry_the_same_graph() {
    let space = root().join("examples/ecommerce");
    let work = scratch("formats");
    let ttl = work.join("space.ttl");
    export(&space, &ttl);
    let turtle = fs::read_to_string(&ttl).unwrap();
    assert!(turtle.contains(":Order a owl:Class"), "entities are OWL classes");
    assert!(turtle.contains(":Order.placed_by a owl:ObjectProperty , owl:FunctionalProperty"), "a many-to-one link is functional");
    assert!(turtle.contains("skos:altLabel \"basket\""), "glossary words are SKOS alternative labels");
    assert!(turtle.contains(":EC-001 a sh:NodeShape") && turtle.contains("sh:deactivated true") && turtle.contains("lorefile:action :cancel_order"), "a guardrail an action checks is a switched-off SHACL shape naming the action");
    assert!(turtle.contains("sh:in ( \"placed\" \"paid\" \"packing\" )"), "a simple rule has its native SHACL form");
    for ext in ["jsonld", "nt", "rdf"] {
        let f = work.join(format!("space.{}", ext));
        export(&space, &f);
        let imp = work.join(format!("imp-{}", ext));
        ok(import(&f, &imp, &[]), &format!("import of the .{} export", ext));
        let back = work.join(format!("back-{}.ttl", ext));
        export(&imp, &back);
        assert_eq!(turtle, fs::read_to_string(&back).unwrap(), ".{} lost something on the way", ext);
    }
    let jsonld: Json = serde_json::from_str(&fs::read_to_string(work.join("space.jsonld")).unwrap()).unwrap();
    assert!(jsonld["@context"]["owl"].is_string() && jsonld["@graph"].is_array());
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn a_slice_of_schema_org_imports() {
    let fixture = root().join("tests/fixtures/rdf/schemaorg-slice.ttl");
    let d = scratch("schemaorg");
    let out = ok(import(&fixture, &d, &[]), "import of the schema.org slice");
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(report.contains("Order is a kind of Intangible"), "a sub-class becomes a tag and a note: {}", report);
    assert!(report.contains("used only for reasoning"), "{}", report);

    let stats = json(&["validate"], &d)["stats"].clone();
    assert_eq!(stats["types"], 4, "Organization, Person, Order and Product; the DataType classes are not entities");
    let order = json(&["describe", "Order"], &d);
    assert_eq!(order["iri"], "https://schema.org/Order");
    assert!(order["description"].as_str().unwrap().starts_with("An order is a confirmation of a transaction"), "{}", order["description"]);
    assert!(order["tags"].as_array().unwrap().iter().any(|t| t == "subclass-of:Intangible"));
    let field = |n: &str| order["properties"].as_array().unwrap().iter().find(|p| p["name"] == n).cloned().unwrap_or_else(|| panic!("Order has no field {}: {}", n, order["properties"]));
    assert_eq!(field("order_date")["type"], "date");
    assert_eq!(field("order_date")["iri"], "https://schema.org/orderDate");
    assert_eq!(field("order_date")["description"], "Date order was placed.");
    assert_eq!(field("order_number")["type"], "string");
    assert_eq!(field("is_gift")["type"], "boolean");
    let link = |n: &str| order["relationships"].as_array().unwrap().iter().find(|r| r["name"] == n).cloned().unwrap_or_else(|| panic!("Order has no link {}", n));
    assert_eq!(link("customer")["to"], "Organization");
    assert_eq!(link("ordered_item")["to"], "Product");
    assert_eq!(link("seller")["description"], "An entity which offers (sells / leases / lends / loans) the services / goods. A seller may also be a provider.");
    // one property, several entities: the same IRI on each field is allowed
    let person = json(&["describe", "Person"], &d);
    assert!(person["properties"].as_array().unwrap().iter().any(|p| p["iri"] == "https://schema.org/address"));
    let yaml = fs::read_to_string(d.join("types/order.yaml")).unwrap();
    assert!(yaml.contains("iri: schema:Order") && yaml.contains("iri: schema:orderDate"), "IRIs are written short: {}", yaml);

    // and back out: schema.org's own IRIs, the shared property with a union domain
    let ttl = d.join("out.ttl");
    export(&d, &ttl);
    let t = fs::read_to_string(&ttl).unwrap();
    assert!(t.contains("schema:Order a owl:Class") && t.contains("schema:orderDate a owl:DatatypeProperty"), "{}", &t[..t.len().min(3000)]);
    assert!(t.contains("owl:unionOf ( schema:Organization schema:Person )"));
    let _ = fs::remove_dir_all(&d);

    // --only, and --depth for the classes those link to
    let d = scratch("schemaorg-only");
    ok(import(&fixture, &d, &["--only", "schema:Order"]), "import --only schema:Order");
    assert_eq!(json(&["validate"], &d)["stats"]["types"], 1);
    let _ = fs::remove_dir_all(&d);
    let d = scratch("schemaorg-depth");
    ok(import(&fixture, &d, &["--only", "Order", "--depth", "1"]), "import --only Order --depth 1");
    assert_eq!(json(&["validate"], &d)["stats"]["types"], 4, "Order links to Organization, Person (via the union) and Product");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_shacl_shape_becomes_a_guardrail_that_check_enforces() {
    let d = scratch("shacl");
    let out = ok(import(&root().join("tests/fixtures/rdf/orders-shacl.ttl"), &d, &[]), "import of the SHACL fixture");
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(report.contains("sh:sparql"), "a constraint with no CEL form is listed for a person: {}", report);
    let order = json(&["describe", "Order"], &d);
    assert_eq!(order["key"], "order_id", "owl:hasKey marks the key");
    let rule = json(&["describe", "OrderShape-status"], &d);
    assert_eq!(rule["severity"], "block");
    assert_eq!(rule["rule"], r#"has(self.status) && status in ["placed", "paid", "shipped"]"#);
    assert_eq!(json(&["describe", "OrderShape-total"], &d)["severity"], "warn", "sh:Warning is a warning");
    assert!(fs::read_to_string(d.join("glossary.yaml")).unwrap().contains("term: basket"), "skos:altLabel is a glossary word");

    write(&d, &[("actions/ship_order.yaml", "action: ship_order\ndescription: Hand an order to the carrier.\ninputs: { order: Order }\nchecks: [OrderShape-status, OrderShape-total, OrderShape-note]\n")]);
    let check = |record: &str| bin().args(["check", "ship_order"]).arg(format!("order={}", record)).arg("--no-examples").arg("--dir").arg(&d).output().unwrap();
    let bad = check(r#"{"order_id": "A1", "status": "cancelled", "total": 10.5}"#);
    assert_eq!(bad.status.code(), Some(2), "a status outside sh:in blocks: {}", String::from_utf8_lossy(&bad.stdout));
    let missing = check(r#"{"order_id": "A1", "total": 10}"#);
    assert_eq!(missing.status.code(), Some(2), "sh:minCount 1: a missing status blocks: {}", String::from_utf8_lossy(&missing.stdout));
    let good = check(r#"{"order_id": "A1", "status": "paid", "total": 10, "note": "leave at the door"}"#);
    assert_eq!(good.status.code(), Some(0), "{}", String::from_utf8_lossy(&good.stdout));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn one_iri_on_two_things_is_an_error() {
    let d = scratch("dup");
    write(
        &d,
        &[
            ("ontology.yaml", "name: dup\nprefixes: { acme: \"https://acme.example/ont#\" }\n"),
            ("types/order.yaml", "type: Order\niri: schema:Order\ndescription: One checkout.\nproperties: { id: { type: string, key: true }, name: { type: string, iri: schema:name } }\n"),
            ("types/basket.yaml", "type: Basket\niri: schema:Order\ndescription: An unpaid order.\nproperties: { id: { type: string, key: true }, name: { type: string, iri: schema:name } }\n"),
            ("types/part.yaml", "type: Part\niri: acme:Part\ndescription: A part.\nproperties: { id: { type: string, key: true, iri: \"nope:id\" } }\n"),
        ],
    );
    let out = bin().args(["--json", "validate", "--no-examples", "--dir"]).arg(&d).output().unwrap();
    assert!(!out.status.success());
    let rep: Json = serde_json::from_slice(&out.stdout).unwrap();
    let diags = rep["diagnostics"].as_array().unwrap();
    let e016: Vec<&Json> = diags.iter().filter(|x| x["code"] == "E016").collect();
    assert_eq!(e016.len(), 1, "two entities, one IRI; the shared schema:name on two entities' fields is fine: {:?}", diags);
    assert!(e016[0]["message"].as_str().unwrap().contains("https://schema.org/Order"));
    assert!(diags.iter().any(|x| x["code"] == "E017" && x["message"].as_str().unwrap().contains("nope")), "an undeclared prefix: {:?}", diags);

    // the same field IRI twice on one entity is an error too
    write(&d, &[("types/basket.yaml", "type: Basket\ndescription: An unpaid order.\nproperties: { id: { type: string, key: true }, name: { type: string, iri: schema:name }, title: { type: string, iri: schema:name } }\n")]);
    let rep = json_any(&["validate"], &d);
    assert!(rep["diagnostics"].as_array().unwrap().iter().any(|x| x["code"] == "E016" && x["message"].as_str().unwrap().contains("Basket.title")), "{}", rep);
    let _ = fs::remove_dir_all(&d);
}

fn json_any(args: &[&str], dir: &Path) -> Json {
    let out = bin().arg("--json").args(args).arg("--no-examples").arg("--dir").arg(dir).output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn an_entitys_own_iri_reaches_rdf_and_ossie() {
    let d = scratch("own-iri");
    write(
        &d,
        &[
            ("ontology.yaml", "name: shop\nnamespace: \"https://shop.example/ont#\"\n"),
            ("types/order.yaml", "type: Order\niri: schema:Order\ndescription: One checkout.\nproperties:\n  order_id: { type: string, key: true }\n  placed_on: { type: date, iri: schema:orderDate }\nsource: { connection: files, table: orders, key: order_id }\n"),
            ("sources/files.yaml", "connection: files\nkind: postgres\ndescription: The shop database.\n"),
        ],
    );
    ok(bin().args(["validate", "--no-examples", "--dir"]).arg(&d).output().unwrap(), "validate");
    let order = json(&["describe", "Order"], &d);
    assert_eq!(order["iri"], "https://schema.org/Order");
    assert_eq!(order["properties"][0]["iri"], "https://shop.example/ont#Order.order_id", "derived from the namespace");
    assert!(stdout(&["summary"], &d).contains("**Order** (schema:Order)"));

    let t = stdout(&["export", "--rdf"], &d);
    assert!(t.contains("schema:Order a owl:Class") && t.contains("schema:orderDate a owl:DatatypeProperty") && t.contains("@prefix : <https://shop.example/ont#>"), "{}", t);
    let bundle = json(&["export", "--ossie"], &d);
    assert_eq!(bundle["ontology"]["ontology"][0]["iri"], "https://schema.org/Order");

    // both bridges bring it back
    let work = scratch("own-iri-back");
    let ttl = work.join("shop.ttl");
    export(&d, &ttl);
    ok(import(&ttl, &work.join("from-rdf"), &[]), "import of the RDF");
    let y = fs::read_to_string(work.join("from-rdf/types/order.yaml")).unwrap();
    assert!(y.contains("iri: schema:Order") && y.contains("iri: schema:orderDate"), "{}", y);
    assert!(fs::read_to_string(work.join("from-rdf/ontology.yaml")).unwrap().contains("namespace: https://shop.example/ont#"));
    ok(bin().args(["export", "--ossie", "--no-examples", "--dir"]).arg(&d).arg("--out").arg(work.join("ossie")).output().unwrap(), "export --ossie");
    ok(
        bin().args(["import", "--from-ossie"]).arg(work.join("ossie/shop.semantic_model.yaml")).arg("--ontology").arg(work.join("ossie/shop.ontology.yaml")).arg("--into").arg(work.join("from-ossie")).output().unwrap(),
        "import of the Ossie export",
    );
    assert_eq!(json(&["describe", "Order"], &work.join("from-ossie"))["iri"], "https://schema.org/Order");
    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn an_rdf_import_that_would_not_compile_writes_nothing() {
    let work = scratch("refused");
    write(
        &work,
        &[(
            "bad.ttl",
            r#"@prefix : <https://lorefile.dev/ns/bad#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix lorefile: <https://lorefile.dev/vocab#> .
<https://lorefile.dev/ns/bad> a owl:Ontology ; rdfs:label "bad" ; lorefile:formatVersion 1 .
:Thing a owl:Class ; rdfs:label "Thing" ; lorefile:file "types/thing.yaml" ; lorefile:order 0 .
:X-1 a sh:NodeShape ; lorefile:id "X-1" ; sh:message "A rule about nothing" ; lorefile:cel "true" ; lorefile:appliesTo "Nowhere" ; lorefile:file "constraints/x.yaml" .
"#,
        )],
    );
    let into = work.join("space");
    let out = import(&work.join("bad.ttl"), &into, &[]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{}", err);
    assert!(err.contains("nothing was written") && err.contains("E004") && err.contains("constraints/x.yaml:"), "names the error, the file and the line: {}", err);
    assert!(!into.exists(), "no folder, no files");
    let _ = fs::remove_dir_all(&work);
}

fn http(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("GET {} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n", path, port);
    for (k, v) in headers {
        req.push_str(&format!("{}: {}\r\n", k, v));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, head.to_lowercase(), body.to_string())
}

#[test]
fn the_server_serves_turtle_and_json_ld_behind_the_login() {
    let port = 7123;
    let token = "rdf-test-token-0123456789abcdef";
    let mut child = bin()
        .args(["serve", "--no-examples", "--dir"])
        .arg(root().join("examples/ecommerce"))
        .args(["--port", &port.to_string(), "--token", token])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && TcpStream::connect(("127.0.0.1", port)).is_err() {
        std::thread::sleep(Duration::from_millis(100));
    }
    let (status, _, _) = http(port, "/api/export?format=turtle", &[]);
    let auth = format!("Bearer {}", token);
    let turtle = http(port, "/api/export?format=turtle", &[("Authorization", &auth)]);
    let jsonld = http(port, "/api/export?format=jsonld", &[("Authorization", &auth)]);
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(status, 401, "no token, no export");
    assert_eq!(turtle.0, 200, "{}", turtle.2);
    assert!(turtle.1.contains("content-type: text/turtle") && turtle.1.contains("filename=\"lumen-ecommerce.ttl\""), "{}", turtle.1);
    assert!(turtle.2.contains(":Order a owl:Class"));
    assert_eq!(jsonld.0, 200);
    assert!(jsonld.1.contains("application/ld+json"));
    let j: Json = serde_json::from_str(&jsonld.2).unwrap();
    assert!(j["@graph"].as_array().unwrap().len() > 100);
}

/// Optional: a second, independent parser. Set LOREFILE_RDFLIB=1 with `uv` on
/// the PATH (it fetches Python's rdflib) to check the Turtle with rdflib too.
#[test]
fn rdflib_reads_the_turtle_too() {
    if std::env::var_os("LOREFILE_RDFLIB").is_none() {
        eprintln!("skipped: set LOREFILE_RDFLIB=1 (needs uv) to parse the export with Python's rdflib");
        return;
    }
    let work = scratch("rdflib");
    let ttl = work.join("space.ttl");
    export(&root().join("examples/ecommerce"), &ttl);
    let out = Command::new("uv")
        .args(["run", "--quiet", "--with", "rdflib", "python", "-I", "-c"])
        .arg("import sys, rdflib; g = rdflib.Graph(); g.parse(sys.argv[1], format='turtle'); print(len(g))")
        .arg(&ttl)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let n: usize = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    assert!(n > 1000, "rdflib read {} triples", n);
    let _ = fs::remove_dir_all(&work);
}

/// One `tools/call` to an agent's MCP server with edits on (stdio).
fn agent_call(dir: &Path, name: &str, args: serde_json::Value) -> String {
    let mut child = bin().args(["mcp", "--no-examples", "--allow-edit", "--dir"]).arg(dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    writeln!(child.stdin.take().unwrap(), "{}", serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A copy of the e-commerce example with every kind of grade: vouched for, checked, and one proposed by an agent.
fn graded_shop(name: &str) -> PathBuf {
    let dir = scratch(name);
    copy_tree(&root().join("examples/ecommerce"), &dir);
    ok(bin().args(["trust", "accept", "--all", "--by", "Priya", "--no-examples", "--dir"]).arg(&dir).output().unwrap(), "trust accept --all");
    ok(bin().args(["trust", "check", "success", "--against", "shop_bigquery", "--note", "412 paid orders for 16-22 Nov, the same as Shopify's own report", "--by", "Priya", "--no-examples", "--dir"]).arg(&dir).output().unwrap(), "trust check");
    let said = agent_call(&dir, "propose_patch", serde_json::json!({"op":"add_metric","metric":"meta_conversions","description":"Conversions Meta reports.","basis":"attributed","connection":"meta_ads","tool":"get_insights","params":{"level":"campaign"},"measure":"sum(conversions)","owner":"growth@lumenhome"}));
    assert!(said.contains("Grade: proposed"), "{}", said);
    dir
}

/// Two people accept one thing each; their names have no ASCII letters, so they slug to the same word.
fn graded_two_names(name: &str) -> PathBuf {
    let dir = scratch(name);
    copy_tree(&root().join("examples/ecommerce"), &dir);
    ok(bin().args(["trust", "accept", "success", "--by", "李", "--no-examples", "--dir"]).arg(&dir).output().unwrap(), "trust accept");
    ok(bin().args(["trust", "accept", "paid_performance", "--by", "王", "--no-examples", "--dir"]).arg(&dir).output().unwrap(), "trust accept");
    dir
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with('.') {
            continue;
        }
        if e.path().is_dir() { copy_tree(&e.path(), &dst.join(&n)) } else { fs::copy(e.path(), dst.join(&n)).unwrap(); }
    }
}

#[test]
fn grades_travel_as_lorefile_trust_and_prov_and_come_back_only_when_asked() {
    let space = graded_shop("graded");
    let work = scratch("graded-work");
    fs::create_dir_all(&work).unwrap();
    let ttl = work.join("lumen.ttl");
    export(&space, &ttl);
    let text = fs::read_to_string(&ttl).unwrap();
    assert!(text.contains("@prefix prov: <http://www.w3.org/ns/prov#>"), "the prov prefix is built in");
    for piece in ["lorefile:trust lorefile:Checked", "lorefile:trust lorefile:Proposed", "lorefile:bindingTrust lorefile:Accepted", "a lorefile:Grading , prov:Activity", "prov:wasAssociatedWith", "prov:used", "^^xsd:dateTime", "a prov:Person", "a prov:SoftwareAgent", "prov:wasAttributedTo", "lorefile:aspect \"binding\""] {
        assert!(text.contains(piece), "the Turtle has {:?}", piece);
    }
    assert!(text.contains("\"412 paid orders for 16-22 Nov, the same as Shopify's own report\""), "the note is the record's comment");
    let basis = text.contains("\\\"basis\\\":\\\"attributed\\\"") || text.contains("\"basis\":\"attributed\"");
    assert!(basis, "basis travels inside the metric's definition");

    // by default: imported facts start ungraded, and the report says what was claimed
    let plain = work.join("plain");
    let out = ok(import(&ttl, &plain, &[]), "import without --keep-grades");
    assert!(!plain.join("trust.yaml").exists());
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(said.contains("arrived with a grade") && said.contains("they start ungraded here"), "{}", said);
    // --keep-grades, your own export: the same trust.yaml
    let kept = work.join("kept");
    ok(import(&ttl, &kept, &["--keep-grades"]), "import --keep-grades");
    assert_eq!(fs::read_to_string(kept.join("trust.yaml")).unwrap(), fs::read_to_string(space.join("trust.yaml")).unwrap(), "the grades come back exactly");
    let rep = json(&["validate"], &kept);
    assert!(rep["diagnostics"].as_array().unwrap().iter().all(|d| d["code"] != "W113" && d["code"] != "W114"), "{}", rep);
    // a grade whose fact was edited in the file is dropped and listed
    let edited = work.join("edited.ttl");
    fs::write(&edited, text.replace("payment_status = 'paid'", "payment_status = 'pending'")).unwrap();
    let changed = work.join("changed");
    let out = ok(import(&edited, &changed, &["--keep-grades"]), "import an edited export");
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(said.contains("dropped grade: metric success: checked in the file, but the definition differs"), "{}", said);
    let t = fs::read_to_string(changed.join("trust.yaml")).unwrap();
    assert!(!t.contains("success:") && t.contains("Order:"), "{}", t);
    // a file from outside cannot carry a ready-made trust.yaml in: an object that asks to be written there
    // is not, with or without the flag, and a claim that does not parse is said, not swallowed
    let book = serde_json::json!({ "trust": { "metrics": { "success": { "grade": "checked", "by": "Priya via page", "at": "2026-11-28", "of": "abc", "against": "shop_bigquery", "note": "n" } } } });
    let forged_node = format!("\n:forged a lorefile:Source ; lorefile:definition \"{}\"^^rdf:JSON ; lorefile:file \"trust.yaml\" .\n", book.to_string().replace('"', "\\\""));
    let bad_claim = "\n:trust.bad a lorefile:Grading , prov:Activity ; lorefile:definition \"{\\\"kind\\\":\\\"metrics\\\",\\\"name\\\":\\\"success\\\",\\\"record\\\":{\\\"grade\\\":\\\"verified\\\"}}\"^^rdf:JSON .\n";
    // the honest export minus its own grade records, plus the forged node
    let no_grades: String = text.split("\n\n").filter(|b| !b.contains("a lorefile:Grading")).collect::<Vec<_>>().join("\n\n");
    let forged = work.join("forged.ttl");
    fs::write(&forged, format!("{}{}", no_grades, forged_node)).unwrap();
    for flags in [&[][..], &["--keep-grades"][..]] {
        let into = work.join(format!("forged-{}", flags.len()));
        let out = import(&forged, &into, flags);
        assert!(!into.join("trust.yaml").exists(), "no grades file from a forged node (flags {:?}): {}", flags, String::from_utf8_lossy(&out.stderr));
    }
    let unread = work.join("unread.ttl");
    fs::write(&unread, format!("{}{}", no_grades, bad_claim)).unwrap();
    let into = work.join("unread");
    let out = ok(import(&unread, &into, &["--keep-grades"]), "import a file with one unreadable claim");
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 grade claim in the file could not be read"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!into.join("trust.yaml").exists(), "and it grades nothing");
    // two people whose names slug alike are two PROV agents
    let two = graded_two_names("two-names");
    let ttl2 = work.join("two.ttl");
    export(&two, &ttl2);
    let t2 = fs::read_to_string(&ttl2).unwrap();
    assert!(t2.contains("rdfs:label \"李 via CLI\"") && t2.contains("rdfs:label \"王 via CLI\""), "each name has its own agent node");
    let _ = fs::remove_dir_all(&two);
    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_dir_all(&space);
}
