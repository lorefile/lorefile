//! Compile-time checks. The compiler is the maintainer: every reference must
//! resolve, every rule must parse, every object should explain itself.

use crate::iri::{self, Key};
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

    // one name, one kind: `describe` and `validate` must agree on what a name is
    for m in ont.metrics.values() {
        if ont.types.contains_key(&m.name) || ont.actions.contains_key(&m.name) || ont.functions.contains_key(&m.name) {
            r.err("E015", &m.file, format!("`{}` is a metric and also an entity, action or function; one name, one kind", m.name));
        }
    }
    for f in ont.functions.values() {
        if ont.types.contains_key(&f.name) || ont.actions.contains_key(&f.name) {
            r.err("E015", &f.file, format!("`{}` is a function and also an entity or action; one name, one kind", f.name));
        }
    }

    // metrics
    for m in ont.metrics.values() {
        if m.description.trim().is_empty() {
            r.warn("W101", &m.file, format!("metric `{}` has no description", m.name));
        }
        if m.owner.trim().is_empty() {
            r.warn("W109", &m.file, format!("metric `{}` has no owner (who decides what it means)", m.name));
        }
        let sources: Vec<&crate::schema::MetricSource> = m.source.iter().chain(m.variants.iter().map(|v| &v.source)).collect();
        if sources.is_empty() && m.sql.is_none() {
            r.err("E013", &m.file, format!("metric `{}` says nothing about where it comes from: add source, variants or sql", m.name));
        }
        for s in sources {
            match ont.sources.get(&s.connection) {
                None => r.err("E008", &m.file, format!("metric `{}` uses unknown connection `{}`", m.name, s.connection)),
                Some(src) => {
                    if src.kind.eq_ignore_ascii_case("mcp") {
                        if s.tool.is_empty() {
                            r.err("E012", &m.file, format!("metric `{}` uses MCP source `{}` but names no `tool`", m.name, s.connection));
                        }
                    } else if s.table.is_empty() {
                        r.err("E012", &m.file, format!("metric `{}` uses `{}` but names no `table`", m.name, s.connection));
                    }
                }
            }
        }
    }

    // functions
    let primitive = |t: &str| matches!(t, "string" | "integer" | "number" | "boolean" | "date" | "datetime" | "json");
    for f in ont.functions.values() {
        if f.description.trim().is_empty() {
            r.warn("W101", &f.file, format!("function `{}` has no description", f.name));
        }
        for (n, t) in f.inputs.iter().chain(f.outputs.iter()) {
            if !primitive(t) && !ont.types.contains_key(t) {
                r.err("E005", &f.file, format!("function `{}`: `{}` has unknown type `{}` (an entity, or string/integer/number/boolean/date/json)", f.name, n, t));
            }
        }
        match &f.runs {
            None => r.warn("W111", &f.file, format!("function `{}` does not say where it runs (`runs: {{kind: mcp|http|manual}}`)", f.name)),
            Some(x) => match x.kind.as_str() {
                "mcp" => {
                    if x.server.is_empty() || x.tool.is_empty() {
                        r.err("E014", &f.file, format!("function `{}` runs on MCP but names no `server` and `tool`", f.name));
                    } else if !ont.sources.values().any(|s| s.kind.eq_ignore_ascii_case("mcp") && (s.connection == x.server || s.server.as_deref() == Some(x.server.as_str()))) {
                        r.err("E014", &f.file, format!("function `{}` runs on MCP server `{}`, which no `kind: mcp` source under sources/ declares", f.name, x.server));
                    }
                }
                "http" => {
                    if x.url.is_empty() {
                        r.err("E014", &f.file, format!("function `{}` runs over http but has no `url`", f.name));
                    }
                }
                "manual" => {}
                other => r.err("E014", &f.file, format!("function `{}`: runs.kind `{}` is not mcp, http or manual", f.name, other)),
            },
        }
        if let Some(w) = &f.writes_back {
            if !ont.types.contains_key(&w.entity) {
                r.err("E005", &f.file, format!("function `{}` writes back `{}`, which is not an entity", f.name, w.entity));
            }
            if !w.connection.is_empty() && !ont.sources.contains_key(&w.connection) {
                r.err("E008", &f.file, format!("function `{}` writes back to unknown connection `{}`", f.name, w.connection));
            }
        }
    }

    // glossary
    for g in &ont.glossary {
        if ont.kind_of(&g.means).is_none() {
            r.warn("W108", "glossary.yaml", format!("glossary term `{}` means `{}`, which does not exist", g.term, g.means));
        }
    }

    // trust.yaml: nothing at all when the space has none
    if let Some(book) = &ont.trust {
        let f = crate::trust::FILE;
        for k in crate::trust::Kind::ALL {
            for (name, rec) in crate::trust::entries(book, k) {
                let what = format!("{} {}", k.word(), name);
                if rec.of.trim().is_empty() {
                    r.err("E018", f, format!("{} has a grade with no fingerprint (`of`), so nobody can tell what was graded", what));
                }
                if rec.grade == crate::schema::Grade::Checked {
                    if !k.can_be_checked() {
                        r.err("E018", f, format!("{} is marked checked, but a word has no rows to check against; a word tops out at accepted", what));
                    }
                    let missing: Vec<&str> = [("against", rec.against.as_deref().unwrap_or("")), ("by", rec.by.as_str()), ("at", rec.at.as_str())].iter().filter(|(_, v)| v.trim().is_empty()).map(|(k, _)| *k).collect();
                    if !missing.is_empty() {
                        r.err("E018", f, format!("{} is marked checked without {}: a check says who, when and against which source", what, missing.join(", ")));
                    }
                }
                if let Some(a) = rec.against.as_deref().filter(|a| !a.trim().is_empty()) {
                    if !ont.sources.contains_key(a) {
                        r.err("E018", f, format!("{} names `{}` as what it was checked against, which is no source under sources/", what, a));
                    }
                }
            }
        }
        for (k, name) in &ont.graded.all {
            let st = ont.graded.get(*k, name);
            if let (true, Some(rec)) = (st.changed, &st.record) {
                r.warn("W113", f, format!("{} {} changed since it was {} ({}, {}); a person has to accept it again", k.word(), name, rec.grade.word(), crate::trust::who_said(&rec.by), rec.at));
            }
        }
        for (k, name, rec) in &ont.graded.orphans {
            r.warn("W114", f, format!("{} {} is {} in trust.yaml but no longer exists; the line can go", k.word(), name, rec.grade.word()));
        }
    }

    // IRIs: each resolves, and no two things share one
    for (file, msg) in &ont.iris.problems {
        r.err("E017", file, msg.clone());
    }
    let terms = ont.term_words();
    for c in iri::clashes(&ont.iris) {
        let what: Vec<String> = c.keys.iter().map(|k| k.describe(&terms)).collect();
        let what = what.join(" and ");
        if c.declared {
            let first = c.keys.iter().find(|k| ont.iris.declared.contains_key(*k)).unwrap_or(&c.keys[0]);
            r.err("E016", &file_of(ont, first), format!("IRI <{}> names {}; each thing needs its own IRI", c.iri, what));
        } else {
            r.warn("W112", &file_of(ont, &c.keys[0]), format!("{} get the same derived IRI <{}>; set `iri:` on one of them so other tools can tell them apart", what, c.iri));
        }
    }

    r
}

/// The file an IRI's owner was written in.
fn file_of(ont: &Ontology, key: &Key) -> String {
    match key {
        Key::Space | Key::Domain(_) => "ontology.yaml".into(),
        Key::Entity(t) | Key::Field(t, _) => ont.types.get(t).map(|t| t.file.clone()).unwrap_or_default(),
        Key::Link(from, name) => ont.relationships.iter().find(|r| &r.from == from && &r.name == name).map(|r| r.file.clone()).unwrap_or_default(),
        Key::Term(_) => "glossary.yaml".into(),
        Key::Guardrail(id) => ont.constraints.get(id).map(|c| c.file.clone()).unwrap_or_default(),
        Key::Action(n) => ont.actions.get(n).map(|a| a.file.clone()).unwrap_or_default(),
        Key::Metric(n) => ont.metrics.get(n).map(|m| m.file.clone()).unwrap_or_default(),
        Key::Function(n) => ont.functions.get(n).map(|f| f.file.clone()).unwrap_or_default(),
        Key::Source(n) => ont.sources.get(n).map(|s| s.file.clone()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{loader, ontology::Ontology};
    use std::path::PathBuf;

    fn space(files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lorefile-strict-{}-{}", std::process::id(), files.len()));
        let _ = std::fs::remove_dir_all(&d);
        for (rel, text) in files {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        d
    }

    #[test]
    fn a_misspelled_key_is_an_error_not_a_silent_downgrade() {
        let d = space(&[
            ("ontology.yaml", "name: strict\n"),
            ("types/order.yaml", "type: Order\nproperties: { order_id: { type: string, key: true }, status: string }\n"),
            ("constraints/o.yaml", "constraints:\n  - { id: O-1, name: x, applies_to: Order, rule: status == 'paid', severty: block, rationale: r }\n"),
        ]);
        let ont = Ontology::from_raw(loader::load(&d).unwrap());
        let report = super::validate(&ont);
        assert!(!report.ok(), "{:?}", report.diags);
        assert!(report.errors().iter().any(|e| e.message.contains("severty")), "the message names the key: {:?}", report.diags);
        assert!(ont.constraints.is_empty(), "the rule was not loaded with a guessed severity");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_from_user_binding_compiles_clean() {
        let d = space(&[
            ("ontology.yaml", "name: t\n"),
            ("glossary.yaml", "terms: []\n"),
            ("sources/user_files.yaml", "connection: user_files\nkind: from_user\ndescription: Files people hand to an agent.\n"),
            ("types/promo_sale.yaml", "type: PromoSale\ndescription: One order line.\nproperties: { line_id: { type: string, key: true, description: One line. }, revenue: { type: number, description: Paid. } }\nsource: { connection: user_files, table: promo_sales, key: line_id }\n"),
        ]);
        let report = super::validate(&Ontology::from_raw(loader::load(&d).unwrap()));
        assert!(report.diags.is_empty(), "no E012, no W106 for a file nobody keeps here: {:?}", report.diags);
        let _ = std::fs::remove_dir_all(&d);
    }
}
