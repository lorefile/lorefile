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
    /// The one item the diagnostic is about, when it names one: what the home page's lights,
    /// the card and the list attach it to. A field's subject is its entity, a link's its `from`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Subject>,
    /// For "said twice" (W115–W117): the two sides, as `trust.yaml`'s `distinct:` names them
    /// (`metric:x`, `entity:X`, `term:word=Entity`), so a person can say they differ on purpose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<[String; 2]>,
}

/// An item of the space: kind is entity, link, metric, function, guardrail, action, source or term.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Subject {
    pub kind: &'static str,
    pub name: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub diags: Vec<Diag>,
}

impl Report {
    fn err(&mut self, code: &'static str, file: &str, msg: impl Into<String>) {
        self.diags.push(Diag { level: Level::Error, code, message: msg.into(), file: file.to_string(), subject: None, pair: None });
    }
    fn warn(&mut self, code: &'static str, file: &str, msg: impl Into<String>) {
        self.diags.push(Diag { level: Level::Warning, code, message: msg.into(), file: file.to_string(), subject: None, pair: None });
    }
    fn err_on(&mut self, code: &'static str, file: &str, on: (&'static str, &str), msg: impl Into<String>) {
        self.diags.push(Diag { level: Level::Error, code, message: msg.into(), file: file.to_string(), subject: Some(Subject { kind: on.0, name: on.1.to_string() }), pair: None });
    }
    fn warn_on(&mut self, code: &'static str, file: &str, on: (&'static str, &str), msg: impl Into<String>) {
        self.diags.push(Diag { level: Level::Warning, code, message: msg.into(), file: file.to_string(), subject: Some(Subject { kind: on.0, name: on.1.to_string() }), pair: None });
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

    if let Some(f) = ont.newer_format {
        r.err("E019", "ontology.yaml", format!("This space needs a newer Lorefile (format {}); this is Lorefile {}, which reads format {}. Upgrade with `npx lorefile@latest` or `cargo install lorefile`.", f, env!("CARGO_PKG_VERSION"), crate::schema::FORMAT));
    }
    for p in &ont.load_problems {
        r.err("E001", "", p.clone());
    }
    for k in &ont.kind_problems {
        r.err_on(k.code, &k.file, ("entity", &k.entity), k.message.clone());
    }
    if ont.uses_format_2() && ont.format < 2 && ont.newer_format.is_none() {
        r.warn("W118", "ontology.yaml", "this space uses extends, where or distinct: but ontology.yaml doesn't say format: 2; Lorefile 0.1.0 will not load it, and the next save that touches a kind writes it");
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
            r.warn_on("W101", &t.file, ("entity", &t.name), format!("type `{}` has no description (agents rely on it)", t.name));
        }
        // a field a kind gets from its parent is the parent's to fix
        let from_parent = |f: &str| t.parent.as_ref().and_then(|p| ont.types.get(p)).map(|p| p.properties.contains_key(f)).unwrap_or(false);
        for (pn, p) in t.properties.iter().filter(|(n, _)| !from_parent(n)) {
            let p = p.normalize();
            if p.kind == "enum" && p.values.is_empty() {
                r.warn_on("W105", &t.file, ("entity", &t.name), format!("`{}.{}` is an enum with no `values`", t.name, pn));
            }
        }
        if let Some(s) = &t.source {
            match ont.sources.get(&s.connection) {
                None => r.err_on("E008", &t.file, ("entity", &t.name), format!("type `{}` uses unknown connection `{}`", t.name, s.connection)),
                Some(src) => {
                    if t.key_property().is_none() {
                        r.warn_on("W103", &t.file, ("entity", &t.name), format!("type `{}` is bound to data but no property has `key: true`", t.name));
                    }
                    if src.kind.eq_ignore_ascii_case("mcp") {
                        if s.tool.trim().is_empty() {
                            r.err_on("E012", &t.file, ("entity", &t.name), format!("type `{}` binds to MCP source `{}` but names no `tool`", t.name, s.connection));
                        }
                    } else if s.table.trim().is_empty() {
                        r.err_on("E012", &t.file, ("entity", &t.name), format!("type `{}` binds to `{}` but names no `table`", t.name, s.connection));
                    }
                    if let Some(path) = local_file(ont, src, &s.table) {
                        if !path.exists() {
                            r.warn_on("W106", &t.file, ("entity", &t.name), format!("type `{}`: {} file not found at {}", t.name, src.kind, path.display()));
                        }
                    }
                }
            }
        }
    }

    // sources
    for s in ont.sources.values() {
        if s.kind.eq_ignore_ascii_case("mcp") && s.server.as_deref().map(str::trim).unwrap_or("").is_empty() {
            r.err_on("E011", &s.file, ("source", &s.connection), format!("source `{}` is `kind: mcp` but has no `server` (the MCP server name agents call)", s.connection));
        }
    }

    // relationships
    for rel in &ont.relationships {
        if !ont.types.contains_key(&rel.from) {
            r.err_on("E003", &rel.file, ("entity", &rel.from), format!("relationship `{}` has unknown `from` type `{}`", rel.name, rel.from));
        }
        if !ont.types.contains_key(&rel.to) {
            r.err_on("E003", &rel.file, ("entity", &rel.from), format!("relationship `{}.{}` points to unknown type `{}`", rel.from, rel.name, rel.to));
        }
        if let Some(s) = &rel.source {
            match ont.sources.get(&s.connection) {
                None => r.err_on("E008", &rel.file, ("entity", &rel.from), format!("relationship `{}` uses unknown connection `{}`", rel.name, s.connection)),
                Some(src) => {
                    if let Some(path) = local_file(ont, src, &s.table) {
                        if !path.exists() {
                            r.warn_on("W106", &rel.file, ("entity", &rel.from), format!("relationship `{}`: {} file not found at {}", rel.name, src.kind, path.display()));
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
            r.err_on("E004", &c.file, ("guardrail", &c.id), format!("rule {} applies_to `{}`, which is not a type, relationship, or action", c.id, c.applies_to));
        }
        if c.severity == Severity::Block && c.rationale.trim().is_empty() {
            r.warn_on("W102", &c.file, ("guardrail", &c.id), format!("blocking rule {} has no rationale", c.id));
        }
        match rules::compile(&c.rule) {
            Err(e) => r.err_on("E009", &c.file, ("guardrail", &c.id), format!("rule {} does not parse: {}", c.id, e)),
            Ok(_) => {
                // which names may the rule use?
                let allowed: Vec<String> = match kind {
                    Some(Kind::Type) => {
                        let t = ont.type_(&c.applies_to).unwrap();
                        let mut v: Vec<String> = t.properties.keys().cloned().collect();
                        // a rule on a kind may name its parents too (`sale.` as well as `promo_sale.`)
                        for n in ont.self_and_ancestors(&t.name) {
                            v.push(snake(&n));
                        }
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
                            r.warn_on(
                                "W104",
                                &c.file,
                                ("guardrail", &c.id),
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
            r.warn_on("W101", &a.file, ("action", &a.name), format!("action `{}` has no description", a.name));
        }
        for (input, ty) in &a.inputs {
            if !ont.types.contains_key(ty) {
                r.err_on("E005", &a.file, ("action", &a.name), format!("action `{}` input `{}` has unknown type `{}`", a.name, input, ty));
            }
        }
        for id in &a.checks {
            if ont.constraint(id).is_none() {
                r.err_on("E006", &a.file, ("action", &a.name), format!("action `{}` checks unknown rule `{}`", a.name, id));
            }
        }
        for e in &a.effects {
            for (what, edge) in [("create", &e.create), ("remove", &e.remove)] {
                if let Some(edge) = edge {
                    let rels = ont.relationship(&edge.relationship);
                    if rels.is_empty() {
                        r.err_on("E007", &a.file, ("action", &a.name), format!("action `{}` {}s unknown relationship `{}`", a.name, what, edge.relationship));
                    }
                    for side in [&edge.from, &edge.to] {
                        if !a.inputs.contains_key(side) {
                            r.err_on("E007", &a.file, ("action", &a.name), format!("action `{}` effect refers to `{}`, which is not an input", a.name, side));
                        }
                    }
                    if let (Some(rel), Some(ft), Some(tt)) = (rels.first(), a.inputs.get(&edge.from), a.inputs.get(&edge.to)) {
                        // an input that is the link's end, or a kind of it, fits (producer extends)
                        let fits = |input: &str, end: &str| ont.is_a(input, end);
                        if !fits(ft, &rel.from) || !fits(tt, &rel.to) {
                            let broader = |input: &str, end: &str| ont.is_a(end, input) && input != end;
                            let msg = if broader(ft, &rel.from) {
                                format!("action `{}`: `{}` is a {}, but `{}` starts at {}; pass a {}, or start the link at {}", a.name, edge.from, ft, edge.relationship, rel.from, rel.from, ft)
                            } else if broader(tt, &rel.to) {
                                format!("action `{}`: `{}` is a {}, but `{}` ends at {}; pass a {}, or end the link at {}", a.name, edge.to, tt, edge.relationship, rel.to, rel.to, tt)
                            } else {
                                format!("action `{}`: `{}` goes {} → {}, but inputs `{}`/`{}` are {} → {}", a.name, edge.relationship, rel.from, rel.to, edge.from, edge.to, ft, tt)
                            };
                            r.warn_on("W107", &a.file, ("action", &a.name), msg);
                        }
                    }
                }
            }
        }
    }

    // one name, one kind: `describe` and `validate` must agree on what a name is
    for m in ont.metrics.values() {
        if ont.types.contains_key(&m.name) || ont.actions.contains_key(&m.name) || ont.functions.contains_key(&m.name) {
            r.err_on("E015", &m.file, ("metric", &m.name), format!("`{}` is a metric and also an entity, action or function; one name, one kind", m.name));
        }
    }
    for f in ont.functions.values() {
        if ont.types.contains_key(&f.name) || ont.actions.contains_key(&f.name) {
            r.err_on("E015", &f.file, ("function", &f.name), format!("`{}` is a function and also an entity or action; one name, one kind", f.name));
        }
    }

    // metrics
    for m in ont.metrics.values() {
        if m.description.trim().is_empty() {
            r.warn_on("W101", &m.file, ("metric", &m.name), format!("metric `{}` has no description", m.name));
        }
        if m.owner.trim().is_empty() {
            r.warn_on("W109", &m.file, ("metric", &m.name), format!("metric `{}` has no owner (who decides what it means)", m.name));
        }
        let sources: Vec<&crate::schema::MetricSource> = m.source.iter().chain(m.variants.iter().map(|v| &v.source)).collect();
        if sources.is_empty() && m.sql.is_none() {
            r.err_on("E013", &m.file, ("metric", &m.name), format!("metric `{}` says nothing about where it comes from: add source, variants or sql", m.name));
        }
        for s in sources {
            match ont.sources.get(&s.connection) {
                None => r.err_on("E008", &m.file, ("metric", &m.name), format!("metric `{}` uses unknown connection `{}`", m.name, s.connection)),
                Some(src) => {
                    if src.kind.eq_ignore_ascii_case("mcp") {
                        if s.tool.is_empty() {
                            r.err_on("E012", &m.file, ("metric", &m.name), format!("metric `{}` uses MCP source `{}` but names no `tool`", m.name, s.connection));
                        }
                    } else if s.table.is_empty() {
                        r.err_on("E012", &m.file, ("metric", &m.name), format!("metric `{}` uses `{}` but names no `table`", m.name, s.connection));
                    }
                }
            }
        }
    }

    // functions
    let primitive = |t: &str| matches!(t, "string" | "integer" | "number" | "boolean" | "date" | "datetime" | "json");
    for f in ont.functions.values() {
        if f.description.trim().is_empty() {
            r.warn_on("W101", &f.file, ("function", &f.name), format!("function `{}` has no description", f.name));
        }
        for (n, t) in f.inputs.iter().chain(f.outputs.iter()) {
            if !primitive(t) && !ont.types.contains_key(t) {
                r.err_on("E005", &f.file, ("function", &f.name), format!("function `{}`: `{}` has unknown type `{}` (an entity, or string/integer/number/boolean/date/json)", f.name, n, t));
            }
        }
        match &f.runs {
            None => r.warn_on("W111", &f.file, ("function", &f.name), format!("function `{}` does not say where it runs (`runs: {{kind: mcp|http|manual}}`)", f.name)),
            Some(x) => match x.kind.as_str() {
                "mcp" => {
                    if x.server.is_empty() || x.tool.is_empty() {
                        r.err_on("E014", &f.file, ("function", &f.name), format!("function `{}` runs on MCP but names no `server` and `tool`", f.name));
                    } else if !ont.sources.values().any(|s| s.kind.eq_ignore_ascii_case("mcp") && (s.connection == x.server || s.server.as_deref() == Some(x.server.as_str()))) {
                        r.err_on("E014", &f.file, ("function", &f.name), format!("function `{}` runs on MCP server `{}`, which no `kind: mcp` source under sources/ declares", f.name, x.server));
                    }
                }
                "http" => {
                    if x.url.is_empty() {
                        r.err_on("E014", &f.file, ("function", &f.name), format!("function `{}` runs over http but has no `url`", f.name));
                    }
                }
                "manual" => {}
                other => r.err_on("E014", &f.file, ("function", &f.name), format!("function `{}`: runs.kind `{}` is not mcp, http or manual", f.name, other)),
            },
        }
        if let Some(w) = &f.writes_back {
            if !ont.types.contains_key(&w.entity) {
                r.err_on("E005", &f.file, ("function", &f.name), format!("function `{}` writes back `{}`, which is not an entity", f.name, w.entity));
            }
            if !w.connection.is_empty() && !ont.sources.contains_key(&w.connection) {
                r.err_on("E008", &f.file, ("function", &f.name), format!("function `{}` writes back to unknown connection `{}`", f.name, w.connection));
            }
        }
    }

    // glossary
    for g in &ont.glossary {
        if ont.kind_of(&g.means).is_none() {
            r.warn_on("W108", "glossary.yaml", ("term", &g.term), format!("glossary term `{}` means `{}`, which does not exist", g.term, g.means));
        }
    }

    // trust.yaml: nothing at all when the space has none
    if let Some(book) = &ont.trust {
        let f = crate::trust::FILE;
        for k in crate::trust::Kind::ALL {
            for (name, rec) in crate::trust::entries(book, k) {
                let what = k.name_words(name);
                if rec.of.trim().is_empty() {
                    r.err_on("E018", f, subject_of(k, name), format!("{} has a grade with no fingerprint (`of`), so nobody can tell what was graded", what));
                }
                if rec.grade == crate::schema::Grade::Checked {
                    if !k.can_be_checked() {
                        r.err_on("E018", f, subject_of(k, name), format!("{} is marked checked, but a word has no rows to check against; a word tops out at accepted", what));
                    }
                    let missing: Vec<&str> = [("against", rec.against.as_deref().unwrap_or("")), ("by", rec.by.as_str()), ("at", rec.at.as_str())].iter().filter(|(_, v)| v.trim().is_empty()).map(|(k, _)| *k).collect();
                    if !missing.is_empty() {
                        r.err_on("E018", f, subject_of(k, name), format!("{} is marked checked without {}: a check says who, when and against which source", what, missing.join(", ")));
                    }
                }
                if let Some(a) = rec.against.as_deref().filter(|a| !a.trim().is_empty()) {
                    if !ont.sources.contains_key(a) {
                        r.err_on("E018", f, subject_of(k, name), format!("{} names `{}` as what it was checked against, which is no source under sources/", what, a));
                    }
                }
            }
        }
        for (k, name) in &ont.graded.all {
            let st = ont.graded.get(*k, name);
            if let (true, Some(rec)) = (st.changed, &st.record) {
                // a kind that picks its parent's rows was graded together with that parent's source
                let via = (*k == crate::trust::Kind::Binding).then(|| ont.types.get(name.as_str())).flatten().filter(|t| t.is_where_kind()).and_then(|t| t.rows_of.clone());
                let why = via.map(|b| format!(", perhaps because {}'s source changed (its rows are {}'s)", b, b)).unwrap_or_default();
                r.warn_on("W113", f, subject_of(*k, name), format!("{} changed since it was {} ({}, {}){}; a person has to accept it again", k.name_words(name), rec.grade.word(), crate::trust::who_said(&rec.by), rec.at, why));
            }
        }
        for (k, name, rec) in &ont.graded.orphans {
            r.warn_on("W114", f, subject_of(*k, name), format!("{} is {} in trust.yaml but no longer exists; the line can go", k.name_words(name), rec.grade.word()));
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
            match subject_of_key(ont, first) {
                Some(on) => r.err_on("E016", &file_of(ont, first), on, format!("IRI <{}> names {}; each thing needs its own IRI", c.iri, what)),
                None => r.err("E016", &file_of(ont, first), format!("IRI <{}> names {}; each thing needs its own IRI", c.iri, what)),
            }
        } else {
            r.warn("W112", &file_of(ont, &c.keys[0]), format!("{} get the same derived IRI <{}>; set `iri:` on one of them so other tools can tell them apart", what, c.iri));
        }
    }

    said_twice(ont, &mut r);

    // what only names a types/*.yaml file is about its entity, when the file defines exactly one
    // (a `types:` list file holds several, and then it is about none of them in particular)
    for d in r.diags.iter_mut().filter(|d| d.subject.is_none() && d.file.starts_with("types/")) {
        let mut in_file = ont.types.values().filter(|t| t.file == d.file);
        if let (Some(t), None) = (in_file.next(), in_file.next()) {
            d.subject = Some(Subject { kind: "entity", name: t.name.clone() });
        }
    }
    r
}

// ---------------------------------------------------------------------------
// Things said twice (W115–W117), quiet where a person said they differ on purpose
// ---------------------------------------------------------------------------

/// A filter's AND-ed terms, each said one way: case, spaces and quotes ignored, `==` as `=`,
/// `NOT x` and `x != true` as `x=false`. Sorted, so their order does not matter.
fn filter_terms(f: &str) -> Vec<String> {
    let lower = f.to_lowercase().replace(['\'', '"', '(', ')'], "");
    let parts: Vec<String> = regex::Regex::new(r"\s+and\s+").map(|r| r.split(&lower).map(|x| x.to_string()).collect()).unwrap_or_default();
    let mut out: Vec<String> = parts
        .into_iter()
        .map(|t| {
            let t = t.trim().to_string();
            let t = if let Some(x) = t.strip_prefix("not ") { format!("{}=false", x.trim()) } else { t };
            let t = t.replace("==", "=").replace(" is ", "=").replace("!=true", "=false").replace("!= true", "=false");
            t.chars().filter(|c| !c.is_whitespace()).collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect();
    out.sort();
    out
}

/// One recipe said one way: the rows (connection and table, or tool and params), the measure and the filter.
fn recipe_key(s: &crate::schema::MetricSource) -> String {
    let rows = if s.tool.is_empty() { format!("{}.{}", s.connection, s.table) } else { format!("{} tool {} {}", s.connection, s.tool, serde_json::to_string(&s.params).unwrap_or_default()) };
    format!("{}|{}|{}", rows, s.measure.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect::<String>(), filter_terms(&s.filter).join("&"))
}

/// What a person said about a pair: quiet while both sides are as they were; a sentence when either changed.
fn said_distinct(ont: &Ontology, a: &str, b: &str) -> Option<Result<(), String>> {
    let d = crate::trust::distinct_of(ont.trust.as_ref()?, a, b)?;
    let (fa, fb) = (crate::trust::pair_fingerprint(ont, &d.a), crate::trust::pair_fingerprint(ont, &d.b));
    if fa.as_deref() == Some(d.of_a.as_str()) && fb.as_deref() == Some(d.of_b.as_str()) {
        Some(Ok(()))
    } else {
        Some(Err(format!(" (changed since {} said these were different, {})", crate::trust::who_said(&d.by), crate::trust::short_date(&d.at))))
    }
}

fn twice(r: &mut Report, ont: &Ontology, code: &'static str, file: &str, on: (&'static str, &str), pair: [String; 2], msg: String) {
    let msg = match said_distinct(ont, &pair[0], &pair[1]) {
        Some(Ok(())) => return,
        Some(Err(since)) => format!("{}{}", msg, since),
        None => msg,
    };
    r.warn_on(code, file, on, msg);
    if let Some(d) = r.diags.last_mut() {
        d.pair = Some(pair);
    }
}

fn said_twice(ont: &Ontology, r: &mut Report) {
    // W115: two metrics with the same recipe
    let ms: Vec<&crate::schema::MetricDef> = ont.metrics.values().collect();
    let key_of = |m: &crate::schema::MetricDef| -> Option<String> {
        let mut keys: Vec<String> = m.source.iter().chain(m.variants.iter().map(|v| &v.source)).map(recipe_key).collect();
        if keys.is_empty() {
            return None;
        }
        keys.sort();
        Some(keys.join(" ; "))
    };
    for (i, a) in ms.iter().enumerate() {
        let Some(ka) = key_of(a) else { continue };
        for b in ms.iter().skip(i + 1) {
            if key_of(b).as_deref() == Some(ka.as_str()) {
                let how = a.source.as_ref().map(|s| if s.measure.to_lowercase().starts_with("count") { "count" } else if s.measure.to_lowercase().starts_with("avg") { "average" } else { "sum" }).unwrap_or("recipe");
                twice(r, ont, "W115", &b.file, ("metric", &b.name), [format!("metric:{}", b.name), format!("metric:{}", a.name)],
                    format!("{} looks the same as {}: same rows, same filter, same {}. Agents would have two names for one number.", label(&b.name), label(&a.name), how));
            }
        }
    }
    // W116: two entities that read the same rows, share most fields, or pick overlapping rows
    let ts: Vec<&crate::schema::TypeDef> = ont.types.values().collect();
    let related = |x: &str, y: &str| ont.is_a(x, y) || ont.is_a(y, x);
    for (i, a) in ts.iter().enumerate() {
        for b in ts.iter().skip(i + 1) {
            if related(&a.name, &b.name) {
                continue;
            }
            let pair = [format!("entity:{}", b.name), format!("entity:{}", a.name)];
            // (a) the same rows
            if let (Some(sa), Some(sb)) = (&a.source, &b.source) {
                let same = sa.connection == sb.connection && ((!sa.table.is_empty() && sa.table == sb.table) || (sa.table.is_empty() && !sa.tool.is_empty() && sa.tool == sb.tool && sa.params == sb.params));
                if same {
                    twice(r, ont, "W116", &b.file, ("entity", &b.name), pair, format!("{} and {} read the same rows ({}.{}): probably one entity.", b.name, a.name, sa.connection, if sa.table.is_empty() { &sa.tool } else { &sa.table }));
                    continue;
                }
            }
            // (c) sibling kinds whose `where`s can match the same row
            if a.is_where_kind() && b.is_where_kind() && a.rows_of == b.rows_of {
                let overlap = a.where_all.iter().all(|(f, va)| b.where_all.get(f).map(|vb| va.iter().any(|x| vb.contains(x))).unwrap_or(true));
                if overlap {
                    let base = a.rows_of.clone().unwrap_or_default();
                    let w = if a.where_all == b.where_all { crate::ontology::Ontology::where_words(&a.where_all) } else { format!("{} or {}", crate::ontology::Ontology::where_words(&a.where_all), crate::ontology::Ontology::where_words(&b.where_all)) };
                    twice(r, ont, "W116", &b.file, ("entity", &b.name), pair, format!("{} and {} both pick {}'s rows where {}: a row can be both.", b.name, a.name, base, w));
                    continue;
                }
            }
            // (b) most of their own fields
            let own = |t: &crate::schema::TypeDef| -> Vec<String> { t.own_properties().map(|(n, _)| crate::learn::normalize_header(n)).collect() };
            let (fa, fb) = (own(a), own(b));
            if fa.len() >= 4 && fb.len() >= 4 {
                let shared = fa.iter().filter(|f| fb.contains(f)).count();
                let larger = fa.len().max(fb.len());
                if shared * 5 >= larger * 4 {
                    twice(r, ont, "W116", &b.file, ("entity", &b.name), pair, format!("{} and {} share {} of {} fields: probably one entity.", b.name, a.name, shared, larger));
                }
            }
        }
    }
    // W117: one word meaning two things, or a word that is another entity's own name
    for (i, g) in ont.glossary.iter().enumerate() {
        for h in ont.glossary.iter().skip(i + 1) {
            if g.term.eq_ignore_ascii_case(&h.term) && g.means != h.means {
                twice(r, ont, "W117", "glossary.yaml", ("term", &h.term), [format!("term:{}={}", h.term, h.means), format!("term:{}={}", g.term, g.means)],
                    format!("\"{}\" means both {} and {}: agents will pick one at random.", h.term, g.means, h.means));
            }
        }
        if let Some(t) = ont.types.values().find(|t| t.name.eq_ignore_ascii_case(&g.term) && t.name != g.means) {
            twice(r, ont, "W117", "glossary.yaml", ("term", &g.term), [format!("term:{}={}", g.term, g.means), format!("entity:{}", t.name)],
                format!("\"{}\" is an entity, and also a word for {}.", t.name, g.means));
        }
    }
}

// ---------------------------------------------------------------------------
// The rule of three, as advice: never a diagnostic, never a light
// ---------------------------------------------------------------------------

/// Three or more entities that share a core of fields could be kinds of one parent.
#[derive(Debug, Clone, Serialize)]
pub struct Advice {
    pub entities: Vec<String>,
    pub fields: Vec<String>,
    pub text: String,
}

/// The shared core of fields (same name, same type) of entities, in the first one's order.
pub fn shared_core(ont: &Ontology, names: &[String]) -> Vec<String> {
    let Some(first) = names.first().and_then(|n| ont.types.get(n)) else { return vec![] };
    first
        .own_properties()
        .filter(|(f, p)| {
            let k = p.normalize().kind;
            names.iter().skip(1).all(|n| ont.types.get(n).and_then(|t| t.properties.get(*f)).map(|q| !t_inherits(ont, n, f) && q.normalize().kind == k).unwrap_or(false))
        })
        .map(|(f, _)| f.clone())
        .collect()
}

fn t_inherits(ont: &Ontology, entity: &str, field: &str) -> bool {
    ont.types.get(entity).map(|t| t.inherited.contains_key(field)).unwrap_or(false)
}

/// Groups of three or more entities (not already kinds of one parent) sharing at least four
/// fields with the same names and types, the core being at least half of each one's own fields.
pub fn advise(ont: &Ontology) -> Vec<Advice> {
    let ents: Vec<&crate::schema::TypeDef> = ont.types.values().filter(|t| t.own_properties().count() >= 4).collect();
    if ents.len() < 3 || ents.len() > 400 {
        return vec![];
    }
    let mut out: Vec<Advice> = Vec::new();
    let mut seen: Vec<Vec<String>> = Vec::new();
    for a in &ents {
        let fa: Vec<(String, String)> = a.own_properties().map(|(f, p)| (f.clone(), p.normalize().kind)).collect();
        let mut group: Vec<String> = vec![a.name.clone()];
        for b in &ents {
            if b.name == a.name || (a.parent.is_some() && a.parent == b.parent) {
                continue;
            }
            let fb: Vec<(String, String)> = b.own_properties().map(|(f, p)| (f.clone(), p.normalize().kind)).collect();
            let shared = fa.iter().filter(|x| fb.contains(x)).count();
            if shared >= 4 && shared * 2 >= fa.len() && shared * 2 >= fb.len() {
                group.push(b.name.clone());
            }
        }
        if group.len() < 3 {
            continue;
        }
        // shrink until the core holds for every member
        loop {
            let core = shared_core(ont, &group);
            let ok = core.len() >= 4 && group.iter().all(|n| ont.types.get(n).map(|t| core.len() * 2 >= t.own_properties().count()).unwrap_or(false));
            if ok || group.len() < 3 {
                break;
            }
            group.pop();
        }
        if group.len() < 3 {
            continue;
        }
        let mut key = group.clone();
        key.sort();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let core = shared_core(ont, &group);
        let names = match group.len() {
            n if n > 1 => format!("{} and {}", group[..n - 1].join(", "), group[n - 1]),
            _ => group[0].clone(),
        };
        out.push(Advice {
            text: format!("{} share {} fields ({}). They could be kinds of one parent.", names, core.len(), core.join(", ")),
            entities: group,
            fields: core,
        });
    }
    out
}

/// "promo_sale_revenue" → "Promo sale revenue" for a sentence.
fn label(name: &str) -> String {
    crate::review::words::label(name)
}

/// What a trust.yaml line is about: a binding's subject is its entity.
fn subject_of(k: crate::trust::Kind, name: &str) -> (&'static str, &str) {
    match k {
        crate::trust::Kind::Metric => ("metric", name),
        crate::trust::Kind::Binding => ("entity", name),
        crate::trust::Kind::Term => ("term", name),
        crate::trust::Kind::Guardrail => ("guardrail", name),
    }
}

/// The item an IRI belongs to; the space itself and a domain have none.
fn subject_of_key<'a>(ont: &'a Ontology, key: &'a Key) -> Option<(&'static str, &'a str)> {
    Some(match key {
        Key::Space | Key::Domain(_) => return None,
        Key::Entity(t) | Key::Field(t, _) | Key::Link(t, _) => ("entity", t.as_str()),
        Key::Term(i) => ("term", ont.glossary.get(*i).map(|g| g.term.as_str())?),
        Key::Guardrail(id) => ("guardrail", id.as_str()),
        Key::Action(n) => ("action", n.as_str()),
        Key::Metric(n) => ("metric", n.as_str()),
        Key::Function(n) => ("function", n.as_str()),
        Key::Source(n) => ("source", n.as_str()),
    })
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
    fn a_file_only_error_names_the_entity_only_when_its_file_holds_one() {
        let d = space(&[
            ("ontology.yaml", "name: subj\n"),
            ("types/one.yaml", "type: One\ndescription: o\niri: \"nope:One\"\nproperties: { id: { type: string, key: true, description: id } }\n"),
            ("types/two.yaml", "types:\n  - type: Two\n    description: t\n    iri: \"nope:Two\"\n    properties: { id: { type: string, key: true, description: id } }\n  - type: Three\n    description: t\n    properties: { id: { type: string, key: true, description: id } }\n"),
        ]);
        let report = super::validate(&Ontology::from_raw(loader::load(&d).unwrap()));
        let e17 = |file: &str| report.diags.iter().find(|x| x.code == "E017" && x.file == file).unwrap_or_else(|| panic!("{:?}", report.diags)).subject.clone();
        assert_eq!(e17("types/one.yaml").map(|s| s.name), Some("One".to_string()));
        assert_eq!(e17("types/two.yaml"), None, "a list file holds two entities: the error is about the file, so the space");
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
