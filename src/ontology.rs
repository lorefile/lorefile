//! The compiled ontology: everything from the YAML folder, indexed and ready for
//! microsecond lookups. Also home to the graph queries, search, describe and the
//! compact Markdown summary that agents load into their context.

use crate::iri::{self, Iris, Key};
use crate::loader::Raw;
use crate::schema::*;
use crate::trust::{self, Kind as TKind};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

/// A relationship after normalisation (inline and standalone look the same).
#[derive(Debug, Clone, Serialize)]
pub struct Relationship {
    pub name: String,
    pub from: String,
    pub to: String,
    pub cardinality: String,
    pub description: String,
    pub properties: IndexMap<String, Property>,
    pub source: Option<EdgeSource>,
    /// The `iri:` written on the link, if any (`Ontology::iris` has the resolved one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
    pub file: String,
}

impl Relationship {
    /// `Supplier.supplies`
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.from, self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Type,
    Relationship,
    Constraint,
    Action,
    Source,
    Term,
    Metric,
    Function,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub kind: Kind,
    pub name: String,
    pub snippet: String,
    pub score: u32,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeInfo {
    pub name: String,
    pub description: String,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeInfo {
    pub relationship: String,
    pub from: String,
    pub to: String,
    pub cardinality: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Subgraph {
    pub root: String,
    pub nodes: Vec<NodeInfo>,
    pub edges: Vec<EdgeInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub types: usize,
    pub relationships: usize,
    pub constraints: usize,
    pub actions: usize,
    pub sources: usize,
    pub glossary_terms: usize,
    pub metrics: usize,
    pub functions: usize,
    pub files: usize,
}

#[derive(Debug, Default)]
pub struct Ontology {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub types: IndexMap<String, TypeDef>,
    pub relationships: Vec<Relationship>,
    pub constraints: IndexMap<String, Constraint>,
    pub actions: IndexMap<String, ActionDef>,
    pub sources: IndexMap<String, SourceDef>,
    pub glossary: Vec<GlossaryTerm>,
    pub metrics: IndexMap<String, MetricDef>,
    pub functions: IndexMap<String, FunctionDef>,
    pub files: Vec<String>,
    /// Problems the loader hit (bad YAML etc). Surfaced by the validator.
    pub load_problems: Vec<String>,
    /// Duplicates the loader saw; the validator turns these into errors.
    pub duplicates: Vec<(Kind, String, String)>,
    /// Every IRI, resolved: declared with `iri:` or derived from the namespace.
    pub iris: Iris,
    /// `trust.yaml` as written; `None` when the space has none.
    pub trust: Option<TrustBook>,
    /// Every grade, resolved against the fingerprints (see `trust::resolve`).
    pub graded: trust::Graded,
}

/// `PurchaseOrder` → `purchase_order`. Used for CEL variable names.
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev_lower = i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
            if i > 0 && (prev_lower || next_lower) && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else if *c == '-' || *c == ' ' {
            out.push('_');
        } else {
            out.push(*c);
        }
    }
    out
}

/// One line saying where a metric comes from: `shop_bigquery.orders · where status = 'paid' · count distinct order_id`.
fn metric_source_brief(s: &MetricSource) -> String {
    let mut parts = vec![if s.tool.is_empty() { format!("{}.{}", s.connection, s.table) } else { format!("{} tool {}{}", s.connection, s.tool, if s.params.is_empty() { String::new() } else { format!(" {}", serde_json::to_string(&s.params).unwrap_or_default()) }) }];
    if !s.filter.is_empty() {
        parts.push(format!("where {}", s.filter));
    }
    if !s.measure.is_empty() {
        parts.push(s.measure.clone());
    }
    parts.join(" · ")
}

/// One line saying how a function runs: `call tool mmm_run_budget_optimizer on the ads MCP server with {...}`.
pub fn function_runs_brief(f: &FunctionDef) -> String {
    match &f.runs {
        None => "nowhere declared yet".to_string(),
        Some(r) => match r.kind.as_str() {
            "mcp" => format!("call tool `{}` on the `{}` MCP server{}", r.tool, r.server, if r.args.is_empty() { String::new() } else { format!(" with {}", serde_json::to_string(&r.args).unwrap_or_default()) }),
            "http" => format!("{} {}", if r.method.is_empty() { "POST" } else { r.method.as_str() }, r.url),
            "manual" => "a person does it; record the request".to_string(),
            other => format!("runs as `{}`", other),
        },
    }
}

fn metric_from(m: &MetricDef) -> String {
    if let Some(s) = &m.source {
        return metric_source_brief(s);
    }
    if !m.variants.is_empty() {
        return m.variants.iter().map(|v| format!("when {} → {}", v.when, metric_source_brief(&v.source))).collect::<Vec<_>>().join("; ");
    }
    if m.sql.is_some() { "sql (see describe)".to_string() } else { "nowhere yet".to_string() }
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim().replace('\n', " ");
    if s.chars().count() <= n {
        s
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

impl Ontology {
    pub fn from_raw(raw: Raw) -> Ontology {
        let mut ont = Ontology {
            root: raw.root,
            manifest: raw.manifest.unwrap_or_else(|| Manifest {
                name: "ontology".into(),
                ..Default::default()
            }),
            files: raw.files,
            load_problems: raw.problems,
            glossary: raw.glossary,
            trust: raw.trust,
            ..Default::default()
        };

        for t in raw.types {
            if let Some(prev) = ont.types.get(&t.name) {
                ont.duplicates.push((Kind::Type, t.name.clone(), format!("{} and {}", prev.file, t.file)));
                continue;
            }
            // inline relationships become first-class relationships
            for (rname, r) in &t.relationships {
                ont.relationships.push(Relationship {
                    name: rname.clone(),
                    from: t.name.clone(),
                    to: r.to.clone(),
                    cardinality: r.cardinality.clone(),
                    description: r.description.clone(),
                    properties: r.properties.iter().map(|(k, v)| (k.clone(), v.normalize())).collect(),
                    source: r.source.clone(),
                    iri: r.iri.clone(),
                    file: t.file.clone(),
                });
            }
            ont.types.insert(t.name.clone(), t);
        }
        for r in raw.relationships {
            let rel = Relationship {
                name: r.name,
                from: r.from,
                to: r.to,
                cardinality: r.cardinality,
                description: r.description,
                properties: r.properties.iter().map(|(k, v)| (k.clone(), v.normalize())).collect(),
                source: r.source,
                iri: r.iri,
                file: r.file,
            };
            if ont.relationships.iter().any(|x| x.from == rel.from && x.name == rel.name) {
                ont.duplicates.push((Kind::Relationship, rel.qualified(), String::new()));
                continue;
            }
            ont.relationships.push(rel);
        }
        for c in raw.constraints {
            if let Some(prev) = ont.constraints.get(&c.id) {
                ont.duplicates.push((Kind::Constraint, c.id.clone(), format!("{} and {}", prev.file, c.file)));
                continue;
            }
            ont.constraints.insert(c.id.clone(), c);
        }
        for a in raw.actions {
            if let Some(prev) = ont.actions.get(&a.name) {
                ont.duplicates.push((Kind::Action, a.name.clone(), format!("{} and {}", prev.file, a.file)));
                continue;
            }
            ont.actions.insert(a.name.clone(), a);
        }
        for s in raw.sources {
            if ont.sources.contains_key(&s.connection) {
                ont.duplicates.push((Kind::Source, s.connection.clone(), String::new()));
                continue;
            }
            ont.sources.insert(s.connection.clone(), s);
        }
        for m in raw.metrics {
            if let Some(prev) = ont.metrics.get(&m.name) {
                ont.duplicates.push((Kind::Metric, m.name.clone(), format!("{} and {}", prev.file, m.file)));
                continue;
            }
            ont.metrics.insert(m.name.clone(), m);
        }
        for f in raw.functions {
            if let Some(prev) = ont.functions.get(&f.name) {
                ont.duplicates.push((Kind::Function, f.name.clone(), format!("{} and {}", prev.file, f.file)));
                continue;
            }
            ont.functions.insert(f.name.clone(), f);
        }
        ont.iris = ont.resolve_iris();
        ont.graded = trust::resolve(&ont);
        ont
    }

    /// Swap in another trust book (the one a save is about to write) and resolve again.
    pub fn set_trust(&mut self, book: Option<TrustBook>) {
        self.trust = book;
        self.graded = trust::resolve(self);
    }

    /// The grade of one thing right now.
    pub fn grade_of(&self, k: TKind, name: &str) -> trust::Status {
        self.graded.get(k, name)
    }

    /// A metric's basis, or a variant's when they differ: "transactional", "attributed · modelled".
    pub fn basis_words(m: &MetricDef) -> Option<String> {
        let mut words: Vec<&str> = Vec::new();
        if let Some(b) = m.basis {
            words.push(b.word());
        }
        for v in &m.variants {
            if let Some(b) = v.basis {
                if !words.contains(&b.word()) {
                    words.push(b.word());
                }
            }
        }
        if words.is_empty() { None } else { Some(words.join(" · ")) }
    }

    /// Resolves the IRI of everything that has one.
    fn resolve_iris(&self) -> Iris {
        let mut iris = iri::base(&self.manifest);
        let terms = self.term_words();
        let man = "ontology.yaml";
        iris.put(Key::Space, None, man, &terms);
        for d in &self.manifest.domains {
            iris.put(Key::Domain(d.name.clone()), None, man, &terms);
        }
        for t in self.types.values() {
            iris.put(Key::Entity(t.name.clone()), t.iri.as_deref(), &t.file, &terms);
            for (pn, p) in &t.properties {
                iris.put(Key::Field(t.name.clone(), pn.clone()), p.normalize().iri.as_deref(), &t.file, &terms);
            }
        }
        for r in &self.relationships {
            iris.put(Key::Link(r.from.clone(), r.name.clone()), r.iri.as_deref(), &r.file, &terms);
        }
        for (i, g) in self.glossary.iter().enumerate() {
            iris.put(Key::Term(i), g.iri.as_deref(), "glossary.yaml", &terms);
        }
        for c in self.constraints.values() {
            iris.put(Key::Guardrail(c.id.clone()), c.iri.as_deref(), &c.file, &terms);
        }
        for a in self.actions.values() {
            iris.put(Key::Action(a.name.clone()), a.iri.as_deref(), &a.file, &terms);
        }
        for m in self.metrics.values() {
            iris.put(Key::Metric(m.name.clone()), None, &m.file, &terms);
        }
        for f in self.functions.values() {
            iris.put(Key::Function(f.name.clone()), None, &f.file, &terms);
        }
        for s in self.sources.values() {
            iris.put(Key::Source(s.connection.clone()), None, &s.file, &terms);
        }
        iris
    }

    /// The glossary's words, in order (term IRIs are derived from them).
    pub fn term_words(&self) -> Vec<String> {
        self.glossary.iter().map(|g| g.term.clone()).collect()
    }

    // ---------- IRIs ----------

    pub fn iri_type(&self, name: &str) -> String {
        self.iris.get(&Key::Entity(name.to_string()))
    }
    pub fn iri_field(&self, type_name: &str, field: &str) -> String {
        self.iris.get(&Key::Field(type_name.to_string(), field.to_string()))
    }
    pub fn iri_link(&self, r: &Relationship) -> String {
        self.iris.get(&Key::Link(r.from.clone(), r.name.clone()))
    }
    pub fn iri_guardrail(&self, id: &str) -> String {
        self.iris.get(&Key::Guardrail(id.to_string()))
    }
    pub fn iri_action(&self, name: &str) -> String {
        self.iris.get(&Key::Action(name.to_string()))
    }
    pub fn iri_term(&self, index: usize) -> String {
        self.iris.map.get(&Key::Term(index)).cloned().unwrap_or_else(|| self.iris.derived(&Key::Term(index), &self.term_words()))
    }
    pub fn iri_metric(&self, name: &str) -> String {
        self.iris.get(&Key::Metric(name.to_string()))
    }
    pub fn iri_function(&self, name: &str) -> String {
        self.iris.get(&Key::Function(name.to_string()))
    }
    pub fn iri_source(&self, connection: &str) -> String {
        self.iris.get(&Key::Source(connection.to_string()))
    }
    pub fn iri_domain(&self, name: &str) -> String {
        self.iris.get(&Key::Domain(name.to_string()))
    }
    /// The IRI of whatever a name refers to (a glossary term's `means`).
    pub fn iri_of_name(&self, name: &str) -> Option<String> {
        if let Some(t) = self.type_(name) {
            return Some(self.iri_type(&t.name));
        }
        if let Some(r) = self.relationship(name).first() {
            return Some(self.iri_link(r));
        }
        if let Some(a) = self.action(name) {
            return Some(self.iri_action(&a.name));
        }
        if let Some(c) = self.constraint(name) {
            return Some(self.iri_guardrail(&c.id));
        }
        if self.metrics.contains_key(name) {
            return Some(self.iri_metric(name));
        }
        if self.functions.contains_key(name) {
            return Some(self.iri_function(name));
        }
        if self.sources.contains_key(name) {
            return Some(self.iri_source(name));
        }
        None
    }

    pub fn stats(&self) -> Stats {
        Stats {
            types: self.types.len(),
            relationships: self.relationships.len(),
            constraints: self.constraints.len(),
            actions: self.actions.len(),
            sources: self.sources.len(),
            glossary_terms: self.glossary.len(),
            metrics: self.metrics.len(),
            functions: self.functions.len(),
            files: self.files.len(),
        }
    }

    // ---------- name resolution ----------

    /// Find a type by exact name, case-insensitive name, or glossary term.
    pub fn type_(&self, name: &str) -> Option<&TypeDef> {
        if let Some(t) = self.types.get(name) {
            return Some(t);
        }
        let lower = name.to_lowercase();
        if let Some((_, t)) = self.types.iter().find(|(k, _)| k.to_lowercase() == lower) {
            return Some(t);
        }
        self.glossary
            .iter()
            .find(|g| g.term.to_lowercase() == lower)
            .and_then(|g| self.types.get(&g.means))
    }

    /// Relationships matching a plain name (`supplies`) or qualified (`Supplier.supplies`).
    pub fn relationship(&self, name: &str) -> Vec<&Relationship> {
        if let Some((from, rel)) = name.split_once('.') {
            return self
                .relationships
                .iter()
                .filter(|r| r.from.eq_ignore_ascii_case(from) && r.name.eq_ignore_ascii_case(rel))
                .collect();
        }
        self.relationships
            .iter()
            .filter(|r| r.name.eq_ignore_ascii_case(name))
            .collect()
    }

    pub fn action(&self, name: &str) -> Option<&ActionDef> {
        self.actions
            .get(name)
            .or_else(|| self.actions.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))
    }

    pub fn constraint(&self, id: &str) -> Option<&Constraint> {
        self.constraints
            .get(id)
            .or_else(|| self.constraints.iter().find(|(k, _)| k.eq_ignore_ascii_case(id)).map(|(_, v)| v))
    }

    /// What does this name refer to?
    pub fn kind_of(&self, name: &str) -> Option<Kind> {
        if self.type_(name).is_some() {
            Some(Kind::Type)
        } else if !self.relationship(name).is_empty() {
            Some(Kind::Relationship)
        } else if self.action(name).is_some() {
            Some(Kind::Action)
        } else if self.constraint(name).is_some() {
            Some(Kind::Constraint)
        } else if self.sources.contains_key(name) {
            Some(Kind::Source)
        } else if self.metrics.contains_key(name) {
            Some(Kind::Metric)
        } else if self.functions.contains_key(name) {
            Some(Kind::Function)
        } else {
            None
        }
    }

    pub fn domain_of(&self, type_name: &str) -> Option<String> {
        if let Some(t) = self.types.get(type_name) {
            if let Some(d) = &t.domain {
                return Some(d.clone());
            }
        }
        self.manifest
            .domains
            .iter()
            .find(|d| d.types.iter().any(|t| t == type_name))
            .map(|d| d.name.clone())
    }

    pub fn outgoing(&self, type_name: &str) -> Vec<&Relationship> {
        self.relationships.iter().filter(|r| r.from == type_name).collect()
    }

    pub fn incoming(&self, type_name: &str) -> Vec<&Relationship> {
        self.relationships.iter().filter(|r| r.to == type_name).collect()
    }

    // ---------- constraints ----------

    /// Constraints that apply to a type, relationship, or action, including the
    /// action's explicit `checks` list and, for a type, rules on its relationships.
    pub fn constraints_for(&self, name: &str) -> Vec<&Constraint> {
        let mut out: Vec<&Constraint> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        let direct: Vec<&Constraint> = self
            .constraints
            .values()
            .filter(|c| c.applies_to.eq_ignore_ascii_case(name))
            .collect();
        for c in direct {
            if seen.insert(&c.id) {
                out.push(c);
            }
        }
        if let Some(t) = self.type_(name) {
            for c in self.constraints.values().filter(|c| c.applies_to.eq_ignore_ascii_case(&t.name)) {
                if seen.insert(&c.id) {
                    out.push(c);
                }
            }
            let rel_names: Vec<String> = self
                .relationships
                .iter()
                .filter(|r| r.from == t.name || r.to == t.name)
                .flat_map(|r| vec![r.name.clone(), r.qualified()])
                .collect();
            for c in self.constraints.values() {
                if rel_names.iter().any(|n| n.eq_ignore_ascii_case(&c.applies_to)) && seen.insert(&c.id) {
                    out.push(c);
                }
            }
        }
        if let Some(a) = self.action(name) {
            for id in &a.checks {
                if let Some(c) = self.constraint(id) {
                    if seen.insert(&c.id) {
                        out.push(c);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.id.cmp(&b.id)));
        out
    }

    /// Actions whose inputs include this type.
    pub fn actions_involving(&self, type_name: &str) -> Vec<&ActionDef> {
        self.actions
            .values()
            .filter(|a| a.inputs.values().any(|t| t == type_name))
            .collect()
    }

    // ---------- graph ----------

    pub fn neighbors(&self, name: &str, depth: usize) -> Option<Subgraph> {
        let root = self.type_(name)?.name.clone();
        let depth = depth.clamp(1, 6);
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut edges: Vec<EdgeInfo> = Vec::new();
        let mut edge_seen: HashSet<String> = HashSet::new();
        seen.insert(root.clone(), 0);
        queue.push_back((root.clone(), 0));
        while let Some((node, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            for r in self.relationships.iter().filter(|r| r.from == node || r.to == node) {
                let key = format!("{}>{}>{}", r.from, r.name, r.to);
                if edge_seen.insert(key) {
                    edges.push(EdgeInfo {
                        relationship: r.name.clone(),
                        from: r.from.clone(),
                        to: r.to.clone(),
                        cardinality: r.cardinality.clone(),
                    });
                }
                let other = if r.from == node { &r.to } else { &r.from };
                if !seen.contains_key(other) {
                    seen.insert(other.clone(), d + 1);
                    queue.push_back((other.clone(), d + 1));
                }
            }
        }
        let mut nodes: Vec<NodeInfo> = seen
            .into_iter()
            .map(|(n, d)| NodeInfo {
                description: self.types.get(&n).map(|t| truncate(&t.description, 140)).unwrap_or_default(),
                name: n,
                depth: d,
            })
            .collect();
        nodes.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.name.cmp(&b.name)));
        Some(Subgraph { root, nodes, edges })
    }

    /// Shortest path between two types, following relationships in either direction.
    pub fn path(&self, from: &str, to: &str) -> Option<Vec<EdgeInfo>> {
        let start = self.type_(from)?.name.clone();
        let goal = self.type_(to)?.name.clone();
        if start == goal {
            return Some(vec![]);
        }
        let mut prev: HashMap<String, (String, EdgeInfo)> = HashMap::new();
        let mut queue = VecDeque::from([start.clone()]);
        let mut seen: HashSet<String> = HashSet::from([start.clone()]);
        while let Some(node) = queue.pop_front() {
            for r in self.relationships.iter().filter(|r| r.from == node || r.to == node) {
                let other = if r.from == node { r.to.clone() } else { r.from.clone() };
                if seen.insert(other.clone()) {
                    prev.insert(
                        other.clone(),
                        (
                            node.clone(),
                            EdgeInfo {
                                relationship: r.name.clone(),
                                from: r.from.clone(),
                                to: r.to.clone(),
                                cardinality: r.cardinality.clone(),
                            },
                        ),
                    );
                    if other == goal {
                        let mut path = Vec::new();
                        let mut cur = goal.clone();
                        while cur != start {
                            let (p, e) = prev.get(&cur).unwrap().clone();
                            path.push(e);
                            cur = p;
                        }
                        path.reverse();
                        return Some(path);
                    }
                    queue.push_back(other);
                }
            }
        }
        None
    }

    // ---------- search ----------

    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return vec![];
        }
        let words: Vec<&str> = q.split_whitespace().collect();
        let score_text = |text: &str, weight: u32| -> u32 {
            let t = text.to_lowercase();
            if t == q {
                return weight + 40;
            }
            if t.contains(&q) {
                return weight;
            }
            let hits = words.iter().filter(|w| t.contains(*w)).count() as u32;
            if hits == words.len() as u32 && hits > 0 {
                weight / 2
            } else if hits > 0 {
                weight / 4
            } else {
                0
            }
        };
        let mut hits: Vec<Hit> = Vec::new();

        for t in self.types.values() {
            let mut s = score_text(&t.name, 100).max(score_text(&snake(&t.name).replace('_', " "), 90));
            s = s.max(score_text(&t.description, 40));
            for tag in &t.tags {
                s = s.max(score_text(tag, 50));
            }
            for (pn, p) in &t.properties {
                s = s.max(score_text(pn, 35)).max(score_text(&p.normalize().description, 20));
            }
            if s > 0 {
                hits.push(Hit { kind: Kind::Type, name: t.name.clone(), snippet: truncate(&t.description, 140), score: s, file: t.file.clone() });
            }
        }
        for r in &self.relationships {
            let s = score_text(&r.name, 80).max(score_text(&r.description, 35));
            if s > 0 {
                hits.push(Hit {
                    kind: Kind::Relationship,
                    name: r.qualified(),
                    snippet: format!("{} → {} ({})", r.from, r.to, r.cardinality),
                    score: s,
                    file: r.file.clone(),
                });
            }
        }
        for c in self.constraints.values() {
            let mut s = score_text(&c.id, 100).max(score_text(&c.name, 70)).max(score_text(&c.description, 35));
            s = s.max(score_text(&c.rule, 30)).max(score_text(&c.rationale, 25)).max(score_text(&c.applies_to, 30));
            for tag in &c.tags {
                s = s.max(score_text(tag, 45));
            }
            if s > 0 {
                hits.push(Hit {
                    kind: Kind::Constraint,
                    name: c.id.clone(),
                    snippet: format!("[{}] {} — on {}", c.severity, c.name, c.applies_to),
                    score: s,
                    file: c.file.clone(),
                });
            }
        }
        for a in self.actions.values() {
            let s = score_text(&a.name, 90).max(score_text(&a.name.replace('_', " "), 85)).max(score_text(&a.description, 40));
            if s > 0 {
                hits.push(Hit { kind: Kind::Action, name: a.name.clone(), snippet: truncate(&a.description, 140), score: s, file: a.file.clone() });
            }
        }
        for g in &self.glossary {
            let s = score_text(&g.term, 95).max(score_text(&g.description, 30));
            if s > 0 {
                hits.push(Hit {
                    kind: Kind::Term,
                    name: g.term.clone(),
                    snippet: format!("means {}{}", g.means, if g.description.is_empty() { String::new() } else { format!(" — {}", truncate(&g.description, 100)) }),
                    score: s,
                    file: "glossary.yaml".into(),
                });
            }
        }
        for m in self.metrics.values() {
            let s = score_text(&m.name, 95).max(score_text(&m.name.replace('_', " "), 85)).max(score_text(&m.description, 40)).max(score_text(&m.caveats, 20));
            if s > 0 {
                hits.push(Hit { kind: Kind::Metric, name: m.name.clone(), snippet: truncate(&m.description, 140), score: s, file: m.file.clone() });
            }
        }
        for f in self.functions.values() {
            let s = score_text(&f.name, 95).max(score_text(&f.name.replace('_', " "), 85)).max(score_text(&f.description, 40));
            if s > 0 {
                hits.push(Hit { kind: Kind::Function, name: f.name.clone(), snippet: truncate(&f.description, 140), score: s, file: f.file.clone() });
            }
        }
        for s in self.sources.values() {
            let sc = score_text(&s.connection, 80).max(score_text(&s.kind, 40)).max(score_text(&s.description, 30));
            if sc > 0 {
                hits.push(Hit { kind: Kind::Source, name: s.connection.clone(), snippet: format!("{} source", s.kind), score: sc, file: s.file.clone() });
            }
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.cmp(&b.name)));
        hits.truncate(limit.max(1));
        hits
    }

    // ---------- describe ----------

    pub fn constraint_brief(&self, c: &Constraint) -> Json {
        let mut j = json!({
            "id": c.id, "name": c.name, "severity": c.severity, "applies_to": c.applies_to,
            "rule": c.rule, "rationale": c.rationale, "iri": self.iri_guardrail(&c.id),
        });
        if self.graded.has_file {
            j["grade"] = json!(self.grade_of(TKind::Guardrail, &c.id).word());
        }
        j
    }

    pub fn describe(&self, name: &str) -> Option<Json> {
        if let Some(f) = self.functions.get(name) {
            let mut j = serde_json::to_value(f).unwrap_or(json!({}));
            j["kind"] = json!("function");
            j["name"] = json!(f.name);
            j["how"] = json!(function_runs_brief(f));
            j["iri"] = json!(self.iri_function(&f.name));
            j["used_by_actions"] = json!(self.actions.values().filter(|a| a.tags.iter().any(|t| t == &format!("does:{}", f.name))).map(|a| a.name.clone()).collect::<Vec<_>>());
            return Some(j);
        }
        if let Some(m) = self.metrics.get(name) {
            let mut j = serde_json::to_value(m).unwrap_or(json!({}));
            j["kind"] = json!("metric");
            j["name"] = json!(m.name);
            j["from"] = json!(metric_from(m));
            j["iri"] = json!(self.iri_metric(&m.name));
            if self.graded.has_file {
                j["trust"] = self.grade_of(TKind::Metric, &m.name).to_json(TKind::Metric);
            }
            return Some(j);
        }
        if let Some(t) = self.type_(name) {
            let props: Vec<Json> = t
                .properties
                .iter()
                .map(|(n, p)| {
                    let p = p.normalize();
                    let mut j = json!({ "name": n, "type": p.kind, "description": p.description, "iri": self.iri_field(&t.name, n) });
                    if p.key { j["key"] = json!(true); }
                    if p.required { j["required"] = json!(true); }
                    if !p.values.is_empty() { j["values"] = serde_json::to_value(&p.values).unwrap_or(Json::Null); }
                    if let Some(u) = &p.unit { j["unit"] = json!(u); }
                    j
                })
                .collect();
            let out: Vec<Json> = self
                .outgoing(&t.name)
                .iter()
                .map(|r| json!({ "name": r.name, "to": r.to, "cardinality": r.cardinality, "description": r.description, "iri": self.iri_link(r) }))
                .collect();
            let inc: Vec<Json> = self
                .incoming(&t.name)
                .iter()
                .map(|r| json!({ "name": r.name, "from": r.from, "cardinality": r.cardinality, "description": r.description, "iri": self.iri_link(r) }))
                .collect();
            let cons: Vec<Json> = self.constraints_for(&t.name).iter().map(|c| self.constraint_brief(c)).collect();
            let acts: Vec<String> = self.actions_involving(&t.name).iter().map(|a| a.name.clone()).collect();
            let mut j = json!({
                "kind": "type",
                "name": t.name,
                "iri": self.iri_type(&t.name),
                "description": t.description,
                "domain": self.domain_of(&t.name),
                "key": t.key_property(),
                "properties": props,
                "relationships": out,
                "incoming": inc,
                "constraints": cons,
                "actions": acts,
                "owner": t.owner,
                "tags": t.tags,
                "file": t.file,
            });
            if let Some(s) = &t.source {
                let kind = self.sources.get(&s.connection).map(|x| x.kind.clone()).unwrap_or_else(|| "unknown".into());
                j["source"] = json!({ "connection": s.connection, "kind": kind, "table": s.table, "key": s.key, "columns": s.columns, "tool": s.tool, "params": s.params });
                if self.graded.has_file {
                    j["source"]["trust"] = self.grade_of(TKind::Binding, &t.name).to_json(TKind::Binding);
                }
            }
            // asked by a glossary word: say what the word means and how well that is backed
            if !t.name.eq_ignore_ascii_case(name) {
                if let Some(g) = self.glossary.iter().find(|g| g.term.eq_ignore_ascii_case(name)) {
                    let mut w = json!({ "term": g.term, "means": g.means, "description": g.description });
                    if self.graded.has_file {
                        w["trust"] = self.grade_of(TKind::Term, &g.term).to_json(TKind::Term);
                    }
                    j["word"] = w;
                }
            }
            if !t.instances.is_empty() {
                j["inline_instances"] = json!(t.instances.len());
            }
            return Some(j);
        }
        let rels = self.relationship(name);
        if !rels.is_empty() {
            let items: Vec<Json> = rels
                .iter()
                .map(|r| {
                    let cons: Vec<Json> = self
                        .constraints
                        .values()
                        .filter(|c| c.applies_to.eq_ignore_ascii_case(&r.name) || c.applies_to.eq_ignore_ascii_case(&r.qualified()))
                        .map(|c| self.constraint_brief(c))
                        .collect();
                    let props: Vec<Json> = r.properties.iter().map(|(n, p)| json!({ "name": n, "type": p.kind, "description": p.description })).collect();
                    json!({
                        "kind": "relationship", "name": r.name, "qualified": r.qualified(), "iri": self.iri_link(r),
                        "from": r.from, "to": r.to, "cardinality": r.cardinality,
                        "description": r.description, "properties": props, "constraints": cons,
                        "source": r.source, "file": r.file,
                        "cel_variables": [snake(&r.from), snake(&r.to), "from", "to"],
                    })
                })
                .collect();
            return Some(if items.len() == 1 { items[0].clone() } else { json!({ "kind": "relationships", "matches": items }) });
        }
        if let Some(a) = self.action(name) {
            let checks: Vec<Json> = self.constraints_for(&a.name).iter().map(|c| self.constraint_brief(c)).collect();
            return Some(json!({
                "kind": "action", "name": a.name, "iri": self.iri_action(&a.name), "description": a.description,
                "inputs": a.inputs, "effects": a.effects, "checks": checks,
                "requires_approval_from": a.requires_approval_from, "side_effects": a.side_effects,
                "tags": a.tags, "file": a.file,
                "how_to_validate": format!("call validate(action=\"{}\", inputs={{{}}}) with key values or inline records", a.name,
                    a.inputs.keys().map(|k| format!("\"{}\": \"<{} key>\"", k, a.inputs[k])).collect::<Vec<_>>().join(", ")),
            }));
        }
        if let Some(c) = self.constraint(name) {
            let mut j = serde_json::to_value(c).unwrap_or(Json::Null);
            j["kind"] = json!("constraint");
            j["iri"] = json!(self.iri_guardrail(&c.id));
            if self.graded.has_file {
                j["trust"] = self.grade_of(TKind::Guardrail, &c.id).to_json(TKind::Guardrail);
            }
            j["applies_to_kind"] = json!(self.kind_of(&c.applies_to));
            j["file"] = json!(c.file);
            return Some(j);
        }
        if let Some(s) = self.sources.get(name) {
            let mut used_by: Vec<String> = self
                .types
                .values()
                .filter(|t| t.source.as_ref().map(|x| x.connection == s.connection).unwrap_or(false))
                .map(|t| t.name.clone())
                .collect();
            for m in self.metrics.values() {
                if m.source.as_ref().map(|x| x.connection == s.connection).unwrap_or(false) || m.variants.iter().any(|v| v.source.connection == s.connection) {
                    used_by.push(format!("metric {}", m.name));
                }
            }
            for f in self.functions.values() {
                if f.runs.as_ref().map(|r| r.kind == "mcp" && (r.server == s.connection || Some(r.server.as_str()) == s.server.as_deref())).unwrap_or(false) {
                    used_by.push(format!("function {}", f.name));
                }
            }
            let mut j = serde_json::to_value(s).unwrap_or(Json::Null);
            j["kind"] = json!("source");
            j["iri"] = json!(self.iri_source(&s.connection));
            j["used_by"] = json!(used_by);
            j["file"] = json!(s.file);
            return Some(j);
        }
        None
    }

    pub fn describe_markdown(&self, name: &str) -> Option<String> {
        if let Some(f) = self.functions.get(name) {
            let io = |m: &IndexMap<String, String>| m.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<_>>().join(", ");
            let mut md = format!("# {} (function)\n\n{}\n\nInputs: {}\nOutputs: {}\nHow: {}\n", f.name, f.description.trim(), io(&f.inputs), io(&f.outputs), function_runs_brief(f));
            if let Some(w) = &f.writes_back {
                md.push_str(&format!("Writes back: {} → {}{}\n", w.entity, w.connection, if w.table.is_empty() { String::new() } else { format!(".{}", w.table) }));
            }
            if !f.may_be_called_by.is_empty() {
                md.push_str(&format!("May be called by: {}\n", f.may_be_called_by.join(", ")));
            }
            if !f.owner.is_empty() {
                md.push_str(&format!("Owner: {}\n", f.owner));
            }
            if !f.caveats.is_empty() {
                md.push_str(&format!("Caveats: {}\n", f.caveats.trim()));
            }
            return Some(md);
        }
        if let Some(m) = self.metrics.get(name) {
            let mut md = format!("# {} (metric{})\n\n{}\n\n", m.name, if m.grain.is_empty() { String::new() } else { format!(", per {}", m.grain) }, m.description.trim());
            md.push_str(&format!("From: {}\n", metric_from(m)));
            if let Some(b) = Self::basis_words(m) {
                md.push_str(&format!("Basis: {}\n", b));
            }
            if self.graded.has_file {
                let st = self.grade_of(TKind::Metric, &m.name);
                md.push_str(&format!("Trust: {}{}\n", st.word(), st.say(TKind::Metric).map(|s| format!(". {}", s)).unwrap_or_default()));
            }
            if let Some(sql) = &m.sql {
                md.push_str(&format!("\n```sql\n{}\n```\n", sql.trim()));
            }
            if !m.owner.is_empty() {
                md.push_str(&format!("Owner: {}\n", m.owner));
            }
            if !m.caveats.is_empty() {
                md.push_str(&format!("Caveats: {}\n", m.caveats.trim()));
            }
            return Some(md);
        }
        let j = self.describe(name)?;
        let mut md = String::new();
        match j["kind"].as_str().unwrap_or("") {
            "type" => {
                md.push_str(&format!("# {} (type)\n\nIRI: {}\n\n{}\n\n", j["name"].as_str().unwrap_or(""), j["iri"].as_str().unwrap_or(""), j["description"].as_str().unwrap_or("")));
                if let Some(d) = j["domain"].as_str() { md.push_str(&format!("Domain: {}\n\n", d)); }
                md.push_str("## Properties\n\n| name | type | description |\n|---|---|---|\n");
                for p in j["properties"].as_array().unwrap_or(&vec![]) {
                    let mut t = p["type"].as_str().unwrap_or("").to_string();
                    if p["key"].as_bool().unwrap_or(false) { t.push_str(" (key)"); }
                    if let Some(v) = p["values"].as_array() { t.push_str(&format!(" {}", serde_json::to_string(v).unwrap_or_default())); }
                    md.push_str(&format!("| {} | {} | {} |\n", p["name"].as_str().unwrap_or(""), t, p["description"].as_str().unwrap_or("")));
                }
                md.push_str("\n## Relationships\n\n");
                for r in j["relationships"].as_array().unwrap_or(&vec![]) {
                    md.push_str(&format!("- {} ─{}→ {} ({}) {}\n", j["name"].as_str().unwrap_or(""), r["name"].as_str().unwrap_or(""), r["to"].as_str().unwrap_or(""), r["cardinality"].as_str().unwrap_or(""), r["description"].as_str().unwrap_or("")));
                }
                for r in j["incoming"].as_array().unwrap_or(&vec![]) {
                    md.push_str(&format!("- {} ─{}→ {} ({}) {}\n", r["from"].as_str().unwrap_or(""), r["name"].as_str().unwrap_or(""), j["name"].as_str().unwrap_or(""), r["cardinality"].as_str().unwrap_or(""), r["description"].as_str().unwrap_or("")));
                }
                md.push_str("\n## Rules\n\n");
                for c in j["constraints"].as_array().unwrap_or(&vec![]) {
                    md.push_str(&format!("- **{}** [{}] {} — `{}`\n", c["id"].as_str().unwrap_or(""), c["severity"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""), c["rule"].as_str().unwrap_or("")));
                }
                if let Some(acts) = j["actions"].as_array() {
                    if !acts.is_empty() {
                        md.push_str(&format!("\n## Actions\n\n{}\n", acts.iter().map(|a| format!("- {}", a.as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n")));
                    }
                }
                if j.get("source").is_some() && !j["source"].is_null() {
                    let s = &j["source"];
                    md.push_str(&format!("\n## Data\n\nconnection `{}` ({}) · table `{}` · key `{}`\n", s["connection"].as_str().unwrap_or(""), s["kind"].as_str().unwrap_or(""), s["table"].as_str().unwrap_or(""), s["key"].as_str().unwrap_or("")));
                }
            }
            _ => {
                md.push_str(&format!("```json\n{}\n```\n", serde_json::to_string_pretty(&j).unwrap_or_default()));
            }
        }
        Some(md)
    }

    // ---------- summary ----------

    /// Compact Markdown of the whole ontology, meant for a system prompt.
    pub fn summary_markdown(&self) -> String {
        let st = self.stats();
        let mut md = String::new();
        md.push_str(&format!("# {} — ontology summary\n\n", self.manifest.name));
        if !self.manifest.description.is_empty() {
            md.push_str(&format!("{}\n\n", self.manifest.description.trim()));
        }
        md.push_str(&format!(
            "{} types · {} relationships · {} rules · {} actions · {} data sources\n\n",
            st.types, st.relationships, st.constraints, st.actions, st.sources
        ));
        md.push_str("Use `describe(name)` for full detail, `constraints_for(name)` for rules, `validate(action, inputs)` before acting, `where_is(type)` to find the data.\n\n");
        md.push_str(&format!("IRIs: each name below is `{}<Name>` unless an IRI follows it.\n\n", self.iris.namespace));
        if self.graded.has_file {
            md.push_str(&format!("{}\n\n", self.graded.line()));
        }
        let mark = |k: TKind, n: &str| -> Option<String> { if self.graded.has_file { self.grade_of(k, n).mark() } else { None } };

        // types grouped by domain
        let mut by_domain: IndexMap<String, Vec<&TypeDef>> = IndexMap::new();
        for d in &self.manifest.domains {
            by_domain.entry(d.name.clone()).or_default();
        }
        for t in self.types.values() {
            let d = self.domain_of(&t.name).unwrap_or_else(|| "Other".into());
            by_domain.entry(d).or_default().push(t);
        }
        md.push_str("## Types\n\n");
        for (domain, types) in &by_domain {
            if types.is_empty() {
                continue;
            }
            if by_domain.len() > 1 {
                md.push_str(&format!("### {}\n", domain));
            }
            for t in types {
                let rels: Vec<String> = self.outgoing(&t.name).iter().map(|r| format!("─{}→ {}", r.name, r.to)).collect();
                let own = if self.iris.declared.contains_key(&Key::Entity(t.name.clone())) { format!(" ({})", self.iris.compact(&self.iri_type(&t.name))) } else { String::new() };
                let data = mark(TKind::Binding, &t.name).map(|m| format!(" [data binding {}]", m)).unwrap_or_default();
                md.push_str(&format!(
                    "- **{}**{}{}: {}{}\n",
                    t.name,
                    own,
                    data,
                    truncate(&t.description, 160),
                    if rels.is_empty() { String::new() } else { format!(" [{}]", rels.join(", ")) }
                ));
            }
            md.push('\n');
        }

        md.push_str("## Rules\n\n");
        let mut blocks: Vec<&Constraint> = self.constraints.values().filter(|c| c.severity == Severity::Block).collect();
        let warns = self.constraints.values().filter(|c| c.severity == Severity::Warn).count();
        let infos = self.constraints.values().filter(|c| c.severity == Severity::Info).count();
        md.push_str(&format!("{} blocking · {} warnings · {} informational\n\n", blocks.len(), warns, infos));
        blocks.sort_by(|a, b| a.id.cmp(&b.id));
        for c in blocks.iter().take(40) {
            let m = mark(TKind::Guardrail, &c.id).map(|m| format!(" [{}]", if m.starts_with("UNVERIFIED") { "UNVERIFIED".to_string() } else { m })).unwrap_or_default();
            md.push_str(&format!("- **{}** ({}){}: {} — `{}`\n", c.id, c.applies_to, m, c.name, c.rule));
        }
        // a rule an agent wrote that is not blocking is still named, so it is never read as policy
        let unverified: Vec<&String> = if self.graded.has_file { self.constraints.values().filter(|c| c.severity != Severity::Block && self.grade_of(TKind::Guardrail, &c.id).grade == Some(Grade::Proposed)).map(|c| &c.id).collect() } else { vec![] };
        if !unverified.is_empty() {
            md.push_str(&format!("- UNVERIFIED warnings and notes (an agent proposed them, nobody has reviewed them): {}\n", unverified.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        }
        if blocks.len() > 40 {
            md.push_str(&format!("- … {} more blocking rules; use `constraints_for`\n", blocks.len() - 40));
        }
        md.push('\n');

        if !self.actions.is_empty() {
            md.push_str("## Actions (what agents may do)\n\n");
            for a in self.actions.values() {
                let inputs: Vec<String> = a.inputs.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
                md.push_str(&format!(
                    "- **{}**({}): {}{}\n",
                    a.name,
                    inputs.join(", "),
                    truncate(&a.description, 140),
                    if a.requires_approval_from.is_empty() { String::new() } else { format!(" Approval: {}.", a.requires_approval_from) }
                ));
            }
            md.push('\n');
        }

        if !self.sources.is_empty() {
            md.push_str("## Data sources\n\n");
            for s in self.sources.values() {
                let mut used: Vec<String> = self
                    .types
                    .values()
                    .filter(|t| t.source.as_ref().map(|x| x.connection == s.connection).unwrap_or(false))
                    .map(|t| t.name.clone())
                    .collect();
                used.extend(self.metrics.values().filter(|m| m.source.as_ref().map(|x| x.connection == s.connection).unwrap_or(false) || m.variants.iter().any(|v| v.source.connection == s.connection)).map(|m| format!("metric {}", m.name)));
                used.extend(self.functions.values().filter(|f| f.runs.as_ref().map(|r| r.kind == "mcp" && (r.server == s.connection || Some(r.server.as_str()) == s.server.as_deref())).unwrap_or(false)).map(|f| format!("function {}", f.name)));
                md.push_str(&format!("- **{}** ({}): {}\n", s.connection, s.kind, if used.is_empty() { truncate(&s.description, 100) } else { used.join(", ") }));
            }
            md.push('\n');
        }

        if !self.metrics.is_empty() {
            md.push_str("## Metrics (what counts, and where it comes from)\n\n");
            for m in self.metrics.values().take(40) {
                let tags: Vec<String> = Self::basis_words(m).into_iter().chain(mark(TKind::Metric, &m.name)).collect();
                md.push_str(&format!(
                    "- **{}**{}{}: {} — {}{}\n",
                    m.name,
                    if m.grain.is_empty() { String::new() } else { format!(" (per {})", m.grain) },
                    if tags.is_empty() { String::new() } else { format!(" [{}]", tags.join(" · ")) },
                    truncate(&m.description, 120),
                    metric_from(m),
                    if m.owner.is_empty() { String::new() } else { format!(" · owner {}", m.owner) }
                ));
            }
            md.push('\n');
        }

        if !self.functions.is_empty() {
            md.push_str("## Functions (what agents can run, and where)\n\n");
            for f in self.functions.values().take(40) {
                let io = |m: &IndexMap<String, String>| m.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<_>>().join(", ");
                md.push_str(&format!("- **{}**({}) → {}: {} — {}\n", f.name, io(&f.inputs), io(&f.outputs), truncate(&f.description, 110), function_runs_brief(f)));
            }
            md.push('\n');
        }

        if !self.glossary.is_empty() {
            md.push_str("## Glossary\n\n");
            for g in self.glossary.iter().take(60) {
                let m = mark(TKind::Term, &g.term).map(|_| " [UNVERIFIED]".to_string()).unwrap_or_default();
                md.push_str(&format!("- \"{}\" means **{}**{}\n", g.term, g.means, m));
            }
            md.push('\n');
        }
        md
    }
}
