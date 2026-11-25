//! Compile-time checks. The compiler is the maintainer: every reference must
//! resolve, every rule must parse, every object should explain itself.

use crate::ontology::{snake, Kind, Ontology};
use crate::rules;
use crate::schema::Severity;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diag {
    pub level: Level,
    pub code: &'static str,
    pub message: String,
    pub file: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub diags: Vec<Diag>,
}

impl Report {
    fn err(&mut self, code: &'static str, file: &str, msg: impl Into<String>) {
        self.diags.push(Diag { level: Level::Error, code, message: msg.into(), file: file.to_string() });
    }
    fn warn(&mut self, code: &'static str, file: &str, msg: impl Into<String>) {
        self.diags.push(Diag { level: Level::Warning, code, message: msg.into(), file: file.to_string() });
    }
    pub fn errors(&self) -> Vec<&Diag> {
        self.diags.iter().filter(|d| d.level == Level::Error).collect()
    }
    pub fn warnings(&self) -> Vec<&Diag> {
        self.diags.iter().filter(|d| d.level == Level::Warning).collect()
    }
    pub fn ok(&self) -> bool {
        self.errors().is_empty()
    }
}

/// The local file a csv / parquet / duckdb binding reads, if the kind is file-based.
fn local_file(ont: &Ontology, src: &crate::schema::SourceDef, table: &str) -> Option<std::path::PathBuf> {
    let kind = src.kind.to_ascii_lowercase();
    match kind.as_str() {
        "csv" | "parquet" => {
            let folder = src.path.clone().unwrap_or_else(|| "data".into());
            let has_ext = std::path::Path::new(table).extension().is_some() || table.contains('*');
            let file = if has_ext { table.to_string() } else { format!("{}.{}", table, kind) };
            Some(ont.root.join(folder).join(file))
        }
        "duckdb" => src.path.as_ref().map(|p| ont.root.join(p)),
        _ => None,
    }
}

pub fn validate(ont: &Ontology) -> Report {
    let mut r = Report::default();

    for p in &ont.load_problems {
        r.err("E001", "", p.clone());
    }
    for (kind, name, where_) in &ont.duplicates {
        r.err("E002", "", format!("duplicate {:?} `{}` {}", kind, name, where_));
    }
    if ont.types.is_empty() {
        r.err("E000", "", "no types found; add at least one `types/*.yaml` file");
    }

    // manifest domains
    for d in &ont.manifest.domains {
        for t in &d.types {
            if !ont.types.contains_key(t) {
                r.err("E010", "ontology.yaml", format!("domain `{}` lists unknown type `{}`", d.name, t));
            }
        }
    }

    // types
    for t in ont.types.values() {
        if t.description.trim().is_empty() {
            r.warn("W101", &t.file, format!("type `{}` has no description (agents rely on it)", t.name));
        }
        for (pn, p) in &t.properties {
            let p = p.normalize();
            if p.kind == "enum" && p.values.is_empty() {
                r.warn("W105", &t.file, format!("`{}.{}` is an enum with no `values`", t.name, pn));
            }
        }
        if let Some(s) = &t.source {
            match ont.sources.get(&s.connection) {
                None => r.err("E008", &t.file, format!("type `{}` uses unknown connection `{}`", t.name, s.connection)),
                Some(src) => {
                    if t.key_property().is_none() {
                        r.warn("W103", &t.file, format!("type `{}` is bound to data but no property has `key: true`", t.name));
                    }
                    if src.kind.eq_ignore_ascii_case("mcp") {
                        if s.tool.trim().is_empty() {
                            r.err("E012", &t.file, format!("type `{}` binds to MCP source `{}` but names no `tool`", t.name, s.connection));
                        }
                    } else if s.table.trim().is_empty() {
                        r.err("E012", &t.file, format!("type `{}` binds to `{}` but names no `table`", t.name, s.connection));
                    }
                    if let Some(path) = local_file(ont, src, &s.table) {
                        if !path.exists() {
                            r.warn("W106", &t.file, format!("type `{}`: {} file not found at {}", t.name, src.kind, path.display()));
                        }
                    }
                }
            }
        }
    }

    // sources
    for s in ont.sources.values() {
        if s.kind.eq_ignore_ascii_case("mcp") && s.server.as_deref().map(str::trim).unwrap_or("").is_empty() {
            r.err("E011", &s.file, format!("source `{}` is `kind: mcp` but has no `server` (the MCP server name agents call)", s.connection));
        }
    }

    // relationships
    for rel in &ont.relationships {
        if !ont.types.contains_key(&rel.from) {
            r.err("E003", &rel.file, format!("relationship `{}` has unknown `from` type `{}`", rel.name, rel.from));
        }
        if !ont.types.contains_key(&rel.to) {
            r.err("E003", &rel.file, format!("relationship `{}.{}` points to unknown type `{}`", rel.from, rel.name, rel.to));
        }
        if let Some(s) = &rel.source {
            match ont.sources.get(&s.connection) {
                None => r.err("E008", &rel.file, format!("relationship `{}` uses unknown connection `{}`", rel.name, s.connection)),
                Some(src) => {
                    if let Some(path) = local_file(ont, src, &s.table) {
                        if !path.exists() {
                            r.warn("W106", &rel.file, format!("relationship `{}`: {} file not found at {}", rel.name, src.kind, path.display()));
                        }
                    }
                }
            }
        }
    }

    // constraints
    for c in ont.constraints.values() {
        let kind = ont.kind_of(&c.applies_to);
        if kind.is_none() {
            r.err("E004", &c.file, format!("rule {} applies_to `{}`, which is not a type, relationship, or action", c.id, c.applies_to));
        }
        if c.severity == Severity::Block && c.rationale.trim().is_empty() {
            r.warn("W102", &c.file, format!("blocking rule {} has no rationale", c.id));
        }
        match rules::compile(&c.rule) {
            Err(e) => r.err("E009", &c.file, format!("rule {} does not parse: {}", c.id, e)),
            Ok(_) => {
                // which names may the rule use?
                let allowed: Vec<String> = match kind {
                    Some(Kind::Type) => {
                        let t = ont.type_(&c.applies_to).unwrap();
                        let mut v: Vec<String> = t.properties.keys().cloned().collect();
                        v.push(snake(&t.name));
                        v.push("self".into());
                        v
                    }
                    Some(Kind::Relationship) => {
                        let mut v = vec!["from".to_string(), "to".to_string(), "edge".to_string()];
                        for rel in ont.relationship(&c.applies_to) {
                            v.push(snake(&rel.from));
                            v.push(snake(&rel.to));
                        }
                        v
                    }
                    Some(Kind::Action) => {
                        let a = ont.action(&c.applies_to).unwrap();
                        let mut v: Vec<String> = a.inputs.keys().cloned().collect();
                        v.push("action".into());
                        v
                    }
                    _ => vec![],
                };
                if !allowed.is_empty() {
                    for var in rules::variables(&c.rule) {
                        if !allowed.iter().any(|a| a == &var) {
                            r.warn(
                                "W104",
                                &c.file,
                                format!("rule {} references `{}`, which is not available on `{}` (available: {})", c.id, var, c.applies_to, allowed.join(", ")),
                            );
                        }
                    }
                }
            }
        }
    }

    // actions
    for a in ont.actions.values() {
        if a.description.trim().is_empty() {
            r.warn("W101", &a.file, format!("action `{}` has no description", a.name));
        }
        for (input, ty) in &a.inputs {
            if !ont.types.contains_key(ty) {
                r.err("E005", &a.file, format!("action `{}` input `{}` has unknown type `{}`", a.name, input, ty));
            }
        }
        for id in &a.checks {
            if ont.constraint(id).is_none() {
                r.err("E006", &a.file, format!("action `{}` checks unknown rule `{}`", a.name, id));
            }
        }
        for e in &a.effects {
            for (what, edge) in [("create", &e.create), ("remove", &e.remove)] {
                if let Some(edge) = edge {
                    let rels = ont.relationship(&edge.relationship);
                    if rels.is_empty() {
                        r.err("E007", &a.file, format!("action `{}` {}s unknown relationship `{}`", a.name, what, edge.relationship));
                    }
                    for side in [&edge.from, &edge.to] {
                        if !a.inputs.contains_key(side) {
                            r.err("E007", &a.file, format!("action `{}` effect refers to `{}`, which is not an input", a.name, side));
                        }
                    }
                    if let (Some(rel), Some(ft), Some(tt)) = (rels.first(), a.inputs.get(&edge.from), a.inputs.get(&edge.to)) {
                        if &rel.from != ft || &rel.to != tt {
                            r.warn("W107", &a.file, format!("action `{}`: `{}` goes {} → {}, but inputs `{}`/`{}` are {} → {}", a.name, edge.relationship, rel.from, rel.to, edge.from, edge.to, ft, tt));
                        }
                    }
                }
            }
        }
    }

    // glossary
    for g in &ont.glossary {
        if ont.kind_of(&g.means).is_none() {
            r.warn("W108", "glossary.yaml", format!("glossary term `{}` means `{}`, which does not exist", g.term, g.means));
        }
    }

    r
}
