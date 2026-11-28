//! Apache Ossie. Every shipped example and pack survives `export --ossie` then
//! `import --from-ossie` into a fresh folder: the same summary, the same counts,
//! every entity and metric described the same, and the same export again.
//! Ossie's own example files (vendored in tests/fixtures/ossie with their ASF
//! headers) import. An import that would not compile writes nothing; one into
//! an existing space adds files and overwrites none. The server serves the
//! export behind the login. And when an Ossie checkout (`OSSIE_DIR`) and `uv`
//! are at hand, Ossie's own validator accepts the ecommerce export.

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
    let d = std::env::temp_dir().join(format!("lorefile-ossie-{}-{}", name, std::process::id()));
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

/// Exports a space into `out` and returns the path stem of the three files.
fn export(space: &Path, out: &Path) -> PathBuf {
    ok(bin().args(["export", "--ossie", "--no-examples", "--dir"]).arg(space).arg("--out").arg(out).output().unwrap(), "export");
    let sm = fs::read_dir(out).unwrap().map(|e| e.unwrap().path()).find(|p| p.to_string_lossy().ends_with(".semantic_model.yaml")).expect("a semantic model file");
    PathBuf::from(sm.to_string_lossy().trim_end_matches(".semantic_model.yaml").to_string())
}

fn with_suffix(stem: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", stem.display(), suffix))
}

fn import(args: &[&Path], into: &Path) -> Output {
    import_with(args, into, &[])
}

fn import_with(args: &[&Path], into: &Path, extra: &[&str]) -> Output {
    let mut c = bin();
    c.arg("import").arg("--from-ossie").arg(args[0]);
    if let Some(o) = args.get(1) {
        c.arg("--ontology").arg(o);
    }
    if let Some(m) = args.get(2) {
        c.arg("--mapping").arg(m);
    }
    c.arg("--into").arg(into).arg("--report").args(extra).output().unwrap()
}

/// What the compiler made of a space, through the public CLI: the summary, the
/// counts, and `describe` of every entity and metric (fields, links both ways,
/// guardrails, actions, data binding, file).
fn compiled(space: &Path, bundle: &Json) -> Json {
    let mut entities = serde_json::Map::new();
    for c in bundle["ontology"]["ontology"].as_array().unwrap() {
        let name = c["concept"].as_str().unwrap();
        entities.insert(name.into(), json(&["describe", name], space));
    }
    let mut metric_names: Vec<String> = Vec::new();
    for ds in bundle["semantic_model"]["datasets"].as_array().unwrap() {
        for m in ds["metrics"].as_array().into_iter().flatten() {
            metric_names.push(m["name"].as_str().unwrap().into());
        }
    }
    for m in bundle["semantic_model"]["metrics"].as_array().into_iter().flatten() {
        metric_names.push(m["name"].as_str().unwrap().into());
    }
    let ext = bundle["semantic_model"]["nonportable_extensions"].as_array().unwrap().iter().find(|e| e["vendor_name"] == "lorefile").unwrap();
    for m in ext["data"]["metrics"].as_array().into_iter().flatten() {
        metric_names.push(m["metric"].as_str().unwrap().into());
    }
    let mut metrics = serde_json::Map::new();
    for n in &metric_names {
        metrics.insert(n.clone(), json(&["describe", n], space));
    }
    serde_json::json!({
        "summary": stdout(&["summary"], space),
        "stats": json(&["validate"], space)["stats"],
        "entities": entities,
        "metrics": metrics,
    })
}

fn round_trip(space: &Path) {
    let name = space.file_name().unwrap().to_string_lossy().to_string();
    {
        {
            let work = scratch(&format!("rt-{}", name));
            let stem = export(space, &work.join("exp"));
            let imp = work.join("imp");
            let sm = with_suffix(&stem, ".semantic_model.yaml");
            let on = with_suffix(&stem, ".ontology.yaml");
            let mp = with_suffix(&stem, ".mapping.yaml");
            ok(import(&[&sm, &on, &mp], &imp), &format!("import of {}", name));

            let before = json(&["export", "--ossie"], space);
            let after = json(&["export", "--ossie"], &imp);
            let (a, b) = (compiled(space, &before), compiled(&imp, &before));
            assert_eq!(a["summary"], b["summary"], "{}: the summary changed", name);
            assert_eq!(a["stats"], b["stats"], "{}: the counts changed", name);
            for (n, d) in a["entities"].as_object().unwrap() {
                assert_eq!(d, &b["entities"][n], "{}: entity {} changed", name, n);
            }
            for (n, d) in a["metrics"].as_object().unwrap() {
                assert_eq!(d, &b["metrics"][n], "{}: metric {} changed", name, n);
            }
            for doc in ["semantic_model", "ontology", "mapping"] {
                assert_eq!(before[doc], after[doc], "{}: exporting the imported space gave a different {}", name, doc);
            }
            let _ = fs::remove_dir_all(&work);
        }
    }
}

#[test]
fn a_format_2_space_survives_export_then_import() {
    // the same round trip as every shipped folder, with --keep-grades so trust.yaml (here: one
    // distinct pair) comes back too; then the files themselves: a kind keeps only its own field,
    // the area its colour, and people.yaml stays behind on purpose
    let space = root().join("tests/fixtures/format2");
    let work = scratch("rt-format2");
    let stem = export(&space, &work.join("exp"));
    let imp = work.join("imp");
    ok(import_with(&[&with_suffix(&stem, ".semantic_model.yaml"), &with_suffix(&stem, ".ontology.yaml"), &with_suffix(&stem, ".mapping.yaml")], &imp, &["--keep-grades"]), "import of format2");
    let before = json(&["export", "--ossie"], &space);
    let after = json(&["export", "--ossie"], &imp);
    let (a, b) = (compiled(&space, &before), compiled(&imp, &before));
    assert_eq!(a["summary"], b["summary"], "format2: the summary changed");
    // every count but the file count: people.yaml stays behind, so the copy has one file fewer
    let (mut sa, mut sb) = (a["stats"].clone(), b["stats"].clone());
    for s in [&mut sa, &mut sb] {
        s.as_object_mut().unwrap().remove("files");
    }
    assert_eq!(sa, sb, "format2: the counts changed");
    for (n, d) in a["entities"].as_object().unwrap() {
        assert_eq!(d, &b["entities"][n], "format2: entity {} changed", n);
    }
    for (n, d) in a["metrics"].as_object().unwrap() {
        assert_eq!(d, &b["metrics"][n], "format2: metric {} changed", n);
    }
    for doc in ["semantic_model", "ontology", "mapping"] {
        assert_eq!(before[doc], after[doc], "format2: exporting the imported space gave a different {}", doc);
    }
    let promo = fs::read_to_string(imp.join("types/promo_sale.yaml")).unwrap();
    assert!(promo.contains("extends: Sale") && promo.contains("promo_code"), "{}", promo);
    for parents_field in ["sale_id", "revenue", "customer_id"] {
        assert!(!promo.contains(parents_field), "the kind does not redeclare {}:\n{}", parents_field, promo);
    }
    let manifest = fs::read_to_string(imp.join("ontology.yaml")).unwrap();
    assert!(manifest.contains("color: teal") && manifest.contains("format: 2"), "{}", manifest);
    let trust = fs::read_to_string(imp.join("trust.yaml")).unwrap();
    assert!(trust.contains("distinct:") && trust.contains("metric:gross_sales"), "{}", trust);
    assert!(!imp.join("people.yaml").exists(), "people.yaml is this team's and does not travel");
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn every_shipped_folder_survives_export_then_import() {
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
fn ossie_example_files_import() {
    let f = root().join("tests/fixtures/ossie");
    let count = |dir: &Path| -> (u64, u64, u64) {
        let s = json(&["validate"], dir)["stats"].clone();
        (s["types"].as_u64().unwrap(), s["metrics"].as_u64().unwrap(), s["relationships"].as_u64().unwrap())
    };

    // the smallest model: one dataset, one model-scoped metric
    let d = scratch("minimal");
    ok(import(&[&f.join("minimal.yaml")], &d), "import of minimal.yaml");
    assert_eq!(count(&d), (1, 1, 0));
    let orders = json(&["describe", "orders"], &d);
    assert_eq!(orders["key"], "order_id");
    assert_eq!(orders["properties"][0]["type"], "integer");
    assert_eq!(orders["source"]["connection"], "sales");
    assert_eq!(orders["source"]["table"], "public.orders");
    assert_eq!(json(&["describe", "order_count"], &d)["sql"], "COUNT(orders.order_id)");
    let _ = fs::remove_dir_all(&d);

    // flights, the semantic model alone: six datasets, no metrics, no joins
    let d = scratch("flights-model");
    ok(import(&[&f.join("flights.semantic_model.yaml")], &d), "import of the flights model");
    assert_eq!(count(&d), (6, 0, 0));
    let _ = fs::remove_dir_all(&d);

    // with its ontology and mapping: concepts become entities (bound to their
    // dataset when there is one), relationships become links, keys and types
    // come from the mapping
    let d = scratch("flights-all");
    let out = ok(import(&[&f.join("flights.semantic_model.yaml"), &f.join("flights.ontology.yaml"), &f.join("flights.mapping.yaml")], &d), "import of flights with ontology and mapping");
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(report.contains("three or more roles dropped"), "the report lists what was dropped: {}", report);
    let (types, metrics, links) = count(&d);
    assert_eq!((types, metrics), (12, 0));
    assert!(links >= 10, "relationships between concepts become links, got {}", links);
    let aircraft = json(&["describe", "Aircraft"], &d);
    assert_eq!(aircraft["key"], "tail_nr", "the key comes from the mapping");
    assert_eq!(aircraft["source"]["table"], "SCHEMA.AIRCRAFT");
    let flight = json(&["describe", "Flight"], &d);
    assert!(flight["relationships"].as_array().unwrap().iter().any(|r| r["name"] == "aircraft" && r["to"] == "Aircraft"), "{}", flight);
    let dep = flight["properties"].as_array().unwrap().iter().find(|p| p["name"] == "dep_delay").unwrap();
    assert_eq!(dep["type"], "number", "typed from the ontology's Delay value type through the mapping");
    assert!(json(&["describe", "Manufacturer"], &d)["source"].is_null(), "a concept with no dataset of its own is an entity without data");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn an_import_that_would_not_compile_writes_nothing() {
    let work = scratch("refused");
    fs::create_dir_all(&work).unwrap();
    let sm = work.join("bad.yaml");
    fs::write(
        &sm,
        r#"version: 0.3.0.dev0
name: bad
datasets:
  - name: things
    source: db.things
    fields:
      - name: id
        expression: { sql: id }
nonportable_extensions:
  - vendor_name: lorefile
    data:
      constraints:
        - id: X-1
          name: A rule about nothing
          applies_to: Nowhere
          rule: "true"
          file: constraints/x.yaml
"#,
    )
    .unwrap();
    let into = work.join("space");
    let out = import(&[&sm], &into);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{}", err);
    assert!(err.contains("nothing was written") && err.contains("E004") && err.contains("constraints/x.yaml:"), "names the error, the file and the line: {}", err);
    assert!(!into.exists(), "no folder, no files");
    let _ = fs::remove_dir_all(&work);
}

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

#[test]
fn an_import_into_an_existing_space_adds_files_and_overwrites_none() {
    let space = scratch("extend");
    copy_dir(&root().join("examples/ecommerce"), &space);
    let manifest = fs::read_to_string(space.join("ontology.yaml")).unwrap();
    ok(import(&[&root().join("tests/fixtures/ossie/minimal.yaml")], &space), "import into the ecommerce copy");
    assert_eq!(fs::read_to_string(space.join("ontology.yaml")).unwrap(), manifest, "the space's own manifest is kept");
    assert!(space.join("types/orders.yaml").exists() && space.join("sources/sales.yaml").exists());
    let stats = json(&["validate"], &space)["stats"].clone();
    assert_eq!(stats["types"], 13);
    assert_eq!(stats["metrics"], 3);
    let _ = fs::remove_dir_all(&space);
}

fn http(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    // HTTP/1.0, so the body comes whole rather than chunked
    let mut req = format!("GET {} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n", path, port);
    for (k, v) in headers {
        req.push_str(&format!("{}: {}\r\n", k, v));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    // bytes first: a chunk boundary can split a UTF-8 character, and read_to_string would then drop the whole reply
    let mut bytes = Vec::new();
    let _ = s.read_to_end(&mut bytes);
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, head.to_lowercase(), body.to_string())
}

#[test]
fn the_server_serves_the_export_behind_the_login() {
    let port = 7121;
    let token = "ossie-test-token-0123456789abcdef";
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
    let (status, _, _) = http(port, "/api/export?format=ossie", &[]);
    assert_eq!(status, 401, "no token, no export");
    let auth = format!("Bearer {}", token);
    let (status, head, body) = http(port, "/api/export?format=ossie", &[("Authorization", &auth)]);
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(status, 200, "{}", body);
    assert!(head.contains("content-disposition: attachment; filename=\"lumen-ecommerce.ossie.json\""), "{}", head);
    let j: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(j["version"], "0.3.0.dev0");
    assert_eq!(j["semantic_model"]["name"], "lumen-ecommerce");
    assert_eq!(j["ontology"]["ontology"].as_array().unwrap().len(), 12);
    assert_eq!(j["mapping"]["ontology_ref"]["name"], "lumen-ecommerce");
}

/// Optional: needs a checkout of github.com/apache/ossie in `OSSIE_DIR` and `uv`
/// (which fetches the validator's Python dependencies). Skipped otherwise.
#[test]
fn ossies_own_validator_accepts_the_ecommerce_export() {
    let Some(ossie) = std::env::var_os("OSSIE_DIR").map(PathBuf::from) else {
        eprintln!("skipped: set OSSIE_DIR to a checkout of github.com/apache/ossie to run Ossie's validator");
        return;
    };
    if Command::new("uv").arg("--version").output().map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("skipped: uv is not installed");
        return;
    }
    let work = scratch("validator");
    let stem = export(&root().join("examples/ecommerce"), &work);
    for (suffix, schema) in [(".semantic_model.yaml", "core-spec/ossie-schema.json"), (".ontology.yaml", "ontology/ontology.json"), (".mapping.yaml", "ontology/mapping.json")] {
        let out = Command::new("uv")
            .args(["run", "--quiet"])
            .arg(ossie.join("validation/validate.py"))
            .arg("--schema")
            .arg(ossie.join(schema))
            .arg(with_suffix(&stem, suffix))
            .output()
            .unwrap();
        let shown = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && shown.contains("PASSED"), "Ossie's validator on {}:\n{}", suffix, shown);
    }
    let _ = fs::remove_dir_all(&work);
}

fn agent_call(dir: &Path, name: &str, args: serde_json::Value) -> String {
    use std::io::Write;
    let mut child = bin().args(["mcp", "--no-examples", "--allow-edit", "--dir"]).arg(dir).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().unwrap();
    writeln!(child.stdin.take().unwrap(), "{}", serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).unwrap();
    String::from_utf8_lossy(&child.wait_with_output().unwrap().stdout).to_string()
}

#[test]
fn grades_ride_in_the_lorefile_box_and_a_proposed_definition_says_so_to_other_ai() {
    let space = scratch("graded");
    copy_dir(&root().join("examples/ecommerce"), &space);
    ok(bin().args(["trust", "accept", "success", "--no-examples", "--dir"]).arg(&space).output().unwrap(), "trust accept");
    // an agent rebinds an ungraded entity and adds a metric: both proposed
    assert!(agent_call(&space, "propose_patch", serde_json::json!({"op":"set_source","type":"Customer","connection":"shop_bigquery","table":"customers","key":"customer_id"})).contains("Grade: proposed"));
    assert!(agent_call(&space, "propose_patch", serde_json::json!({"op":"add_metric","metric":"aov","description":"Average order value.","basis":"transactional","connection":"shop_files","table":"orders","measure":"sum(total_usd)","owner":"growth@lumenhome"})).contains("Grade: proposed"));
    let bundle = json(&["export", "--ossie"], &space);
    let sm = &bundle["semantic_model"];
    let ext = sm["nonportable_extensions"].as_array().unwrap().iter().find(|e| e["vendor_name"] == "lorefile").unwrap();
    assert_eq!(ext["data"]["trust"]["metrics"]["success"]["grade"], "accepted", "the trust book rides in the root box");
    let customer = sm["datasets"].as_array().unwrap().iter().find(|d| d["name"] == "Customer").unwrap();
    assert_eq!(customer["ai_context"]["instructions"], "Unverified: an agent proposed this definition and no person has reviewed it.");
    let orders = sm["datasets"].as_array().unwrap().iter().find(|d| d["name"] == "Order").unwrap();
    let aov = orders["metrics"].as_array().into_iter().flatten().chain(sm["metrics"].as_array().into_iter().flatten()).find(|m| m["name"] == "aov").expect("aov is exported");
    assert!(aov["ai_context"]["instructions"].as_str().unwrap().starts_with("Unverified:"), "{}", aov);
    let success = orders["metrics"].as_array().into_iter().flatten().chain(sm["metrics"].as_array().into_iter().flatten()).find(|m| m["name"] == "success");
    assert!(success.map(|m| m["ai_context"].get("instructions").is_none()).unwrap_or(true), "an accepted metric carries no such sentence");

    let work = scratch("graded-work");
    let stem = export(&space, &work.join("exp"));
    let (sm_f, on_f, mp_f) = (with_suffix(&stem, ".semantic_model.yaml"), with_suffix(&stem, ".ontology.yaml"), with_suffix(&stem, ".mapping.yaml"));
    // default: ungraded, the claim listed; the generated sentence is not turned into a description
    let plain = work.join("plain");
    let out = ok(import(&[&sm_f, &on_f, &mp_f], &plain), "import");
    assert!(!plain.join("trust.yaml").exists());
    assert!(String::from_utf8_lossy(&out.stderr).contains("arrived with a grade"), "{}", String::from_utf8_lossy(&out.stderr));
    let c = json(&["describe", "Customer"], &plain);
    assert!(!c["description"].as_str().unwrap().contains("Unverified"), "{}", c["description"]);
    assert_eq!(json(&["describe", "aov"], &plain)["basis"], "transactional", "basis comes back with the metric");
    // a model from outside: a "source" that asks to be written as trust.yaml is not, with or without the flag
    let mut forged = bundle["semantic_model"].clone();
    {
        let ext = forged["nonportable_extensions"].as_array_mut().unwrap().iter_mut().find(|e| e["vendor_name"] == "lorefile").unwrap();
        let claim = ext["data"]["trust"].clone();
        ext["data"]["sources"].as_array_mut().unwrap().push(serde_json::json!({ "trust": claim, "file": "trust.yaml" }));
        ext["data"].as_object_mut().unwrap().remove("trust");
    }
    let forged_f = work.join("forged.semantic_model.yaml");
    fs::write(&forged_f, serde_json::to_string_pretty(&forged).unwrap()).unwrap();
    for keep in [false, true] {
        let into = work.join(format!("forged-{}", keep));
        let mut cmd = bin();
        cmd.arg("import").arg("--from-ossie").arg(&forged_f).arg("--into").arg(&into);
        if keep { cmd.arg("--keep-grades"); }
        let out = cmd.output().unwrap();
        assert!(!into.join("trust.yaml").exists(), "no grades file from a forged source (keep {}): {}", keep, String::from_utf8_lossy(&out.stderr));
    }
    // a claim that does not parse is said and the others still count; our sentence is dropped from a description
    // even when another tool appended to it and flattened ai_context to a string
    let mut odd = bundle["semantic_model"].clone();
    {
        let ext = odd["nonportable_extensions"].as_array_mut().unwrap().iter_mut().find(|e| e["vendor_name"] == "lorefile").unwrap();
        ext["data"]["trust"]["metrics"]["success"]["grade"] = serde_json::json!("verified");
        let customer = odd["datasets"].as_array_mut().unwrap().iter_mut().find(|d| d["name"] == "Customer").unwrap();
        customer["ai_context"] = serde_json::json!("Unverified: an agent proposed this definition and no person has reviewed it. Join on customer_id.");
    }
    let odd_f = work.join("odd.semantic_model.yaml");
    fs::write(&odd_f, serde_json::to_string_pretty(&odd).unwrap()).unwrap();
    let into = work.join("odd");
    let mut cmd = bin();
    cmd.arg("import").arg("--from-ossie").arg(&odd_f).arg("--into").arg(&into).arg("--keep-grades").arg("--report");
    let out = ok(cmd.output().unwrap(), "import a model with one unreadable claim");
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(said.contains("1 grade claim in the file could not be read"), "{}", said);
    let book = fs::read_to_string(into.join("trust.yaml")).unwrap();
    assert!(!book.contains("success:") && book.contains("aov:"), "the unreadable claim grades nothing, the readable ones come back: {}", book);
    let d = json(&["describe", "Customer"], &into)["description"].as_str().unwrap().to_string();
    assert!(!d.contains("Unverified") && d.contains("Join on customer_id."), "{}", d);
    // --keep-grades: the same trust.yaml
    let kept = work.join("kept");
    let mut cmd = bin();
    cmd.arg("import").arg("--from-ossie").arg(&sm_f).arg("--ontology").arg(&on_f).arg("--mapping").arg(&mp_f).arg("--into").arg(&kept).arg("--keep-grades");
    ok(cmd.output().unwrap(), "import --keep-grades");
    assert_eq!(fs::read_to_string(kept.join("trust.yaml")).unwrap(), fs::read_to_string(space.join("trust.yaml")).unwrap());
    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_dir_all(&space);
}
