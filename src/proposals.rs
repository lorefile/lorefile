//! Changes proposed by agents over MCP that were not written straight away,
//! waiting for a person to accept them in the web page, plus a small activity
//! log. Both live under `<ontology>/.lorefile/`, which the loader ignores.

use crate::edit::Outcome;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub file: String,
    pub content: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub via: String,
    pub created_at: String,
    /// pending | accepted | dismissed
    pub status: String,
    /// True when the proposed file compiled without errors.
    pub ok: bool,
    #[serde(default)]
    pub errors: usize,
    #[serde(default)]
    pub diagnostics: Vec<Json>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub resolved_at: String,
    /// The file's current text when the proposal was made, so the page can show a diff.
    #[serde(default)]
    pub before: String,
    /// What the change does, in words: "added metric success". Empty for a raw file edit.
    #[serde(default)]
    pub summary: String,
    /// Every file of a batch; `file`, `content` and `before` are the first of them.
    #[serde(default)]
    pub files: Vec<ProposedFile>,
    /// An agent reporting a check (`record_check`): no file content; a person's
    /// "Yes, mark as checked" writes the grade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckClaim>,
}

/// What an agent says it compared: the thing, the source whose rows it used, and what matched.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckClaim {
    /// metric | binding | guardrail
    pub kind: String,
    pub name: String,
    pub against: String,
    pub note: String,
    /// The fingerprint of the definition when the agent compared it: a check
    /// reported for one recipe never grades another.
    #[serde(default)]
    pub of: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProposedFile {
    pub file: String,
    pub content: String,
    #[serde(default)]
    pub before: String,
    #[serde(default)]
    pub summary: String,
}

fn lorefile_dir(root: &Path) -> PathBuf {
    root.join(".lorefile")
}

fn proposals_dir(root: &Path) -> PathBuf {
    lorefile_dir(root).join("proposals")
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Record a proposal that was validated but not written.
/// A batch: several files, one proposal, accepted as one.
pub fn store_many(root: &Path, files: &[ProposedFile], reason: &str, via: &str, out: &Outcome, summary: &str) -> Result<Proposal, String> {
    let first = files.first().ok_or_else(|| "nothing to propose".to_string())?;
    let mut p = store(root, &first.file, &first.content, reason, via, out, summary)?;
    p.files = files.to_vec();
    p.kind = if files.len() > 1 { "batch".into() } else { out.kind.clone() };
    p.name = if files.len() > 1 { format!("{} files", files.len()) } else { out.name.clone() };
    write(root, &p)?;
    Ok(p)
}

pub fn store(root: &Path, file: &str, content: &str, reason: &str, via: &str, out: &Outcome, summary: &str) -> Result<Proposal, String> {
    let dir = proposals_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {}", dir.display(), e))?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) + 1;
    let id = format!("p-{}-{}", stamp, n);
    let before = std::fs::read_to_string(root.join(file)).unwrap_or_default();
    let p = Proposal {
        id: id.clone(),
        file: file.to_string(),
        content: content.to_string(),
        reason: reason.to_string(),
        via: via.to_string(),
        created_at: now(),
        status: "pending".into(),
        ok: out.ok,
        errors: out.errors(),
        diagnostics: out.diagnostics.iter().map(|d| serde_json::to_value(d).unwrap_or(Json::Null)).collect(),
        kind: out.kind.clone(),
        name: out.name.clone(),
        resolved_at: String::new(),
        before,
        summary: summary.to_string(),
        files: Vec::new(),
        check: None,
    };
    let mut p = p;
    write_new(root, &mut p)?;
    Ok(p)
}

/// Queue an agent's report of a check. Always queued, whatever the edit mode:
/// an agent saying "I checked" is itself a model's output, so a person promotes it.
pub fn store_check(root: &Path, claim: &CheckClaim, reason: &str, via: &str) -> Result<Proposal, String> {
    let dir = proposals_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {}", dir.display(), e))?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) + 1;
    let mut p = Proposal {
        id: format!("p-{}-{}", stamp, n),
        file: String::new(),
        content: String::new(),
        reason: reason.to_string(),
        via: via.to_string(),
        created_at: now(),
        status: "pending".into(),
        ok: true,
        kind: "check".into(),
        name: claim.name.clone(),
        summary: format!("reports a check: {} {} against {}: {}", claim.kind, claim.name, claim.against, claim.note),
        check: Some(claim.clone()),
        ..Default::default()
    };
    write_new(root, &mut p)?;
    Ok(p)
}

fn write(root: &Path, p: &Proposal) -> Result<(), String> {
    let path = proposals_dir(root).join(format!("{}.json", p.id));
    let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {}", path.display(), e))
}

/// Create the proposal file without ever overwriting one: two processes that
/// compute the same id in the same second both keep their proposal, the
/// second under `<id>-2`, `<id>-3`, …
fn write_new(root: &Path, p: &mut Proposal) -> Result<(), String> {
    let dir = proposals_dir(root);
    let base = p.id.clone();
    for n in 1..=200u32 {
        let id = if n == 1 { base.clone() } else { format!("{}-{}", base, n) };
        let path = dir.join(format!("{}.json", id));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                p.id = id;
                let text = serde_json::to_string_pretty(&*p).map_err(|e| e.to_string())?;
                return std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| format!("writing {}: {}", path.display(), e));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("creating {}: {}", path.display(), e)),
        }
    }
    Err("could not find a free proposal id".into())
}

/// Every proposal, newest first.
pub fn list(root: &Path) -> Vec<Proposal> {
    let dir = proposals_dir(root);
    let mut out: Vec<Proposal> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Proposal>(&t).ok())
            .collect(),
        Err(_) => vec![],
    };
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    out
}

pub fn get(root: &Path, id: &str) -> Option<Proposal> {
    if id.contains('/') || id.contains("..") {
        return None;
    }
    let path = proposals_dir(root).join(format!("{}.json", id));
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok())
}

pub fn set_status(root: &Path, id: &str, status: &str) -> Result<Proposal, String> {
    let mut p = get(root, id).ok_or_else(|| format!("no proposal {}", id))?;
    p.status = status.to_string();
    p.resolved_at = now();
    write(root, &p)?;
    Ok(p)
}

/// Append one line to the activity log. Never fails loudly: the log is a convenience.
pub fn log(root: &Path, who: &str, text: &str, file: &str) {
    let dir = lorefile_dir(root);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let line = json!({ "at": now(), "who": who, "text": text, "file": file });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("activity.jsonl")) {
        let _ = writeln!(f, "{}", line);
    }
}

/// The most recent activity entries, newest first.
pub fn activity(root: &Path, limit: usize) -> Vec<Json> {
    let path = lorefile_dir(root).join("activity.jsonl");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut rows: Vec<Json> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    rows.reverse();
    rows.truncate(limit);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_id_gets_a_suffix_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!("lorefile-proposals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(proposals_dir(&root)).unwrap();
        let mut a = Proposal { id: "p-x-1".into(), file: "a.yaml".into(), content: "a".into(), created_at: now(), status: "pending".into(), ok: true, ..Default::default() };
        write_new(&root, &mut a).unwrap();
        let mut b = Proposal { id: "p-x-1".into(), file: "b.yaml".into(), content: "b".into(), created_at: now(), status: "pending".into(), ok: true, ..Default::default() };
        write_new(&root, &mut b).unwrap();
        assert_eq!(a.id, "p-x-1");
        assert_eq!(b.id, "p-x-1-2");
        assert_eq!(list(&root).len(), 2);
        assert_eq!(get(&root, "p-x-1").unwrap().file, "a.yaml", "the first proposal is intact");
        let _ = std::fs::remove_dir_all(&root);
    }
}
