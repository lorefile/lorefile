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
    assert!(held.contains("data binding Order is accepted (Priya,") && held.contains("so this change waits for a person: queued as p-"), "{}", held);
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
    assert!(c["source"]["trust"]["say"].as_str().unwrap().starts_with("Unverified: an agent proposed this data binding"), "{}", c["source"]);
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
