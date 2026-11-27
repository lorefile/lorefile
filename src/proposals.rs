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
    /// pending | accepted | set_aside | replaced. Files written before 2026-11-27 say
    /// `dismissed`, which reads as set_aside (or replaced, when `replaced_by` is set).
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
    /// Made by `learn_file`: which entity, and the file's normalized column names, so the next
    /// file of the same shape is recognised while this waits. Names only, never values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learned: Option<Learned>,
    /// Set aside because a newer `learn_file` for the same file replaced it: that proposal's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    /// How to make this proposal again on today's files, without values: `propose_patch`'s
    /// ops, or `learn_file`'s input without its examples. None for `propose_change`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<Replay>,
    /// The person who accepted it ("Priya via page").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_by: Option<String>,
    /// The person who set it aside or looked again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    /// Why it was set aside, in the person's words (the agent sees this).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The person who took an Accept back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken_back_by: Option<String>,
    /// The person's fixes that rode along with Accept, as sentences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixes: Vec<String>,
    /// History off: the texts before and after Accept, so Take it back can put them back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
    /// History on: the commit Accept made, so Take it back is that commit's Undo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The errors the space had before Accept, so Take it back may bring them back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors_before: Vec<KnownError>,
}

/// How a proposal made from ops is made again when the files moved on since.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Replay {
    /// `propose_patch`: the ops as the agent sent them.
    Ops { ops: Vec<Json> },
    /// `learn_file`: the column names, their types, units and descriptions, the key and the
    /// answers. Never the examples: they are dropped before this is made.
    Learn { input: Json },
}

/// The files as they were before Accept and as Accept wrote them; `None` means the file did not exist.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub files: Vec<SnapFile>,
    #[serde(default)]
    pub trust_before: Option<String>,
    #[serde(default)]
    pub trust_after: Option<String>,
}

/// An error the space already had before Accept (a blank space's "no types found"): taking the
/// Accept back may bring exactly these back, and nothing else.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct KnownError {
    pub code: String,
    #[serde(default)]
    pub file: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SnapFile {
    pub file: String,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

impl Proposal {
    /// Every file of the proposal as (file, proposed text, text when proposed).
    pub fn parts(&self) -> Vec<(String, String, String)> {
        if self.files.is_empty() {
            if self.file.is_empty() {
                return vec![];
            }
            vec![(self.file.clone(), self.content.clone(), self.before.clone())]
        } else {
            self.files.iter().map(|f| (f.file.clone(), f.content.clone(), f.before.clone())).collect()
        }
    }

    /// The person's name from `by` words: "Priya via page" → "Priya", "person via page" → "a person".
    pub fn who(by: &str) -> String {
        crate::trust::who_said(by)
    }

    /// What happened to it, in words, for a refusal or a linked screen: "accepted by Priya 3 minutes ago".
    pub fn state_words(&self) -> String {
        let when = ago(&self.resolved_at);
        match self.status.as_str() {
            "accepted" => format!("accepted by {} {}", Self::who(self.accepted_by.as_deref().unwrap_or("")), when),
            "set_aside" => {
                let why = self.why.as_ref().filter(|w| !w.trim().is_empty()).map(|w| format!(": \"{}\"", w)).unwrap_or_default();
                format!("set aside by {} {}{}", Self::who(self.decided_by.as_deref().unwrap_or("")), when, why)
            }
            "replaced" => format!("replaced by a newer version from the same chat ({})", self.replaced_by.as_deref().unwrap_or("?")),
            "pending" => "waiting for review".to_string(),
            other => other.to_string(),
        }
    }

    /// Old files said `dismissed`; it means set aside, or replaced when a newer learn replaced it.
    fn normalize(mut self) -> Proposal {
        if self.status == "dismissed" {
            self.status = if self.replaced_by.is_some() { "replaced".into() } else { "set_aside".into() };
        }
        self
    }
}

/// "a moment ago", "5 minutes ago", "2 hours ago", "on 27 Nov": no time zone needed.
pub fn ago(at: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(at) else { return String::new() };
    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds();
    match secs {
        s if s < 90 => "a moment ago".into(),
        s if s < 3600 => format!("{} minutes ago", s / 60),
        s if s < 2 * 3600 => "an hour ago".into(),
        s if s < 24 * 3600 => format!("{} hours ago", s / 3600),
        _ => format!("on {}", t.format("%-d %b")),
    }
}

/// What a `learn_file` proposal is about.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Learned {
    pub entity: String,
    /// The file's column names, normalized (trimmed, lower case, `_`/space/`-` alike).
    pub headers: Vec<String>,
    #[serde(default)]
    pub file_name: String,
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
        ..Default::default()
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

pub fn save(root: &Path, p: &Proposal) -> Result<(), String> {
    write(root, p)
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
            .map(Proposal::normalize)
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
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Proposal>(&t).ok()).map(Proposal::normalize)
}

/// Record what a `learn_file` proposal is about, so `match_file` recognises the file while it waits.
pub fn set_learned(root: &Path, id: &str, learned: &Learned) -> Result<Proposal, String> {
    let mut p = get(root, id).ok_or_else(|| format!("no proposal {}", id))?;
    p.learned = Some(learned.clone());
    write(root, &p)?;
    Ok(p)
}

/// Set a waiting proposal aside because a newer one for the same file replaced it.
pub fn dismiss_replaced(root: &Path, id: &str, by: &str) -> Result<Proposal, String> {
    let mut p = get(root, id).ok_or_else(|| format!("no proposal {}", id))?;
    if p.status != "pending" {
        return Ok(p);
    }
    p.status = "replaced".into();
    p.resolved_at = now();
    p.replaced_by = Some(by.to_string());
    write(root, &p)?;
    Ok(p)
}

/// Not now: the proposal waits no more, with who and why. Only a waiting proposal.
pub fn set_aside(root: &Path, id: &str, by: &str, why: &str) -> Result<Proposal, String> {
    let mut p = get(root, id).ok_or_else(|| format!("there is no proposal {}", id))?;
    if p.status != "pending" {
        return Err(format!("{} is {}", p.id, p.state_words()));
    }
    p.status = "set_aside".into();
    p.resolved_at = now();
    p.decided_by = Some(by.to_string());
    let why: String = why.trim().chars().take(300).collect();
    p.why = if why.is_empty() { None } else { Some(why) };
    write(root, &p)?;
    Ok(p)
}

/// Look again: a set-aside proposal waits again. A replaced one never does: its replacement is the one to review.
pub fn reopen(root: &Path, id: &str, by: &str) -> Result<Proposal, String> {
    let mut p = get(root, id).ok_or_else(|| format!("there is no proposal {}", id))?;
    match p.status.as_str() {
        "set_aside" => {}
        "replaced" => return Err(format!("{} was replaced by a newer version from the same chat ({}); review that one instead", p.id, p.replaced_by.as_deref().unwrap_or("?"))),
        _ => return Err(format!("{} is {}, not set aside", p.id, p.state_words())),
    }
    p.status = "pending".into();
    p.resolved_at = String::new();
    p.decided_by = Some(by.to_string());
    p.why = None;
    write(root, &p)?;
    Ok(p)
}

/// Accept wrote it: who, the fixes that rode along, and how to take it back.
pub fn mark_accepted(root: &Path, id: &str, by: &str, fixes: Vec<String>, snapshot: Option<Snapshot>, commit: Option<String>, errors_before: Vec<KnownError>) -> Result<Proposal, String> {
    let mut p = get(root, id).ok_or_else(|| format!("there is no proposal {}", id))?;
    p.status = "accepted".into();
    p.resolved_at = now();
    p.accepted_by = Some(by.to_string());
    p.fixes = fixes;
    p.snapshot = snapshot;
    p.commit = commit;
    p.errors_before = errors_before;
    p.taken_back_by = None;
    write(root, &p)?;
    Ok(p)
}

/// Take it back: the files are as they were, and the proposal waits again.
pub fn mark_taken_back(root: &Path, id: &str, by: &str) -> Result<Proposal, String> {
    let mut p = get(root, id).ok_or_else(|| format!("there is no proposal {}", id))?;
    p.status = "pending".into();
    p.resolved_at = String::new();
    p.taken_back_by = Some(by.to_string());
    p.snapshot = None;
    p.commit = None;
    p.fixes = Vec::new();
    p.errors_before = Vec::new();
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
