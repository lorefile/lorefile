//! Walks an ontology folder and parses every YAML file into raw schema objects.
//! Files are classified by their top-level keys, not by folder name, so people
//! can organise folders however they like.

use crate::schema::*;
use anyhow::{Context, Result};
use serde_yaml::Value as Yaml;
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
    pub files: Vec<String>,
    /// Problems found while loading (bad YAML, unrecognised files).
    pub problems: Vec<String>,
}

pub fn load(root: &Path) -> Result<Raw> {
    let root = root
        .canonicalize()
        .with_context(|| format!("ontology folder not found: {}", root.display()))?;
    let mut raw = Raw {
        root: root.clone(),
        ..Default::default()
    };

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
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", rel))?;
        if text.trim().is_empty() {
            continue;
        }
        let value: Yaml = match serde_yaml::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                raw.problems.push(format!("{}: invalid YAML: {}", rel, e));
                continue;
            }
        };
        raw.files.push(rel.clone());
        if let Err(e) = classify(&mut raw, &rel, value) {
            raw.problems.push(format!("{}: {}", rel, e));
        }
    }
    Ok(raw)
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
        let r: RelationshipStandalone =
            serde_yaml::from_value(value).context("parsing relationship")?;
        raw.relationships.push(r);
    } else if has(&value, "relationships") {
        let f: RelationshipsFile =
            serde_yaml::from_value(value).context("parsing relationships list")?;
        raw.relationships.extend(f.relationships);
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
            "unrecognised file: expected one of type/types, relationship(s), constraints, action(s), connection/sources, terms, or ontology.yaml"
        );
    }
    Ok(())
}
