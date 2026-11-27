//! End to end over stdio: the real binary, the real example, the real loop.
//! An agent lists tools, reads the summary, saves a metric with propose_patch
//! and gets it queued for review with a sentence a person can read; with
//! edits on, what it saves lands as one commit in the space's history.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let to = dst.join(&name);
        if e.path().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            fs::copy(e.path(), to).unwrap();
        }
    }
}

fn scratch_copy(example: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorefile-e2e-{}-{}", example, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(example), &dir);
    dir
}

/// Send JSON-RPC lines to the server and get one reply per request, in order.
fn talk(dir: &Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["mcp", "--dir", dir.to_str().unwrap(), "--no-examples"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("binary starts");
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{}", r).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).expect("valid JSON per line")).collect()
}

fn text(reply: &serde_json::Value) -> String {
    reply["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()
}

#[test]
fn an_agent_reads_the_summary_and_its_saves_are_queued_in_words() {
    let dir = scratch_copy("ecommerce");
    let replies = talk(&dir, &[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"summary","arguments":{}}}),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_metric","metric":"trial_starts","description":"Trials on the SaaS line.","connection":"shop_bigquery","table":"analytics.ga4_events","filter":"event_name = 'trial_started'","measure":"count distinct user_pseudo_id","grain":"day","owner":"growth@lumenhome","reason":"The user said trials are the win for SaaS."}}}),
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"describe","arguments":{"name":"forecast_success"}}}),
        serde_json::json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_metric","metric":"bad","connection":"meta_ads"}}}),
    ]);
    assert_eq!(replies.len(), 6, "one reply per request: {:?}", replies);
    let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("offer to save it with `propose_patch`"), "the offer-to-save sentence is in the instructions");
    let tools: Vec<&str> = replies[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tools.contains(&"propose_patch") && tools.contains(&"validate") && tools.contains(&"where_is"), "{:?}", tools);
    let summary = text(&replies[2]);
    assert!(summary.contains("## Metrics") && summary.contains("**success**"), "the summary names the success metric:\n{}", summary);
    assert!(summary.contains("## Functions") && summary.contains("forecast_success"), "and the forecasting function");
    let saved = text(&replies[3]);
    assert!(saved.contains("queued for review"), "read-only server queues the save: {}", saved);
    assert!(saved.contains("\"summary\": \"added metric trial_starts"), "{}", saved);
    let described = text(&replies[4]);
    assert!(described.contains("mmm_run_budget_optimizer") && described.contains("\"kind\": \"function\""), "{}", described);
    assert!(replies[5]["result"]["isError"].as_bool().unwrap_or(false), "an MCP source without a tool is refused");

    let proposals: Vec<PathBuf> = fs::read_dir(dir.join(".lorefile/proposals")).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(proposals.len(), 1, "exactly one proposal was queued");
    let p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&proposals[0]).unwrap()).unwrap();
    assert_eq!(p["status"], "pending");
    assert_eq!(p["summary"], "added metric trial_starts (shop_bigquery.analytics.ga4_events)");
    assert_eq!(p["ok"], true);
    let activity = fs::read_to_string(dir.join(".lorefile/activity.jsonl")).unwrap();
    assert!(activity.contains("proposed added metric trial_starts"), "{}", activity);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_space_of_the_users_own_takes_the_write_when_edits_are_allowed() {
    let dir = scratch_copy("supply-chain");
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["mcp", "--dir", dir.to_str().unwrap(), "--no-examples", "--allow-edit"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{}", serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"propose_patch","arguments":{"op":"add_term","term":"seller","means":"Supplier","reason":"Finance says seller."}}})).unwrap();
        writeln!(stdin, "{}", serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"describe","arguments":{"name":"seller"}}})).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let replies: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(text(&replies[0]).contains("written to glossary.yaml"), "{}", text(&replies[0]));
    assert!(fs::read_to_string(dir.join("glossary.yaml")).unwrap().contains("- term: seller"));
    assert!(text(&replies[1]).contains("Supplier"), "the server reloaded and resolves the new word: {}", text(&replies[1]));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_blank_space_is_taught_in_one_sitting_over_mcp() {
    let dir = std::env::temp_dir().join(format!("lorefile-e2e-blank-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("types")).unwrap();
    fs::write(dir.join("ontology.yaml"), "name: brand\ndomains:\n  - name: Core\n    types: []\n").unwrap();
    fs::write(dir.join("glossary.yaml"), "terms: []\n").unwrap();
    // the space's own history, as the page starts it for a new space
    let git = |args: &[&str]| -> String {
        let out = Command::new("git").arg("-C").arg(&dir).args(args).output().expect("git runs");
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let has_git = Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if has_git {
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Lorefile"]);
        git(&["config", "user.email", "lorefile@localhost"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "--author=person via page <person-via-page@localhost>", "-m", "Start the space"]);
    }
    let batch = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"propose_patch","arguments":{"reason":"First reporting conversation.","ops":[
        {"op":"add_source","connection":"shop","kind":"bigquery","project":"acme","dataset":"shop","description":"The warehouse."},
        {"op":"add_type","name":"Order","domain":"Core","description":"One checkout."},
        {"op":"set_source","type":"Order","connection":"shop","table":"orders","key":"order_id"},
        {"op":"add_metric","metric":"success","description":"Paid orders.","connection":"shop","table":"orders","filter":"payment_status = 'paid'","measure":"count distinct order_id","grain":"day","owner":"growth"},
        {"op":"add_term","term":"basket","means":"Order"}
    ]}}});
    // read-only: queued as one proposal holding every file
    let replies = talk(&dir, &[batch.clone()]);
    let t = text(&replies[0]);
    assert!(t.contains("queued for review"), "{}", t);
    for piece in ["5 changes: added the bigquery source shop", "created Order", "bound Order to shop.orders", "added metric success (shop.orders)", "basket"] {
        assert!(t.contains(piece), "missing {:?} in:\n{}", piece, t);
    }
    let proposals: Vec<PathBuf> = fs::read_dir(dir.join(".lorefile/proposals")).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(proposals.len(), 1, "one proposal, not five");
    let p: serde_json::Value = serde_json::from_str(&fs::read_to_string(&proposals[0]).unwrap()).unwrap();
    assert_eq!(p["files"].as_array().unwrap().len(), 4, "four files: source, entity, metrics, glossary");
    assert_eq!(p["ok"], true);
    assert!(!dir.join("types/order.yaml").exists(), "nothing written on a read-only server");

    // with edits allowed: written, and the space now answers
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["mcp", "--dir", dir.to_str().unwrap(), "--no-examples", "--allow-edit"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{}", batch).unwrap();
        writeln!(stdin, "{}", serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"summary","arguments":{}}})).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let replies: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(text(&replies[0]).contains("\"written\": true"), "{}", text(&replies[0]));
    let summary = text(&replies[1]);
    assert!(summary.contains("**success**") && summary.contains("Order") && summary.contains("basket"), "the next question sees everything:\n{}", summary);
    assert!(fs::read_to_string(dir.join("types/order.yaml")).unwrap().contains("table: orders"));
    if has_git {
        // the batch is one commit in the space's history, in words, by the agent
        let log = git(&["log", "--format=%an <%ae>|%s"]);
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2, "{}", log);
        assert!(lines[0].starts_with("agent via MCP <agent-via-mcp@localhost>|An agent learned: 5 changes: added the bigquery source shop"), "{}", lines[0]);
        assert_eq!(lines[1], "person via page <person-via-page@localhost>|Start the space");
        assert_eq!(git(&["show", "--format=%b", "-s", "HEAD"]), "First reporting conversation.", "the agent's reason is the body");
        assert_eq!(git(&["show", "--format=", "--name-only", "HEAD"]).lines().count(), 5, "four files and trust.yaml, one commit");
        let trust = fs::read_to_string(dir.join("trust.yaml")).unwrap();
        for line in ["Order: { grade: proposed, by: \"agent via MCP\"", "success: { grade: proposed, by: \"agent via MCP\"", "basket: { grade: proposed, by: \"agent via MCP\""] {
            assert!(trust.contains(line), "the batch stamps every graded thing it wrote as proposed: {}", trust);
        }
        assert!(text(&replies[0]).contains("Grade: proposed. Tell the user a person still has to accept it"), "{}", text(&replies[0]));
        assert!(summary.contains("UNVERIFIED") && summary.contains("Trust ("), "{}", summary);
        assert!(summary.contains("History: on, 2 changes, last: An agent learned: 5 changes"), "the summary says so:\n{}", summary);
        assert_eq!(git(&["status", "--porcelain", "--", "ontology.yaml", "glossary.yaml", "types", "sources", "logic"]), "", "nothing left uncommitted");
    }
    let _ = fs::remove_dir_all(&dir);
}

/// `lorefile <args> --dir <dir> --no-examples`, as a person at the command line.
fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lorefile")).args(args).args(["--no-examples", "--dir", dir.to_str().unwrap()]).output().unwrap()
}

fn stdio_edit(dir: &Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile"))
        .args(["mcp", "--dir", dir.to_str().unwrap(), "--no-examples", "--allow-edit"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{}", r).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn call(id: u64, name: &str, args: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
}

#[test]
fn agents_cannot_promote_themselves_or_overwrite_what_a_person_stands_behind() {
    let dir = scratch_copy("ecommerce");
    let dir = dir.with_file_name(format!("{}-grades", dir.file_name().unwrap().to_string_lossy()));
    let _ = fs::remove_dir_all(&dir);
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ecommerce"), &dir);
    // a person grades two things at the command line
    let out = cli(&dir, &["trust", "accept", "Order", "--by", "Priya"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(cli(&dir, &["trust", "accept", "success"]).status.success());
    let trust_before = fs::read_to_string(dir.join("trust.yaml")).unwrap();
    assert!(trust_before.contains("Order: { grade: accepted, by: \"Priya via CLI\""), "{}", trust_before);
    let listed = String::from_utf8_lossy(&cli(&dir, &["trust"]).stdout).to_string();
    assert!(listed.contains("accepted (2)") && listed.contains("ungraded ("), "{}", listed);

    let replies = stdio_edit(&dir, &[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}),
        call(2, "propose_change", serde_json::json!({"file":"trust.yaml","content":"trust:\n  metrics: {}\n","reason":"I checked it."})),
        call(3, "propose_patch", serde_json::json!({"op":"set_source","type":"Order","connection":"shop_bigquery","table":"orders_v2","key":"order_id","reason":"New table."})),
        call(4, "propose_patch", serde_json::json!({"op":"set_source","type":"Customer","connection":"shop_bigquery","table":"customers","key":"customer_id","reason":"The warehouse has them."})),
        call(5, "propose_patch", serde_json::json!({"op":"record_check","kind":"metric","name":"success","against":"shop_bigquery","note":"412 paid orders for 16-22 Nov; the user said Shopify shows the same","reason":"We compared it together."})),
        call(6, "propose_patch", serde_json::json!({"op":"record_check","kind":"binding","name":"Customer","against":"shop_bigquery","note":"looks fine"})),
        call(7, "describe", serde_json::json!({"name":"success"})),
        call(8, "summary", serde_json::json!({})),
        call(9, "describe", serde_json::json!({"name":"Customer"})),
    ]);
    assert_eq!(replies.len(), 9, "{:?}", replies);
    let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("When you rely on one marked proposed or UNVERIFIED, say so before the number or the action"), "{}", instructions);
    assert!(instructions.contains("You cannot set a grade") && instructions.contains("never added"), "{}", instructions);
    // trust.yaml is never an agent's to write, edits on or off
    assert!(replies[1]["result"]["isError"].as_bool().unwrap_or(false), "{}", replies[1]);
    assert!(text(&replies[1]).contains("trust.yaml is written by Lorefile when a person accepts or checks something"), "{}", text(&replies[1]));
    // the accepted binding: queued, not written, and the reply says why
    let held = text(&replies[2]);
    assert!(held.contains("source of Order is accepted (Priya,") && held.contains("so this change waits for a person: queued as p-"), "{}", held);
    assert!(fs::read_to_string(dir.join("types/order.yaml")).unwrap().contains("connection: shop_files"), "Order's binding is untouched");
    // the ungraded binding: written, as proposed
    assert!(text(&replies[3]).contains("Grade: proposed"), "{}", text(&replies[3]));
    let trust_now = fs::read_to_string(dir.join("trust.yaml")).unwrap();
    assert!(trust_now.contains("Customer: { grade: proposed, by: \"agent via MCP\""), "{}", trust_now);
    assert!(trust_now.contains("success: { grade: accepted"), "an agent's report changes no grade: {}", trust_now);
    // record_check: queued whatever the mode; refused on what nobody accepted yet
    assert!(text(&replies[4]).contains("queued as p-") && text(&replies[4]).contains("nothing is graded yet"), "{}", text(&replies[4]));
    assert!(replies[5]["result"]["isError"].as_bool().unwrap_or(false) && text(&replies[5]).contains("accept it first"), "{}", text(&replies[5]));
    // what an agent reads
    let d: serde_json::Value = serde_json::from_str(&text(&replies[6])).unwrap();
    assert_eq!(d["trust"]["grade"], "accepted");
    assert!(d["trust"]["say"].as_str().unwrap().starts_with("Accepted by a person on"), "{}", d["trust"]);
    let summary = text(&replies[7]);
    assert!(summary.contains("Trust (metrics, data bindings, glossary words and guardrails): 0 checked against data · 2 accepted by a person · 1 proposed by an agent"), "{}", summary);
    assert!(summary.contains("**Customer** [data binding UNVERIFIED"), "{}", summary);
    assert!(!summary.contains("**Order** [data"), "accepted lines carry no mark: {}", summary);
    let c: serde_json::Value = serde_json::from_str(&text(&replies[8])).unwrap();
    assert!(c["source"]["trust"]["say"].as_str().unwrap().starts_with("Unverified: an agent proposed this choice of source"), "{}", c["source"]);
    let proposals: Vec<serde_json::Value> = fs::read_dir(dir.join(".lorefile/proposals")).unwrap().map(|e| serde_json::from_str(&fs::read_to_string(e.unwrap().path()).unwrap()).unwrap()).collect();
    assert_eq!(proposals.len(), 2, "the held binding and the check, nothing for trust.yaml: {:?}", proposals);
    assert!(proposals.iter().any(|p| p["check"]["name"] == "success" && p["check"]["against"] == "shop_bigquery"), "{:?}", proposals);

    // edits off: the same refusal, nothing queued for it
    let replies = talk(&dir, &[call(1, "propose_change", serde_json::json!({"file":"trust.yaml","content":"trust: {}\n"}))]);
    assert!(replies[0]["result"]["isError"].as_bool().unwrap_or(false), "{}", replies[0]);
    assert_eq!(fs::read_dir(dir.join(".lorefile/proposals")).unwrap().count(), 2);
    // a grade key typed into a fact file is an unknown key, refused by the compiler
    let metric_with_grade = "metrics:\n  - metric: sneaky\n    description: x\n    owner: y\n    grade: checked\n    source: { connection: shop_bigquery, table: orders }\n";
    let replies = stdio_edit(&dir, &[call(1, "propose_change", serde_json::json!({"file":"logic/sneaky.yaml","content":metric_with_grade}))]);
    assert!(text(&replies[0]).contains("unknown field `grade`") && !dir.join("logic/sneaky.yaml").exists(), "{}", text(&replies[0]));
    // basis: one of four words
    let replies = stdio_edit(&dir, &[call(1, "propose_patch", serde_json::json!({"op":"add_metric","metric":"guess","description":"x","connection":"shop_bigquery","table":"orders","basis":"guessed"}))]);
    assert!(text(&replies[0]).contains("basis must be transactional, attributed, modelled or experimental"), "{}", text(&replies[0]));
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// A file from the user: match_file, learn_file, next month (specs/2026-11-27-learn-file.md)
// ---------------------------------------------------------------------------

const SENTINEL: &str = "ZZ-SENTINEL-41";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/learn").join(name)
}

/// A blank space like the one `lorefile new` makes for Branda: one domain, no types yet.
fn blank_branda(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lorefile-e2e-learn-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("types")).unwrap();
    fs::write(dir.join("ontology.yaml"), "name: branda\nversion: 0.1.0\ndomains:\n  - name: Core\n    description: The main things this business works with.\n    types: []\n").unwrap();
    fs::write(dir.join("glossary.yaml"), "# Business words and what they mean here.\nterms: []\n").unwrap();
    dir
}

/// What an agent sends for a CSV: the file name, the headers, five examples per column, counts.
fn file_args(file: &str, file_name: &str, described: bool, sentinel: bool) -> serde_json::Value {
    let mut rdr = csv::Reader::from_path(fixture(file)).unwrap();
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(|s| s.to_string()).collect();
    let rows: Vec<Vec<String>> = rdr.records().map(|r| r.unwrap().iter().map(|s| s.to_string()).collect()).collect();
    let words = |h: &str| match h {
        "line_id" => "Identifier of one order line.",
        "order_id" => "The order this line belongs to; an order can have several lines.",
        "revenue" => "What the customer paid for the line: units × unit_price − discount.",
        "refunded" => "True when the line was refunded.",
        _ => "A column of the monthly promo-sales export.",
    };
    let columns: Vec<serde_json::Value> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let vals: Vec<String> = rows.iter().map(|r| r[i].clone()).collect();
            let mut ex: Vec<String> = vals.iter().take(5).cloned().collect();
            if sentinel && h == "promo_code" {
                ex[0] = SENTINEL.to_string();
            }
            let distinct = vals.iter().collect::<std::collections::HashSet<_>>().len();
            let mut c = serde_json::json!({ "name": h, "examples": ex, "distinct": distinct, "empty": 0 });
            if described {
                c["description"] = serde_json::json!(words(h));
            }
            c
        })
        .collect();
    serde_json::json!({ "file_name": file_name, "row_count": rows.len(), "columns": columns })
}

fn answers() -> serde_json::Value {
    serde_json::json!({ "success": { "column": "revenue", "how": "sum" }, "exclude": [{ "column": "refunded", "equals": true }], "exclude_words": "Refunded lines never count.", "currency": "USD", "also_called": ["promo report"] })
}

fn learn_with_answers(sentinel: bool) -> serde_json::Value {
    let mut a = file_args("promo_sales_2026-11.csv", "promo_sales.csv", true, sentinel);
    a["entity"] = serde_json::json!("PromoSale");
    a["description"] = serde_json::json!("One order line sold with a promo code, from the monthly promo-sales export.");
    a["answers"] = answers();
    a["reason"] = serde_json::json!("The user uploaded promo_sales.csv and said: save this; revenue counts, refunds do not.");
    a
}

fn body(reply: &serde_json::Value) -> serde_json::Value {
    serde_json::from_str(&text(reply)).unwrap_or_else(|e| panic!("not JSON ({}): {}", e, text(reply)))
}

fn proposals_of(dir: &Path) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = fs::read_dir(dir.join(".lorefile/proposals")).map(|rd| rd.map(|e| serde_json::from_str(&fs::read_to_string(e.unwrap().path()).unwrap()).unwrap()).collect()).unwrap_or_default();
    out.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    out
}

fn run(dir: &Path, extra: &[&str], requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lorefile")).args(["mcp", "--dir", dir.to_str().unwrap()]).args(extra).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{}", r).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn a_file_is_matched_learned_as_one_proposal_and_recognised_while_it_waits() {
    let dir = blank_branda("loop");
    let r = talk(&dir, &[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        call(3, "match_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv", false, false)),
        call(4, "learn_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv", true, false)),
        call(5, "learn_file", learn_with_answers(false)),
        call(6, "match_file", file_args("promo_sales_2026-12.csv", "promo_sales_2026-12.csv", false, false)),
        call(7, "where_is", serde_json::json!({ "type": "PromoSale" })),
    ]);
    let instructions = r[0]["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("call `match_file`") && instructions.contains("they mean its shape and meaning, not its rows"), "the file recipe: {}", instructions);
    let tools = r[1]["result"]["tools"].as_array().unwrap();
    for name in ["match_file", "learn_file"] {
        let t = tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{} listed", name));
        assert_eq!(t["inputSchema"]["additionalProperties"], false, "{}: a complete schema", name);
        assert_eq!(t["inputSchema"]["properties"]["columns"]["items"]["additionalProperties"], false);
        assert_eq!(t["annotations"]["readOnlyHint"], name == "match_file");
    }
    assert!(tools.iter().find(|t| t["name"] == "learn_file").unwrap()["inputSchema"]["properties"]["answers"]["properties"]["exclude"].is_object());

    // November on a blank space: new, a draft, two or three questions
    let m = body(&r[2]);
    assert_eq!(m["match"], "new");
    assert_eq!(m["draft"]["entity"], "PromoSale");
    assert_eq!(m["draft"]["grain"], "one row per line (key line_id)");
    let q: Vec<&str> = m["questions"].as_array().unwrap().iter().map(|q| q["ask"].as_str().unwrap()).collect();
    assert!((2..=3).contains(&q.len()), "{:?}", q);
    assert_eq!(q[0], "Which number in this file is the result you care about: revenue?");
    assert_eq!(q[1], "Do refunded rows count?");

    // without answers nothing is saved; with "line" in the descriptions the question names the grain
    let dry = body(&r[3]);
    assert_eq!(dry["saved"], false);
    assert_eq!(dry["questions"][1]["ask"], "Do refunded lines count?");
    // with answers: one proposal, carrying what was learned (names, never values)
    let saved = body(&r[4]);
    assert_eq!(saved["saved"], true, "{}", saved);
    assert_eq!(saved["ok"], true, "compiles clean: {}", saved);
    assert_eq!(saved["errors"], 0);
    assert_eq!(saved["learned"], "learned promo_sales.csv: PromoSale (14 fields, key line_id) from user_files; metric promo_sale_revenue = sum(revenue) where refunded = false");
    let ps = proposals_of(&dir);
    assert_eq!(ps.len(), 1, "exactly one proposal");
    let p = &ps[0];
    assert_eq!(p["status"], "pending");
    assert_eq!(p["summary"], saved["learned"], "the card's title is the one line");
    assert_eq!(p["learned"]["entity"], "PromoSale");
    assert_eq!(p["learned"]["headers"].as_array().unwrap().len(), 14);
    assert_eq!(p["files"].as_array().unwrap().len(), 4);
    assert!(!dir.join("types/promo_sale.yaml").exists(), "nothing written with edits off");

    // December while it waits: recognised, read the same way, and said to be unaccepted
    let w = body(&r[5]);
    assert_eq!(w["match"], "waiting", "{}", w);
    assert_eq!(w["proposal"], p["id"]);
    assert_eq!(w["read_it"][0]["filter"], "refunded = false");
    assert!(w["say"].as_str().unwrap().contains("not accepted yet"), "{}", w);
    assert!(r[6]["result"]["isError"].as_bool().unwrap_or(false), "PromoSale does not exist until a person accepts it");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn learning_the_same_file_again_replaces_what_waits_instead_of_piling_up() {
    let dir = blank_branda("replace");
    let mut more = learn_with_answers(false);
    more["answers"]["also_called"] = serde_json::json!(["promo report", "promo export"]);
    let r = talk(&dir, &[call(1, "learn_file", learn_with_answers(false)), call(2, "learn_file", learn_with_answers(false)), call(3, "learn_file", more)]);
    let first = body(&r[0])["queued"].as_str().unwrap().to_string();
    let again = body(&r[1]);
    assert_eq!((again["saved"].as_bool(), again["queued"].as_str()), (Some(false), Some(first.as_str())), "identical: no second proposal: {}", again);
    assert!(again["say"].as_str().unwrap().starts_with("Already waiting as"));
    let third = body(&r[2]);
    assert_eq!(third["replaced"], first.as_str(), "{}", third);
    let ps = proposals_of(&dir);
    assert_eq!(ps.len(), 2);
    let old = ps.iter().find(|p| p["id"] == first.as_str()).unwrap();
    assert_eq!((old["status"].as_str(), old["replaced_by"].as_str()), (Some("replaced"), third["queued"].as_str()));
    assert_eq!(ps.iter().filter(|p| p["status"] == "pending").count(), 1, "one waiting, never a pile");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn learn_file_is_refused_on_a_shipped_example_and_match_file_works_there() {
    let dir = blank_branda("shipped");
    let mut learn = learn_with_answers(false);
    learn["space"] = serde_json::json!("@ecommerce");
    let mut look = file_args("promo_sales_2026-12.csv", "promo_sales_2026-12.csv", false, false);
    look["space"] = serde_json::json!("@ecommerce");
    let r = run(&dir, &[], &[call(1, "learn_file", learn), call(2, "match_file", look)]);
    assert!(r[0]["result"]["isError"].as_bool().unwrap_or(false));
    assert!(text(&r[0]).contains("shipped example and stays read-only"), "{}", text(&r[0]));
    assert_eq!(body(&r[1])["match"], "new");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn with_edits_on_a_learned_file_is_written_and_graded_proposed() {
    let dir = blank_branda("edits");
    let r = run(&dir, &["--no-examples", "--allow-edit"], &[
        call(1, "learn_file", learn_with_answers(false)),
        call(2, "match_file", file_args("promo_sales_2026-12.csv", "promo_sales_2026-12.csv", false, false)),
        call(3, "where_is", serde_json::json!({ "type": "PromoSale" })),
        call(4, "instances", serde_json::json!({ "type": "PromoSale" })),
        call(5, "query", serde_json::json!({ "sql": "select sum(revenue) from PromoSale" })),
        call(6, "summary", serde_json::json!({})),
    ]);
    let saved = body(&r[0]);
    assert_eq!((saved["written"].as_bool(), saved["saved"].as_bool()), (Some(true), Some(true)), "{}", saved);
    assert!(saved["say"].as_str().unwrap().contains("Grade: proposed"));
    for f in ["sources/user_files.yaml", "types/promo_sale.yaml", "logic/metrics.yaml"] {
        assert!(dir.join(f).exists(), "{} written", f);
    }
    let trust = fs::read_to_string(dir.join("trust.yaml")).unwrap();
    for line in ["promo_sale_revenue: { grade: proposed", "PromoSale: { grade: proposed", "\"promo report\": { grade: proposed"] {
        assert!(trust.contains(line), "{} in\n{}", line, trust);
    }
    let same = body(&r[1]);
    assert_eq!(same["match"], "same");
    assert_eq!(same["read_it"][0]["trust"]["grade"], "proposed");
    assert!(same["read_it"][0]["trust"]["say"].as_str().unwrap().starts_with("Unverified"), "{}", same);
    assert!(same["say"].as_str().unwrap().contains("UNVERIFIED"), "the agent says unverified before the number: {}", same);
    let w = body(&r[2]);
    assert_eq!((w["kind"].as_str(), w["readable_here"].as_bool()), (Some("from_user"), Some(false)));
    assert!(w["hint"].as_str().unwrap().starts_with("Rows of PromoSale arrive as a file a person gives you (usually named promo_sales…)"), "{}", w);
    assert!(text(&r[3]).contains("arrive as a file a person gives you"), "instances says so instead of 0 rows: {}", text(&r[3]));
    assert!(text(&r[4]).contains("arrive as a file a person gives you"), "query too: {}", text(&r[4]));
    let line = text(&r[5]).lines().find(|l| l.starts_with("- **PromoSale**")).unwrap_or("").to_string();
    assert!(line.contains("[file from the user]") && line.contains("UNVERIFIED"), "{}", line);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn example_values_never_land_anywhere() {
    let has_git = Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    for edits in [false, true] {
        let dir = blank_branda(if edits { "sentinel-on" } else { "sentinel-off" });
        let git = |args: &[&str]| -> String {
            let out = Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap();
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        if has_git {
            git(&["init", "-q", "-b", "main"]);
            git(&["config", "user.name", "Lorefile"]);
            git(&["config", "user.email", "lorefile@localhost"]);
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", "Start the space"]);
        }
        let mut quoting = learn_with_answers(true);
        quoting["columns"][3]["description"] = serde_json::json!(format!("The promo code, e.g. {}.", SENTINEL));
        let extra: &[&str] = if edits { &["--no-examples", "--allow-edit"] } else { &["--no-examples"] };
        let r = run(&dir, extra, &[
            call(1, "match_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv", false, true)),
            call(2, "learn_file", file_args("promo_sales_2026-11.csv", "promo_sales.csv", true, true)),
            call(3, "learn_file", quoting),
            call(4, "match_file", file_args("promo_sales_2026-12.csv", "promo_sales_2026-12.csv", false, true)),
        ]);
        assert_eq!(r.len(), 4);
        for reply in &r {
            assert!(!reply.to_string().contains(SENTINEL), "a reply repeats the value: {}", reply);
        }
        let saved = body(&r[2]);
        assert_eq!(saved["saved"], true, "{}", saved);
        assert!(saved["left_out"][0].as_str().unwrap().contains("`promo_code`"), "the quoting description is left out and named: {}", saved);
        for e in walkdir::WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()) {
            let bytes = fs::read(e.path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains(SENTINEL), "{} holds the value", e.path().display());
        }
        if has_git {
            if edits {
                assert!(git(&["log", "--format=%s"]).contains("learned promo_sales.csv"), "the learn is a commit: {}", git(&["log", "--format=%s"]));
            }
            let revs = git(&["rev-list", "--all"]);
            let revs: Vec<&str> = revs.split_whitespace().collect();
            let mut args = vec!["grep", "-l", SENTINEL];
            args.extend(revs.iter());
            assert_eq!(git(&args), "", "no commit in history holds the value");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
