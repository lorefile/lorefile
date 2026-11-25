//! Editing the ontology from the page and from MCP while the YAML files stay
//! the only source of truth. Every change goes the same way: the folder is
//! loaded with the change laid over it (nothing touches the disk), the full
//! validator runs, and the file is written only when there are no errors.
//! Git is optional: with `--commit-edits` each write becomes one commit;
//! otherwise the change is left uncommitted for a person to review.

use crate::loader::{self, Overrides};
use crate::ontology::{snake, Ontology};
use crate::validate::{self, Diag, Level, Report};
use serde_json::{json, Value as Json};
use serde_yaml::Value as Yaml;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// How `serve` and `mcp` were started.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditConfig {
    /// `--allow-edit`: writes are accepted. Off by default so a deployment stays read-only.
    pub allow: bool,
    /// `--commit-edits`: each accepted write is committed when the folder is in a git repository.
    pub commit: bool,
}

/// What a request wants to do with one file.
#[derive(Debug, Clone)]
pub enum Change {
    /// Replace (or create) the file with this text.
    Write(String),
    /// Remove the file.
    Delete,
}

/// The result of checking (and maybe applying) a change.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Path relative to the ontology folder.
    pub file: String,
    /// What was changed, for commit messages and the page ("type", "Supplier").
    pub kind: String,
    pub name: String,
    /// True when the ontology with the change applied has no errors.
    pub ok: bool,
    /// True when the file was written or deleted.
    pub written: bool,
    /// Diagnostics worth showing: every error, everything on this file, and anything the change introduced.
    pub diagnostics: Vec<Diag>,
    /// Set when the request itself was wrong (bad path, missing file, unwritable).
    pub refused: Option<String>,
    /// Short hash when `--commit-edits` made a commit.
    pub commit: Option<String>,
    /// Git trouble that did not stop the write.
    pub git_error: Option<String>,
}

impl Outcome {
    fn refuse(file: &str, why: impl Into<String>) -> Outcome {
        Outcome { file: file.to_string(), refused: Some(why.into()), ..Default::default() }
    }

    pub fn errors(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.level == Level::Error).count()
    }

    pub fn to_json(&self) -> Json {
        let errors = self.errors();
        let mut j = json!({
            "ok": self.ok,
            "written": self.written,
            "file": self.file,
            "kind": self.kind,
            "name": self.name,
            "errors": errors,
            "warnings": self.diagnostics.len() - errors,
            "diagnostics": self.diagnostics,
        });
        if let Some(r) = &self.refused {
            j["error"] = json!(r);
        }
        if let Some(c) = &self.commit {
            j["commit"] = json!(c);
        }
        if let Some(e) = &self.git_error {
            j["git_error"] = json!(e);
        }
        j
    }

    /// HTTP status for this outcome: 400 bad request, 422 refused by the validator, 200 otherwise.
    pub fn status(&self) -> u16 {
        if self.refused.is_some() {
            400
        } else if !self.ok {
            422
        } else {
            200
        }
    }
}

// ---------------------------------------------------------------------------
// Path safety
// ---------------------------------------------------------------------------

/// Top-level folders the loader never reads; writing there would silently do nothing.
const IGNORED_TOP: [&str; 2] = ["data", "target"];

/// Normalise a user-supplied path and refuse anything that is not a YAML file
/// strictly inside the ontology folder. Pure: looks at the string only.
pub fn safe_rel_path(file: &str) -> Result<String, String> {
    let trimmed = file.trim();
    if trimmed.is_empty() {
        return Err("file is required, e.g. types/supplier.yaml".into());
    }
    if trimmed.contains('\\') || trimmed.contains('\0') {
        return Err("file path may not contain backslashes or NUL".into());
    }
    if trimmed.starts_with('~') {
        return Err("file path must be relative to the ontology folder".into());
    }
    let p = Path::new(trimmed);
    let mut parts: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(s) => {
                let s = s.to_string_lossy().to_string();
                if s.starts_with('.') {
                    return Err(format!("`{}` is hidden; the loader skips hidden files and folders", s));
                }
                parts.push(s);
            }
            Component::ParentDir => {
                return Err("file path may not contain `..`; it must stay inside the ontology folder".into());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("file path must be relative to the ontology folder".into());
            }
        }
    }
    let Some(last) = parts.last() else {
        return Err("file is required, e.g. types/supplier.yaml".into());
    };
    let lp = Path::new(last);
    let ext = lp.extension().and_then(|e| e.to_str()).unwrap_or("");
    let stem = lp.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !(ext == "yaml" || ext == "yml") || stem.is_empty() {
        return Err(format!("`{}` must be a YAML file (.yaml)", last));
    }
    if parts.len() > 1 && IGNORED_TOP.contains(&parts[0].as_str()) {
        return Err(format!("the loader does not read files under {}/", parts[0]));
    }
    Ok(parts.join("/"))
}

/// Turn a safe relative path into an absolute one and make sure it really is
/// under the root, symlinks included: the deepest existing ancestor is
/// canonicalised and must still be inside the (canonical) root.
pub fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let root_c = root.canonicalize().map_err(|e| format!("ontology folder {}: {}", root.display(), e))?;
    let full = root_c.join(rel);
    let mut probe: &Path = full.as_path();
    loop {
        if probe.exists() {
            let c = probe.canonicalize().map_err(|e| e.to_string())?;
            if !c.starts_with(&root_c) {
                return Err(format!("`{}` resolves outside the ontology folder", rel));
            }
            break;
        }
        match probe.parent() {
            Some(p) => probe = p,
            None => break,
        }
    }
    Ok(full)
}

// ---------------------------------------------------------------------------
// What a file declares
// ---------------------------------------------------------------------------

fn scalar(v: &Yaml) -> Option<String> {
    match v {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Number(n) => Some(n.to_string()),
        Yaml::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Names of the items in a `key:` list, joined ("SC-001, SC-002, +3 more").
fn list_names(v: &Yaml, list: &str, key: &str) -> Option<String> {
    let seq = v.get(list)?.as_sequence()?;
    let names: Vec<String> = seq.iter().filter_map(|i| i.get(key).and_then(scalar)).collect();
    if names.is_empty() {
        return None;
    }
    let mut s = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 3 {
        s.push_str(&format!(", +{} more", names.len() - 3));
    }
    Some(s)
}

/// What a YAML text declares, for commit messages and the page: ("type", "Supplier").
pub fn describe_content(rel: &str, content: &str) -> (String, String) {
    let fallback = || ("file".to_string(), rel.to_string());
    let v: Yaml = match serde_yaml::from_str(content) {
        Ok(v) => v,
        Err(_) => return fallback(),
    };
    let one = |key: &str, kind: &str| v.get(key).and_then(scalar).map(|n| (kind.to_string(), n));
    let many = |list: &str, key: &str, kind: &str| list_names(&v, list, key).map(|n| (kind.to_string(), n));
    one("type", "type")
        .or_else(|| many("types", "type", "type"))
        .or_else(|| one("relationship", "relationship"))
        .or_else(|| many("relationships", "relationship", "relationship"))
        .or_else(|| many("constraints", "id", "rule"))
        .or_else(|| one("action", "action"))
        .or_else(|| many("actions", "action", "action"))
        .or_else(|| one("connection", "source"))
        .or_else(|| many("sources", "connection", "source"))
        .or_else(|| v.get("terms").map(|_| ("glossary".to_string(), rel.to_string())))
        .or_else(|| {
            let is_manifest = v.get("domains").is_some() || Path::new(rel).file_name().and_then(|f| f.to_str()).map(|f| f.starts_with("ontology.")).unwrap_or(false);
            if is_manifest {
                Some(("manifest".to_string(), v.get("name").and_then(scalar).unwrap_or_else(|| rel.to_string())))
            } else {
                None
            }
        })
        .unwrap_or_else(fallback)
}

// ---------------------------------------------------------------------------
// Check and apply
// ---------------------------------------------------------------------------

/// Load the folder with the change laid over it and run the validator.
/// Returns the would-be ontology and its full report. Nothing is written.
pub fn check(root: &Path, rel: &str, change: &Change) -> anyhow::Result<(Ontology, Report)> {
    let mut ov = Overrides::new();
    ov.insert(
        rel.to_string(),
        match change {
            Change::Write(text) => Some(text.clone()),
            Change::Delete => None,
        },
    );
    let raw = loader::load_with(root, &ov)?;
    let ont = Ontology::from_raw(raw);
    let report = validate::validate(&ont);
    Ok((ont, report))
}

/// Which diagnostics to show for a change: every error, everything on the
/// changed file, and anything that was not there before the change.
pub fn relevant(before: &Report, after: &Report, rel: &str) -> Vec<Diag> {
    let existed = |d: &Diag| before.diags.iter().any(|b| b.code == d.code && b.message == d.message && b.file == d.file);
    let mut out: Vec<Diag> = after
        .diags
        .iter()
        .filter(|d| d.level == Level::Error || d.file == rel || !existed(d))
        .cloned()
        .collect();
    out.sort_by_key(|d| if d.level == Level::Error { 0 } else { 1 });
    out
}

fn write_atomic(full: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("creating {}: {}", parent.display(), e))?;
    }
    let tmp = full.with_extension(format!("yaml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, full).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Check a change and, when `cfg.allow` is on and there are no errors, write
/// it and optionally commit. `before` is the report of the ontology as it is
/// now, used to tell new diagnostics from old ones. `via` names the caller
/// ("UI", "MCP"); `label` overrides the kind/name derived from the content;
/// `reason` becomes the commit body.
pub fn apply(root: &Path, cfg: EditConfig, before: &Report, file: &str, change: Change, via: &str, label: Option<(&str, &str)>, reason: &str) -> Outcome {
    let rel = match safe_rel_path(file) {
        Ok(r) => r,
        Err(e) => return Outcome::refuse(file, e),
    };
    let full = match resolve_in_root(root, &rel) {
        Ok(p) => p,
        Err(e) => return Outcome::refuse(&rel, e),
    };
    let existed = full.is_file();
    let (kind, name) = match &change {
        Change::Write(text) => {
            if text.trim().is_empty() {
                return Outcome::refuse(&rel, "content is empty; to remove a file use delete");
            }
            describe_content(&rel, text)
        }
        Change::Delete => {
            if !existed {
                return Outcome::refuse(&rel, format!("{} does not exist", rel));
            }
            let text = std::fs::read_to_string(&full).unwrap_or_default();
            describe_content(&rel, &text)
        }
    };
    let (kind, name) = match label {
        Some((k, n)) if !k.is_empty() && !n.is_empty() => (k.to_string(), n.to_string()),
        _ => (kind, name),
    };
    let mut out = Outcome { file: rel.clone(), kind, name, ..Default::default() };

    let (_, after) = match check(root, &rel, &change) {
        Ok(x) => x,
        Err(e) => {
            out.refused = Some(format!("{:#}", e));
            return out;
        }
    };
    out.diagnostics = relevant(before, &after, &rel);
    out.ok = after.ok();
    if !out.ok || !cfg.allow {
        return out;
    }

    let written = match &change {
        Change::Write(text) => write_atomic(&full, text),
        Change::Delete => std::fs::remove_file(&full).map_err(|e| e.to_string()),
    };
    if let Err(e) = written {
        out.ok = false;
        out.refused = Some(format!("could not write {}: {}", rel, e));
        return out;
    }
    out.written = true;

    if cfg.commit {
        let verb = match &change {
            Change::Delete => "deleted",
            // a create that lands in an existing list file (a rule appended to constraints/x.yaml)
            Change::Write(_) if existed && label.is_some() => "added",
            Change::Write(_) if existed => "edited",
            Change::Write(_) => "created",
        };
        let mut message = format!("ontology: {} {} {} via {}", out.kind, out.name, verb, via);
        if !reason.trim().is_empty() {
            message.push_str("\n\n");
            message.push_str(reason.trim());
        }
        match commit(root, &full, &message) {
            Ok(h) => out.commit = Some(h),
            Err(e) => out.git_error = Some(e),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Creating new objects
// ---------------------------------------------------------------------------

/// `PurchaseOrder` → `purchase_order`, `SC-099` → `sc_099`: a file name.
pub fn file_slug(name: &str) -> String {
    snake(name.trim())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn top_level(content: &str) -> Result<Yaml, String> {
    let v: Yaml = serde_yaml::from_str(content).map_err(|e| format!("content is not valid YAML: {}", e))?;
    if !v.is_mapping() {
        return Err("content must be a YAML mapping (key: value lines)".into());
    }
    Ok(v)
}

fn expect_key(v: &Yaml, keys: &[&str], kind: &str) -> Result<(), String> {
    if keys.iter().any(|k| v.get(*k).is_some()) {
        Ok(())
    } else {
        Err(format!("a {} file needs a top-level `{}:` key", kind, keys[0]))
    }
}

/// Re-indent a mapping so it becomes one item of a YAML list:
/// `id: X\nname: Y` → `  - id: X\n    name: Y` (with `dash_indent` = 2).
/// Comments and multi-line scalars survive because every line moves by the same amount.
fn indent_item(content: &str, dash_indent: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let common = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let dash = format!("{}- ", " ".repeat(dash_indent));
    let body = " ".repeat(dash_indent + 2);
    let mut out = String::new();
    let mut first = true;
    for line in lines {
        if line.trim().is_empty() {
            if !first {
                out.push('\n');
            }
            continue;
        }
        let stripped = if line.len() >= common { &line[common..] } else { line.trim_start() };
        if first {
            out.push_str(&dash);
            first = false;
        } else {
            out.push_str(&body);
        }
        out.push_str(stripped);
        out.push('\n');
    }
    out
}

/// The items a rule submission contains: either a `constraints:` list file
/// (items re-serialised) or one rule mapping (kept as written).
fn constraint_items(content: &str) -> Result<Vec<String>, String> {
    let v = top_level(content)?;
    if let Some(seq) = v.get("constraints").and_then(|c| c.as_sequence()) {
        if seq.is_empty() {
            return Err("the `constraints:` list is empty".into());
        }
        return seq.iter().map(|i| serde_yaml::to_string(i).map_err(|e| e.to_string())).collect();
    }
    expect_key(&v, &["id"], "rule")?;
    for k in ["applies_to", "rule"] {
        if v.get(k).is_none() {
            return Err(format!("a rule needs `{}`", k));
        }
    }
    Ok(vec![content.to_string()])
}

/// A fresh `constraints:` file holding these items.
fn wrap_constraints(items: &[String]) -> String {
    let mut s = String::from("constraints:\n");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            s.push('\n');
        }
        s.push_str(&indent_item(item, 2));
    }
    s
}

/// Append rules to an existing `constraints:` file, keeping its text as it is.
fn append_constraints(existing: &str, items: &[String]) -> Result<String, String> {
    let v: Yaml = serde_yaml::from_str(existing).map_err(|e| format!("the existing file does not parse: {}", e))?;
    let seq = v.get("constraints").and_then(|c| c.as_sequence()).ok_or_else(|| "the existing file has no `constraints:` list".to_string())?;
    // indentation of the existing list items, if they are written block-style
    let dash_indent = existing.lines().skip_while(|l| !l.trim_start().starts_with("constraints:")).skip(1).find_map(|l| {
        let t = l.trim_start();
        if t.starts_with("- ") { Some(l.len() - t.len()) } else { None }
    });
    let out = match dash_indent {
        Some(ind) if !seq.is_empty() => {
            let mut s = existing.trim_end().to_string();
            s.push_str("\n\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    s.push('\n');
                }
                s.push_str(&indent_item(item, ind));
            }
            s
        }
        _ => {
            // flow style or empty list: rewrite the whole list
            let mut all: Vec<String> = seq.iter().map(|i| serde_yaml::to_string(i).map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
            all.extend(items.iter().cloned());
            wrap_constraints(&all)
        }
    };
    serde_yaml::from_str::<Yaml>(&out).map_err(|e| format!("the combined file would not parse: {}", e))?;
    Ok(out)
}

/// Work out where a new object goes and what the file should contain.
/// Returns (relative path, full file text). Nothing is written.
pub fn plan_create(root: &Path, kind: &str, name: &str, content: &str, file: Option<&str>) -> Result<(String, String), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let slug = file_slug(name);
    if slug.is_empty() {
        return Err(format!("`{}` has no characters usable in a file name", name));
    }
    let file = file.map(str::trim).filter(|f| !f.is_empty());
    let (folder, keys): (&str, &[&str]) = match kind {
        "type" => ("types", &["type", "types"]),
        "action" => ("actions", &["action", "actions"]),
        "relationship" => ("relationships", &["relationship", "relationships"]),
        "source" => ("sources", &["connection", "sources"]),
        "constraint" | "rule" => {
            let rel = safe_rel_path(file.unwrap_or(&format!("constraints/{}.yaml", slug)))?;
            let full = resolve_in_root(root, &rel)?;
            let items = constraint_items(content)?;
            let text = if full.is_file() {
                let existing = std::fs::read_to_string(&full).map_err(|e| format!("reading {}: {}", rel, e))?;
                append_constraints(&existing, &items)?
            } else {
                wrap_constraints(&items)
            };
            return Ok((rel, text));
        }
        other => return Err(format!("unknown kind `{}`; use type, constraint, action, relationship or source", other)),
    };
    let v = top_level(content)?;
    expect_key(&v, keys, kind)?;
    let rel = safe_rel_path(file.unwrap_or(&format!("{}/{}.yaml", folder, slug)))?;
    let full = resolve_in_root(root, &rel)?;
    if full.exists() {
        return Err(format!("{} already exists; edit it instead", rel));
    }
    Ok((rel, content.to_string()))
}

/// Templates for new objects, with `{{name}}`, `{{snake}}`, `{{applies_to}}`
/// and `{{input}}` placeholders the page fills in.
pub fn templates() -> Json {
    json!({
        "type": include_str!("starter/templates/type.yaml"),
        "constraint": include_str!("starter/templates/constraint.yaml"),
        "action": include_str!("starter/templates/action.yaml"),
        "relationship": include_str!("starter/templates/relationship.yaml"),
        "source": include_str!("starter/templates/source.yaml"),
    })
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

/// Run git in `repo`; stdout untrimmed (porcelain lines start with a space).
fn git_raw(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(repo).args(args).output().map_err(|e| format!("git: {}", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("git {} failed", args.first().unwrap_or(&"")) } else { err })
    }
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    git_raw(repo, args).map(|s| s.trim().to_string())
}

/// Paths from `git status --porcelain` that lie under `root_c`, relative to it.
fn status_paths(porcelain: &str, repo: &Path, root_c: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for line in porcelain.lines() {
        if line.len() < 4 {
            continue;
        }
        let mut p = line[3..].trim();
        if let Some((_, renamed_to)) = p.split_once(" -> ") {
            p = renamed_to;
        }
        let p = p.trim_matches('"');
        if let Ok(rel) = repo.join(p).strip_prefix(root_c) {
            files.push(rel.to_string_lossy().to_string());
        }
    }
    files
}

/// The git repository containing `dir`, if any.
pub fn git_root(dir: &Path) -> Option<PathBuf> {
    let top = git(dir, &["rev-parse", "--show-toplevel"]).ok()?;
    if top.is_empty() {
        return None;
    }
    let p = PathBuf::from(top);
    Some(p.canonicalize().unwrap_or(p))
}

fn commit(root: &Path, file: &Path, message: &str) -> Result<String, String> {
    let repo = git_root(root).ok_or_else(|| "the ontology folder is not inside a git repository, so nothing was committed".to_string())?;
    let f = file.to_string_lossy().to_string();
    git(&repo, &["add", "-A", "--", &f])?;
    git(&repo, &["commit", "-q", "-m", message, "--", &f])?;
    git(&repo, &["rev-parse", "--short", "HEAD"])
}

/// Uncommitted changes under the ontology folder, for the page footer.
/// `None` when the folder is not in a git repository.
pub fn git_status(root: &Path) -> Option<Json> {
    let repo = git_root(root)?;
    let root_c = root.canonicalize().ok()?;
    let branch = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let root_s = root_c.to_string_lossy().to_string();
    let out = git_raw(&repo, &["status", "--porcelain", "--untracked-files=all", "--", &root_s]).ok()?;
    let files = status_paths(&out, &repo, &root_c);
    Some(json!({ "repo": true, "branch": branch, "uncommitted": files }))
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn paths_outside_the_root_or_not_yaml_are_refused() {
        for bad in ["../x.yaml", "types/../../x.yaml", "/etc/passwd.yaml", "~/x.yaml", "types/x.txt", "types/x", ".yaml", "types/.yaml", "data/x.yaml", "target/x.yaml", ".git/x.yaml", "types/.hidden.yaml", "", "   ", "types\\x.yaml"] {
            assert!(safe_rel_path(bad).is_err(), "{:?} should be refused", bad);
        }
        assert_eq!(safe_rel_path("types/x.yaml").unwrap(), "types/x.yaml");
        assert_eq!(safe_rel_path("./types//x.yml").unwrap(), "types/x.yml");
        assert_eq!(safe_rel_path("ontology.yaml").unwrap(), "ontology.yaml");
        assert_eq!(safe_rel_path("constraints/supply_chain/risk.yaml").unwrap(), "constraints/supply_chain/risk.yaml");
    }

    #[test]
    fn symlinks_cannot_escape_the_root() {
        let dir = scratch("symlink");
        std::fs::create_dir_all(dir.join("types")).unwrap();
        let outside = scratch("outside");
        std::fs::create_dir_all(&outside).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, dir.join("escape")).unwrap();
            assert!(resolve_in_root(&dir, "escape/x.yaml").is_err());
        }
        assert!(resolve_in_root(&dir, "types/new.yaml").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn git_status_lines_keep_modified_untracked_and_renamed_files() {
        let repo = Path::new("/repo");
        let root = Path::new("/repo/ontology");
        let out = " M ontology/types/a.yaml\n?? ontology/types/b.yaml\nR  ontology/old.yaml -> ontology/new.yaml\n M README.md\nA  ontology/c.yaml\n";
        assert_eq!(status_paths(out, repo, root), vec!["types/a.yaml", "types/b.yaml", "new.yaml", "c.yaml"]);
    }

    #[test]
    fn describe_content_names_what_a_file_declares() {
        assert_eq!(describe_content("t.yaml", "type: Widget\n"), ("type".into(), "Widget".into()));
        assert_eq!(describe_content("c.yaml", "constraints:\n  - id: A-1\n  - id: A-2\n"), ("rule".into(), "A-1, A-2".into()));
        assert_eq!(describe_content("a.yaml", "action: ship\n"), ("action".into(), "ship".into()));
        assert_eq!(describe_content("ontology.yaml", "name: acme\n"), ("manifest".into(), "acme".into()));
        assert_eq!(describe_content("x.yaml", "nonsense: 1\n"), ("file".into(), "x.yaml".into()));
    }

    #[test]
    fn an_edit_that_introduces_an_unknown_type_is_refused() {
        let dir = starter("unknown-type");
        let cfg = EditConfig { allow: true, commit: false };
        let before = validate::validate(&load(&dir));
        let original = std::fs::read_to_string(dir.join("constraints/basics.yaml")).unwrap();
        let typo = original.replace("applies_to: Account", "applies_to: Acount");
        let out = apply(&dir, cfg, &before, "constraints/basics.yaml", Change::Write(typo), "test", None, "");
        assert!(out.refused.is_none(), "{:?}", out.refused);
        assert!(!out.ok && !out.written);
        assert_eq!(out.status(), 422);
        let e004: Vec<&Diag> = out.diagnostics.iter().filter(|d| d.code == "E004").collect();
        assert_eq!(e004.len(), 2, "{:?}", out.diagnostics);
        assert!(e004[0].message.contains("Acount"));
        assert_eq!(std::fs::read_to_string(dir.join("constraints/basics.yaml")).unwrap(), original, "file must be untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_valid_edit_is_written_and_a_dry_run_is_not() {
        let dir = starter("valid-edit");
        let before = validate::validate(&load(&dir));
        let text = "type: Widget\ndescription: A thing we sell.\nproperties:\n  widget_id: { type: string, key: true }\n";
        // edit mode off: validated, not written
        let dry = apply(&dir, EditConfig::default(), &before, "types/widget.yaml", Change::Write(text.into()), "test", None, "");
        assert!(dry.ok && !dry.written);
        assert!(!dir.join("types/widget.yaml").exists());
        // edit mode on: written, and the reloaded ontology has the type
        let out = apply(&dir, EditConfig { allow: true, commit: false }, &before, "types/widget.yaml", Change::Write(text.into()), "test", None, "");
        assert!(out.ok && out.written, "{:?}", out);
        assert_eq!((out.kind.as_str(), out.name.as_str()), ("type", "Widget"));
        assert!(load(&dir).types.contains_key("Widget"));
        assert!(out.diagnostics.iter().all(|d| d.level == Level::Warning));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_file_others_depend_on_is_refused() {
        let dir = starter("delete");
        let before = validate::validate(&load(&dir));
        let cfg = EditConfig { allow: true, commit: false };
        // the action checks ACC-001/ACC-002, so the rule file cannot go
        let out = apply(&dir, cfg, &before, "constraints/basics.yaml", Change::Delete, "test", None, "");
        assert!(!out.ok && !out.written);
        assert!(out.diagnostics.iter().any(|d| d.code == "E006"));
        assert!(dir.join("constraints/basics.yaml").exists());
        // the glossary is referenced by nothing
        let out = apply(&dir, cfg, &before, "glossary.yaml", Change::Delete, "test", None, "");
        assert!(out.ok && out.written, "{:?}", out);
        assert!(!dir.join("glossary.yaml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_picks_paths_and_appends_rules_to_an_existing_list() {
        let dir = starter("create");
        let (rel, text) = plan_create(&dir, "type", "PurchaseOrder", "type: PurchaseOrder\ndescription: x\n", None).unwrap();
        assert_eq!(rel, "types/purchase_order.yaml");
        assert!(text.starts_with("type: PurchaseOrder"));
        assert!(plan_create(&dir, "type", "Customer", "type: Customer\n", Some("types/customer.yaml")).unwrap_err().contains("already exists"));
        assert!(plan_create(&dir, "type", "X", "action: X\n", None).unwrap_err().contains("`type:`"));

        let rule = "id: ACC-003\nname: Big balances need a VIP\napplies_to: Account\nrule: balance <= 1000000.0\nseverity: warn\nrationale: >\n  Two lines of\n  rationale.\n";
        let (rel, text) = plan_create(&dir, "constraint", "ACC-003", rule, Some("constraints/basics.yaml")).unwrap();
        assert_eq!(rel, "constraints/basics.yaml");
        let original = std::fs::read_to_string(dir.join("constraints/basics.yaml")).unwrap();
        assert!(text.starts_with(original.trim_end()), "existing text is kept verbatim");
        assert!(text.contains("\n  - id: ACC-003\n    name: Big balances need a VIP\n"), "{}", text);
        let parsed: Yaml = serde_yaml::from_str(&text).unwrap();
        assert_eq!(parsed["constraints"].as_sequence().unwrap().len(), 3);
        assert_eq!(parsed["constraints"][2]["rationale"].as_str().unwrap(), "Two lines of rationale.\n");

        let (rel, text) = plan_create(&dir, "rule", "ACC-004", "id: ACC-004\nname: n\napplies_to: Account\nrule: \"true\"\n", None).unwrap();
        assert_eq!(rel, "constraints/acc_004.yaml");
        assert!(text.starts_with("constraints:\n  - id: ACC-004\n"));
        assert!(plan_create(&dir, "rule", "ACC-005", "id: ACC-005\nname: n\n", None).unwrap_err().contains("applies_to"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- helpers --

    static N: AtomicUsize = AtomicUsize::new(0);

    fn scratch(tag: &str) -> PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("stella-edit-{}-{}-{}", std::process::id(), n, tag))
    }

    /// A copy of the starter ontology (what `init` writes) in a scratch folder.
    fn starter(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        let files: [(&str, &str); 5] = [
            ("ontology.yaml", include_str!("starter/ontology.yaml")),
            ("glossary.yaml", include_str!("starter/glossary.yaml")),
            ("types/customer.yaml", include_str!("starter/types/customer.yaml")),
            ("constraints/basics.yaml", include_str!("starter/constraints/basics.yaml")),
            ("actions/close_account.yaml", include_str!("starter/actions/close_account.yaml")),
        ];
        for (rel, content) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        dir
    }

    fn load(dir: &Path) -> Ontology {
        Ontology::from_raw(loader::load(dir).unwrap())
    }
}
