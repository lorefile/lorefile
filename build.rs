//! Embeds `examples/` and `packs/` into the binary so every install ships the
//! same read-only example spaces. Generates `$OUT_DIR/builtin.rs` holding one
//! `(group, "name/relative/path", bytes)` entry per file; `spaces.rs` writes
//! them out under `.lorefile/examples/` at start-up.

use std::env;
use std::fs;
use std::path::Path;

fn walk(base: &Path, dir: &Path, group: &str, out: &mut String) {
    let mut entries: Vec<_> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // .lorefile, .DS_Store, .gitignore
        }
        if p.is_dir() {
            walk(base, &p, group, out);
        } else {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            out.push_str(&format!("    ({:?}, {:?}, include_bytes!({:?})),\n", group, rel, p.display().to_string()));
        }
    }
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    let mut body = String::new();
    for (group, folder) in [("example", "examples"), ("pack", "packs")] {
        let base = Path::new(&manifest).join(folder);
        println!("cargo:rerun-if-changed={}", base.display());
        walk(&base, &base, group, &mut body);
    }
    let code = format!(
        "/// (group, \"space-name/relative/path\", bytes) for every file shipped inside the binary.\npub static FILES: &[(&str, &str, &[u8])] = &[\n{}];\n",
        body
    );
    fs::write(Path::new(&out_dir).join("builtin.rs"), code).unwrap();
}
