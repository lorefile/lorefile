//! Walks an ontology folder and parses every YAML file into raw schema objects.
//! Files are classified by their top-level keys, not by folder name, so people
//! can organise folders however they like.

use crate::schema::*;
use anyhow::{Context, Result};
use serde_yaml::Value as Yaml;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub struct Raw {
    pub root: PathBuf,
    pub manifest: Option<Manifest>,
    pub types: Vec<TypeDef>,
    pub relationships: Vec<RelationshipStandalone>,
    pub constraints: Vec<Constraint>,
    pub actions: Vec<ActionDef>,
    pub sources: Vec<SourceDef>,
    pub glossary: Vec<GlossaryTerm>,
    pub metrics: Vec<MetricDef>,
    pub functions: Vec<FunctionDef>,
    /// `trust.yaml`: who stands behind what. `None` when the space has no such file.
    pub trust: Option<TrustBook>,
    /// `people.yaml`: other spellings, teams, who left. `None` when the space has no such file.
    pub people: Option<PeopleFile>,
    pub files: Vec<String>,
    /// Problems found while loading (bad YAML, unrecognised files).
    pub problems: Vec<String>,
    /// `format:` in ontology.yaml, read before anything else is parsed.
    pub format: Option<u32>,
    /// The space needs a newer Lorefile: its format is above `schema::FORMAT`. The
    /// unknown-field problems such a space gives are folded into this one sentence.
    pub newer_format: Option<u32>,
}

/// Changes laid over the folder without touching the disk: relative path →
/// `Some(text)` for a new or replaced file, `None` for a deleted one. The edit
/// flow checks a proposed change by loading the folder with it applied.
pub type Overrides = HashMap<String, Option<String>>;

pub fn load(root: &Path) -> Result<Raw> {
    load_with(root, &Overrides::new())
}

/// Load the folder as it would look with `overrides` applied.
pub fn load_with(root: &Path, overrides: &Overrides) -> Result<Raw> {
    let root = root
        .canonicalize()
        .with_context(|| format!("ontology folder not found: {}", root.display()))?;
    let mut raw = Raw {
        root: root.clone(),
        ..Default::default()
    };
    let mut seen: HashSet<String> = HashSet::new();

    for entry in WalkDir::new(&root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(name == "data" || name.starts_with('.') || name == "target")
        })
    {
        let entry = entry?;
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !entry.file_type().is_file() || !(ext == "yaml" || ext == "yml") {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        seen.insert(rel.clone());
        let text = match overrides.get(&rel) {
            // deleted in this proposal
            Some(None) => continue,
            Some(Some(text)) => text.clone(),
            None => std::fs::read_to_string(path).with_context(|| format!("reading {}", rel))?,
        };
        ingest(&mut raw, &rel, &text);
    }

    // files the proposal adds
    let mut added: Vec<(&String, &String)> = overrides
        .iter()
        .filter_map(|(rel, text)| text.as_ref().map(|t| (rel, t)))
        .filter(|(rel, _)| !seen.contains(*rel))
        .collect();
    added.sort();
    for (rel, text) in added {
        ingest(&mut raw, rel, text);
    }
    if let Some(f) = raw.format.filter(|f| *f > FORMAT) {
        raw.newer_format = Some(f);
        raw.problems.retain(|p| !p.contains("unknown field") && !p.contains("unknown variant"));
    }
    Ok(raw)
}

/// Parse one file's text and file it under the right list.
fn ingest(raw: &mut Raw, rel: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    raw.files.push(rel.to_string());
    let value: Yaml = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            raw.problems.push(format!("{}: invalid YAML: {}", rel, e));
            return;
        }
    };
    if rel == "ontology.yaml" {
        raw.format = value.get("format").and_then(|f| f.as_u64()).map(|f| f as u32);
    }
    if let Err(e) = classify(raw, rel, value) {
        // keep the cause: "parsing constraints: unknown field `severty`, expected one of …"
        raw.problems.push(format!("{}: {:#}", rel, e));
    }
}

fn has(value: &Yaml, key: &str) -> bool {
    value.get(key).is_some()
}

fn classify(raw: &mut Raw, rel: &str, value: Yaml) -> Result<()> {
    let file_name = Path::new(rel)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    if has(&value, "type") {
        let mut t: TypeDef = serde_yaml::from_value(value).context("parsing type")?;
        t.file = rel.to_string();
        raw.types.push(t);
    } else if has(&value, "types") {
        let f: TypesFile = serde_yaml::from_value(value).context("parsing types list")?;
        for mut t in f.types {
            t.file = rel.to_string();
            raw.types.push(t);
        }
    } else if has(&value, "relationship") {
        let mut r: RelationshipStandalone =
            serde_yaml::from_value(value).context("parsing relationship")?;
        r.file = rel.to_string();
        raw.relationships.push(r);
    } else if has(&value, "relationships") {
        let f: RelationshipsFile =
            serde_yaml::from_value(value).context("parsing relationships list")?;
        for mut r in f.relationships {
            r.file = rel.to_string();
            raw.relationships.push(r);
        }
    } else if has(&value, "constraints") {
        let f: ConstraintsFile =
            serde_yaml::from_value(value).context("parsing constraints")?;
        for mut c in f.constraints {
            c.file = rel.to_string();
            raw.constraints.push(c);
        }
    } else if has(&value, "action") {
        let mut a: ActionDef = serde_yaml::from_value(value).context("parsing action")?;
        a.file = rel.to_string();
        raw.actions.push(a);
    } else if has(&value, "actions") {
        let f: ActionsFile = serde_yaml::from_value(value).context("parsing actions list")?;
        for mut a in f.actions {
            a.file = rel.to_string();
            raw.actions.push(a);
        }
    } else if has(&value, "connection") {
        let mut s: SourceDef = serde_yaml::from_value(value).context("parsing source")?;
        s.file = rel.to_string();
        raw.sources.push(s);
    } else if has(&value, "sources") {
        let f: SourcesFile = serde_yaml::from_value(value).context("parsing sources list")?;
        for mut s in f.sources {
            s.file = rel.to_string();
            raw.sources.push(s);
        }
    } else if has(&value, "metric") {
        let mut m: MetricDef = serde_yaml::from_value(value).context("parsing metric")?;
        m.file = rel.to_string();
        raw.metrics.push(m);
    } else if has(&value, "metrics") {
        let f: MetricsFile = serde_yaml::from_value(value).context("parsing metrics list")?;
        for mut m in f.metrics {
            m.file = rel.to_string();
            raw.metrics.push(m);
        }
    } else if has(&value, "function") {
        let mut f: FunctionDef = serde_yaml::from_value(value).context("parsing function")?;
        f.file = rel.to_string();
        raw.functions.push(f);
    } else if has(&value, "functions") {
        let f: FunctionsFile = serde_yaml::from_value(value).context("parsing functions list")?;
        for mut x in f.functions {
            x.file = rel.to_string();
            raw.functions.push(x);
        }
    } else if has(&value, "trust") {
        if rel != crate::trust::FILE {
            anyhow::bail!("a `trust:` file is written by Lorefile as trust.yaml at the root of the space, not here (and not under another name: rename a trust.yml to trust.yaml)");
        }
        let t: TrustFile = serde_yaml::from_value(value).context("parsing trust.yaml")?;
        if raw.trust.is_some() {
            anyhow::bail!("second trust file found; only one trust.yaml is allowed");
        }
        raw.trust = Some(t.trust);
    } else if has(&value, "people") {
        if rel != crate::people::FILE {
            anyhow::bail!("a `people:` file is written by Lorefile as people.yaml at the root of the space, not here (and not under another name)");
        }
        let f: PeopleFile = serde_yaml::from_value(value).context("parsing people.yaml")?;
        raw.people = Some(f);
    } else if has(&value, "terms") {
        let g: Glossary = serde_yaml::from_value(value).context("parsing glossary")?;
        raw.glossary.extend(g.terms);
    } else if file_name == "ontology.yaml" || file_name == "ontology.yml" || has(&value, "domains") {
        let m: Manifest = serde_yaml::from_value(value).context("parsing manifest")?;
        if raw.manifest.is_some() {
            anyhow::bail!("second manifest found; only one ontology.yaml is allowed");
        }
        raw.manifest = Some(m);
    } else {
        anyhow::bail!(
            "unrecognised file: expected one of type/types, relationship(s), constraints, action(s), connection/sources, metric(s), function(s), terms, trust, people, or ontology.yaml"
        );
    }
    Ok(())
}
