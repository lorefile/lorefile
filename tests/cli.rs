//! The command line against the shipped folders: every example and pack must
//! compile clean, and the documented commands must answer.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lorefile"))
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn every_example_and_pack_validates_with_no_errors_or_warnings() {
    for folder in ["examples", "packs"] {
        for e in std::fs::read_dir(root().join(folder)).unwrap() {
            let dir = e.unwrap().path();
            if !dir.join("ontology.yaml").exists() {
                continue;
            }
            let out = bin().args(["validate", "--no-examples", "--dir"]).arg(&dir).output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{} has errors:\n{}", dir.display(), err);
            assert!(err.contains("0 warnings"), "{} has warnings:\n{}", dir.display(), err);
        }
    }
}

#[test]
fn a_folder_of_spaces_validates_each_space() {
    let out = bin().args(["validate", "--no-examples", "--dir"]).arg(root().join("examples")).output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{}", err);
    assert!(err.contains("── ecommerce") && err.contains("── supply-chain"), "{}", err);
}

#[test]
fn the_documented_commands_answer() {
    let sc = root().join("examples/supply-chain");
    let out = bin().args(["describe", "Supplier", "--no-examples", "--dir"]).arg(&sc).output().unwrap();
    assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("# Supplier"));
    let out = bin().args(["check", "switch_supplier", "part=BRK-220", "from=V-10001", "to=V-10105", "--no-examples", "--dir"]).arg(&sc).output().unwrap();
    let code = out.status.code().unwrap_or(-1);
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(code == 0 || code == 2, "check exits 0 (allowed) or 2 (blocked), got {}: {}", code, String::from_utf8_lossy(&out.stderr));
    assert!(shown.contains("SC-") || shown.contains("ok"), "it lists the rules it ran: {}", shown);
    let out = bin().args(["summary", "--no-examples", "--dir"]).arg(root().join("examples/ecommerce")).output().unwrap();
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(md.contains("## Metrics") && md.contains("## Functions") && md.contains("## Glossary"), "{}", md);
    let out = bin().args(["--json", "describe", "success", "--no-examples", "--dir"]).arg(root().join("examples/ecommerce")).output().unwrap();
    let j: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(j["kind"], "metric");
    assert!(j["source"]["measure"].as_str().unwrap().starts_with("count distinct order_id"), "{}", j);
    let out = bin().args(["check", "cancel_order", "order=ORD-99999", "--no-examples", "--dir"]).arg(root().join("examples/ecommerce")).output().unwrap();
    assert_eq!(out.status.code(), Some(3), "a key no row matched answers unknown, exit 3");
    assert!(String::from_utf8_lossy(&out.stdout).contains("could not be checked"));
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with('.') {
            continue;
        }
        if e.path().is_dir() { copy_dir(&e.path(), &dst.join(&n)) } else { std::fs::copy(e.path(), dst.join(&n)).unwrap(); }
    }
}

#[test]
fn a_hand_edit_behind_a_grade_is_w113_until_a_person_accepts_again() {
    let dir = std::env::temp_dir().join(format!("lorefile-cli-trust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copy_dir(&root().join("examples/ecommerce"), &dir);
    let run = |args: &[&str]| bin().args(args).args(["--no-examples", "--dir"]).arg(&dir).output().unwrap();
    assert!(run(&["trust", "accept", "success", "--by", "Priya"]).status.success());
    let file = dir.join("logic/measurement.yaml");
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("payment_status = 'paid'", "payment_status = 'captured'")).unwrap();
    let out = run(&["validate"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "a warning, not an error: {}", err);
    assert!(err.contains("W113: metric success changed since it was accepted (Priya, "), "{}", err);
    let listed = String::from_utf8_lossy(&run(&["trust"]).stdout).to_string();
    assert!(listed.contains("changed since a person graded it (1)"), "{}", listed);
    let out = run(&["trust", "accept", "success"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!String::from_utf8_lossy(&run(&["validate"]).stderr).contains("W113"));
    // a basis is one of four words
    std::fs::write(&file, std::fs::read_to_string(&file).unwrap().replacen("basis: transactional", "basis: guessed", 1)).unwrap();
    let err = String::from_utf8_lossy(&run(&["validate"]).stderr).to_string();
    assert!(err.contains("unknown variant `guessed`") && err.contains("transactional") && err.contains("experimental"), "{}", err);
    let _ = std::fs::remove_dir_all(&dir);
}
