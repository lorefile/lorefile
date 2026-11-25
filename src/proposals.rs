//! Changes proposed by agents over MCP that were not written straight away,
//! waiting for a person to accept them in the web page, plus a small activity
//! log. Both live under `<ontology>/.stella/`, which the loader ignores.

use crate::edit::Outcome;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

fn stella_dir(root: &Path) -> PathBuf {
    root.join(".stella")
}

fn proposals_dir(root: &Path) -> PathBuf {
    stella_dir(root).join("proposals")
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Record a proposal that was validated but not written.
pub fn store(root: &Path, file: &str, content: &str, reason: &str, via: &str, out: &Outcome) -> Result<Proposal, String> {
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
    };
    write(root, &p)?;
    Ok(p)
}

fn write(root: &Path, p: &Proposal) -> Result<(), String> {
    let path = proposals_dir(root).join(format!("{}.json", p.id));
    let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {}", path.display(), e))
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
    let dir = stella_dir(root);
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
    let path = stella_dir(root).join("activity.jsonl");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut rows: Vec<Json> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    rows.reverse();
    rows.truncate(limit);
    rows
}
