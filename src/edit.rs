//! Editing the ontology from the page and from MCP while the YAML files stay
//! the only source of truth. Every change goes the same way: the folder is
//! loaded with the change laid over it (nothing touches the disk), the full
//! validator runs, and the file is written only when there are no errors.
//!
//! History is git, the one on the machine. A new space starts its own
//! repository, and every accepted change in a repository that belongs to the
//! ontology becomes one commit, the summary in words as its message and the
//! caller ("agent via MCP", "person via page") as its author. A space nested
//! inside some other project's repository is left uncommitted unless the
//! server runs with `--commit-edits`. Without git everything still works;
//! history is simply off.

use crate::loader::{self, Overrides};
use crate::ontology::{snake, Ontology};
use crate::schema::{Grade, TrustBook, TrustRecord};
use crate::trust::{self, Kind as TKind, Who};
use crate::validate::{self, Diag, Level, Report};
use serde_json::{json, Value as Json};
use serde_yaml::Value as Yaml;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// When accepted changes become commits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommitPolicy {
    /// Commit when the repository belongs to the ontology (the default).
    #[default]
    Auto,
    /// `--commit-edits`: commit even inside some other project's repository.
    Always,
    /// `--no-commit-edits`: never commit.
    Never,
}

/// How `serve` and `mcp` were started, and, after `for_space`, what that
/// means for one space.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditConfig {
    /// `--allow-edit`: writes are accepted. Off by default so a deployment stays read-only.
    pub allow: bool,
    /// Commit each accepted write. The servers decide it per space with `for_space`.
    pub commit: bool,
    /// What the flags asked for.
    pub policy: CommitPolicy,
}

impl EditConfig {
    /// The config for one space: commits are on when edits are and the
    /// repository holding `dir` is the ontology's own (its top is the space
    /// folder or the folder of spaces), or always / never as the flags say.
    pub fn for_space(self, dir: &Path, spaces_root: &Path) -> EditConfig {
        let commit = self.allow
            && match self.policy {
                CommitPolicy::Never => false,
                CommitPolicy::Always => true,
                CommitPolicy::Auto => owns_history(dir, spaces_root),
            };
        EditConfig { commit, ..self }
    }
}

/// Who a change is by and what it says, for its commit.
#[derive(Debug, Clone, Copy, Default)]
pub struct Note<'a> {
    /// "person via page", "agent via MCP": the commit's author, with the email `<via>@localhost`.
    pub via: &'a str,
    /// The summary in words: the commit's first line. Empty means it is said from what changed ("Edited types/order.yaml").
    pub summary: &'a str,
    /// Why, in the words of whoever asked: the commit body.
    pub reason: &'a str,
    /// The person who accepted an agent's proposal: the committer, while the agent stays the author.
    pub accepted_by: Option<&'a str>,
    /// Leave `trust.yaml` as the files have it instead of stamping grades: Undo,
    /// where the file goes back with the facts it graded.
    pub keep_grades: bool,
}

impl<'a> Note<'a> {
    pub fn via(via: &'a str) -> Note<'a> {
        Note { via, ..Default::default() }
    }

    /// Who the save gate stamps the grade for, and the words it writes as `by`:
    /// the person who accepted a proposal, else whoever saved.
    pub fn grader(&self) -> (Who, &'a str) {
        match self.accepted_by {
            Some(by) => (Who::Person, by),
            None => (Who::of(self.via), if self.via.trim().is_empty() { PAGE } else { self.via }),
        }
    }
}

/// The page's name in history: a person using the web page.
pub const PAGE: &str = "person via page";

/// "described Order.basket" → "Described Order.basket": a history line starts with a capital.
pub fn sentence(s: &str) -> String {
    let s = s.trim();
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The history line for something an agent settled: "An agent learned: added metric success".
pub fn agent_line(via: &str, summary: &str) -> String {
    if summary.trim().is_empty() {
        String::new()
    } else if via.to_ascii_lowercase().contains("agent") {
        format!("An agent learned: {}", summary.trim())
    } else {
        sentence(summary)
    }
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
    /// Short hash of the commit this change became, when it was committed.
    pub commit: Option<String>,
    /// Git trouble that did not stop the write.
    pub git_error: Option<String>,
    /// Every file of a batch, in order; `file` is the first of them.
    pub files: Vec<String>,
    /// Why an agent's change was queued instead of written: it would change something a person stands behind.
    pub held: Option<String>,
    /// The grades this save stamped in trust.yaml: "metric blended_roas: proposed".
    pub stamped: Vec<String>,
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
        if !self.files.is_empty() {
            j["files"] = json!(self.files);
        }
        if let Some(r) = &self.refused {
            j["error"] = json!(r);
        }
        if let Some(c) = &self.commit {
            j["commit"] = json!(c);
        }
        if let Some(e) = &self.git_error {
            j["git_error"] = json!(e);
        }
        // only what was really stamped: a change that was queued or refused graded nothing
        if self.written && !self.stamped.is_empty() {
            j["grades"] = json!(self.stamped);
        }
        if let Some(h) = &self.held {
            j["held"] = json!(h);
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
        .or_else(|| one("metric", "metric"))
        .or_else(|| many("metrics", "metric", "metric"))
        .or_else(|| one("function", "function"))
        .or_else(|| many("functions", "function", "function"))
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
    check_many(root, &ov)
}

/// The ontology as it would be with every override applied, compiled and checked.
pub fn check_many(root: &Path, ov: &Overrides) -> anyhow::Result<(Ontology, Report)> {
    let raw = loader::load_with(root, ov)?;
    let ont = Ontology::from_raw(raw);
    let report = validate::validate(&ont);
    Ok((ont, report))
}

/// Several files as one change: compiled together, judged by the diagnostics
/// that are new or about one of these files, written together, committed as
/// one commit. The way a batch of `propose_patch` ops lands.
pub fn apply_many(root: &Path, cfg: EditConfig, before: &Report, files: &[(String, String)], note: &Note, label: Option<(&str, &str)>) -> Outcome {
    let set: Vec<(String, Option<String>)> = files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect();
    apply_files(root, cfg, before, &set, note, label)
}

/// `apply_many` where a file can also go: `None` deletes it. Undo uses this,
/// so putting a change back passes the same gate as making it.
pub fn apply_files(root: &Path, cfg: EditConfig, before: &Report, files: &[(String, Option<String>)], note: &Note, label: Option<(&str, &str)>) -> Outcome {
    if files.is_empty() {
        return Outcome::refuse("", "nothing to change");
    }
    let mut rels: Vec<String> = Vec::new();
    let mut fulls: Vec<PathBuf> = Vec::new();
    for (file, text) in files {
        let rel = match safe_rel_path(file) {
            Ok(r) => r,
            Err(e) => return Outcome::refuse(file, e),
        };
        if is_trust_file(&rel) && !note.keep_grades {
            return Outcome::refuse(&rel, trust::REFUSED);
        }
        let full = match resolve_in_root(root, &rel) {
            Ok(p) => p,
            Err(e) => return Outcome::refuse(&rel, e),
        };
        match text {
            Some(t) if t.trim().is_empty() => return Outcome::refuse(&rel, "content is empty; to remove a file use delete"),
            None if !full.is_file() => return Outcome::refuse(&rel, format!("{} does not exist", rel)),
            _ => {}
        }
        rels.push(rel);
        fulls.push(full);
    }
    let (kind, name) = match label {
        Some((k, n)) if !k.is_empty() && !n.is_empty() => (k.to_string(), n.to_string()),
        _ => ("batch".to_string(), format!("{} files", rels.len())),
    };
    let mut out = Outcome { file: rels[0].clone(), files: rels.clone(), kind, name, ..Default::default() };
    // one writer at a time for the read-modify-write of trust.yaml, across processes
    let _lock = if cfg.allow {
        match TrustLock::take(root) {
            Ok(l) => Some(l),
            Err(e) => return Outcome::refuse(&rels[0], e),
        }
    } else {
        None
    };
    let ov: Overrides = rels.iter().zip(files.iter()).map(|(rel, (_, text))| (rel.clone(), text.clone())).collect();
    let (mut after_ont, after) = match check_many(root, &ov) {
        Ok(x) => x,
        Err(e) => {
            out.refused = Some(format!("{:#}", e));
            return out;
        }
    };
    let stamp = match stamp_for(root, note, &after_ont) {
        Ok(s) => s,
        Err(e) => return Outcome::refuse(trust::FILE, e),
    };
    let after = match &stamp {
        Some((_, book, said)) => {
            after_ont.set_trust(Some(book.clone()));
            out.stamped = said.clone();
            validate::validate(&after_ont)
        }
        None => after,
    };
    let existed = |d: &Diag| before.diags.iter().any(|b| b.code == d.code && b.message == d.message && b.file == d.file);
    out.diagnostics = after.diags.iter().filter(|d| rels.contains(&d.file) || !existed(d)).cloned().collect();
    out.diagnostics.sort_by_key(|d| if d.level == Level::Error { 0 } else { 1 });
    out.ok = !out.diagnostics.iter().any(|d| d.level == Level::Error);
    if !out.ok || !cfg.allow {
        return out;
    }
    // trust.yaml first: a crash between the two leaves a harmless orphan grade (W114), never an unflagged fact
    let trust_full = match write_stamp(root, &stamp) {
        Ok(p) => p,
        Err(e) => {
            out.ok = false;
            out.refused = Some(e);
            return out;
        }
    };
    for (full, (_, text)) in fulls.iter().zip(files.iter()) {
        let done = match text {
            Some(t) => write_atomic(full, t),
            None => std::fs::remove_file(full).map_err(|e| e.to_string()),
        };
        if let Err(e) = done {
            out.ok = false;
            out.refused = Some(format!("could not write {}: {}", full.display(), e));
            return out;
        }
    }
    out.written = true;
    if cfg.commit {
        let fallback = match (label, rels.len()) {
            (Some((k, n)), _) if !k.is_empty() && !n.is_empty() => format!("Changed the {} {}", k, n),
            (_, 1) if files[0].1.is_none() => format!("Deleted {}", rels[0]),
            (_, 1) => format!("Edited {}", rels[0]),
            (_, n) => format!("Changed {} files: {}", n, rels.join(", ")),
        };
        let mut paths = fulls.clone();
        paths.extend(trust_full);
        match commit_many(root, &paths, &message(note, &fallback), note) {
            Ok(h) => out.commit = h,
            Err(e) => out.git_error = Some(e),
        }
    }
    out
}

/// The commit message: the summary (or what changed, said plainly) and the reason as its body.
fn message(note: &Note, fallback: &str) -> String {
    let mut m = if note.summary.trim().is_empty() { fallback.to_string() } else { sentence(note.summary) };
    let reason = note.reason.trim();
    if !reason.is_empty() && reason != note.summary.trim() {
        m.push_str("\n\n");
        m.push_str(reason);
    }
    m
}

/// Which diagnostics to show for a change: every error, everything on the
/// changed file, and anything that was not there before the change.
/// The diagnostics a change is judged by: anything new, plus anything about
/// the file itself. A problem that was already there and lives elsewhere (an
/// empty space's "no types yet", a broken file you have not touched) is not
/// this change's fault and must not stop it; it stays visible in the footer.
pub fn relevant(before: &Report, after: &Report, rel: &str) -> Vec<Diag> {
    let existed = |d: &Diag| before.diags.iter().any(|b| b.code == d.code && b.message == d.message && b.file == d.file);
    let mut out: Vec<Diag> = after
        .diags
        .iter()
        .filter(|d| d.file == rel || !existed(d))
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
/// it and, when `cfg.commit` is on, commit it. `before` is the report of the
/// ontology as it is now, used to tell new diagnostics from old ones. `note`
/// says who asked and what the change is, for the commit; `label` overrides
/// the kind/name derived from the content.
pub fn apply(root: &Path, cfg: EditConfig, before: &Report, file: &str, change: Change, note: &Note, label: Option<(&str, &str)>) -> Outcome {
    let rel = match safe_rel_path(file) {
        Ok(r) => r,
        Err(e) => return Outcome::refuse(file, e),
    };
    if is_trust_file(&rel) && !note.keep_grades {
        return Outcome::refuse(&rel, trust::REFUSED);
    }
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

    let _lock = if cfg.allow {
        match TrustLock::take(root) {
            Ok(l) => Some(l),
            Err(e) => return Outcome::refuse(&rel, e),
        }
    } else {
        None
    };
    let (mut after_ont, after) = match check(root, &rel, &change) {
        Ok(x) => x,
        Err(e) => {
            out.refused = Some(format!("{:#}", e));
            return out;
        }
    };
    let stamp = match stamp_for(root, note, &after_ont) {
        Ok(s) => s,
        Err(e) => return Outcome::refuse(trust::FILE, e),
    };
    let after = match &stamp {
        Some((_, book, said)) => {
            after_ont.set_trust(Some(book.clone()));
            out.stamped = said.clone();
            validate::validate(&after_ont)
        }
        None => after,
    };
    out.diagnostics = relevant(before, &after, &rel);
    // clean means: this change adds no error of its own (see `relevant`)
    out.ok = !out.diagnostics.iter().any(|d| d.level == Level::Error);
    if !out.ok || !cfg.allow {
        return out;
    }

    let trust_full = match write_stamp(root, &stamp) {
        Ok(p) => p,
        Err(e) => {
            out.ok = false;
            out.refused = Some(e);
            return out;
        }
    };
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
        let fallback = match &change {
            Change::Delete => format!("Deleted {}", rel),
            // a create that lands in an existing list file (a rule appended to constraints/x.yaml)
            Change::Write(_) if existed && label.is_some() => format!("Added the {} {} to {}", out.kind, out.name, rel),
            Change::Write(_) if existed => format!("Edited {}", rel),
            Change::Write(_) if label.is_some() => format!("Added the {} {}", out.kind, out.name),
            Change::Write(_) => format!("Created {}", rel),
        };
        let mut paths = vec![full.clone()];
        paths.extend(trust_full);
        match commit_many(root, &paths, &message(note, &fallback), note) {
            Ok(h) => out.commit = h,
            Err(e) => out.git_error = Some(e),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Grades
// ---------------------------------------------------------------------------

/// `trust.yaml` at the root of the space, however it is spelled: `Trust.yaml`
/// is the same file on a disk that ignores case, and `trust.yml` is refused so
/// nobody can put a second grades file beside the real one.
pub fn is_trust_file(rel: &str) -> bool {
    let r = rel.to_ascii_lowercase();
    r == trust::FILE || r == "trust.yml"
}

/// A lock file under `.lorefile/`: `trust.lock` around the read-modify-write of
/// `trust.yaml`, `decide.lock` around a person's decision on a proposal (Accept,
/// Not now, Look again, Take it back, and the replace rule). `mcp` on stdio and
/// `serve` are separate processes that may work on the same space at once. A
/// lock older than 30 seconds is stale.
pub struct SpaceLock(PathBuf);

/// The lock around a decision on a proposal; it is held outside the save gate's own lock.
pub const DECIDE_LOCK: &str = "decide.lock";

struct TrustLock;

impl TrustLock {
    fn take(root: &Path) -> Result<SpaceLock, String> {
        SpaceLock::take(root, "trust.lock")
    }
}

impl SpaceLock {
    pub fn take(root: &Path, name: &str) -> Result<SpaceLock, String> {
        let dir = crate::spaces::lorefile_dir(root);
        let path = dir.join(name);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = std::io::Write::write_all(&mut f, std::process::id().to_string().as_bytes());
                    return Ok(SpaceLock(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|age| age.as_secs() > 30).unwrap_or(false);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if std::time::Instant::now() > deadline {
                        return Err("another save into this space is still running; try again in a moment".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                // a folder we cannot write into: the write itself will say so
                Err(_) if !dir.is_dir() => return Ok(SpaceLock(PathBuf::new())),
                Err(e) => return Err(format!("could not take the save lock {}: {}", path.display(), e)),
            }
        }
    }
}

impl Drop for SpaceLock {
    fn drop(&mut self) {
        if !self.0.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

/// The new `trust.yaml` a save makes, if any: the grades of everything the
/// change touched, stamped for whoever saved (see `trust::restamp`), laid over
/// the file as it is on disk now, so only this save's entries change.
type Stamp = Option<(String, TrustBook, Vec<String>)>;

fn stamp_for(root: &Path, note: &Note, after: &Ontology) -> Result<Stamp, String> {
    if note.keep_grades {
        return Ok(None);
    }
    let before = Ontology::from_raw(loader::load(root).map_err(|e| format!("{:#}", e))?);
    let (who, by) = note.grader();
    let entries = trust::restamp(&before, after, who, by);
    if entries.is_empty() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(root.join(trust::FILE)).unwrap_or_default();
    let mut book = trust::parse(&text).map_err(|e| format!("{}; fix or remove it by hand, then save again", e))?;
    if !trust::apply(&mut book, &entries) {
        return Ok(None);
    }
    let said: Vec<String> = entries
        .iter()
        .map(|e| match e {
            trust::Entry::Set(k, n, r) => format!("{} {}: {}", k.word(), n, r.grade.word()),
            trust::Entry::Remove(k, n) => format!("{} {}: grade removed", k.word(), n),
        })
        .collect();
    Ok(Some((trust::render(&book), book, said)))
}

/// Write the stamped `trust.yaml`; its path, for the commit.
fn write_stamp(root: &Path, stamp: &Stamp) -> Result<Option<PathBuf>, String> {
    let Some((text, _, _)) = stamp else { return Ok(None) };
    let full = resolve_in_root(root, trust::FILE)?;
    write_atomic(&full, text).map_err(|e| format!("could not write {}: {}", trust::FILE, e))?;
    Ok(Some(full))
}

/// Decision 5: the accepted or checked things these files would change or
/// remove, each as a sentence. An agent's write that touches one is queued
/// for a person instead of written, even with edits on.
pub fn graded_overwrites(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let Ok(raw) = loader::load(root) else { return vec![] };
    let before = Ontology::from_raw(raw);
    if !before.graded.has_file {
        return vec![];
    }
    let ov: Overrides = files.iter().filter_map(|(f, t)| safe_rel_path(f).ok().map(|r| (r, Some(t.clone())))).collect();
    match check_many(root, &ov) {
        Ok((after, _)) => trust::would_overwrite_graded(&before, &after),
        Err(_) => vec![],
    }
}

/// What a person does to a grade from the page or the CLI.
#[derive(Debug, Clone)]
pub enum GradeOp {
    /// "Looks right": a proposed or changed thing becomes accepted.
    Accept { kind: TKind, name: String },
    /// "Mark as checked": a person ran it against `against` and compared it as `note` says.
    Check { kind: TKind, name: String, against: String, note: String },
    /// "Vouch for all": every ungraded (or changed) thing becomes accepted.
    VouchAll,
}

/// The name of a thing as the space spells it ("order" finds the binding of `Order`).
pub fn find_graded(ont: &Ontology, kind: Option<TKind>, name: &str) -> Result<(TKind, String), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("say which definition: a metric, an entity (for its data binding), a glossary word or a guardrail id".into());
    }
    let kinds: Vec<TKind> = match kind { Some(k) => vec![k], None => TKind::ALL.to_vec() };
    let all: Vec<(TKind, String)> = kinds.iter().flat_map(|k| trust::things(ont, *k).into_keys().map(move |n| (*k, n))).collect();
    let mut found: Vec<(TKind, String)> = all.iter().filter(|(_, n)| n == name).cloned().collect();
    if found.is_empty() {
        found = all.iter().filter(|(_, n)| n.eq_ignore_ascii_case(name)).cloned().collect();
    }
    match found.len() {
        0 => {
            let why = match kind {
                Some(k) if k != TKind::Binding => format!("there is no {} named `{}`", k.word(), name),
                _ if ont.type_(name).is_some() => format!("`{}` is an entity with no data binding; only its binding carries a grade", name),
                Some(k) => format!("there is no {} named `{}`", k.word(), name),
                None => format!("nothing gradable is named `{}`: grades go on metrics, data bindings (by entity name), glossary words and guardrails", name),
            };
            Err(why)
        }
        1 => Ok(found.remove(0)),
        _ => Err(format!("`{}` names {}; say which with kind", name, found.iter().map(|(k, n)| format!("the {} {}", k.word(), n)).collect::<Vec<_>>().join(" and "))),
    }
}

/// Set a grade by a person's click (or `lorefile trust`): Looks right, Mark
/// as checked, Vouch for all. `trust.yaml` only, through the same gate and
/// lock as any save, committed as one change in words.
pub fn grade(root: &Path, cfg: EditConfig, before: &Report, op: &GradeOp, by: &str) -> Outcome {
    let label = |k: TKind, n: &str| (k.word().to_string(), n.to_string());
    let mut out = Outcome { file: trust::FILE.into(), files: vec![trust::FILE.into()], ..Default::default() };
    if !cfg.allow {
        out.refused = Some("edit mode is off; start the server with --allow-edit".into());
        return out;
    }
    let _lock = match TrustLock::take(root) {
        Ok(l) => l,
        Err(e) => return Outcome::refuse(trust::FILE, e),
    };
    let ont = match loader::load(root) {
        Ok(r) => Ontology::from_raw(r),
        Err(e) => return Outcome::refuse(trust::FILE, format!("{:#}", e)),
    };
    let text = std::fs::read_to_string(root.join(trust::FILE)).unwrap_or_default();
    let mut book = match trust::parse(&text) {
        Ok(b) => b,
        Err(e) => return Outcome::refuse(trust::FILE, format!("{}; fix or remove it by hand first", e)),
    };
    let at = trust::today();
    let by = if by.trim().is_empty() { PAGE } else { by.trim() };
    let record = |k: TKind, n: &str, grade: Grade, against: Option<String>, note: Option<String>| -> Option<TrustRecord> {
        let v = trust::things(&ont, k).get(n).cloned()?;
        Some(TrustRecord { grade, by: by.to_string(), at: at.clone(), of: trust::fingerprint(&v), against, note })
    };
    let summary = match op {
        GradeOp::Accept { kind, name } => {
            let st = ont.grade_of(*kind, name);
            match st.grade {
                Some(Grade::Checked) => return Outcome::refuse(trust::FILE, format!("{} {} is already checked", kind.word(), name)),
                Some(Grade::Accepted) => return Outcome::refuse(trust::FILE, format!("{} {} is already accepted", kind.word(), name)),
                _ => {}
            }
            let Some(r) = record(*kind, name, Grade::Accepted, None, None) else { return Outcome::refuse(trust::FILE, format!("there is no {} {}", kind.word(), name)) };
            trust::entries_mut(&mut book, *kind).insert(name.clone(), r);
            (out.kind, out.name) = label(*kind, name);
            format!("Accepted {} {}", kind.word(), name)
        }
        GradeOp::Check { kind, name, against, note } => {
            if !kind.can_be_checked() {
                return Outcome::refuse(trust::FILE, "a word has no rows to check against; accept it instead");
            }
            let note = trust::clean_note(note);
            let (against, note) = (against.trim(), note.as_str());
            if against.is_empty() || note.is_empty() {
                return Outcome::refuse(trust::FILE, "say which source's rows you used and what you compared them with; both are needed to mark something as checked");
            }
            if !ont.sources.contains_key(against) {
                return Outcome::refuse(trust::FILE, format!("`{}` is not a source under sources/", against));
            }
            let st = ont.grade_of(*kind, name);
            if st.grade.is_none() || st.grade == Some(Grade::Proposed) {
                return Outcome::refuse(trust::FILE, format!("{} {} is {}; a person accepts it first (Looks right), then marks it as checked", kind.word(), name, if st.changed { "changed since it was graded" } else { st.word() }));
            }
            let Some(r) = record(*kind, name, Grade::Checked, Some(against.to_string()), Some(note.to_string())) else { return Outcome::refuse(trust::FILE, format!("there is no {} {}", kind.word(), name)) };
            trust::entries_mut(&mut book, *kind).insert(name.clone(), r);
            (out.kind, out.name) = label(*kind, name);
            format!("Checked {} {} against {}", kind.word(), name, against)
        }
        GradeOp::VouchAll => {
            let mut n = 0;
            // only what nobody has ever graded: a proposed thing, and one that
            // changed since a person graded it, each need their own look
            for (k, name) in &ont.graded.all {
                if ont.grade_of(*k, name).record.is_none() {
                    if let Some(r) = record(*k, name, Grade::Accepted, None, None) {
                        trust::entries_mut(&mut book, *k).insert(name.clone(), r);
                        n += 1;
                    }
                }
            }
            if n == 0 {
                return Outcome::refuse(trust::FILE, "nothing is left to vouch for: every definition has a grade, or was proposed by an agent or changed since it was graded, and those need a look each (Looks right)");
            }
            out.kind = "definitions".into();
            out.name = n.to_string();
            format!("Vouched for {} definition{}", n, if n == 1 { "" } else { "s" })
        }
    };
    let new_text = trust::render(&book);
    let mut ov = Overrides::new();
    ov.insert(trust::FILE.to_string(), Some(new_text.clone()));
    let (_, after) = match check_many(root, &ov) {
        Ok(x) => x,
        Err(e) => return Outcome::refuse(trust::FILE, format!("{:#}", e)),
    };
    out.diagnostics = relevant(before, &after, "");
    out.ok = !out.diagnostics.iter().any(|d| d.level == Level::Error);
    if !out.ok {
        return out;
    }
    let full = match resolve_in_root(root, trust::FILE) {
        Ok(p) => p,
        Err(e) => return Outcome::refuse(trust::FILE, e),
    };
    if let Err(e) = write_atomic(&full, &new_text) {
        out.ok = false;
        out.refused = Some(format!("could not write {}: {}", trust::FILE, e));
        return out;
    }
    out.written = true;
    if cfg.commit {
        let note = Note { via: by, summary: &summary, ..Default::default() };
        match commit_many(root, std::slice::from_ref(&full), &message(&note, &summary), &note) {
            Ok(h) => out.commit = h,
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
        "metric" => ("logic", &["metric", "metrics"]),
        "function" => ("logic", &["function", "functions"]),
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
        other => return Err(format!("unknown kind `{}`; use type, constraint, action, relationship, source, metric or function", other)),
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
        "metric": include_str!("starter/templates/metric.yaml"),
        "function": include_str!("starter/templates/function.yaml"),
    })
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

/// The git program. Tests swap it for one that is not there, per thread.
#[cfg(not(test))]
fn git_program() -> &'static str {
    "git"
}
#[cfg(test)]
thread_local! {
    static FAKE_GIT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn git_program() -> String {
    FAKE_GIT.with(|g| g.borrow().clone()).unwrap_or_else(|| "git".into())
}

fn git_cmd(repo: &Path) -> Command {
    let mut c = Command::new(git_program());
    c.arg("-C").arg(repo).env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null());
    c
}

/// Run git in `repo`; stdout untrimmed (porcelain lines start with a space).
fn git_raw(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_cmd(repo).args(args).output().map_err(|e| format!("git: {}", e))?;
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

/// True when git ran and said yes (exit code 0): `diff --quiet`, `check-ignore -q`.
fn git_ok(repo: &Path, args: &[&str]) -> bool {
    git_cmd(repo).args(args).output().map(|o| o.status.success()).unwrap_or(false)
}

fn probe_git() -> bool {
    Command::new(git_program()).arg("--version").stdin(Stdio::null()).output().map(|o| o.status.success()).unwrap_or(false)
}

/// Is there a git on this machine? Asked once per process.
pub fn git_available() -> bool {
    #[cfg(not(test))]
    {
        static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *OK.get_or_init(probe_git)
    }
    #[cfg(test)]
    {
        probe_git()
    }
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
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
    Some(canon(&PathBuf::from(top)))
}

/// Where a space's history lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Home {
    /// No git on this machine.
    NoGit,
    /// No repository yet (or only one that ignores this folder, such as a
    /// project whose `.gitignore` lists `spaces/`).
    Nowhere,
    /// The ontology's own repository: its top is the space folder or the folder of spaces.
    Own(PathBuf),
    /// Some other project's repository, with the space somewhere inside it.
    Other(PathBuf),
}

impl Home {
    /// The `history` word the page and the API show.
    pub fn word(&self) -> &'static str {
        match self {
            Home::NoGit => "off (git not found)",
            Home::Nowhere => "off (no repository yet)",
            Home::Own(_) => "on",
            Home::Other(_) => "off (inside another repository)",
        }
    }
}

pub fn home(dir: &Path, spaces_root: &Path) -> Home {
    if !git_available() {
        return Home::NoGit;
    }
    match git_root(dir) {
        None => Home::Nowhere,
        Some(r) if r == canon(dir) || r == canon(spaces_root) => Home::Own(r),
        Some(r) if ignored_by(&r, dir) => Home::Nowhere,
        Some(r) => Home::Other(r),
    }
}

/// True when the repository at `repo` ignores `dir`, so it will never hold its files.
fn ignored_by(repo: &Path, dir: &Path) -> bool {
    let d = canon(dir);
    git_ok(repo, &["check-ignore", "-q", &d.to_string_lossy()])
}

/// The rule for committing by default: the repository belongs to the ontology.
pub fn owns_history(dir: &Path, spaces_root: &Path) -> bool {
    match git_root(dir) {
        Some(r) => r == canon(dir) || r == canon(spaces_root),
        None => false,
    }
}

/// Lorefile's own name in a repository it starts, so a commit never stops at
/// "please tell me who you are" on a server with no git identity.
const LOREFILE_NAME: &str = "Lorefile";
const LOREFILE_EMAIL: &str = "lorefile@localhost";

/// `"agent via MCP"` → `("agent via MCP", "agent-via-mcp@localhost")`.
pub fn identity(via: &str) -> (String, String) {
    let name = if via.trim().is_empty() { PAGE } else { via.trim() };
    let slug = crate::spaces::slug(name);
    (name.to_string(), format!("{}@localhost", if slug.is_empty() { "someone" } else { slug.as_str() }))
}

fn author_arg(via: &str) -> String {
    let (n, e) = identity(via);
    format!("{} <{}>", n, e)
}

/// Commit these files: the author is whoever made the change, the committer
/// whoever accepted it (or the repository's own identity). `Ok(None)` when
/// the files were already committed as they are.
fn commit_many(root: &Path, files: &[PathBuf], message: &str, note: &Note) -> Result<Option<String>, String> {
    let repo = git_root(root).ok_or_else(|| "the ontology folder is not inside a git repository, so nothing was committed".to_string())?;
    let paths: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();
    let mut add: Vec<&str> = vec!["add", "-A", "--"];
    add.extend(paths.iter().map(|p| p.as_str()));
    git(&repo, &add)?;
    let mut same: Vec<&str> = vec!["diff", "--cached", "--quiet", "--"];
    same.extend(paths.iter().map(|p| p.as_str()));
    if git_ok(&repo, &same) {
        return Ok(None);
    }
    let mut com: Vec<String> = Vec::new();
    let committer = match note.accepted_by {
        Some(by) => Some(identity(by)),
        None if git(&repo, &["config", "user.email"]).map(|s| s.is_empty()).unwrap_or(true) => Some((LOREFILE_NAME.to_string(), LOREFILE_EMAIL.to_string())),
        None => None,
    };
    if let Some((n, e)) = committer {
        com.extend(["-c".to_string(), format!("user.name={}", n), "-c".to_string(), format!("user.email={}", e)]);
    }
    com.extend(["commit".to_string(), "-q".to_string(), format!("--author={}", author_arg(note.via)), "-m".to_string(), message.to_string(), "--".to_string()]);
    com.extend(paths.iter().cloned());
    let com_ref: Vec<&str> = com.iter().map(|s| s.as_str()).collect();
    git(&repo, &com_ref)?;
    git(&repo, &["rev-parse", "--short", "HEAD"]).map(Some)
}

/// Start history in a new space: `git init -b main`, Lorefile's identity kept
/// in the repository, and a first commit "Start the space" by `via`. Skipped,
/// never failed, when git is missing or the folder already lies inside a
/// repository; when that repository is the folder of spaces, the new space
/// becomes its next commit instead. Returns the `history` word.
pub fn init_space(dir: &Path, spaces_root: &Path, via: &str) -> String {
    match home(dir, spaces_root) {
        Home::NoGit => return Home::NoGit.word().into(),
        Home::Other(_) => return Home::Other(PathBuf::new()).word().into(),
        Home::Own(repo) => {
            if repo != canon(dir) {
                let id = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let _ = commit_all_as(dir, &format!("Start the space {}", id), via);
            }
            return "on".into();
        }
        Home::Nowhere => {}
    }
    match start_repo(dir, via) {
        Ok(_) => "on".into(),
        Err(e) => format!("off (git could not start a repository: {})", e.lines().next().unwrap_or("")),
    }
}

fn start_repo(dir: &Path, via: &str) -> Result<String, String> {
    // the state folder hides itself before the first `git add`
    crate::spaces::lorefile_dir(dir);
    if git(dir, &["init", "-q", "-b", "main"]).is_err() {
        // git older than 2.28 has no -b
        git(dir, &["init", "-q"])?;
        let _ = git(dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    }
    git(dir, &["config", "user.name", LOREFILE_NAME])?;
    git(dir, &["config", "user.email", LOREFILE_EMAIL])?;
    git(dir, &["add", "-A", "."])?;
    git(dir, &["commit", "-q", "--allow-empty", &format!("--author={}", author_arg(via)), "-m", "Start the space"])?;
    git(dir, &["rev-parse", "--short", "HEAD"])
}

/// Commit everything under `root` as `via`.
fn commit_all_as(root: &Path, message: &str, via: &str) -> Result<Option<String>, String> {
    let repo = git_root(root).ok_or_else(|| "not inside a git repository".to_string())?;
    let scope = canon(root).to_string_lossy().to_string();
    git(&repo, &["add", "-A", "--", &scope])?;
    if git_ok(&repo, &["diff", "--cached", "--quiet", "--", &scope]) {
        return Ok(None);
    }
    let mut args: Vec<String> = Vec::new();
    if git(&repo, &["config", "user.email"]).map(|s| s.is_empty()).unwrap_or(true) {
        args.extend(["-c".into(), format!("user.name={}", LOREFILE_NAME), "-c".into(), format!("user.email={}", LOREFILE_EMAIL)]);
    }
    args.extend(["commit".into(), "-q".into(), format!("--author={}", author_arg(via)), "-m".into(), message.to_string(), "--".into(), scope]);
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    git(&repo, &a)?;
    git(&repo, &["rev-parse", "--short", "HEAD"]).map(Some)
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

/// Where a remote lives, for the Share card: (`"github.com"`, `"github.com/you/brand-two"`).
/// A folder on this machine or a drive is `("this computer", "<path>")`.
pub fn remote_label(url: &str) -> (String, String) {
    let u = url.trim();
    let local = || ("this computer".to_string(), u.trim_start_matches("file://").to_string());
    if let Some((scheme, rest)) = u.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return local();
        }
        let (hostpart, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = hostpart.rsplit('@').next().unwrap_or(hostpart);
        let host = host.split(':').next().unwrap_or(host).to_string();
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let label = if path.is_empty() { host.clone() } else { format!("{}/{}", host, path) };
        return (host, label);
    }
    let looks_local = u.starts_with('/') || u.starts_with('.') || u.starts_with('~') || u.chars().nth(1) == Some(':') && u.chars().nth(2).map(|c| c == '\\' || c == '/').unwrap_or(false);
    if looks_local {
        return local();
    }
    if let Some((left, path)) = u.split_once(':') {
        let host = left.rsplit('@').next().unwrap_or(left).to_string();
        let path = path.trim_start_matches('/').trim_end_matches('/').trim_end_matches(".git");
        return (host.clone(), format!("{}/{}", host, path));
    }
    local()
}

/// For the Settings page: whether history is on, the repository, its branch
/// and remote, what is uncommitted under `root`, how many changes the history
/// holds, and how far this copy and the shared one have drifted apart.
pub fn git_info(root: &Path, spaces_root: &Path) -> Json {
    let h = home(root, spaces_root);
    let mut j = json!({ "history": h.word(), "own": matches!(h, Home::Own(_)), "nested": matches!(h, Home::Other(_)) });
    let repo = match &h {
        Home::Own(r) | Home::Other(r) => r.clone(),
        _ => {
            j["repo"] = json!(false);
            j["commits"] = json!(0);
            return j;
        }
    };
    let scope = canon(root).to_string_lossy().to_string();
    let st = git_status(root).unwrap_or_else(|| json!({ "branch": "", "uncommitted": [] }));
    j["repo"] = json!(true);
    j["branch"] = st["branch"].clone();
    j["uncommitted"] = st["uncommitted"].clone();
    j["root"] = json!(repo.display().to_string());
    let count = |range: &str, scoped: bool| -> u64 {
        let mut a = vec!["rev-list", "--count", range];
        if scoped {
            a.extend(["--", scope.as_str()]);
        }
        git(&repo, &a).ok().and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    j["commits"] = json!(count("HEAD", true));
    let remote = git(&repo, &["remote", "get-url", "origin"]).ok().filter(|s| !s.is_empty());
    j["remote"] = json!(remote);
    if let Some(url) = &remote {
        let (host, label) = remote_label(url);
        j["remote_host"] = json!(host);
        j["remote_label"] = json!(label);
        let upstream = git_ok(&repo, &["rev-parse", "--verify", "--quiet", "@{u}"]);
        j["pushed"] = json!(upstream);
        if upstream {
            j["ahead"] = json!(count("@{u}..HEAD", false));
            j["behind"] = json!(count("HEAD..@{u}", false));
        } else {
            // never pushed: everything is waiting
            j["ahead"] = json!(count("HEAD", false));
            j["behind"] = json!(0);
        }
    }
    j["last_commit"] = json!(git(&repo, &["log", "-1", "--format=%h %s (%cr)", "--", &scope]).unwrap_or_default());
    j
}

/// One line for an agent: "History: on, 14 changes, last: Described Order.basket (2 hours ago)."
pub fn history_line(root: &Path, spaces_root: &Path) -> String {
    match home(root, spaces_root) {
        Home::Own(repo) => {
            let scope = canon(root).to_string_lossy().to_string();
            let n: u64 = git(&repo, &["rev-list", "--count", "HEAD", "--", &scope]).ok().and_then(|n| n.parse().ok()).unwrap_or(0);
            let last = git(&repo, &["log", "-1", "--format=%s (%ar)", "--", &scope]).unwrap_or_default();
            format!("History: on, {} change{}{}. Every accepted change is a commit; a person can see and undo them on the History page.", n, if n == 1 { "" } else { "s" }, if last.is_empty() { String::new() } else { format!(", last: {}", last) })
        }
        Home::Other(repo) => format!("History: kept by the repository at {}, outside the ontology; accepted changes stay uncommitted unless the server runs with --commit-edits.", repo.display()),
        Home::Nowhere => "History: off, no repository yet (a person can start it in the web page, Settings).".into(),
        Home::NoGit => "History: off (git not found on this machine).".into(),
    }
}

/// A commit id as a person or the page sends it: 4 to 64 hex characters, nothing else.
fn valid_hash(h: &str) -> bool {
    (4..=64).contains(&h.len()) && h.chars().all(|c| c.is_ascii_hexdigit())
}

/// The full id of a commit, or a sentence saying it is not in this history.
fn resolve_commit(repo: &Path, hash: &str) -> Result<String, String> {
    let hash = hash.trim();
    if !valid_hash(hash) {
        return Err("give the change's id as History shows it, e.g. 3f2a1bc".into());
    }
    git(repo, &["rev-parse", "--verify", "--quiet", &format!("{}^{{commit}}", hash)]).map_err(|_| format!("there is no change {} in this history", hash))
}

fn kind_of(author: &str) -> &'static str {
    if author.to_ascii_lowercase().contains("agent") {
        "agent"
    } else {
        "person"
    }
}

/// The space's changes, newest first: id, author (agent or person), when,
/// the message and the files, relative to the space. `file` narrows it to one file.
pub fn git_log(root: &Path, limit: usize, file: Option<&str>) -> Result<Json, String> {
    let repo = git_root(root).ok_or_else(|| "this space has no history yet".to_string())?;
    let root_c = canon(root);
    let scope = match file.map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => root_c.join(safe_rel_path(f)?),
        None => root_c.clone(),
    };
    let scope_s = scope.to_string_lossy().to_string();
    let n = format!("-n{}", limit.clamp(1, 500));
    let raw = match git_raw(&repo, &["-c", "core.quotePath=false", "log", &n, "--format=%x1e%H%x1f%h%x1f%an%x1f%ae%x1f%aI%x1f%ar%x1f%P%x1f%s", "--name-only", "--", &scope_s]) {
        Ok(r) => r,
        Err(e) if e.contains("does not have any commits") => String::new(),
        Err(e) => return Err(e),
    };
    let mut commits = Vec::new();
    for rec in raw.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = rec.lines();
        let head = lines.next().unwrap_or("");
        let f: Vec<&str> = head.split('\u{1f}').collect();
        if f.len() < 8 {
            continue;
        }
        let files: Vec<String> = lines
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .filter_map(|l| repo.join(l).strip_prefix(&root_c).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
            .collect();
        commits.push(json!({
            "hash": f[0], "short": f[1], "author": f[2], "email": f[3], "kind": kind_of(f[2]),
            "at": f[4], "ago": f[5], "first": f[6].trim().is_empty(), "message": f[7], "files": files,
        }));
    }
    let total: u64 = git(&repo, &["rev-list", "--count", "HEAD", "--", &scope_s]).ok().and_then(|n| n.parse().ok()).unwrap_or(commits.len() as u64);
    Ok(json!({ "commits": commits, "total": total }))
}

/// What one change did: its message and author, and the unified diff,
/// limited to the space folder with paths relative to it.
pub fn git_show(root: &Path, hash: &str) -> Result<Json, String> {
    let repo = git_root(root).ok_or_else(|| "this space has no history yet".to_string())?;
    let full = resolve_commit(&repo, hash)?;
    let meta = git(&repo, &["log", "-1", "--format=%h%x1f%an%x1f%aI%x1f%ar%x1f%s", &full]).unwrap_or_default();
    let m: Vec<&str> = meta.split('\u{1f}').collect();
    let at = |i: usize| m.get(i).copied().unwrap_or("");
    let mut diff = git_raw(&canon(root), &["-c", "core.quotePath=false", "show", "--no-color", "--no-ext-diff", "--format=", "--relative", &full, "--", "."])?;
    const MAX: usize = 400 * 1024;
    if diff.len() > MAX {
        let mut cut = MAX;
        while !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        diff.truncate(cut);
        diff.push_str("\n… (the rest is cut off here; run git show in a terminal for all of it)\n");
    }
    Ok(json!({ "hash": full, "short": at(0), "author": at(1), "kind": kind_of(at(1)), "at": at(2), "ago": at(3), "message": at(4), "diff": diff }))
}

/// Three-way merge of one file with git merge-file: `ours` with the change
/// from `base` to `theirs` laid on top. `None` when the two clash.
fn merge3(ours: &str, base: &str, theirs: &str) -> Option<String> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("lorefile-undo-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).ok()?;
    let (a, b, c) = (dir.join("ours"), dir.join("base"), dir.join("theirs"));
    let written = std::fs::write(&a, ours).and(std::fs::write(&b, base)).and(std::fs::write(&c, theirs));
    let out = written.ok().and_then(|_| Command::new(git_program()).arg("merge-file").arg("-p").arg("-q").arg(&a).arg(&b).arg(&c).stdin(Stdio::null()).output().ok());
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    if out.status.code() == Some(0) {
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    }
}

/// When the lines clash, merge the YAML itself: a patch rewrites a whole file
/// the first time it touches it, so a later one-line change sits inside lines
/// the earlier change also "changed". Key by key, `ours` keeps what it changed
/// and takes what `theirs` changed; a key both changed differently, or lists
/// that grew or shrank on both sides, is a clash (`None`). The result is
/// written in the compiler's own YAML style, comments dropped.
fn merge_yaml(ours: &str, base: &str, theirs: &str) -> Option<String> {
    let o: Yaml = serde_yaml::from_str(ours).ok()?;
    let b: Yaml = serde_yaml::from_str(base).ok()?;
    let t: Yaml = serde_yaml::from_str(theirs).ok()?;
    serde_yaml::to_string(&merge_value(&o, &b, &t)?).ok()
}

fn merge_value(o: &Yaml, b: &Yaml, t: &Yaml) -> Option<Yaml> {
    if o == b {
        return Some(t.clone());
    }
    if t == b || o == t {
        return Some(o.clone());
    }
    match (o, b, t) {
        (Yaml::Mapping(om), Yaml::Mapping(bm), Yaml::Mapping(tm)) => {
            let mut out = serde_yaml::Mapping::new();
            for (k, ov) in om {
                match (bm.get(k), tm.get(k)) {
                    (Some(bv), Some(tv)) => {
                        out.insert(k.clone(), merge_value(ov, bv, tv)?);
                    }
                    // theirs removed it: fine if ours left it alone
                    (Some(bv), None) if ov == bv => {}
                    (Some(_), None) => return None,
                    // both added it
                    (None, Some(tv)) if ov == tv => {
                        out.insert(k.clone(), ov.clone());
                    }
                    (None, Some(_)) => return None,
                    // ours added it
                    (None, None) => {
                        out.insert(k.clone(), ov.clone());
                    }
                }
            }
            for (k, tv) in tm {
                if om.contains_key(k) {
                    continue;
                }
                match bm.get(k) {
                    // theirs added it
                    None => {
                        out.insert(k.clone(), tv.clone());
                    }
                    // ours removed it and theirs left it alone: stays removed
                    Some(bv) if bv == tv => {}
                    Some(_) => return None,
                }
            }
            Some(Yaml::Mapping(out))
        }
        (Yaml::Sequence(os), Yaml::Sequence(bs), Yaml::Sequence(ts)) if os.len() == bs.len() && bs.len() == ts.len() => {
            os.iter().zip(bs).zip(ts).map(|((o, b), t)| merge_value(o, b, t)).collect::<Option<Vec<_>>>().map(Yaml::Sequence)
        }
        _ => None,
    }
}

/// Undo one change: every file it touched goes back to how it was just
/// before it, through the same gate as a save (compiled with the files in
/// place, refused on errors), committed as "Undo: <message>". A file that
/// changed again since keeps those later edits: merged line by line, or key by
/// key when the lines clash; when the edits really overlap, nothing is written
/// and the reason says which file.
pub fn undo(root: &Path, cfg: EditConfig, before: &Report, hash: &str, via: &str) -> Outcome {
    let Some(repo) = git_root(root) else { return Outcome::refuse("", "this space has no history to undo from") };
    let full = match resolve_commit(&repo, hash) {
        Ok(h) => h,
        Err(e) => return Outcome::refuse("", e),
    };
    let subject = git(&repo, &["log", "-1", "--format=%s", &full]).unwrap_or_default();
    let parent = format!("{}^", full);
    if !git_ok(&repo, &["rev-parse", "--verify", "--quiet", &parent]) {
        return Outcome::refuse("", format!("'{}' is where this history starts; there is nothing before it to go back to", subject));
    }
    let root_c = canon(root);
    let scope = root_c.to_string_lossy().to_string();
    let listed = match git_raw(&repo, &["diff-tree", "-r", "--no-renames", "--name-status", "-z", &parent, &full, "--", &scope]) {
        Ok(s) => s,
        Err(e) => return Outcome::refuse("", e),
    };
    let parts: Vec<&str> = listed.split('\0').filter(|s| !s.is_empty()).collect();
    let blob = |rev: &str, path: &str| git_raw(&repo, &["cat-file", "blob", &format!("{}:{}", rev, path)]).ok();
    let mut changes: Vec<(String, Option<String>)> = Vec::new();
    for pair in parts.chunks(2) {
        let [status, path] = pair else { continue };
        let rel = match repo.join(path).strip_prefix(&root_c) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if safe_rel_path(&rel).is_err() {
            return Outcome::refuse(&rel, format!("'{}' also changed {}, which the page does not edit; undo it with git revert {} in a terminal", subject, rel, &full[..7.min(full.len())]));
        }
        let then = if status.starts_with('D') { None } else { blob(&full, path) };
        let was = if status.starts_with('A') { None } else { blob(&parent, path) };
        let now = std::fs::read_to_string(repo.join(path)).ok();
        // grades go back with the facts they graded, entry by entry; never a reason to refuse
        if is_trust_file(&rel) {
            let target = if now == then { was } else { trust::merge_undo(now.as_deref(), then.as_deref(), was.as_deref()) };
            if target != now {
                changes.push((rel, target));
            }
            continue;
        }
        let target = if now == then {
            was
        } else if now == was {
            continue; // already back as it was
        } else {
            match (&now, &then, &was) {
                (Some(n), Some(t), Some(w)) => match merge3(n, t, w).or_else(|| merge_yaml(n, t, w)) {
                    Some(m) => Some(m),
                    None => return Outcome::refuse(&rel, format!("{} has changed since '{}' in the same lines, so it cannot be put back by itself; undo the newer change first, or edit the file", rel, subject)),
                },
                _ => return Outcome::refuse(&rel, format!("{} was created or removed again since '{}'; undo the newer change first", rel, subject)),
            }
        };
        if target.is_none() && now.is_none() {
            continue;
        }
        changes.push((rel, target));
    }
    if changes.is_empty() {
        return Outcome::refuse("", format!("nothing to undo: the files already look as they did before '{}'", subject));
    }
    let summary = format!("Undo: {}", subject);
    let name = if subject.is_empty() { full[..7.min(full.len())].to_string() } else { subject.clone() };
    apply_files(root, cfg, before, &changes, &Note { via, summary: &summary, keep_grades: true, ..Default::default() }, Some(("undo", &name)))
}

/// Commit every change under `root` (one space, or the whole spaces folder).
pub fn git_commit_all(root: &Path, message: &str) -> Result<String, String> {
    let repo = git_root(root).ok_or_else(|| "not inside a git repository".to_string())?;
    let scope = root.canonicalize().map_err(|e| e.to_string())?.to_string_lossy().to_string();
    git(&repo, &["add", "-A", "--", &scope])?;
    let msg = if message.trim().is_empty() { "Update the ontology" } else { message.trim() };
    // A machine with no git identity (a fresh server, the Docker image) commits
    // as Lorefile, as the other commit paths do, instead of failing.
    let mut args: Vec<String> = Vec::new();
    if git(&repo, &["config", "user.email"]).map(|s| s.is_empty()).unwrap_or(true) {
        args.extend(["-c".into(), format!("user.name={}", LOREFILE_NAME), "-c".into(), format!("user.email={}", LOREFILE_EMAIL)]);
    }
    args.extend(["commit".into(), "-q".into(), "-m".into(), msg.to_string(), "--".into(), scope]);
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    git(&repo, &a)?;
    git(&repo, &["rev-parse", "--short", "HEAD"])
}

/// Run git against a remote: no prompts (HTTPS and SSH both fail instead of
/// asking), English errors to read, and stopped after 90 seconds.
fn git_net(repo: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = git_cmd(repo);
    cmd.args(args).env("LC_ALL", "C").stdout(Stdio::piped()).stderr(Stdio::piped());
    let own_ssh = std::env::var_os("GIT_SSH_COMMAND").is_some() || git(repo, &["config", "core.sshCommand"]).map(|s| !s.is_empty()).unwrap_or(false);
    if !own_ssh {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=20");
    }
    let mut child = cmd.spawn().map_err(|e| format!("git: {}", e))?;
    let read = |p: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = p {
                let _ = r.read_to_string(&mut s);
            }
            s
        })
    };
    let out_t = read(child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>));
    let err_t = read(child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(format!("git: {}", e)),
        }
    };
    let out = out_t.join().unwrap_or_default();
    let err = err_t.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        Err(if err.trim().is_empty() { format!("git {} failed", args.first().unwrap_or(&"")) } else { err.trim().to_string() })
    }
}

/// What git said when it could not reach or line up with the shared copy, in words.
fn in_words(err: &str, host: &str) -> String {
    let e = err.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| e.contains(n));
    if e == "timed out" {
        format!("git took longer than 90 seconds to reach {}, so it was stopped. Check the address and the network.", host)
    } else if has(&["could not read username", "could not read password", "authentication failed", "permission denied (publickey", "terminal prompts disabled", "invalid username or password", "access denied", "permission to", "error: 403"]) {
        format!("git could not sign in to {}. Push once from a terminal to set up access, or use an SSH address.", host)
    } else if has(&["host key verification failed"]) {
        format!("git does not know {} yet. Connect once from a terminal (for GitHub: ssh -T git@github.com) and accept its key, then try again.", host)
    } else if has(&["could not resolve host", "name or service not known", "nodename nor servname", "connection timed out", "operation timed out", "connection refused", "network is unreachable"]) {
        format!("could not reach {}. Check the address and the network.", host)
    } else if has(&["does not appear to be a git repository", "repository not found", "not a git repository", "no such file or directory"]) {
        "There is no repository at that address that git can reach. Create an empty repository there first, and check the address.".to_string()
    } else if has(&["couldn't find remote ref", "could not find remote ref"]) {
        "The shared copy is still empty: Push first.".to_string()
    } else if has(&["non-fast-forward", "fetch first", "[rejected]"]) {
        "The shared copy has changes this space does not have yet. Pull first, then push.".to_string()
    } else if has(&["not possible to fast-forward", "diverging", "unrelated histories"]) {
        "This space and the shared copy have both changed, so they cannot be lined up by themselves. Merge them in a terminal (git pull), then come back.".to_string()
    } else if has(&["would be overwritten", "local changes"]) {
        "Some files here have changes that are not in history yet; commit them first, then pull.".to_string()
    } else {
        format!("git: {}", err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or(err).trim())
    }
}

/// The ontology's own repository, or a sentence on why sharing is not done from here.
fn own_repo(root: &Path, spaces_root: &Path) -> Result<PathBuf, String> {
    match home(root, spaces_root) {
        Home::Own(r) => Ok(r),
        Home::Other(r) => Err(format!("this space lives inside the repository at {}; share that repository with git there", r.display())),
        Home::Nowhere => Err("this space has no history yet: start it first (Settings, Start history)".into()),
        Home::NoGit => Err("git was not found on this machine, so there is no history to share".into()),
    }
}

/// Point `origin` at `url`, any address git accepts: SSH, HTTPS, or a folder
/// holding a bare repository. Nothing is sent yet and nothing is stored but
/// the address; the person's own git login is used when pushing.
pub fn git_set_remote(root: &Path, spaces_root: &Path, url: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("paste the address of an empty repository, e.g. git@github.com:you/brand-two.git".into());
    }
    if url.starts_with('-') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("that does not look like a git address: no spaces, and it cannot start with a dash".into());
    }
    let repo = own_repo(root, spaces_root)?;
    if git(&repo, &["remote", "get-url", "origin"]).is_ok() {
        git(&repo, &["remote", "set-url", "origin", url])?;
    } else {
        git(&repo, &["remote", "add", "origin", url])?;
    }
    Ok(format!("Connected to {}. Push to send the history there.", remote_label(url).1))
}

/// Push the current branch to `origin`, setting the upstream the first time.
pub fn git_push(root: &Path) -> Result<String, String> {
    let repo = git_root(root).ok_or_else(|| "not inside a git repository".to_string())?;
    let url = git(&repo, &["remote", "get-url", "origin"]).map_err(|_| "nothing to push to yet: paste an address under Share first".to_string())?;
    let (host, label) = remote_label(&url);
    git_net(&repo, &["push", "-q", "-u", "origin", "HEAD"]).map_err(|e| in_words(&e, &host))?;
    Ok(format!("Pushed to {}", label))
}

/// Bring in what the shared copy has and this one does not, fast-forward
/// only: two copies that both changed are left for a person to merge.
pub fn git_pull(root: &Path, spaces_root: &Path) -> Result<String, String> {
    let repo = own_repo(root, spaces_root)?;
    let url = git(&repo, &["remote", "get-url", "origin"]).map_err(|_| "nothing to pull from yet: paste an address under Share first".to_string())?;
    let (host, label) = remote_label(&url);
    let branch = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let was = git(&repo, &["rev-parse", "HEAD"]).unwrap_or_default();
    git_net(&repo, &["pull", "-q", "--ff-only", "origin", &branch]).map_err(|e| in_words(&e, &host))?;
    if !git_ok(&repo, &["rev-parse", "--verify", "--quiet", "@{u}"]) {
        let _ = git(&repo, &["branch", "-q", &format!("--set-upstream-to=origin/{}", branch)]);
    }
    let n: u64 = if was.is_empty() { 0 } else { git(&repo, &["rev-list", "--count", &format!("{}..HEAD", was)]).ok().and_then(|n| n.parse().ok()).unwrap_or(0) };
    Ok(if n == 0 { format!("Already up to date with {}", label) } else { format!("Pulled {} change{} from {}", n, if n == 1 { "" } else { "s" }, label) })
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
    fn a_pre_existing_error_elsewhere_does_not_block_an_unrelated_save() {
        let dir = scratch("preexisting");
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::write(dir.join("ontology.yaml"), "name: empty\ndomains:\n  - name: Core\n    types: []\n").unwrap();
        let (_, before) = check(&dir, "ontology.yaml", &Change::Write(std::fs::read_to_string(dir.join("ontology.yaml")).unwrap())).unwrap();
        assert!(!before.ok(), "an empty space has the no-types error");
        let out = apply(&dir, EditConfig { allow: true, ..Default::default() }, &before, "sources/files.yaml", Change::Write("connection: files\nkind: csv\npath: data\n".into()), &Note::via("test"), None);
        assert!(out.written, "{:?}", out.diagnostics);
        let out = apply(&dir, EditConfig { allow: true, ..Default::default() }, &before, "types/bad.yaml", Change::Write("type: Bad\nproperties: { id: { type: string, key: true } }\nrelationships:\n  - { name: has, to: Nope }\n".into()), &Note::via("test"), None);
        assert!(!out.written, "a new error of its own is still refused");
    }

    #[test]
    fn an_edit_that_introduces_an_unknown_type_is_refused() {
        let dir = starter("unknown-type");
        let cfg = EditConfig { allow: true, ..Default::default() };
        let before = validate::validate(&load(&dir));
        let original = std::fs::read_to_string(dir.join("constraints/basics.yaml")).unwrap();
        let typo = original.replace("applies_to: Account", "applies_to: Acount");
        let out = apply(&dir, cfg, &before, "constraints/basics.yaml", Change::Write(typo), &Note::via("test"), None);
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
        let dry = apply(&dir, EditConfig::default(), &before, "types/widget.yaml", Change::Write(text.into()), &Note::via("test"), None);
        assert!(dry.ok && !dry.written);
        assert!(!dir.join("types/widget.yaml").exists());
        // edit mode on: written, and the reloaded ontology has the type
        let out = apply(&dir, EditConfig { allow: true, ..Default::default() }, &before, "types/widget.yaml", Change::Write(text.into()), &Note::via("test"), None);
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
        let cfg = EditConfig { allow: true, ..Default::default() };
        // the action checks ACC-001/ACC-002, so the rule file cannot go
        let out = apply(&dir, cfg, &before, "constraints/basics.yaml", Change::Delete, &Note::via("test"), None);
        assert!(!out.ok && !out.written);
        assert!(out.diagnostics.iter().any(|d| d.code == "E006"));
        assert!(dir.join("constraints/basics.yaml").exists());
        // the glossary is referenced by nothing
        let out = apply(&dir, cfg, &before, "glossary.yaml", Change::Delete, &Note::via("test"), None);
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

    // -- history --

    /// `git` in `dir`, for checking what the code did.
    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn outer_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        sh(dir, &["init", "-q"]);
        sh(dir, &["config", "user.name", "Dev"]);
        sh(dir, &["config", "user.email", "dev@example.com"]);
        std::fs::write(dir.join("README.md"), "a project\n").unwrap();
        sh(dir, &["add", "-A"]);
        sh(dir, &["commit", "-q", "-m", "project"]);
    }

    #[test]
    fn a_new_space_starts_its_own_history_with_lorefiles_identity() {
        if !git_available() {
            return;
        }
        let dir = starter("init");
        assert_eq!(init_space(&dir, &dir, PAGE), "on");
        assert!(dir.join(".git").is_dir());
        assert_eq!(sh(&dir, &["config", "user.name"]), "Lorefile");
        assert_eq!(sh(&dir, &["config", "user.email"]), "lorefile@localhost");
        assert_eq!(sh(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
        assert_eq!(sh(&dir, &["log", "--format=%an <%ae>|%s"]), "person via page <person-via-page@localhost>|Start the space");
        assert!(sh(&dir, &["ls-files"]).contains("types/customer.yaml"), "the files are in the first commit");
        assert!(dir.join(".lorefile/.gitignore").exists(), "the state folder hides itself");
        assert_eq!(sh(&dir, &["status", "--porcelain"]), "", "nothing left over");
        assert_eq!(home(&dir, &dir), Home::Own(canon(&dir)));
        assert_eq!(init_space(&dir, &dir, PAGE), "on", "a second call changes nothing");
        assert_eq!(sh(&dir, &["rev-list", "--count", "HEAD"]), "1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_is_skipped_inside_another_repository_or_without_git() {
        if !git_available() {
            return;
        }
        let outer = scratch("outer");
        outer_repo(&outer);
        // a space somewhere inside a project: the project keeps its own history
        let nested = outer.join("work/onto");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("ontology.yaml"), "name: n\n").unwrap();
        assert_eq!(init_space(&nested, &nested, PAGE), "off (inside another repository)");
        assert!(!nested.join(".git").exists());
        assert_eq!(sh(&outer, &["rev-list", "--count", "HEAD"]), "1", "the project was not committed to");
        // unless the project ignores the folder (a gitignored spaces/): then the space keeps its own
        std::fs::write(outer.join(".gitignore"), "spaces/\n").unwrap();
        let ignored = outer.join("spaces/brand");
        std::fs::create_dir_all(&ignored).unwrap();
        std::fs::write(ignored.join("ontology.yaml"), "name: b\n").unwrap();
        assert_eq!(home(&ignored, &outer.join("spaces")), Home::Nowhere);
        assert_eq!(init_space(&ignored, &outer.join("spaces"), PAGE), "on");
        assert!(ignored.join(".git").is_dir());
        // no git on this machine: skipped, not failed
        let bare = starter("nogit");
        FAKE_GIT.with(|g| *g.borrow_mut() = Some(bare.join("no-such-dir/git").to_string_lossy().to_string()));
        let said = init_space(&bare, &bare, PAGE);
        let info = git_info(&bare, &bare);
        FAKE_GIT.with(|g| *g.borrow_mut() = None);
        assert_eq!(said, "off (git not found)");
        assert_eq!(info["history"], "off (git not found)");
        assert!(!bare.join(".git").exists());
        let _ = std::fs::remove_dir_all(&outer);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn changes_commit_by_default_only_where_the_repository_is_the_ontologys() {
        if !git_available() {
            return;
        }
        let on = EditConfig { allow: true, ..Default::default() };
        // the space's own repository
        let own = starter("own");
        init_space(&own, &own, PAGE);
        assert!(on.for_space(&own, &own).commit);
        assert!(!EditConfig { policy: CommitPolicy::Never, ..on }.for_space(&own, &own).commit, "--no-commit-edits");
        assert!(!EditConfig::default().for_space(&own, &own).commit, "no edits, no commits");
        // one repository for the whole folder of spaces
        let root = scratch("spaces-root");
        std::fs::create_dir_all(&root).unwrap();
        outer_repo(&root);
        let space = root.join("brand");
        std::fs::create_dir_all(&space).unwrap();
        std::fs::write(space.join("ontology.yaml"), "name: brand\n").unwrap();
        assert!(on.for_space(&space, &root).commit);
        assert_eq!(init_space(&space, &root, PAGE), "on");
        assert!(!space.join(".git").exists(), "no repository inside a repository");
        assert_eq!(sh(&root, &["log", "-1", "--format=%an|%s"]), "person via page|Start the space brand");
        // some other project, the space deeper inside it: left alone unless --commit-edits
        let deep = root.join("brand/../projects/x/onto");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(!on.for_space(&deep, &root.join("projects/x/onto")).commit);
        assert!(EditConfig { policy: CommitPolicy::Always, ..on }.for_space(&deep, &deep).commit, "--commit-edits");

        // and a write in the space's own history is one commit in words, by whoever made it
        let cfg = on.for_space(&own, &own);
        let before = validate::validate(&load(&own));
        let text = "type: Widget\ndescription: A thing we sell.\nproperties:\n  widget_id: { type: string, key: true }\n";
        let out = apply(&own, cfg, &before, "types/widget.yaml", Change::Write(text.into()), &Note { via: "agent via MCP", summary: "An agent learned: created Widget", reason: "The user sells widgets.", accepted_by: Some(PAGE), ..Default::default() }, None);
        assert!(out.written && out.commit.is_some() && out.git_error.is_none(), "{:?}", out);
        assert_eq!(sh(&own, &["log", "-1", "--format=%an <%ae>|%cn|%s|%b"]), "agent via MCP <agent-via-mcp@localhost>|person via page|An agent learned: created Widget|The user sells widgets.");
        // saving the same text again is not a new commit
        let again = apply(&own, cfg, &validate::validate(&load(&own)), "types/widget.yaml", Change::Write(text.into()), &Note::via(PAGE), None);
        assert!(again.written && again.commit.is_none() && again.git_error.is_none(), "{:?}", again);
        assert_eq!(sh(&own, &["rev-list", "--count", "HEAD"]), "2");
        let _ = std::fs::remove_dir_all(&own);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn undo_goes_through_the_same_gate_as_a_save() {
        if !git_available() {
            return;
        }
        let dir = starter("undo");
        init_space(&dir, &dir, PAGE);
        let cfg = EditConfig { allow: true, ..Default::default() }.for_space(&dir, &dir);
        let report = |d: &Path| validate::validate(&load(d));
        let head = |d: &Path| sh(d, &["rev-parse", "HEAD"]);
        // A: a new entity; B: another that links to it; C: a glossary word; D: another word, far from C
        let widget = "type: Widget\nproperties:\n  widget_id: { type: string, key: true }\n";
        let a = apply(&dir, cfg, &report(&dir), "types/widget.yaml", Change::Write(widget.into()), &Note { via: PAGE, summary: "created Widget", ..Default::default() }, None);
        let gadget = "type: Gadget\nproperties:\n  gadget_id: { type: string, key: true }\nrelationships:\n  uses: { to: Widget, cardinality: many-to-one }\n";
        let b = apply(&dir, cfg, &report(&dir), "types/gadget.yaml", Change::Write(gadget.into()), &Note { via: PAGE, summary: "created Gadget", ..Default::default() }, None);
        assert!(a.commit.is_some() && b.commit.is_some(), "{:?} {:?}", a, b);
        let glossary = std::fs::read_to_string(dir.join("glossary.yaml")).unwrap();
        let c_text = glossary.replace("  - term: client\n", "  - term: patron\n    means: Customer\n  - term: client\n");
        let c = apply(&dir, cfg, &report(&dir), "glossary.yaml", Change::Write(c_text.clone()), &Note { via: PAGE, summary: "added the word patron", ..Default::default() }, None);
        let d_text = format!("{}  - term: purse\n    means: Account\n", c_text);
        let d = apply(&dir, cfg, &report(&dir), "glossary.yaml", Change::Write(d_text), &Note { via: PAGE, summary: "added the word purse", ..Default::default() }, None);
        assert!(c.commit.is_some() && d.commit.is_some());

        // undoing A would leave Gadget pointing at nothing: refused, nothing written, no commit
        let tip = head(&dir);
        let out = undo(&dir, cfg, &report(&dir), a.commit.as_deref().unwrap(), PAGE);
        assert!(!out.written && !out.ok, "{:?}", out);
        assert!(out.diagnostics.iter().any(|x| x.level == Level::Error && x.message.contains("Widget")), "{:?}", out.diagnostics);
        assert!(dir.join("types/widget.yaml").exists(), "the tree is unchanged");
        assert_eq!(head(&dir), tip, "and so is the history");

        // undoing C, an older change, keeps D's later word
        let out = undo(&dir, cfg, &report(&dir), c.commit.as_deref().unwrap(), PAGE);
        assert!(out.written && out.commit.is_some(), "{:?}", out);
        let now = std::fs::read_to_string(dir.join("glossary.yaml")).unwrap();
        assert!(!now.contains("patron") && now.contains("purse"), "{}", now);
        assert_eq!(sh(&dir, &["log", "-1", "--format=%an|%s"]), "person via page|Undo: Added the word patron");

        // X rewrites a whole file (as a patch does the first time) and changes one sentence; Y then changes
        // another sentence in the rewritten text: the lines clash, the YAML does not, so X alone is undone
        let original = std::fs::read_to_string(dir.join("types/customer.yaml")).unwrap();
        let mut doc: Yaml = serde_yaml::from_str(&original).unwrap();
        doc["types"][0]["description"] = Yaml::String("Someone who buys from us.".into());
        let x = apply(&dir, cfg, &report(&dir), "types/customer.yaml", Change::Write(serde_yaml::to_string(&doc).unwrap()), &Note { via: PAGE, summary: "described Customer", ..Default::default() }, None);
        doc["types"][1]["description"] = Yaml::String("Money we hold for a customer.".into());
        let y = apply(&dir, cfg, &report(&dir), "types/customer.yaml", Change::Write(serde_yaml::to_string(&doc).unwrap()), &Note { via: "agent via MCP", summary: "described Account", ..Default::default() }, None);
        assert!(x.commit.is_some() && y.commit.is_some());
        let out = undo(&dir, cfg, &report(&dir), x.commit.as_deref().unwrap(), PAGE);
        assert!(out.written, "{:?}", out);
        let back: Yaml = serde_yaml::from_str(&std::fs::read_to_string(dir.join("types/customer.yaml")).unwrap()).unwrap();
        assert_eq!(back["types"][0]["description"].as_str(), Some("A person or company that has signed up with us."), "X is undone");
        assert_eq!(back["types"][1]["description"].as_str(), Some("Money we hold for a customer."), "Y stays");
        // and two edits of the very same sentence really clash: refused, nothing written
        doc["types"][1]["description"] = Yaml::String("Money held.".into());
        let z = apply(&dir, cfg, &report(&dir), "types/customer.yaml", Change::Write(serde_yaml::to_string(&doc).unwrap()), &Note { via: PAGE, summary: "shortened Account", ..Default::default() }, None);
        assert!(z.commit.is_some());
        let tip = head(&dir);
        let out = undo(&dir, cfg, &report(&dir), y.commit.as_deref().unwrap(), PAGE);
        assert!(!out.written && out.refused.as_deref().unwrap_or("").contains("in the same lines"), "{:?}", out);
        assert_eq!(head(&dir), tip);

        // B then A, newest first, both go
        assert!(undo(&dir, cfg, &report(&dir), b.commit.as_deref().unwrap(), PAGE).written);
        assert!(!dir.join("types/gadget.yaml").exists());
        assert!(undo(&dir, cfg, &report(&dir), a.commit.as_deref().unwrap(), PAGE).written);
        assert!(!dir.join("types/widget.yaml").exists());
        // the same undo twice has nothing left to do; the first commit has nothing before it
        assert!(undo(&dir, cfg, &report(&dir), a.commit.as_deref().unwrap(), PAGE).refused.unwrap().contains("nothing to undo"));
        let first = sh(&dir, &["rev-list", "--max-parents=0", "HEAD"]);
        assert!(undo(&dir, cfg, &report(&dir), &first, PAGE).refused.unwrap().contains("where this history starts"));
        assert!(undo(&dir, cfg, &report(&dir), "--upload-pack=x", PAGE).refused.is_some(), "only hex ids");

        let log = git_log(&dir, 50, None).unwrap();
        let messages: Vec<&str> = log["commits"].as_array().unwrap().iter().map(|c| c["message"].as_str().unwrap()).collect();
        assert_eq!(messages[0], "Undo: Created Widget");
        assert_eq!(*messages.last().unwrap(), "Start the space");
        assert_eq!(log["total"], 12, "start, seven changes, four undos");
        let shown = git_show(&dir, b.commit.as_deref().unwrap()).unwrap();
        assert!(shown["diff"].as_str().unwrap().contains("+++ b/types/gadget.yaml"), "{}", shown["diff"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remotes_are_named_by_where_they_live() {
        assert_eq!(remote_label("git@github.com:you/brand-two.git"), ("github.com".into(), "github.com/you/brand-two".into()));
        assert_eq!(remote_label("https://gitlab.com/team/onto.git"), ("gitlab.com".into(), "gitlab.com/team/onto".into()));
        assert_eq!(remote_label("ssh://git@bitbucket.org:22/a/b.git"), ("bitbucket.org".into(), "bitbucket.org/a/b".into()));
        assert_eq!(remote_label("/Volumes/Shared/onto.git"), ("this computer".into(), "/Volumes/Shared/onto.git".into()));
        assert_eq!(remote_label("file:///srv/onto.git"), ("this computer".into(), "/srv/onto.git".into()));
        assert_eq!(identity("agent via MCP"), ("agent via MCP".into(), "agent-via-mcp@localhost".into()));
        assert!(in_words("fatal: could not read Username for 'https://github.com': terminal prompts disabled", "github.com").starts_with("git could not sign in to github.com"));
        // "403" in a folder name (a process id, a year) is not git's HTTP 403
        assert!(in_words("fatal: '/tmp/lorefile-http-spaces-7105-24031/nowhere.git' does not appear to be a git repository", "this computer").starts_with("There is no repository"));
        assert!(in_words("fatal: unable to access 'https://github.com/you/x.git/': The requested URL returned error: 403", "github.com").starts_with("git could not sign in to github.com"));
        assert!(in_words("git@github.com: Permission denied (publickey).", "github.com").contains("SSH address"));
    }

    // -- grades --

    /// A copy of the e-commerce example with its own history, no trust.yaml.
    fn shop(tag: &str) -> (PathBuf, EditConfig) {
        let dir = scratch(tag);
        copy(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ecommerce"), &dir);
        let cfg = if git_available() {
            init_space(&dir, &dir, PAGE);
            EditConfig { allow: true, ..Default::default() }.for_space(&dir, &dir)
        } else {
            EditConfig { allow: true, ..Default::default() }
        };
        (dir, cfg)
    }

    fn copy(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for e in std::fs::read_dir(src).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if e.path().is_dir() { copy(&e.path(), &dst.join(&name)) } else { std::fs::copy(e.path(), dst.join(&name)).unwrap(); }
        }
    }

    fn book(dir: &Path) -> TrustBook {
        trust::parse(&std::fs::read_to_string(dir.join(trust::FILE)).unwrap_or_default()).unwrap()
    }

    fn metric_op(name: &str) -> Json {
        json!({ "op": "add_metric", "metric": name, "description": "Meta's reported conversion value over spend.", "basis": "attributed", "connection": "meta_ads", "tool": "get_insights", "params": { "level": "campaign" }, "measure": "sum(conversion_value) / sum(spend)", "grain": "week", "owner": "growth" })
    }

    #[test]
    fn an_agents_save_is_proposed_and_a_persons_accept_or_edit_makes_it_accepted() {
        let (dir, cfg) = shop("ladder");
        let ont = load(&dir);
        let before = validate::validate(&ont);
        assert!(!ont.graded.has_file && before.diags.iter().all(|d| !d.code.starts_with("E018") && d.code != "W113" && d.code != "W114"));
        // an agent with edits on: written, and in the same commit the grade is proposed
        let p = crate::patch::plan(&ont, &metric_op("blended_roas")).unwrap();
        let out = apply(&dir, cfg, &before, &p.file, Change::Write(p.text.clone()), &Note { via: "agent via MCP", summary: "An agent learned: added metric blended_roas", ..Default::default() }, Some(("metric", "blended_roas")));
        assert!(out.written, "{:?}", out);
        assert_eq!(out.stamped, vec!["metric blended_roas: proposed".to_string()]);
        let b = book(&dir);
        assert_eq!(b.metrics["blended_roas"].grade, Grade::Proposed);
        assert_eq!(b.metrics["blended_roas"].by, "agent via MCP");
        assert_eq!(b.metrics.len(), 1, "nothing else was graded");
        if cfg.commit {
            assert_eq!(sh(&dir, &["log", "-1", "--format=%an"]), "agent via MCP");
            assert_eq!(sh(&dir, &["show", "--format=", "--name-only", "HEAD"]), "logic/measurement.yaml\ntrust.yaml", "fact and grade in one commit");
        }
        let ont = load(&dir);
        assert_eq!(ont.grade_of(TKind::Metric, "blended_roas").grade, Some(Grade::Proposed));
        let d = ont.describe("blended_roas").unwrap();
        assert_eq!(d["basis"], "attributed");
        assert!(d["trust"]["say"].as_str().unwrap().starts_with("Unverified: an agent proposed this definition on"), "{}", d["trust"]);
        // a person's edit in the page: accepted
        let text = std::fs::read_to_string(dir.join(&p.file)).unwrap().replace("Meta's reported conversion value over spend.", "Meta's reported conversion value over Meta spend.");
        let out = apply(&dir, cfg, &validate::validate(&ont), &p.file, Change::Write(text), &Note::via("Priya via page"), None);
        assert!(out.written, "{:?}", out);
        let r = &book(&dir).metrics["blended_roas"];
        assert_eq!((r.grade, r.by.as_str()), (Grade::Accepted, "Priya via page"));
        // an agent's proposal accepted by a person: the person's grade, the agent the author
        let ont = load(&dir);
        let p = crate::patch::plan(&ont, &json!({ "op": "add_term", "term": "cart", "means": "Order" })).unwrap();
        let out = apply(&dir, cfg, &validate::validate(&ont), &p.file, Change::Write(p.text), &Note { via: "agent via MCP", summary: "An agent learned: added the word cart", accepted_by: Some("Priya via page"), ..Default::default() }, None);
        assert!(out.written, "{:?}", out);
        let r = &book(&dir).terms["cart"];
        assert_eq!((r.grade, r.by.as_str()), (Grade::Accepted, "Priya via page"));
        if cfg.commit {
            assert_eq!(sh(&dir, &["log", "-1", "--format=%an|%cn"]), "agent via MCP|Priya via page");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_check_needs_a_source_and_a_note_and_only_a_recipe_change_undoes_it() {
        let (dir, cfg) = shop("check");
        let report = |d: &Path| validate::validate(&load(d));
        let check = |against: &str, note: &str| GradeOp::Check { kind: TKind::Metric, name: "success".into(), against: against.into(), note: note.into() };
        // ungraded: accept first
        let out = grade(&dir, cfg, &report(&dir), &check("shop_bigquery", "412 paid orders"), "Priya via page");
        assert!(out.refused.as_deref().unwrap_or("").contains("accepts it first"), "{:?}", out);
        let out = grade(&dir, cfg, &report(&dir), &GradeOp::Accept { kind: TKind::Metric, name: "success".into() }, "Priya via page");
        assert!(out.written, "{:?}", out);
        assert!(grade(&dir, cfg, &report(&dir), &check("", "412"), "Priya via page").refused.is_some(), "no source");
        assert!(grade(&dir, cfg, &report(&dir), &check("shop_bigquery", " "), "Priya via page").refused.is_some(), "no note");
        assert!(grade(&dir, cfg, &report(&dir), &check("nowhere", "412"), "Priya via page").refused.is_some(), "not a source");
        let out = grade(&dir, cfg, &report(&dir), &check("shop_bigquery", "412 paid orders for 16–22 Nov, the same as Shopify's own report"), "Priya via page");
        assert!(out.written, "{:?}", out);
        if cfg.commit {
            assert_eq!(sh(&dir, &["log", "-1", "--format=%an|%s"]), "Priya via page|Checked metric success against shop_bigquery");
        }
        let checked = book(&dir).metrics["success"].clone();
        assert_eq!((checked.grade, checked.against.as_deref()), (Grade::Checked, Some("shop_bigquery")));
        assert_eq!(load(&dir).describe("success").unwrap()["trust"]["say"], "Checked against shop_bigquery by Priya on ".to_string() + &trust::short_date(&checked.at) + ".");
        // a person changes only the words: still checked, fresh fingerprint
        let file = dir.join("logic/measurement.yaml");
        let original = std::fs::read_to_string(&file).unwrap();
        let words = original.replace("The one number the business is run on: paid orders.", "The one number the business runs on: paid orders.");
        let out = apply(&dir, cfg, &report(&dir), "logic/measurement.yaml", Change::Write(words.clone()), &Note::via(PAGE), None);
        assert!(out.written, "{:?}", out);
        let now = book(&dir).metrics["success"].clone();
        assert_eq!(now.grade, Grade::Checked);
        assert_ne!(now.of, checked.of, "a fresh fingerprint");
        assert_eq!(load(&dir).grade_of(TKind::Metric, "success").grade, Some(Grade::Checked));
        // the recipe (the filter): the check was of the old recipe, so accepted
        let recipe = words.replace("status not in ('cancelled', 'returned')", "status not in ('cancelled')");
        let out = apply(&dir, cfg, &report(&dir), "logic/measurement.yaml", Change::Write(recipe), &Note::via(PAGE), None);
        assert!(out.written, "{:?}", out);
        let now = book(&dir).metrics["success"].clone();
        assert_eq!((now.grade, now.against), (Grade::Accepted, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_or_renaming_leaves_no_orphan_and_a_batch_stamps_in_one_commit() {
        let (dir, cfg) = shop("delete");
        let ont = load(&dir);
        let plan = crate::patch::plan_many(&ont, &[metric_op("m_one"), metric_op("m_two"), metric_op("m_three"), json!({ "op": "add_term", "term": "basket2", "means": "Order" }), json!({ "op": "add_term", "term": "trolley", "means": "Order" })]).unwrap();
        let out = apply_many(&dir, cfg, &validate::validate(&ont), &plan.files, &Note::via("agent via MCP"), None);
        assert!(out.written, "{:?}", out);
        assert_eq!(out.stamped.len(), 5, "{:?}", out.stamped);
        if cfg.commit {
            assert_eq!(sh(&dir, &["rev-list", "--count", "HEAD"]), "2", "one commit for the batch");
        }
        // rename m_one to m_uno, delete m_two
        let file = dir.join("logic/measurement.yaml");
        let text = std::fs::read_to_string(&file).unwrap().replace("metric: m_one", "metric: m_uno");
        let mut doc: Yaml = serde_yaml::from_str(&text).unwrap();
        let list = doc["metrics"].as_sequence_mut().unwrap();
        list.retain(|m| m["metric"].as_str() != Some("m_two"));
        let out = apply(&dir, cfg, &validate::validate(&load(&dir)), "logic/measurement.yaml", Change::Write(serde_yaml::to_string(&doc).unwrap()), &Note::via(PAGE), None);
        assert!(out.written, "{:?}", out);
        let b = book(&dir);
        assert!(!b.metrics.contains_key("m_one") && !b.metrics.contains_key("m_two"), "{:?}", b.metrics.keys());
        assert_eq!(b.metrics["m_uno"].grade, Grade::Accepted, "the new name is a person's");
        assert_eq!(b.metrics["m_three"].grade, Grade::Proposed, "untouched entries stay as they were");
        assert!(validate::validate(&load(&dir)).diags.iter().all(|d| d.code != "W114"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_behind_the_grades_back_stops_it_counting_until_accepted_again() {
        let (dir, cfg) = shop("behind");
        let report = |d: &Path| validate::validate(&load(d));
        assert!(grade(&dir, cfg, &report(&dir), &GradeOp::Accept { kind: TKind::Metric, name: "success".into() }, PAGE).written);
        // comments, spacing and key order do not count
        let file = dir.join("logic/measurement.yaml");
        let original = std::fs::read_to_string(&file).unwrap();
        // the owner moves up to the top of the metric, with a comment and a blank line
        let shuffled = original
            .replacen("    owner: growth@lumenhome\n    caveats: >\n      Platform", "    caveats: >\n      Platform", 1)
            .replacen("  - metric: success\n", "  - metric: success\n    # who decides\n    owner: growth@lumenhome\n\n", 1);
        assert_ne!(shuffled, original);
        std::fs::write(&file, &shuffled).unwrap();
        assert_eq!(load(&dir).grade_of(TKind::Metric, "success").grade, Some(Grade::Accepted), "still the same thing");
        // the filter edited by hand: W113, ungraded with changed_since
        std::fs::write(&file, original.replace("payment_status = 'paid'", "payment_status in ('paid', 'pending')")).unwrap();
        let ont = load(&dir);
        let rep = validate::validate(&ont);
        let w: Vec<&Diag> = rep.diags.iter().filter(|d| d.code == "W113").collect();
        assert_eq!(w.len(), 1, "{:?}", rep.diags);
        assert!(w[0].message.starts_with("metric success changed since it was accepted (a person, "), "{}", w[0].message);
        let t = &ont.describe("success").unwrap()["trust"];
        assert_eq!(t["grade"], "ungraded");
        assert!(t["changed_since"].as_str().unwrap().starts_with("accepted "), "{}", t);
        // accept again: cleared
        assert!(grade(&dir, cfg, &rep, &GradeOp::Accept { kind: TKind::Metric, name: "success".into() }, "person via CLI").written);
        assert!(report(&dir).diags.iter().all(|d| d.code != "W113"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_grade_covers_the_source_it_reads_and_every_meaning_of_a_word() {
        let (dir, cfg) = shop("reads");
        let report = |d: &Path| validate::validate(&load(d));
        for (k, n) in [(TKind::Metric, "success"), (TKind::Binding, "Order"), (TKind::Term, "basket")] {
            assert!(grade(&dir, cfg, &report(&dir), &GradeOp::Accept { kind: k, name: n.into() }, PAGE).written);
        }
        // the same recipe over another dataset is another number: an agent re-pointing the source would overwrite the grade
        let src = std::fs::read_to_string(dir.join("sources/shop_bigquery.yaml")).unwrap();
        assert!(src.contains("dataset: shop\n"));
        let moved = src.replace("dataset: shop\n", "dataset: someone_elses\n");
        let held = graded_overwrites(&dir, &[("sources/shop_bigquery.yaml".to_string(), moved.clone())]);
        assert_eq!(held.len(), 1, "{:?}", held);
        assert!(held[0].starts_with("metric success is accepted"), "{:?}", held);
        // only the words of the source changed: nothing a grade depends on
        assert!(graded_overwrites(&dir, &[("sources/shop_bigquery.yaml".to_string(), src.replace("The analytics warehouse", "The warehouse"))]).is_empty());
        // re-pointed by hand: the grade stops counting
        std::fs::write(dir.join("sources/shop_bigquery.yaml"), &moved).unwrap();
        let ont = load(&dir);
        assert!(ont.grade_of(TKind::Metric, "success").changed, "the metric reads the moved source");
        assert_eq!(ont.grade_of(TKind::Binding, "Order").grade, Some(Grade::Accepted), "Order reads another source");
        assert!(validate::validate(&ont).diags.iter().any(|d| d.code == "W113" && d.message.starts_with("metric success changed")));
        std::fs::write(dir.join("sources/shop_bigquery.yaml"), &src).unwrap();
        // a second meaning for an accepted word, in another file, is a change to that word
        // the tool an agent asks "where do Order rows live" says who stands behind the answer
        let ont = load(&dir);
        assert_eq!(crate::data::DataLayer::new(&ont).where_is("Order").unwrap()["trust"]["grade"], "accepted");
        // --keep-grades never restores a line the compiler would reject, even when the fingerprint matches
        let mut claimed = book(&dir);
        let good = claimed.metrics["success"].clone();
        claimed.metrics.insert("success".into(), TrustRecord { grade: Grade::Checked, against: Some("warehouse_x".into()), note: Some("n".into()), ..good.clone() });
        claimed.terms.insert("basket".into(), TrustRecord { grade: Grade::Checked, against: Some("shop_bigquery".into()), note: Some("n".into()), ..claimed.terms["basket"].clone() });
        let (kept, dropped) = trust::keep_matching(&ont, &claimed);
        assert!(kept.metrics.is_empty() && kept.terms.is_empty() && kept.bindings.contains_key("Order"), "{:?}", kept);
        assert_eq!(dropped.len(), 2, "{:?}", dropped);
        let held = graded_overwrites(&dir, &[("a_terms.yaml".to_string(), "terms:\n  - term: basket\n    means: Customer\n".to_string())]);
        assert!(held.len() == 1 && held[0].starts_with("word basket is accepted"), "{:?}", held);
        std::fs::write(dir.join("a_terms.yaml"), "terms:\n  - term: basket\n    means: Customer\n").unwrap();
        assert!(load(&dir).grade_of(TKind::Term, "basket").changed, "written by hand, whatever order the files load in");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vouching_for_all_leaves_what_changed_or_was_proposed_for_a_look_each() {
        let (dir, cfg) = shop("vouch");
        let report = |d: &Path| validate::validate(&load(d));
        assert!(grade(&dir, cfg, &report(&dir), &GradeOp::Accept { kind: TKind::Metric, name: "success".into() }, PAGE).written);
        let file = dir.join("logic/measurement.yaml");
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, text.replace("payment_status = 'paid'", "payment_status in ('paid', 'pending')")).unwrap();
        assert!(load(&dir).grade_of(TKind::Metric, "success").changed);
        let out = grade(&dir, cfg, &report(&dir), &GradeOp::VouchAll, PAGE);
        assert!(out.written, "{:?}", out);
        let ont = load(&dir);
        assert!(ont.grade_of(TKind::Metric, "success").changed, "nobody looked at the edited metric, so it is still changed");
        assert_eq!(ont.grade_of(TKind::Metric, "paid_performance").grade, Some(Grade::Accepted));
        assert_eq!(ont.graded.counts().ungraded, 1, "only the changed one is left");
        assert!(grade(&dir, cfg, &report(&dir), &GradeOp::VouchAll, PAGE).refused.unwrap().contains("need a look each"));
        // other spellings of the grades file are the grades file
        for f in ["Trust.yaml", "TRUST.yaml", "trust.yml"] {
            assert!(is_trust_file(f), "{}", f);
            assert_eq!(apply(&dir, cfg, &report(&dir), f, Change::Write("trust: {}\n".into()), &Note::via("agent via MCP"), None).refused.as_deref(), Some(trust::REFUSED), "{}", f);
        }
        assert!(!is_trust_file("logic/trust.yaml"), "elsewhere it is just a file the loader refuses");
        // a note is kept short and plain
        assert_eq!(trust::clean_note("  412 =\u{7f}\n 412  "), "412 = 412");
        assert_eq!(trust::clean_note(&"x".repeat(2000)).chars().count(), trust::NOTE_MAX + 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grades_that_make_no_sense_or_name_nothing_are_flagged() {
        let (dir, _) = shop("e018");
        std::fs::write(dir.join("trust.yaml"), "trust:\n  metrics:\n    gone: { grade: accepted, by: x, at: 2026-11-26, of: abc }\n    success: { grade: checked, by: x, at: 2026-11-26, of: abc, against: nowhere, note: n }\n  terms:\n    basket: { grade: checked, by: x, at: 2026-11-26, of: abc, against: shop_bigquery, note: n }\n  guardrails:\n    EC-001: { grade: checked, by: x, at: 2026-11-26, of: \"\" }\n").unwrap();
        let rep = validate::validate(&load(&dir));
        let has = |code: &str, bit: &str| rep.diags.iter().any(|d| d.code == code && d.message.contains(bit));
        assert!(has("W114", "metric gone"), "{:?}", rep.diags);
        assert!(has("E018", "`nowhere`"), "{:?}", rep.diags);
        assert!(has("E018", "word basket is marked checked"), "{:?}", rep.diags);
        assert!(has("E018", "guardrail EC-001 has a grade with no fingerprint"), "{:?}", rep.diags);
        assert!(has("E018", "without against"), "{:?}", rep.diags);
        // a trust file anywhere else, or a grade key in a fact file, is an error
        std::fs::write(dir.join("logic/trust.yaml"), "trust: {}\n").unwrap();
        assert!(validate::validate(&load(&dir)).diags.iter().any(|d| d.code == "E001" && d.message.contains("logic/trust.yaml")));
        std::fs::remove_file(dir.join("logic/trust.yaml")).unwrap();
        let m = std::fs::read_to_string(dir.join("logic/measurement.yaml")).unwrap().replacen("    grain: day\n", "    grain: day\n    grade: checked\n", 1);
        std::fs::write(dir.join("logic/measurement.yaml"), m).unwrap();
        assert!(validate::validate(&load(&dir)).diags.iter().any(|d| d.code == "E001" && d.message.contains("unknown field `grade`")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_trust_file_changes_nothing_and_nobody_but_lorefile_writes_one() {
        for ex in ["ecommerce", "supply-chain", "b2b-sales"] {
            let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(ex);
            let ont = load(&d);
            assert!(!ont.graded.has_file);
            assert!(!ont.summary_markdown().contains("Trust ("), "no Trust line without trust.yaml");
            // an empty trust file adds no diagnostic
            let (dir, _) = (scratch(ex), ());
            copy(&d, &dir);
            let plain = serde_json::to_string(&validate::validate(&load(&dir)).diags).unwrap();
            std::fs::write(dir.join("trust.yaml"), "trust: {}\n").unwrap();
            assert_eq!(serde_json::to_string(&validate::validate(&load(&dir)).diags).unwrap(), plain);
            let _ = std::fs::remove_dir_all(&dir);
        }
        let (dir, cfg) = shop("refuse");
        let before = validate::validate(&load(&dir));
        let out = apply(&dir, cfg, &before, "trust.yaml", Change::Write("trust: {}\n".into()), &Note::via(PAGE), None);
        assert_eq!(out.refused.as_deref(), Some(trust::REFUSED));
        let out = apply_many(&dir, cfg, &before, &[("trust.yaml".into(), "trust: {}\n".into())], &Note::via("agent via MCP"), None);
        assert_eq!(out.refused.as_deref(), Some(trust::REFUSED));
        assert!(!dir.join("trust.yaml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_takes_the_grade_back_with_the_fact() {
        if !git_available() {
            return;
        }
        let (dir, cfg) = shop("undo-grade");
        let report = |d: &Path| validate::validate(&load(d));
        let p = crate::patch::plan(&load(&dir), &metric_op("blended_roas")).unwrap();
        let added = apply(&dir, cfg, &report(&dir), &p.file, Change::Write(p.text), &Note::via("agent via MCP"), None);
        assert!(added.commit.is_some());
        assert!(grade(&dir, cfg, &report(&dir), &GradeOp::Accept { kind: TKind::Metric, name: "success".into() }, PAGE).written);
        let checked = grade(&dir, cfg, &report(&dir), &GradeOp::Check { kind: TKind::Metric, name: "success".into(), against: "shop_bigquery".into(), note: "412 both".into() }, PAGE);
        assert!(checked.commit.is_some());
        // undo of the check: accepted again
        let out = undo(&dir, cfg, &report(&dir), checked.commit.as_deref().unwrap(), PAGE);
        assert!(out.written, "{:?}", out);
        assert_eq!(book(&dir).metrics["success"].grade, Grade::Accepted);
        // undo of the commit that added the metric: its entry goes too, success stays
        let out = undo(&dir, cfg, &report(&dir), added.commit.as_deref().unwrap(), PAGE);
        assert!(out.written, "{:?}", out);
        let b = book(&dir);
        assert!(!b.metrics.contains_key("blended_roas") && b.metrics.contains_key("success"), "{:?}", b.metrics.keys());
        assert!(out.stamped.is_empty(), "undo stamps nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_saves_at_once_both_land_their_grades() {
        let (dir, cfg) = shop("lock");
        let before = validate::validate(&load(&dir));
        let threads: Vec<_> = ["first", "second", "third"]
            .iter()
            .map(|n| {
                let (dir, before) = (dir.clone(), Report { diags: before.diags.clone() });
                let n = n.to_string();
                std::thread::spawn(move || {
                    let text = format!("metrics:\n  - metric: m_{}\n    description: x\n    owner: y\n    source: {{ connection: shop_bigquery, table: orders }}\n", n);
                    apply(&dir, cfg, &before, &format!("logic/{}.yaml", n), Change::Write(text), &Note::via("agent via MCP"), None)
                })
            })
            .collect();
        for t in threads {
            let out = t.join().unwrap();
            assert!(out.written, "{:?}", out);
        }
        let b = book(&dir);
        for n in ["m_first", "m_second", "m_third"] {
            assert!(b.metrics.contains_key(n), "{} landed: {:?}", n, b.metrics.keys());
        }
        assert!(!dir.join(".lorefile/trust.lock").exists(), "the lock is let go");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- helpers --

    static N: AtomicUsize = AtomicUsize::new(0);

    fn scratch(tag: &str) -> PathBuf {
        let n = N.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("lorefile-edit-{}-{}-{}", std::process::id(), n, tag))
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
