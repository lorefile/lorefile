//! Reviewing what agents propose, in plain words (specs/2026-11-27-review-flow.md).
//!
//! A proposal is reviewed by meaning, not by lines: the space is compiled as it
//! is and with the proposal's files laid over it, and the two are compared thing
//! by thing (entities and their columns, links, numbers, rules, words, sources,
//! functions, actions). The page only renders the answer, so every rule lives here.
//! Nothing here writes; the only I/O is reading the space through `check_many`.

use crate::edit;
use crate::learn;
use crate::loader::Overrides;
use crate::ontology::{snake, Ontology};
use crate::patch;
use crate::proposals::{Proposal, Replay};
use crate::schema::{Grade, MetricDef, MetricSource, Property, TypeDef};
use crate::validate::{Diag, Level, Report};
use regex::Regex;
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

pub mod words {
    use super::*;

    /// `PromoSale` → "Promo sale", `promo_sale_revenue` → "Promo sale revenue". A name in capitals stays.
    pub fn label(name: &str) -> String {
        if !name.chars().any(|c| c.is_lowercase()) {
            return name.to_string();
        }
        let s = snake(name).replace('_', " ");
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }

    /// A currency is three capital letters (USD, EUR) or a currency sign.
    pub fn is_currency(unit: &str) -> bool {
        let u = unit.trim();
        (u.len() == 3 && u.chars().all(|c| c.is_ascii_uppercase())) || matches!(u, "$" | "€" | "£" | "¥" | "₹")
    }

    /// The type of a column in words: "text", "whole number", "money (USD)", "yes / no".
    pub fn type_word(p: &Property) -> String {
        let unit = p.unit.as_deref().unwrap_or("").trim();
        match p.kind.as_str() {
            "string" => "text".into(),
            "integer" if !unit.is_empty() => format!("whole number ({})", unit),
            "integer" => "whole number".into(),
            "number" if is_currency(unit) => format!("money ({})", unit),
            "number" if !unit.is_empty() => format!("number ({})", unit),
            "number" => "number".into(),
            "boolean" => "yes / no".into(),
            "date" => "date".into(),
            "datetime" => "date and time".into(),
            "enum" if p.values.is_empty() => "one of (no values listed)".into(),
            "enum" => format!("one of {}", p.values.iter().map(yaml_text).collect::<Vec<_>>().join(", ")),
            "json" => "structured data".into(),
            other => other.to_string(),
        }
    }

    fn yaml_text(v: &serde_yaml::Value) -> String {
        match v {
            serde_yaml::Value::String(s) => s.clone(),
            other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
        }
    }

    /// What one row is called, from the key: `line_id` → "lines", `order_id` → "orders", else "rows".
    pub fn rows_noun(key: Option<&str>) -> String {
        match key.and_then(|k| k.strip_suffix("_id")) {
            Some(stem) if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => format!("{}s", stem.replace('_', " ")),
            _ => "rows".into(),
        }
    }

    /// `sum(revenue)` → "add up revenue"; `None` when it is not one of the plain forms.
    pub fn measure_words(m: &str, rows: &str) -> Option<String> {
        let m = m.trim();
        let pats: [(&str, &str); 9] = [
            (r"(?i)^sum\s*\(\s*([A-Za-z_][\w]*)\s*\)$", "add up $1"),
            (r"(?i)^sum\s+(?:of\s+)?([A-Za-z_][\w]*)$", "add up $1"),
            (r"(?i)^count\s*\(\s*distinct\s+([A-Za-z_][\w]*)\s*\)$", "count the different $1"),
            (r"(?i)^count\s+distinct\s+([A-Za-z_][\w]*)$", "count the different $1"),
            (r"(?i)^(?:avg|average|mean)\s*\(\s*([A-Za-z_][\w]*)\s*\)$", "average $1"),
            (r"(?i)^(?:avg|average|mean)\s+(?:of\s+)?([A-Za-z_][\w]*)$", "average $1"),
            (r"(?i)^max\s*\(\s*([A-Za-z_][\w]*)\s*\)$", "the largest $1"),
            (r"(?i)^min\s*\(\s*([A-Za-z_][\w]*)\s*\)$", "the smallest $1"),
            (r"(?i)^count\s*\(\s*([A-Za-z_][\w]*)\s*\)$", "count rows with a $1"),
        ];
        if Regex::new(r"(?i)^count\s*(\(\s*(\*|1)\s*\)|\s+rows)?$").map(|r| r.is_match(m)).unwrap_or(false) {
            return Some(format!("count {}", rows));
        }
        for (p, out) in pats {
            if let Ok(re) = Regex::new(p) {
                if let Some(c) = re.captures(m) {
                    return Some(out.replace("$1", &c[1]));
                }
            }
        }
        None
    }

    fn unquote(v: &str) -> String {
        let v = v.trim();
        if v.len() >= 2 && ((v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"'))) {
            v[1..v.len() - 1].to_string()
        } else {
            v.to_string()
        }
    }

    /// One comparison of a filter in words, or `None`.
    fn condition(c: &str, rows: &str) -> Option<String> {
        let c = c.trim();
        let id = r"([A-Za-z_][\w]*)";
        let lit = r#"('[^']*'|"[^"]*"|-?\d+(?:\.\d+)?|true|false|TRUE|FALSE)"#;
        let cap = |p: &str| Regex::new(p).ok().and_then(|r| r.captures(c).map(|m| (1..m.len()).map(|i| m.get(i).map(|x| x.as_str().to_string()).unwrap_or_default()).collect::<Vec<_>>()));
        if let Some(v) = cap(&format!(r"(?i)^not\s+{}$", id)) {
            return Some(format!("leaving out {} where {} is true", rows, v[0]));
        }
        if let Some(v) = cap(&format!(r"^{}$", id)) {
            return Some(format!("only {} where {} is true", rows, v[0]));
        }
        if let Some(v) = cap(&format!(r"(?i)^{}\s+is\s+not\s+null$", id)) {
            return Some(format!("leaving out {} where {} is empty", rows, v[0]));
        }
        if let Some(v) = cap(&format!(r"(?i)^{}\s+is\s+null$", id)) {
            return Some(format!("only {} where {} is empty", rows, v[0]));
        }
        if let Some(v) = cap(&format!(r"(?i)^{}\s+not\s+in\s*\(([^()]*)\)$", id)) {
            return Some(format!("leaving out {} where {} is one of {}", rows, v[0], v[1].split(',').map(unquote).collect::<Vec<_>>().join(", ")));
        }
        if let Some(v) = cap(&format!(r"(?i)^{}\s+in\s*\(([^()]*)\)$", id)) {
            return Some(format!("only {} where {} is one of {}", rows, v[0], v[1].split(',').map(unquote).collect::<Vec<_>>().join(", ")));
        }
        if let Some(v) = cap(&format!(r"^{}\s*(==|=|!=|<>)\s*{}$", id, lit)) {
            let val = unquote(&v[2]);
            let negated = v[1] == "!=" || v[1] == "<>";
            // a flag: `refunded = false` leaves out the refunded rows
            return Some(match (val.to_ascii_lowercase().as_str(), negated) {
                ("false", false) | ("true", true) => format!("leaving out {} where {} is true", rows, v[0]),
                ("true", false) | ("false", true) => format!("only {} where {} is true", rows, v[0]),
                (_, false) => format!("only {} where {} is {}", rows, v[0], val),
                (_, true) => format!("leaving out {} where {} is {}", rows, v[0], val),
            });
        }
        None
    }

    /// A filter of plain comparisons joined by `and`, in words; `None` when it is anything else.
    pub fn filter_words(f: &str, rows: &str) -> Option<String> {
        let f = f.trim();
        if f.is_empty() {
            return Some(String::new());
        }
        if Regex::new(r"(?i)\s+or\s+").map(|r| r.is_match(f)).unwrap_or(true) {
            return None;
        }
        let parts: Vec<&str> = Regex::new(r"(?i)\s+and\s+").ok()?.split(f).collect();
        let said: Option<Vec<String>> = parts.iter().map(|p| condition(p, rows)).collect();
        said.map(|s| s.join(", "))
    }

    /// One recipe in words: (sentence, what is shown as written because it is not plain).
    pub fn recipe_sentence(src: &MetricSource, rows: &str) -> (String, Option<String>) {
        let measure = if src.measure.trim().is_empty() { None } else { measure_words(&src.measure, rows) };
        let head = match (&measure, src.measure.trim().is_empty()) {
            (Some(m), _) => m.clone(),
            (None, true) => format!("count {}", rows),
            (None, false) => "measure it as written".into(),
        };
        let mut as_written = Vec::new();
        if measure.is_none() && !src.measure.trim().is_empty() {
            as_written.push(src.measure.trim().to_string());
        }
        let filt = filter_words(&src.filter, rows);
        let sentence = match filt {
            Some(f) if f.is_empty() => head,
            Some(f) => format!("{}, {}", head, f),
            None => {
                as_written.push(format!("where {}", src.filter.trim()));
                format!("{}, only {} that pass the filter as written", head, rows)
            }
        };
        (sentence, if as_written.is_empty() { None } else { Some(as_written.join(" · ")) })
    }

    /// The rows of a metric are called by the grain of the entity bound to its table.
    pub fn rows_of(ont: &Ontology, src: &MetricSource) -> String {
        let key = ont.types.values().find(|t| t.source.as_ref().map(|s| s.connection == src.connection && !src.table.is_empty() && s.table == src.table).unwrap_or(false)).and_then(|t| t.key_property());
        rows_noun(key.as_deref())
    }

    /// A whole metric in words, plus what is shown as written.
    pub fn metric_sentence(m: &MetricDef, ont: &Ontology) -> (String, Option<String>) {
        if let Some(src) = &m.source {
            return recipe_sentence(src, &rows_of(ont, src));
        }
        if !m.variants.is_empty() {
            let mut code = Vec::new();
            let parts: Vec<String> = m
                .variants
                .iter()
                .map(|v| {
                    let (s, w) = recipe_sentence(&v.source, &rows_of(ont, &v.source));
                    if let Some(w) = w {
                        code.push(w);
                    }
                    format!("when {}: {}", v.when, s)
                })
                .collect();
            return (parts.join("; "), if code.is_empty() { None } else { Some(code.join(" · ")) });
        }
        if let Some(sql) = &m.sql {
            return ("counted by its own SQL".into(), Some(sql.trim().to_string()));
        }
        ("says nothing about where it comes from".into(), None)
    }

    /// `refunded == false` → "refunded must be false"; `None` when the rule is not that plain.
    pub fn rule_sentence(rule: &str) -> Option<String> {
        let re = Regex::new(r#"^\s*([A-Za-z_][\w.]*)\s*(==|!=|>=|<=|>|<)\s*('[^']*'|"[^"]*"|-?\d+(?:\.\d+)?|true|false|[A-Za-z_][\w.]*)\s*$"#).ok()?;
        let c = re.captures(rule)?;
        let lhs = c[1].trim_start_matches("this.").to_string();
        let rhs = unquote(&c[3]);
        let how = match &c[2] {
            "==" => "must be",
            "!=" => "must not be",
            ">=" => "must be at least",
            "<=" => "must be at most",
            ">" => "must be more than",
            _ => "must be less than",
        };
        Some(format!("{} {} {}", lhs, how, rhs))
    }

    pub fn severity_word(s: &str) -> &'static str {
        match s {
            "block" => "stops the action",
            "warn" => "warns",
            _ => "notes",
        }
    }

    /// Where rows come from, by the source's kind.
    pub fn source_sentence(kind: &str, server: Option<&str>) -> String {
        match kind.to_ascii_lowercase().as_str() {
            "from_user" => "files people give an agent".into(),
            "mcp" => format!("tools on the {} MCP server", server.unwrap_or("?")),
            "bigquery" => "a BigQuery dataset".into(),
            "snowflake" => "a Snowflake database".into(),
            "postgres" | "postgresql" => "a Postgres database".into(),
            "duckdb" => "a DuckDB database".into(),
            "csv" => "CSV files".into(),
            "parquet" => "Parquet files".into(),
            "json" => "JSON files".into(),
            "inline" => "rows written in the YAML".into(),
            other => format!("a {} source", other),
        }
    }
}

use words::label;

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

fn same<T: serde::Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

fn props(t: &TypeDef) -> Vec<(String, Property)> {
    t.properties.iter().map(|(k, v)| (k.clone(), v.normalize())).collect()
}

fn field_json(op: &str, name: &str, p: &Property) -> Json {
    json!({ "op": op, "name": name, "type": words::type_word(p), "description": p.description, "key": p.key })
}

fn binding_words(ont: &Ontology, t: &TypeDef) -> Option<String> {
    let s = t.source.as_ref()?;
    let kind = ont.sources.get(&s.connection).map(|x| words::source_sentence(&x.kind, x.server.as_deref())).unwrap_or_else(|| "a source that is not declared".into());
    let what = if !s.tool.is_empty() { format!("tool {}", s.tool) } else { s.table.clone() };
    Some(format!("rows come from {} ({}){}", s.connection, kind, if what.is_empty() { String::new() } else { format!(", {}", what) }))
}

fn word_in(text: &str, name: &str) -> bool {
    Regex::new(&format!(r"\b{}\b", regex::escape(name))).map(|r| r.is_match(text)).unwrap_or(false)
}

/// Everything in `ont` that mentions entity `name`, as short phrases.
fn uses_of_entity(ont: &Ontology, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for k in ont.kinds_of(name) {
        out.push(format!("the entity {}, a kind of {}", k, name));
    }
    for r in &ont.relationships {
        if r.from == name || r.to == name {
            out.push(format!("the link {} ({} → {})", r.name, r.from, r.to));
        }
    }
    for c in ont.constraints.values() {
        if c.applies_to == name {
            out.push(format!("the guardrail {}", c.id));
        }
    }
    for a in ont.actions.values() {
        if a.inputs.values().any(|t| t == name) {
            out.push(format!("the action {}", a.name));
        }
    }
    for g in &ont.glossary {
        if g.means == name {
            out.push(format!("the also-called word \"{}\"", g.term));
        }
    }
    let bound = ont.type_(name).and_then(|t| t.source.clone());
    for m in ont.metrics.values() {
        let recipe = serde_json::to_string(&(&m.source, &m.variants, &m.sql)).unwrap_or_default();
        let on_table = bound.as_ref().map(|b| m.source.as_ref().map(|s| s.connection == b.connection && s.table == b.table && !b.table.is_empty()).unwrap_or(false)).unwrap_or(false);
        if on_table || word_in(&recipe, name) || word_in(&m.description, name) {
            out.push(format!("the metric {}", m.name));
        }
    }
    for f in ont.functions.values() {
        if f.inputs.values().chain(f.outputs.values()).any(|t| t == name) {
            out.push(format!("the function {}", f.name));
        }
    }
    out
}

/// Everything in `ont` that mentions column `col` of entity `entity`: metric recipes on its table.
fn uses_of_column(ont: &Ontology, entity: &str, col: &str) -> Vec<String> {
    let Some(b) = ont.type_(entity).and_then(|t| t.source.clone()) else { return vec![] };
    let mut out = Vec::new();
    for m in ont.metrics.values() {
        let srcs: Vec<&MetricSource> = m.source.iter().chain(m.variants.iter().map(|v| &v.source)).collect();
        if srcs.iter().any(|s| s.connection == b.connection && s.table == b.table && (word_in(&s.filter, col) || word_in(&s.measure, col))) {
            out.push(format!("the metric {} (its recipe names {})", m.name, col));
        }
    }
    for c in ont.constraints.values() {
        if c.applies_to == entity && word_in(&c.rule, col) {
            out.push(format!("the guardrail {}", c.id));
        }
    }
    out
}

fn recipe_of(m: &MetricDef) -> Json {
    json!({ "source": m.source, "variants": m.variants, "sql": m.sql })
}

/// Before → after when they differ.
fn diff_text(what: &str, a: &str, b: &str) -> Option<Json> {
    (a.trim() != b.trim()).then(|| json!({ "what": what, "before": a, "after": b }))
}

/// The space without the proposal against the space with it, thing by thing.
pub fn compare(before: &Ontology, after: &Ontology) -> Vec<Json> {
    let mut out = Vec::new();

    // --- entities, with renames: a thing removed and an identical one added under a new name ---
    let gone: Vec<&TypeDef> = before.types.values().filter(|t| after.type_(&t.name).is_none()).collect();
    let new: Vec<&TypeDef> = after.types.values().filter(|t| before.type_(&t.name).is_none()).collect();
    let shape = |t: &TypeDef| serde_json::to_string(&(props(t).iter().map(|(n, p)| (n.clone(), p.kind.clone())).collect::<Vec<_>>(), &t.source)).unwrap_or_default();
    let mut renamed: HashMap<String, String> = HashMap::new();
    for g in &gone {
        let matches: Vec<&&TypeDef> = new.iter().filter(|n| shape(n) == shape(g) && !renamed.values().any(|v| v == &n.name)).collect();
        if matches.len() == 1 {
            renamed.insert(g.name.clone(), matches[0].name.clone());
        }
    }
    for t in after.types.values() {
        match before.type_(&t.name) {
            None if renamed.values().any(|v| v == &t.name) => {}
            None => {
                let ps = props(t);
                let key = ps.iter().find(|(_, p)| p.key).map(|(n, _)| n.clone());
                let mut j = json!({
                    "kind": "entity", "op": "add", "name": t.name, "label": label(&t.name),
                    "description": t.description, "key": key,
                    "grain": key.as_ref().map(|k| grain_words(k)),
                    "binding": binding_words(after, t),
                    "fields": ps.iter().filter(|(n, _)| !t.inherited.contains_key(n)).map(|(n, p)| field_json("add", n, p)).collect::<Vec<_>>(),
                    "structural": true,
                });
                // a kind: its own fields listed, its parent's folded into one line
                if let Some(p) = &t.parent {
                    let inh: Vec<String> = t.inherited.keys().cloned().collect();
                    j["kind_of"] = json!(p);
                    j["inherited"] = json!({ "from": p, "count": inh.len(), "names": inh });
                    j["rows"] = json!(after.kind_rows_words(t));
                }
                out.push(j);
            }
            Some(b) if same(b, t) => {}
            Some(b) => out.push(entity_change(before, after, b, t)),
        }
    }
    for (old, newn) in &renamed {
        let refs = uses_of_entity(after, newn).len();
        out.push(json!({ "kind": "entity", "op": "rename", "name": newn, "from": old, "label": label(newn), "sentence": format!("renamed {} → {}", old, newn), "updated": refs, "structural": true }));
    }
    for t in &gone {
        if renamed.contains_key(&t.name) {
            continue;
        }
        out.push(json!({ "kind": "entity", "op": "remove", "name": t.name, "label": label(&t.name), "uses": uses_of_entity(before, &t.name), "structural": true }));
    }

    // --- links, read through any entity rename so a renamed thing's links are not "new" ---
    let ren = |n: &str| renamed.get(n).cloned().unwrap_or_else(|| n.to_string());
    let before_links: BTreeMap<String, &crate::ontology::Relationship> = before.relationships.iter().map(|r| (format!("{}.{}", ren(&r.from), r.name), r)).collect();
    let after_links: BTreeMap<String, &crate::ontology::Relationship> = after.relationships.iter().map(|r| (r.qualified(), r)).collect();
    for (q, r) in &after_links {
        match before_links.get(q) {
            None => out.push(json!({ "kind": "link", "op": "add", "name": q, "label": link_label(&r.from, &r.name, &r.to), "link": r.name, "from": r.from, "to": r.to, "cardinality": r.cardinality, "description": r.description, "structural": true })),
            Some(b) => {
                let mut diffs = Vec::new();
                if ren(&b.to) != r.to {
                    diffs.push(json!({ "what": "points to", "before": b.to, "after": r.to }));
                }
                diffs.extend(diff_text("cardinality", &b.cardinality, &r.cardinality));
                diffs.extend(diff_text("description", &b.description, &r.description));
                let structural = diffs.iter().any(|d| d["what"] != "description");
                if diffs.is_empty() && !(same(&b.properties, &r.properties) && same(&b.source, &r.source)) {
                    diffs.push(json!({ "what": "its fields or rows", "before": "", "after": "changed" }));
                }
                if !diffs.is_empty() {
                    out.push(json!({ "kind": "link", "op": "change", "name": q, "label": link_label(&r.from, &r.name, &r.to), "link": r.name, "from": r.from, "to": r.to, "diffs": diffs, "structural": structural }));
                }
            }
        }
    }
    for (q, r) in &before_links {
        if !after_links.contains_key(q) {
            let uses: Vec<String> = after.constraints.values().filter(|c| c.applies_to == r.name || c.applies_to == r.qualified()).map(|c| format!("the guardrail {}", c.id)).collect();
            out.push(json!({ "kind": "link", "op": "remove", "name": r.qualified(), "label": link_label(&r.from, &r.name, &r.to), "link": r.name, "from": r.from, "to": r.to, "uses": uses, "structural": true }));
        }
    }

    // --- areas: added, renamed, described, deleted, reordered (Part 3, "Review words") ---
    let (bd, ad) = (&before.manifest.domains, &after.manifest.domains);
    let mut area_ren: HashMap<String, String> = HashMap::new();
    for (i, b) in bd.iter().enumerate() {
        if ad.iter().any(|a| a.name == b.name) {
            continue;
        }
        // gone under its name: a rename when an area new under its name sits where it was with the same entities
        if let Some(a) = ad.get(i).filter(|a| !bd.iter().any(|x| x.name == a.name) && a.types == b.types) {
            area_ren.insert(b.name.clone(), a.name.clone());
        }
    }
    // an entity whose `domain:` line only followed its area's new name hasn't moved
    out.retain(|c| {
        let only_area = c["kind"] == "entity" && c["op"] == "change" && c["fields"].as_array().map(|f| f.is_empty()).unwrap_or(false) && c["diffs"].as_array().map(|d| d.len() == 1 && d[0]["what"] == "area").unwrap_or(false);
        !(only_area && area_ren.get(c["diffs"][0]["before"].as_str().unwrap_or("")).map(|n| c["diffs"][0]["after"] == n.as_str()).unwrap_or(false))
    });
    for a in ad {
        match bd.iter().find(|b| b.name == a.name) {
            Some(b) => {
                let diffs: Vec<Json> = diff_text("description", &b.description, &a.description).into_iter().collect();
                if !diffs.is_empty() {
                    out.push(json!({ "kind": "area", "op": "change", "name": a.name, "label": a.name, "diffs": diffs, "sentence": format!("Describes the area {}.", a.name), "summary": "described differently" }));
                }
            }
            None => match area_ren.iter().find(|(_, n)| *n == &a.name) {
                Some((old, _)) => {
                    let iri = after.iri_domain(&a.name);
                    let short = iri.rsplit_once('/').map(|(_, t)| format!("…/{}", t)).unwrap_or(iri.clone());
                    out.push(json!({ "kind": "area", "op": "rename", "name": a.name, "from": old, "label": a.name, "sentence": format!("Renames the area {} to {} (its IRI becomes {}).", old, a.name, short), "summary": format!("renamed from {}", old) }));
                }
                None => out.push(json!({ "kind": "area", "op": "add", "name": a.name, "label": a.name, "description": a.description, "sentence": format!("Adds the area {}.", a.name) })),
            },
        }
    }
    for b in bd {
        if !ad.iter().any(|a| a.name == b.name) && !area_ren.contains_key(&b.name) {
            let empty = !before.types.values().any(|t| before.domain_of(&t.name).as_deref() == Some(b.name.as_str()));
            out.push(json!({ "kind": "area", "op": "remove", "name": b.name, "label": b.name, "sentence": if empty { format!("Deletes the empty area {}.", b.name) } else { format!("Deletes the area {}.", b.name) } }));
        }
    }
    let kept_b: Vec<String> = bd.iter().map(|d| area_ren.get(&d.name).cloned().unwrap_or_else(|| d.name.clone())).filter(|n| ad.iter().any(|a| &a.name == n)).collect();
    let kept_a: Vec<String> = ad.iter().map(|d| d.name.clone()).filter(|n| kept_b.contains(n)).collect();
    if kept_b != kept_a {
        out.push(json!({ "kind": "area", "op": "change", "name": "", "label": "The areas", "sentence": "Reorders the areas.", "summary": "reordered" }));
    }
    // an entity moved by an area's list only: its file is the same, its area isn't
    for t in after.types.values() {
        let Some(b) = before.type_(&t.name) else { continue };
        let (was, now) = (before.domain_of(&b.name).unwrap_or_else(|| "Other".into()), after.domain_of(&t.name).unwrap_or_else(|| "Other".into()));
        let renamed_area = area_ren.get(&was) == Some(&now);
        if same(b, t) && was != now && !renamed_area {
            out.push(json!({ "kind": "entity", "op": "change", "name": t.name, "label": label(&t.name), "fields": [], "diffs": [{ "what": "area", "before": was, "after": now }], "sentence": format!("Moves {} from {} to {}.", t.name, was, now), "summary": format!("moves from {} to {}", was, now), "structural": false }));
        }
    }

    // --- metrics ---
    let mgone: Vec<&MetricDef> = before.metrics.values().filter(|m| !after.metrics.contains_key(&m.name)).collect();
    let mnew: Vec<&MetricDef> = after.metrics.values().filter(|m| !before.metrics.contains_key(&m.name)).collect();
    let mut mren: HashMap<String, String> = HashMap::new();
    for g in &mgone {
        let ms: Vec<&&MetricDef> = mnew.iter().filter(|n| recipe_of(n) == recipe_of(g)).collect();
        if ms.len() == 1 {
            mren.insert(g.name.clone(), ms[0].name.clone());
        }
    }
    for m in after.metrics.values() {
        match before.metrics.get(&m.name) {
            None if mren.values().any(|v| v == &m.name) => {}
            None => {
                let (sentence, as_written) = words::metric_sentence(m, after);
                let from_user = m.source.iter().chain(m.variants.iter().map(|v| &v.source)).any(|s| after.sources.get(&s.connection).map(|x| learn::is_from_user(&x.kind)).unwrap_or(false));
                out.push(json!({
                    "kind": "metric", "op": "add", "name": m.name, "label": label(&m.name), "sentence": sentence, "as_written": as_written,
                    "description": m.description, "grain": m.grain, "owner": m.owner, "basis": m.basis.map(|b| b.word()), "caveats": m.caveats,
                    "from": m.source.as_ref().map(|s| format!("{}.{}", s.connection, if s.tool.is_empty() { &s.table } else { &s.tool })),
                    "preview": if from_user { Json::from("Lorefile can't preview this metric: the rows stay with the agent.") } else { Json::Null },
                }));
            }
            Some(b) if same(b, m) => {}
            Some(b) => {
                let mut diffs = Vec::new();
                let recipe_changed = recipe_of(b) != recipe_of(m);
                if recipe_changed {
                    let (sb, wb) = words::metric_sentence(b, before);
                    let (sa, wa) = words::metric_sentence(m, after);
                    let (sb, sa) = if sb == sa { (wb.clone().unwrap_or(sb), wa.clone().unwrap_or(sa)) } else { (sb, sa) };
                    diffs.push(json!({ "what": "how it is counted", "before": sb, "after": sa }));
                }
                diffs.extend(diff_text("description", &b.description, &m.description));
                diffs.extend(diff_text("owner", &b.owner, &m.owner));
                diffs.extend(diff_text("grain", &b.grain, &m.grain));
                diffs.extend(diff_text("caveats", &b.caveats, &m.caveats));
                diffs.extend(diff_text("basis", b.basis.map(|x| x.word()).unwrap_or(""), m.basis.map(|x| x.word()).unwrap_or("")));
                let words_only = !recipe_changed;
                out.push(json!({ "kind": "metric", "op": "change", "name": m.name, "label": label(&m.name), "diffs": diffs, "recipe_changed": recipe_changed, "sentence": if words_only { Json::from("described differently") } else { Json::Null } }));
            }
        }
    }
    for (old, newn) in &mren {
        out.push(json!({ "kind": "metric", "op": "rename", "name": newn, "from": old, "label": label(newn), "sentence": format!("renamed {} → {} (same recipe)", old, newn) }));
    }
    for m in &mgone {
        if !mren.contains_key(&m.name) {
            out.push(json!({ "kind": "metric", "op": "remove", "name": m.name, "label": label(&m.name), "sentence": "Agents can no longer quote it." }));
        }
    }

    // --- rules ---
    for c in after.constraints.values() {
        let sev = c.severity.to_string();
        match before.constraints.get(&c.id) {
            None => out.push(json!({ "kind": "rule", "op": "add", "name": c.id, "label": if c.name.is_empty() { c.id.clone() } else { c.name.clone() }, "applies_to": ren(&c.applies_to), "sentence": words::rule_sentence(&c.rule), "as_written": c.rule, "severity": words::severity_word(&sev), "rationale": c.rationale })),
            Some(b) if same(b, c) => {}
            Some(b) => {
                let mut diffs = Vec::new();
                if b.rule.trim() != c.rule.trim() {
                    diffs.push(json!({ "what": "the rule", "before": words::rule_sentence(&b.rule).unwrap_or_else(|| b.rule.clone()), "after": words::rule_sentence(&c.rule).unwrap_or_else(|| c.rule.clone()) }));
                }
                if b.severity != c.severity {
                    diffs.push(json!({ "what": "what it does", "before": words::severity_word(&b.severity.to_string()), "after": words::severity_word(&sev) }));
                }
                if ren(&b.applies_to) != c.applies_to {
                    diffs.push(json!({ "what": "applies to", "before": b.applies_to, "after": c.applies_to }));
                }
                diffs.extend(diff_text("name", &b.name, &c.name));
                diffs.extend(diff_text("why", &b.rationale, &c.rationale));
                diffs.extend(diff_text("description", &b.description, &c.description));
                diffs.extend(diff_text("owner", &b.owner, &c.owner));
                if !diffs.is_empty() {
                    out.push(json!({ "kind": "rule", "op": "change", "name": c.id, "label": if c.name.is_empty() { c.id.clone() } else { c.name.clone() }, "applies_to": c.applies_to, "diffs": diffs }));
                }
            }
        }
    }
    for b in before.constraints.values() {
        if !after.constraints.contains_key(&b.id) {
            let verb = match b.severity.to_string().as_str() {
                "block" => "stopped",
                "warn" => "warned",
                _ => "told",
            };
            out.push(json!({ "kind": "rule", "op": "remove", "name": b.id, "label": if b.name.is_empty() { b.id.clone() } else { b.name.clone() }, "applies_to": b.applies_to, "sentence": format!("agents will no longer be {} by {}", verb, b.id) }));
        }
    }

    // --- words ---
    let gl = |o: &Ontology| -> BTreeMap<String, crate::schema::GlossaryTerm> { o.glossary.iter().map(|g| (g.term.to_lowercase(), g.clone())).collect() };
    let (gb, ga) = (gl(before), gl(after));
    for (k, g) in &ga {
        match gb.get(k) {
            None => out.push(json!({ "kind": "word", "op": "add", "name": g.term, "label": g.term, "means": g.means, "sentence": format!("people say \"{}\" for {}", g.term, label(&g.means)), "description": g.description })),
            Some(b) if same(b, g) => {}
            Some(b) => {
                let mut diffs = Vec::new();
                if ren(&b.means) != g.means {
                    diffs.push(json!({ "what": "what it means", "before": label(&b.means), "after": label(&g.means) }));
                }
                diffs.extend(diff_text("description", &b.description, &g.description));
                if !diffs.is_empty() {
                    out.push(json!({ "kind": "word", "op": "change", "name": g.term, "label": g.term, "diffs": diffs }));
                }
            }
        }
    }
    for (k, b) in &gb {
        if !ga.contains_key(k) {
            out.push(json!({ "kind": "word", "op": "remove", "name": b.term, "label": b.term, "sentence": format!("\"{}\" will no longer mean {}", b.term, label(&b.means)) }));
        }
    }

    // --- sources ---
    for s in after.sources.values() {
        let sentence = words::source_sentence(&s.kind, s.server.as_deref());
        match before.sources.get(&s.connection) {
            None => out.push(json!({ "kind": "source", "op": "add", "name": s.connection, "label": s.connection, "sentence": sentence, "description": s.description })),
            Some(b) if same(b, s) => {}
            Some(b) => {
                let mut diffs = Vec::new();
                diffs.extend(diff_text("kind", &words::source_sentence(&b.kind, b.server.as_deref()), &sentence));
                diffs.extend(diff_text("description", &b.description, &s.description));
                diffs.extend(diff_text("path", b.path.as_deref().unwrap_or(""), s.path.as_deref().unwrap_or("")));
                if !same(&b.config, &s.config) {
                    diffs.push(json!({ "what": "settings", "before": serde_json::to_string(&b.config).unwrap_or_default(), "after": serde_json::to_string(&s.config).unwrap_or_default() }));
                }
                let graded: Vec<String> = before
                    .types
                    .values()
                    .filter(|t| t.source.as_ref().map(|x| x.connection == s.connection).unwrap_or(false))
                    .map(|t| format!("the rows of {}", t.name))
                    .chain(before.metrics.values().filter(|m| m.source.iter().chain(m.variants.iter().map(|v| &v.source)).any(|x| x.connection == s.connection)).map(|m| format!("the metric {}", m.name)))
                    .collect();
                out.push(json!({ "kind": "source", "op": "change", "name": s.connection, "label": s.connection, "diffs": diffs, "depends": graded }));
            }
        }
    }
    for b in before.sources.values() {
        if !after.sources.contains_key(&b.connection) {
            let bound: Vec<String> = before
                .types
                .values()
                .filter(|t| t.source.as_ref().map(|x| x.connection == b.connection).unwrap_or(false))
                .map(|t| format!("the rows of {}", t.name))
                .chain(before.metrics.values().filter(|m| m.source.iter().chain(m.variants.iter().map(|v| &v.source)).any(|x| x.connection == b.connection)).map(|m| format!("the metric {}", m.name)))
                .collect();
            out.push(json!({ "kind": "source", "op": "remove", "name": b.connection, "label": b.connection, "uses": bound }));
        }
    }

    // --- functions and actions: a name and a sentence ---
    for f in after.functions.values() {
        match before.functions.get(&f.name) {
            None => out.push(json!({ "kind": "function", "op": "add", "name": f.name, "label": label(&f.name), "sentence": f.description })),
            Some(b) if same(b, f) => {}
            Some(b) => {
                let mut diffs: Vec<Json> = diff_text("description", &b.description, &f.description).into_iter().chain(diff_text("owner", &b.owner, &f.owner)).collect();
                let mut rest_b = b.clone();
                rest_b.description = f.description.clone();
                rest_b.owner = f.owner.clone();
                if !same(&rest_b, f) || diffs.is_empty() {
                    diffs.push(json!({ "what": "how it runs, its inputs or outputs", "before": "", "after": "changed" }));
                }
                out.push(json!({ "kind": "function", "op": "change", "name": f.name, "label": label(&f.name), "diffs": diffs }))
            }
        }
    }
    for b in before.functions.values() {
        if !after.functions.contains_key(&b.name) {
            out.push(json!({ "kind": "function", "op": "remove", "name": b.name, "label": label(&b.name), "uses": Vec::<String>::new() }));
        }
    }
    for a in after.actions.values() {
        match before.actions.get(&a.name) {
            None => out.push(json!({ "kind": "action", "op": "add", "name": a.name, "label": label(&a.name), "sentence": a.description })),
            Some(b) if same(b, a) => {}
            Some(b) => out.push(json!({ "kind": "action", "op": "change", "name": a.name, "label": label(&a.name), "diffs": diff_text("description", &b.description, &a.description).map(|d| vec![d]).unwrap_or_else(|| vec![json!({ "what": "its inputs, checks or effects", "before": "", "after": "changed" })]) })),
        }
    }
    for b in before.actions.values() {
        if !after.actions.contains_key(&b.name) {
            let uses: Vec<String> = after.constraints.values().filter(|c| c.applies_to == b.name).map(|c| format!("the guardrail {}", c.id)).collect();
            out.push(json!({ "kind": "action", "op": "remove", "name": b.name, "label": label(&b.name), "uses": uses }));
        }
    }
    out
}

/// "one row per line (key line_id)"; a key that does not name its grain: "one row per code (the key)".
fn grain_words(key: &str) -> String {
    match words::rows_noun(Some(key)).as_str() {
        "rows" => format!("one row per {} (the key)", key),
        noun => format!("one row per {} (key {})", noun.trim_end_matches('s'), key),
    }
}

/// "Customer holds Voucher": a link by its name.
fn link_label(from: &str, name: &str, to: &str) -> String {
    format!("{} {} {}", label(from), name.replace('_', " "), label(to))
}

fn entity_change(before: &Ontology, after: &Ontology, b: &TypeDef, t: &TypeDef) -> Json {
    let (pb, pa) = (props(b), props(t));
    let bmap: HashMap<&str, &Property> = pb.iter().map(|(n, p)| (n.as_str(), p)).collect();
    let amap: HashMap<&str, &Property> = pa.iter().map(|(n, p)| (n.as_str(), p)).collect();
    let mut fields = Vec::new();
    let mut structural = false;
    let added: Vec<&(String, Property)> = pa.iter().filter(|(n, _)| !bmap.contains_key(n.as_str())).collect();
    let removed: Vec<&(String, Property)> = pb.iter().filter(|(n, _)| !amap.contains_key(n.as_str())).collect();
    // a column removed and one added with the same type and description: likely a rename
    let mut paired: HashMap<String, String> = HashMap::new();
    for (rn, rp) in &removed {
        let cands: Vec<&&(String, Property)> = added.iter().filter(|(_, ap)| ap.kind == rp.kind && ap.unit == rp.unit && ap.description.trim() == rp.description.trim() && !rp.description.trim().is_empty()).collect();
        if cands.len() == 1 && !paired.values().any(|v| v == &cands[0].0) {
            paired.insert(rn.clone(), cands[0].0.clone());
        }
    }
    for (n, p) in &pa {
        match bmap.get(n.as_str()) {
            None if paired.values().any(|v| v == n) => {}
            None => {
                structural = true;
                fields.push(field_json("add", n, p));
            }
            Some(o) if same(*o, p) => {}
            Some(o) => {
                let mut diffs = Vec::new();
                diffs.extend(diff_text("type", &words::type_word(o), &words::type_word(p)));
                diffs.extend(diff_text("description", &o.description, &p.description));
                if o.key != p.key {
                    diffs.push(json!({ "what": "key", "before": if o.key { "the key" } else { "not the key" }, "after": if p.key { "the key" } else { "not the key" } }));
                }
                if o.required != p.required {
                    diffs.push(json!({ "what": "required", "before": o.required.to_string(), "after": p.required.to_string() }));
                }
                if diffs.iter().any(|d| d["what"] == "type" || d["what"] == "key") {
                    structural = true;
                }
                fields.push(json!({ "op": "change", "name": n, "type": words::type_word(p), "description": p.description, "diffs": diffs }));
            }
        }
    }
    for (from, to) in &paired {
        structural = true;
        let p = amap.get(to.as_str()).copied().cloned().unwrap_or_default();
        fields.push(json!({ "op": "rename", "name": to, "from": from, "type": words::type_word(&p), "description": p.description, "sentence": format!("looks like a rename of {} → {}", from, to) }));
    }
    for (n, p) in &removed {
        if paired.contains_key(n) {
            continue;
        }
        structural = true;
        let mut f = field_json("remove", n, p);
        f["uses"] = json!(uses_of_column(after, &t.name, n).into_iter().chain(uses_of_column(before, &b.name, n)).fold(Vec::<String>::new(), |mut acc, u| {
            if !acc.contains(&u) {
                acc.push(u);
            }
            acc
        }));
        fields.push(f);
    }
    let mut diffs = Vec::new();
    diffs.extend(diff_text("description", &b.description, &t.description));
    let (bb, ba) = (binding_words(before, b).unwrap_or_else(|| "no rows bound".into()), binding_words(after, t).unwrap_or_else(|| "no rows bound".into()));
    if !same(&b.source, &t.source) {
        structural = true;
        if bb != ba {
            diffs.push(json!({ "what": "where rows come from", "before": bb, "after": ba }));
        } else {
            diffs.push(json!({ "what": "how columns map to the rows", "before": "", "after": "changed" }));
        }
    }
    diffs.extend(diff_text("area", before.domain_of(&b.name).as_deref().unwrap_or("Other"), after.domain_of(&t.name).as_deref().unwrap_or("Other")));
    diffs.extend(diff_text("owner", &b.owner, &t.owner));
    // a kind: "is now a kind of Sale", "now picks Sale's rows where kind is promo"
    let mut kind_said: Vec<String> = Vec::new();
    if b.parent != t.parent {
        structural = true;
        let s = match (&b.parent, &t.parent) {
            (None, Some(p)) => format!("{} is now a kind of {}", label(&t.name), p),
            (Some(p), None) => format!("{} is no longer a kind of {}", label(&t.name), p),
            (Some(a), Some(p)) => format!("{} is now a kind of {} (was {})", label(&t.name), p, a),
            _ => String::new(),
        };
        diffs.push(json!({ "what": "a kind of", "before": b.parent.clone().unwrap_or_default(), "after": t.parent.clone().unwrap_or_default() }));
        kind_said.push(s);
    }
    if b.where_all != t.where_all {
        structural = true;
        let base = t.rows_of.clone().or(t.parent.clone()).unwrap_or_default();
        kind_said.push(if t.where_all.is_empty() { format!("{} no longer picks {}'s rows", label(&t.name), base) } else { format!("{} now picks {}'s rows where {}", label(&t.name), base, crate::ontology::Ontology::where_words(&t.where_all)) });
    }
    // fields that a kind now gets from its parent are not removals
    let now_from: Vec<String> = fields.iter().filter(|f| f["op"] == "remove" && t.inherited.contains_key(f["name"].as_str().unwrap_or(""))).filter_map(|f| f["name"].as_str().map(|x| x.to_string())).collect();
    fields.retain(|f| !(f["op"] == "remove" && now_from.iter().any(|n| f["name"] == n.as_str())));
    let added_n = fields.iter().filter(|f| f["op"] == "add").count();
    let removed_n = fields.iter().filter(|f| f["op"] == "remove").count();
    let changed_n = fields.iter().filter(|f| f["op"] == "change" || f["op"] == "rename").count();
    let mut said = Vec::new();
    if added_n > 0 {
        said.push(format!("{} added", plural(added_n, "field")));
    }
    if changed_n > 0 {
        said.push(format!("{} changed", plural(changed_n, "field")));
    }
    if removed_n > 0 {
        said.push(format!("{} removed", plural(removed_n, "field")));
    }
    if diffs.iter().any(|d| d["what"] == "description") {
        said.push("described differently".into());
    }
    if diffs.iter().any(|d| d["what"] == "where rows come from" || d["what"] == "how columns map to the rows") {
        said.push("rows bound differently".into());
    }
    if let Some(d) = diffs.iter().find(|d| d["what"] == "area") {
        said.push(format!("moves from {} to {}", d["before"].as_str().unwrap_or(""), d["after"].as_str().unwrap_or("")));
    }
    if said.is_empty() && fields.is_empty() && diffs.is_empty() {
        said.push("changed in ways this screen does not list (see the files)".into());
    }
    let mut j = json!({ "kind": "entity", "op": "change", "name": t.name, "label": label(&t.name), "fields": fields, "diffs": diffs, "summary": said.join(", "), "structural": structural });
    if !kind_said.is_empty() {
        j["sentence"] = json!(kind_said.join("; "));
        j["summary"] = json!(format!("{}{}", kind_said.join("; "), if said.is_empty() { String::new() } else { format!("; {}", said.join(", ")) }));
    }
    j
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{} {}{}", n, word, if n == 1 { "" } else { "s" })
}

// ---------------------------------------------------------------------------
// The counts line and the headline
// ---------------------------------------------------------------------------

fn kind_noun(kind: &str, n: usize) -> String {
    let (one, many) = match kind {
        "entity" => ("entity", "entities"),
        "metric" => ("metric", "metrics"),
        "source" => ("source", "sources"),
        "word" => ("also-called word", "also-called words"),
        "rule" => ("guardrail", "guardrails"),
        "link" => ("link", "links"),
        "function" => ("function", "functions"),
        "action" => ("action", "actions"),
        "area" => ("area", "areas"),
        _ => ("change", "changes"),
    };
    format!("{} {}", n, if n == 1 { one } else { many })
}

/// "1 entity (Promo sale, 14 fields)", grouped by kind, for adds, changes and removes.
pub fn counts(changes: &[Json]) -> Json {
    let order = ["area", "entity", "metric", "source", "link", "rule", "word", "function", "action"];
    let mut adds = Vec::new();
    let mut changed = Vec::new();
    let mut removes = Vec::new();
    for k in order {
        let of = |op: &str| changes.iter().filter(|c| c["kind"] == k && (c["op"] == op || (op == "change" && c["op"] == "rename"))).collect::<Vec<_>>();
        let a = of("add");
        if !a.is_empty() {
            let names: Vec<String> = a
                .iter()
                .take(3)
                .map(|c| {
                    let l = c["label"].as_str().unwrap_or("").to_string();
                    match c["fields"].as_array() {
                        Some(f) if k == "entity" => format!("{}, {}", l, plural(f.len(), "field")),
                        _ => l,
                    }
                })
                .collect();
            adds.push(if k == "source" { kind_noun(k, a.len()) } else { format!("{} ({}{})", kind_noun(k, a.len()), names.join("; "), if a.len() > 3 { "; …" } else { "" }) });
        }
        for c in of("change") {
            let l = c["label"].as_str().unwrap_or("");
            let s = c["summary"].as_str().or_else(|| c["sentence"].as_str()).unwrap_or("changed");
            changed.push(format!("{}: {}", l, s));
        }
        let r = of("remove");
        if !r.is_empty() {
            removes.push(format!("{} ({})", kind_noun(k, r.len()), r.iter().take(3).map(|c| c["label"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join("; ")));
        }
    }
    json!({ "adds": adds, "changes": changed, "removes": removes })
}

/// `agent via MCP (connection "ChatGPT", approved 27 Nov)` → "ChatGPT"; any other agent → "An agent".
pub fn agent_name(via: &str) -> String {
    if let Some(rest) = via.split("connection \"").nth(1) {
        if let Some(name) = rest.split('"').next() {
            if !name.trim().is_empty() {
                return name.trim().to_string();
            }
        }
    }
    "An agent".into()
}

/// How it came: "asked in a chat with ChatGPT", "from an agent on this machine".
fn how_words(via: &str) -> String {
    let who = agent_name(via);
    if who != "An agent" {
        format!("asked in a chat with {}", who)
    } else if via.contains("via API") {
        "from the page's own tool runner".into()
    } else {
        "from an agent over MCP".into()
    }
}

/// Who wants what, from the comparison.
pub fn headline(p: &Proposal, changes: &[Json]) -> String {
    let who = agent_name(&p.via);
    let suggests = if who == "An agent" { "An agent suggests".to_string() } else { format!("{} suggests", who) };
    let wants = if who == "An agent" { "An agent wants".to_string() } else { format!("{} wants", who) };
    if let Some(l) = &p.learned {
        if changes.iter().any(|c| c["kind"] == "entity" && c["op"] == "add") {
            return format!("{} Lorefile to remember a file", wants);
        }
        if changes.iter().any(|c| c["kind"] == "entity" && c["name"] == l.entity.as_str()) {
            return format!("{} to update {}", wants, label(&l.entity));
        }
        if changes.iter().any(|c| c["kind"] == "metric" && c["op"] == "add") {
            return format!("{} to say which number counts in {}", wants, if l.file_name.is_empty() { "a file".to_string() } else { l.file_name.clone() });
        }
    }
    if changes.is_empty() {
        return format!("{} a change that changes nothing in meaning", suggests);
    }
    if let Some(r) = changes.iter().find(|c| c["op"] == "remove") {
        return format!("{} to remove {}", wants, r["label"].as_str().unwrap_or("something"));
    }
    if let Some(r) = changes.iter().find(|c| c["op"] == "rename") {
        return format!("{} to rename {} to {}", wants, r["from"].as_str().unwrap_or(""), r["name"].as_str().unwrap_or(""));
    }
    if changes.len() == 1 {
        let c = &changes[0];
        let l = c["label"].as_str().unwrap_or("");
        return match (c["kind"].as_str().unwrap_or(""), c["op"].as_str().unwrap_or("")) {
            ("rule", "add") => format!("{} a new guardrail for {}", suggests, label(c["applies_to"].as_str().unwrap_or(""))),
            ("rule", _) => format!("{} to change the guardrail {}", wants, l),
            ("metric", "add") => format!("{} a new metric: {}", suggests, l),
            ("metric", _) if c["recipe_changed"] == true => format!("{} to change how {} is counted", wants, l),
            ("metric", _) => format!("{} to describe {} differently", wants, l),
            ("entity", "add") => format!("{} a new entity: {}", suggests, l),
            ("entity", _) => format!("{} to change {}", wants, l),
            ("word", "add") => format!("{} an also-called word: \"{}\"", suggests, l),
            ("word", _) => format!("{} to change what \"{}\" means", wants, l),
            ("source", "add") => format!("{} a new source: {}", suggests, l),
            ("source", _) => format!("{} to change the source {}", wants, l),
            ("link", "add") => format!("{} a link: {}", suggests, l),
            ("link", _) => format!("{} to change the link {}", wants, l),
            ("function", _) => format!("{} a change to the function {}", suggests, l),
            ("action", _) => format!("{} a change to the action {}", suggests, l),
            _ => format!("{} a change", suggests),
        };
    }
    let labels: Vec<String> = changes.iter().filter(|c| c["kind"] != "source").take(3).map(|c| c["label"].as_str().unwrap_or("").to_string()).collect();
    format!("{} {}: {}{}", suggests, plural(changes.len(), "change"), labels.join(", "), if changes.len() > 3 { ", …" } else { "" })
}

// ---------------------------------------------------------------------------
// Warnings in words
// ---------------------------------------------------------------------------

fn first_name(message: &str) -> String {
    message.split('`').nth(1).unwrap_or("").to_string()
}

fn is_new(d: &Diag, before: &Report) -> bool {
    !before.diags.iter().any(|b| b.code == d.code && b.message == d.message)
}

/// The diagnostics this proposal adds, as questions (with a fix where the screen offers one),
/// plus the errors, and how many warnings the space already had.
pub fn questions(before_report: &Report, after: &Ontology, after_report: &Report, p: &Proposal) -> (Vec<Json>, Vec<Json>, usize) {
    let mut qs = Vec::new();
    let mut errors = Vec::new();
    for d in after_report.diags.iter().filter(|d| is_new(d, before_report)) {
        if d.level == Level::Error {
            errors.push(json!({ "code": d.code, "text": d.message, "file": d.file }));
            continue;
        }
        // what the warning is about comes from its subject; a field's name is still in the message
        let n = match (&d.subject, d.code) {
            (Some(s), "W109" | "W101" | "W102" | "W103" | "W107" | "W111") => s.name.clone(),
            _ => first_name(&d.message),
        };
        let about = d.subject.as_ref().map(|s| s.kind).unwrap_or("");
        let q = match d.code {
            "W109" => json!({ "code": d.code, "ask": format!("Who decides what {} means?", label(&n)), "name": n, "fix": { "op": "set_metric", "metric": n, "field": "owner" } }),
            "W101" if about == "entity" => json!({ "code": d.code, "ask": format!("What is one {}?", label(&n)), "name": n, "fix": { "op": "set_type_description", "type": n, "field": "description" } }),
            "W101" if about == "metric" => json!({ "code": d.code, "ask": format!("What does {} measure?", label(&n)), "name": n, "fix": { "op": "set_metric", "metric": n, "field": "description" } }),
            "W101" => json!({ "code": d.code, "text": format!("{} has no description, so agents have only its name to go on.", n) }),
            "W103" => json!({ "code": d.code, "text": format!("No field of {} says which row is which, so agents cannot look one up.", label(&n)) }),
            "W102" => json!({ "code": d.code, "text": format!("The guardrail {} stops actions but doesn't say why.", n) }),
            "W105" => json!({ "code": d.code, "text": format!("{} is \"one of\" but lists none.", n) }),
            "W106" => json!({ "code": d.code, "text": format!("Lorefile can't find {}, so rows can't be read here.", d.message.rsplit(" at ").next().unwrap_or("the file")) }),
            "W107" => json!({ "code": d.code, "text": format!("{} links the wrong way round for its inputs.", n) }),
            "W108" => json!({ "code": d.code, "text": format!("\"{}\" points at {}, which doesn't exist.", n, d.message.split('`').nth(3).unwrap_or("")) }),
            "W111" => json!({ "code": d.code, "text": format!("Nobody can run {}: it doesn't say where.", n) }),
            // said twice: usually Not now (the agent should reuse what exists), or different on purpose with a why
            "W115" | "W116" | "W117" => match &d.pair {
                Some([a, b]) => json!({ "code": d.code, "text": d.message, "ask": "Accept anyway: they are different on purpose", "fix": { "op": "distinct", "a": a, "b": b } }),
                None => json!({ "code": d.code, "text": d.message }),
            },
            _ => json!({ "code": d.code, "text": d.message }),
        };
        qs.push(q);
    }
    // a learned file with no number: which one counts
    if let Some(l) = &p.learned {
        if let Some(t) = after.type_(&l.entity) {
            if let Some(src) = &t.source {
                let has_metric = after.metrics.values().any(|m| m.source.iter().chain(m.variants.iter().map(|v| &v.source)).any(|s| s.connection == src.connection && s.table == src.table));
                if !has_metric {
                    let ps = props(t);
                    let numbers: Vec<Json> = ps.iter().filter(|(_, p)| p.kind == "number" || p.kind == "integer").filter(|(_, p)| !p.key).map(|(n, p)| json!({ "name": n, "type": words::type_word(p) })).collect();
                    let flags: Vec<String> = ps.iter().filter(|(_, p)| p.kind == "boolean").map(|(n, _)| n.clone()).collect();
                    let file = if l.file_name.is_empty() { label(&l.entity) } else { l.file_name.clone() };
                    qs.push(json!({
                        "code": "which_number", "ask": format!("Which number in {} is the result?", file), "name": l.entity,
                        "fix": { "op": "add_metric", "entity": l.entity, "numbers": numbers, "flags": flags, "key": t.key_property(), "metric_prefix": snake(&l.entity) },
                    }));
                }
            }
        }
    }
    let other = before_report.diags.iter().filter(|d| d.level == Level::Warning).count();
    (qs, errors, other)
}

/// A fix the person filled in on the screen, turned into the op it stands for, if the screen offered it.
/// `questions` is what the review offered for this proposal; anything else is refused.
pub fn fix_op(fix: &Json, questions: &[Json], after: &Ontology) -> Result<(Json, String), String> {
    let op = fix["op"].as_str().unwrap_or("");
    let allowed = |field: &str, key: &str, name: &str| questions.iter().any(|q| q["fix"]["op"] == op && q["fix"]["field"] == field && q["fix"][key] == name);
    let text = |k: &str| fix[k].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let keys: Vec<&str> = fix.as_object().map(|o| o.keys().map(|k| k.as_str()).collect()).unwrap_or_default();
    let only = |allowed: &[&str]| -> Result<(), String> {
        match keys.iter().find(|k| !allowed.contains(k)) {
            Some(k) => Err(format!("this screen does not offer `{}` here; it offers {}", k, allowed.iter().filter(|a| **a != "op").copied().collect::<Vec<_>>().join(", "))),
            None => Ok(()),
        }
    };
    match op {
        "distinct" => {
            only(&["op", "a", "b", "why"])?;
            let (a, b) = (fix["a"].as_str().unwrap_or(""), fix["b"].as_str().unwrap_or(""));
            if !questions.iter().any(|q| q["fix"]["op"] == "distinct" && ((q["fix"]["a"] == a && q["fix"]["b"] == b) || (q["fix"]["a"] == b && q["fix"]["b"] == a))) {
                return Err("this screen did not ask whether those two are different".into());
            }
            let side = crate::trust::side_words;
            let why = text("why").unwrap_or_default();
            let _ = after;
            Ok((fix.clone(), format!("{} and {} are different on purpose{}", side(a), side(b), if why.is_empty() { String::new() } else { format!(": \"{}\"", why) })))
        }
        "set_metric" => {
            only(&["op", "metric", "owner", "description"])?;
            let metric = fix["metric"].as_str().unwrap_or("");
            let mut out = json!({ "op": "set_metric", "metric": metric });
            let mut said = Vec::new();
            for field in ["owner", "description"] {
                if let Some(v) = text(field) {
                    if !allowed(field, "metric", metric) {
                        return Err(format!("this screen did not offer to set the {} of {}", field, metric));
                    }
                    if field == "owner" && (v.contains('@') && v.contains(' ')) {
                        return Err("an owner is a name or a team, like Vinay or finance@".into());
                    }
                    out[field] = json!(v);
                    said.push(if field == "owner" { format!("owner of {} set to {}", metric, v) } else { format!("described {}", metric) });
                }
            }
            if said.is_empty() {
                return Err("the fix is empty".into());
            }
            Ok((out, said.join("; ")))
        }
        "set_type_description" => {
            only(&["op", "type", "description"])?;
            let t = fix["type"].as_str().unwrap_or("");
            if !allowed("description", "type", t) {
                return Err(format!("this screen did not offer to describe {}", t));
            }
            let d = text("description").ok_or("the description is empty")?;
            Ok((json!({ "op": "set_type_description", "type": t, "description": d }), format!("described {}", t)))
        }
        "add_metric" => {
            only(&["op", "entity", "how", "column", "leave_out", "owner"])?;
            let entity = fix["entity"].as_str().unwrap_or("");
            let q = questions.iter().find(|q| q["code"] == "which_number" && q["fix"]["entity"] == entity).ok_or("this screen did not ask which number counts")?;
            let t = after.type_(entity).ok_or_else(|| format!("{} is not in the space as accepted", entity))?;
            let src = t.source.as_ref().ok_or_else(|| format!("{} has no rows bound", entity))?;
            let how = fix["how"].as_str().unwrap_or("sum");
            let column = fix["column"].as_str().unwrap_or("");
            let numbers: Vec<&str> = q["fix"]["numbers"].as_array().map(|a| a.iter().filter_map(|n| n["name"].as_str()).collect()).unwrap_or_default();
            let key = t.key_property();
            let (measure, desc, default_name) = match how {
                "sum" if numbers.contains(&column) => (format!("sum({})", column), format!("Sum of {} in {}", column, entity), format!("{}_{}", snake(entity), column)),
                "count_distinct" if key.as_deref() == Some(column) => (format!("count(distinct {})", column), format!("Number of distinct {} in {}", column, entity), format!("{}_{}_count", snake(entity), column)),
                "count" => ("count(*)".to_string(), format!("Number of rows in {}", entity), format!("{}_count", snake(entity))),
                _ => return Err(format!("pick one of the number columns offered (or count the rows); `{}` with `{}` is not one", column, how)),
            };
            let flags: Vec<&str> = q["fix"]["flags"].as_array().map(|a| a.iter().filter_map(|f| f.as_str()).collect()).unwrap_or_default();
            let mut op = json!({ "op": "add_metric", "metric": default_name, "description": desc, "connection": src.connection, "table": src.table, "measure": measure });
            let mut said = format!("added the number {} ({})", default_name, measure);
            if let Some(flag) = text("leave_out") {
                if !flags.contains(&flag.as_str()) {
                    return Err(format!("`{}` is not one of the yes / no columns offered", flag));
                }
                op["filter"] = json!(format!("{} = false", flag));
                op["caveats"] = json!(format!("Rows where {} is true do not count.", flag));
                said.push_str(&format!(", leaving out rows where {} is true", flag));
            }
            if let Some(owner) = text("owner") {
                op["owner"] = json!(owner);
            }
            Ok((op, said))
        }
        other => Err(format!("`{}` is not a fix this screen offers", other)),
    }
}

// ---------------------------------------------------------------------------
// Replay, fingerprint, comments
// ---------------------------------------------------------------------------

/// Plan an op-made proposal again on today's files.
pub fn replay(ont: &Ontology, r: &Replay) -> Result<patch::BatchPlan, String> {
    match r {
        Replay::Ops { ops } => patch::plan_many(ont, ops),
        Replay::Learn { input } => {
            let parsed = learn::parse(input)?;
            let m = learn::match_shape(ont, &[], &parsed);
            match learn::plan(ont, &parsed, &m)? {
                learn::Plan::Ops(p) => patch::plan_many(ont, &p.ops),
                learn::Plan::Nothing(why) => Err(format!("nothing is left to save: {}", why)),
            }
        }
    }
}

/// What a person saw, as a short hash of the files Accept would write.
pub fn fingerprint(files: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort();
    let mut h = Sha256::new();
    for (f, t) in sorted {
        h.update(f.as_bytes());
        h.update([0u8]);
        h.update(t.as_bytes());
        h.update([0u8]);
    }
    h.finalize().iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

/// How many full-line `#` comments a rewrite drops.
pub fn comments_lost(before: &str, after: &str) -> usize {
    let n = |t: &str| t.lines().filter(|l| l.trim_start().starts_with('#')).count();
    n(before).saturating_sub(n(after))
}

/// The ontology with these texts laid over the folder (`None` deletes a file).
pub fn overlay(root: &Path, files: &[(String, Option<String>)]) -> Option<(Ontology, Report)> {
    let ov: Overrides = files.iter().map(|(f, t)| (f.clone(), t.clone())).collect();
    edit::check_many(root, &ov).ok()
}

/// What Accept would write today.
pub enum Prepared {
    /// The files are as the agent saw them: its texts, as proposed.
    Fresh { files: Vec<(String, String)>, summaries: HashMap<String, String> },
    /// The files moved on, and the ops were planned again on today's texts.
    Replayed { files: Vec<(String, String)>, summaries: HashMap<String, String>, changed: Vec<String> },
    /// The files moved on and it cannot be made again: why, in words.
    Stale { why: String },
}

pub fn prepare(ont: &Ontology, p: &Proposal) -> Prepared {
    let parts = p.parts();
    let changed: Vec<String> = parts.iter().filter(|(f, _, before)| std::fs::read_to_string(ont.root.join(f)).unwrap_or_default() != *before).map(|(f, _, _)| f.clone()).collect();
    let own: HashMap<String, String> = if p.files.is_empty() { [(p.file.clone(), p.summary.clone())].into_iter().collect() } else { p.files.iter().map(|f| (f.file.clone(), f.summary.clone())).collect() };
    if changed.is_empty() {
        return Prepared::Fresh { files: parts.into_iter().map(|(f, t, _)| (f, t)).collect(), summaries: own };
    }
    let Some(r) = &p.replay else {
        return Prepared::Stale { why: format!("{} changed since this was proposed, and this proposal replaces the whole file, so accepting it would undo that. Set it aside and ask the agent to propose it again from the current file.", changed.join(" and ")) };
    };
    match replay(ont, r) {
        Ok(plan) => {
            let summaries = plan.files.iter().map(|(f, _)| (f.clone(), plan.summary_for(f))).collect();
            Prepared::Replayed { files: plan.files, summaries, changed }
        }
        Err(e) => Prepared::Stale { why: format!("{} changed since this was proposed, and it no longer applies to today's files: {}", changed.join(" and "), e) },
    }
}

// ---------------------------------------------------------------------------
// The review screen's answer
// ---------------------------------------------------------------------------

pub struct Ctx<'a> {
    /// The server runs with `--allow-edit`.
    pub editable: bool,
    /// Accept becomes a commit (history on for this space).
    pub history_on: bool,
    pub space: &'a str,
    /// Every proposal of the space, to find the one this replaced.
    pub all: &'a [Proposal],
}

const READ_ONLY: &str = "This Lorefile server is read-only (started without --allow-edit), so nobody can accept here. Restart it with --allow-edit.";

/// The texts of a proposal laid over today's space, before and after: what it changes, whatever its state.
fn own_texts(ont: &Ontology, p: &Proposal) -> Option<(Ontology, Ontology)> {
    let parts = p.parts();
    // an empty text means the file did not exist when it was proposed (an empty file is never written)
    let before: Vec<(String, Option<String>)> = parts.iter().map(|(f, _, b)| (f.clone(), if b.is_empty() { None } else { Some(b.clone()) })).collect();
    let after: Vec<(String, Option<String>)> = parts.iter().map(|(f, t, _)| (f.clone(), Some(t.clone()))).collect();
    Some((overlay(&ont.root, &before)?.0, overlay(&ont.root, &after)?.0))
}

/// The gradable things a proposal adds or changes (a metric, an entity's source, an also-called
/// word, a guardrail): what its Accept graded, for "Put your name on it".
pub fn graded_by(ont: &Ontology, p: &Proposal) -> Vec<(crate::trust::Kind, String)> {
    let Some((before, after)) = own_texts(ont, p) else { return vec![] };
    let mut out = Vec::new();
    for k in crate::trust::Kind::ALL {
        let was = crate::trust::things(&before, k);
        for (n, v) in crate::trust::things(&after, k) {
            if was.get(&n).map(|o| crate::trust::fingerprint(o) != crate::trust::fingerprint(&v)).unwrap_or(true) {
                out.push((k, n));
            }
        }
    }
    out
}

const KIND_NOUN: [(&str, &str); 9] = [("area", "the area"), ("entity", "the entity"), ("metric", "the metric"), ("source", "the source"), ("rule", "the guardrail"), ("function", "the function"), ("action", "the action"), ("link", "the link"), ("word", "the also-called word")];

/// The whole answer of `GET /api/proposals/<id>/review`.
pub fn review(ont: &Ontology, report: &Report, p: &Proposal, ctx: &Ctx) -> Json {
    let who = agent_name(&p.via);
    let mut j = json!({
        "id": p.id, "status": p.status, "space": ctx.space,
        "context": { "agent": who, "how": how_words(&p.via), "via": p.via, "at": p.created_at, "file_name": p.learned.as_ref().map(|l| l.file_name.clone()), "says": p.reason },
        "decided": { "accepted_by": p.accepted_by.as_deref().map(Proposal::who), "decided_by": p.decided_by.as_deref().map(Proposal::who), "why": p.why, "taken_back_by": p.taken_back_by.as_deref().map(Proposal::who), "replaced_by": p.replaced_by, "at": p.resolved_at, "fixes": p.fixes, "state": p.state_words() },
    });
    if let Some(c) = &p.check {
        j["headline"] = json!(format!("{} reports a check of {} {} against {}", who, c.kind, c.name, c.against));
        j["changes"] = json!([]);
        j["check"] = json!(c);
        j["can"] = json!({ "accept": ctx.editable && p.status == "pending", "dismiss": ctx.editable && p.status == "pending", "why_not": if ctx.editable { Json::Null } else { json!(READ_ONLY) } });
        return j;
    }
    let mut notes: Vec<Json> = Vec::new();
    let mut can_accept = ctx.editable && p.status == "pending" && p.ok;
    let mut why_not: Option<String> = if !ctx.editable { Some(READ_ONLY.into()) } else { None };
    let changes: Vec<Json>;
    let after_files: Vec<(String, String)>;
    let summaries: HashMap<String, String>;
    let after_pair: Option<(Ontology, Report)>;
    if p.status == "pending" {
        let prepared = prepare(ont, p);
        match &prepared {
            Prepared::Fresh { files, summaries: s } | Prepared::Replayed { files, summaries: s, .. } => {
                let ov: Vec<(String, Option<String>)> = files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect();
                match overlay(&ont.root, &ov) {
                    Some((a, r)) => {
                        changes = compare(ont, &a);
                        after_pair = Some((a, r));
                    }
                    None => {
                        changes = vec![];
                        after_pair = None;
                        can_accept = false;
                        why_not = why_not.or(Some("The proposed files do not load; set it aside and ask the agent again.".into()));
                    }
                }
                after_files = files.clone();
                summaries = s.clone();
                if let Prepared::Replayed { changed, .. } = &prepared {
                    // what moved on since, and whether the replay gives what the agent meant
                    let since = own_texts(ont, p).map(|(then, _)| compare(&then, ont)).unwrap_or_default();
                    let said: Vec<String> = since.iter().map(|c| format!("{} {}", match c["op"].as_str().unwrap_or("") { "add" => "added", "remove" => "removed", "rename" => "renamed", _ => "changed" }, c["name"].as_str().unwrap_or(""))).collect();
                    let original = own_texts(ont, p).map(|(b, a)| compare(&b, &a)).unwrap_or_default();
                    let same_result = original == changes;
                    let by = last_who(&ont.root, changed);
                    notes.push(json!({ "kind": "replayed", "text": format!(
                        "The space changed since {} proposed this: {} changed{}{}. {}",
                        who, changed.join(" and "),
                        if said.is_empty() { String::new() } else { format!(" ({})", said.join(", ")) },
                        by.map(|b| format!(", by {}", b)).unwrap_or_default(),
                        if same_result { "Nothing in this proposal touches that, so here it is applied to today's files: same result." } else { "Here it is applied to today's files; what it adds now is shown below, and it differs from what was first proposed." }
                    ) }));
                }
            }
            Prepared::Stale { why, .. } => {
                let pair = own_texts(ont, p);
                changes = pair.as_ref().map(|(b, a)| compare(b, a)).unwrap_or_default();
                after_files = vec![];
                summaries = HashMap::new();
                after_pair = None;
                can_accept = false;
                notes.push(json!({ "kind": "stale", "text": why, "copy": format!("The Lorefile files changed since your proposal {}; propose it again from the current files.", p.id) }));
                why_not = why_not.or(Some("This can't be accepted as it is.".into()));
            }
        }
    } else {
        let pair = own_texts(ont, p);
        changes = pair.as_ref().map(|(b, a)| compare(b, a)).unwrap_or_default();
        after_files = p.parts().into_iter().map(|(f, t, _)| (f, t)).collect();
        summaries = if p.files.is_empty() { [(p.file.clone(), p.summary.clone())].into_iter().collect() } else { p.files.iter().map(|f| (f.file.clone(), f.summary.clone())).collect() };
        after_pair = None;
        can_accept = false;
    }
    j["headline"] = json!(headline(p, &changes));
    j["counts"] = counts(&changes);
    j["map"] = json!(changes.iter().any(|c| (c["kind"] == "entity" && c["structural"] == true) || c["kind"] == "link"));

    // warnings in words: only what this proposal adds
    let (mut qs, errors, other) = match &after_pair {
        Some((a, r)) => questions(report, a, r, p),
        None => (vec![], vec![], report.diags.iter().filter(|d| d.level == Level::Warning).count()),
    };
    // the rule of three: a suggestion, not a problem, when this proposal makes the third
    if let Some((a, _)) = &after_pair {
        let added: Vec<&str> = changes.iter().filter(|c| c["kind"] == "entity" && c["op"] == "add").filter_map(|c| c["name"].as_str()).collect();
        let advice: Vec<crate::validate::Advice> = crate::validate::advise(a).into_iter().filter(|x| x.entities.iter().any(|e| added.contains(&e.as_str()))).collect();
        if !advice.is_empty() {
            j["advice"] = json!(advice);
        }
    }
    if !errors.is_empty() {
        can_accept = false;
        why_not = why_not.or(Some("This can't be accepted as it is: it leaves errors in the space.".into()));
    }
    if !p.ok && p.status == "pending" && errors.is_empty() {
        // an old proposal stored with errors: its own words
        for d in &p.diagnostics {
            if d["level"] == "error" {
                qs.push(json!({ "code": d["code"], "text": d["message"] }));
            }
        }
        why_not = why_not.or(Some("This was stored with errors, so it can't be accepted. Set it aside and ask the agent again.".into()));
    }
    j["questions"] = json!(qs);
    j["errors"] = json!(errors);
    j["other_warnings"] = json!(other);

    // grades on the screen: a change to something a person stands behind
    if let Some((a, _)) = &after_pair {
        for k in crate::trust::Kind::ALL {
            let now = crate::trust::things(a, k);
            for (name, old) in crate::trust::things(ont, k) {
                let st = ont.graded.get(k, &name);
                let Some(g) = st.grade.filter(|g| *g >= Grade::Accepted) else { continue };
                let touched = now.get(&name).map(|v| crate::trust::fingerprint(v) != crate::trust::fingerprint(&old)).unwrap_or(true);
                if touched {
                    let by = st.record.as_ref().map(|r| crate::trust::who_said(&r.by)).unwrap_or_else(|| "a person".into());
                    // only its owner changes: who stands behind it stays (decision 9)
                    if now.get(&name).map(|v| crate::trust::owner_only(&old, v)).unwrap_or(false) {
                        notes.push(json!({ "kind": "grade", "text": format!("{} is {} by {} and stays so: only its owner changes.", label(&name), g.word(), by) }));
                        continue;
                    }
                    let text = if g == Grade::Checked {
                        format!("{} is checked by {}. After you accept, it is accepted, not checked, until someone checks it again.", label(&name), by)
                    } else {
                        format!("{} is accepted by {}. After you accept, it is accepted by you instead.", label(&name), by)
                    };
                    notes.push(json!({ "kind": "grade", "text": text }));
                }
            }
        }
    }
    // a name nobody here has used yet: a note, not a question
    if let Some((a, _)) = &after_pair {
        let known = crate::people::derive(ont, &[]);
        let mut new_names: Vec<String> = Vec::new();
        for (_, _, who, _) in crate::people::owners(a) {
            if known.find(&who).is_none() && !new_names.iter().any(|n| crate::people::fold(n) == crate::people::fold(&who)) {
                new_names.push(who);
            }
        }
        for n in new_names {
            notes.push(json!({ "kind": "new_name", "text": format!("New name: nobody here is called {} yet.", crate::people::shown(&n)) }));
        }
    }
    // proposals stored before 2026-11-27 headed each file by position (a bug): say what each file holds from the comparison instead
    let derived: HashMap<String, String> = if p.replay.is_none() && p.files.len() > 1 {
        let owned = if after_pair.is_none() { own_texts(ont, p).map(|(_, a)| a) } else { None };
        let after: Option<&Ontology> = after_pair.as_ref().map(|(a, _)| a).or(owned.as_ref());
        let mut by_file: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(a) = after {
            for c in &changes {
                let n = c["name"].as_str().unwrap_or("");
                let file = match c["kind"].as_str().unwrap_or("") {
                    "entity" => a.type_(n).map(|t| t.file.clone()),
                    "metric" => a.metrics.get(n).map(|m| m.file.clone()),
                    "source" => a.sources.get(n).map(|s| s.file.clone()),
                    "rule" => a.constraints.get(n).map(|r| r.file.clone()),
                    "function" => a.functions.get(n).map(|f| f.file.clone()),
                    "action" => a.actions.get(n).map(|x| x.file.clone()),
                    "link" => a.relationships.iter().find(|r| r.qualified() == n).map(|r| r.file.clone()),
                    _ => None,
                };
                if let Some(f) = file {
                    let verb = match c["op"].as_str().unwrap_or("") { "add" => "adds", "remove" => "removes", "rename" => "renames", _ => "changes" };
                    by_file.entry(f).or_default().push(format!("{} {} {}", verb, KIND_NOUN.iter().find(|(k, _)| *k == c["kind"].as_str().unwrap_or("")).map(|(_, w)| *w).unwrap_or("the item"), n));
                }
            }
        }
        by_file.into_iter().map(|(f, v)| (f, v.join("; "))).collect()
    } else {
        HashMap::new()
    };
    let summaries: HashMap<String, String> = summaries.into_iter().map(|(f, s)| { let d = derived.get(&f).cloned(); (f, d.unwrap_or(s)) }).collect();
    // comments a rewrite would drop
    let mut files_out = Vec::new();
    for (f, t) in &after_files {
        let before_text = std::fs::read_to_string(ont.root.join(f)).unwrap_or_default();
        let then = p.parts().into_iter().find(|(pf, _, _)| pf == f).map(|(_, _, b)| b).unwrap_or_default();
        let base = if p.status == "pending" { before_text } else { then };
        let lost = comments_lost(&base, t);
        if lost > 0 {
            notes.push(json!({ "kind": "comments", "text": format!("{} would lose {} (Lorefile rewrites these files and does not keep comments yet).", f, plural(lost, "comment")) }));
        }
        files_out.push(json!({ "file": f, "summaries": summaries.get(f).map(|s| s.split("; ").map(|x| x.to_string()).collect::<Vec<_>>()).unwrap_or_default(), "before": base, "after": t, "comments_lost": lost }));
    }
    j["files"] = json!(files_out);
    if p.status == "pending" && !after_files.is_empty() {
        j["fingerprint"] = json!(fingerprint(&after_files));
    }
    // replaced: what this one does differently from the version it replaced (while it waits: after, today's files say it)
    if let Some(old) = ctx.all.iter().find(|o| o.replaced_by.as_deref() == Some(p.id.as_str())).filter(|_| p.status == "pending") {
        let mine = overlay(&ont.root, &after_files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect::<Vec<_>>());
        let theirs = overlay(&ont.root, &old.parts().into_iter().map(|(f, t, _)| (f, Some(t))).collect::<Vec<_>>());
        if let (Some((m, _)), Some((o, _))) = (mine, theirs) {
            let d = compare(&o, &m);
            let c = counts(&d);
            let mut said: Vec<String> = Vec::new();
            for (k, w) in [("adds", "adds"), ("changes", "changes"), ("removes", "removes")] {
                if let Some(a) = c[k].as_array().filter(|a| !a.is_empty()) {
                    said.push(format!("{} {}", w, a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")));
                }
            }
            notes.push(json!({ "kind": "replaces", "id": old.id, "at": old.created_at, "text": if said.is_empty() { "Replaces an earlier version from the same chat; the two say the same.".to_string() } else { format!("Replaces an earlier version from the same chat: this one {}.", said.join("; ")) } }));
        }
    }
    // taking it back
    if p.status == "pending" {
        notes.push(json!({ "kind": "undo", "text": if ctx.history_on { "History is on: after you accept, you can undo this from the decided card or History.".to_string() } else { format!("You can take this back until something else changes (history is off for {}).", ctx.space) } }));
    } else if p.status == "accepted" {
        let (ok, why) = take_back_state(ont, p, ctx.history_on);
        j["take_back"] = json!({ "possible": ok && ctx.editable, "why_not": why, "undo": p.commit });
    }
    j["notes"] = json!(notes);
    // "Put your name on it": the decision, or a grade it gave, was saved with no name
    let anon = |b: &Option<String>| b.as_deref().map(|b| crate::trust::who_said(b) == "a person").unwrap_or(false);
    let sign = ctx.editable
        && match p.status.as_str() {
            "accepted" => anon(&p.accepted_by) || graded_by(ont, p).iter().any(|(k, n)| crate::edit::sign_refusal(ont, *k, n).is_none()),
            "set_aside" => anon(&p.decided_by),
            _ => false,
        };
    if p.status == "set_aside" {
        j["can"] = json!({ "accept": false, "dismiss": false, "reopen": ctx.editable, "sign": sign, "why_not": why_not });
    } else {
        j["can"] = json!({ "accept": can_accept && why_not.is_none(), "dismiss": ctx.editable && p.status == "pending", "sign": sign, "why_not": why_not });
    }
    j["changes"] = json!(changes);
    j
}

/// Whether an accepted proposal can be taken back now, and why not.
pub fn take_back_state(ont: &Ontology, p: &Proposal, history_on: bool) -> (bool, Option<String>) {
    if let Some(s) = &p.snapshot {
        for f in &s.files {
            let now = std::fs::read_to_string(ont.root.join(&f.file)).ok();
            if now != f.after {
                return (false, Some(format!("Can't take back: {} has changed since. Press Start history in Settings to be able to undo any change.", f.file)));
            }
        }
        let trust_now = std::fs::read_to_string(ont.root.join(crate::trust::FILE)).ok();
        if trust_now != s.trust_after {
            return (false, Some(format!("Can't take back: {} has changed since (another grade was given). Press Start history in Settings to be able to undo any change.", crate::trust::FILE)));
        }
        return (true, None);
    }
    if p.commit.is_some() && history_on {
        return (true, None);
    }
    (false, Some("Can't take back: this was accepted before Lorefile kept what it needs to (no snapshot or commit).".into()))
}

/// Every code the validator gives, so an error remembered as text becomes a diagnostic again.
const CODES: [&str; 33] = [
    "E000", "E001", "E002", "E003", "E004", "E005", "E006", "E007", "E008", "E009", "E010", "E011", "E012", "E013", "E014", "E015", "E016", "E017", "E018",
    "W101", "W102", "W103", "W104", "W105", "W106", "W107", "W108", "W109", "W111", "W112", "W113", "W114", "W110",
];

/// An error remembered at Accept, as the diagnostic it was.
pub fn known_diag(k: &crate::proposals::KnownError) -> Option<Diag> {
    let code = CODES.iter().find(|c| **c == k.code)?;
    Some(Diag { level: Level::Error, code, message: k.message.clone(), file: k.file.clone(), subject: None, pair: None })
}

/// Who last changed these files, from the activity log: "Vinay".
fn last_who(root: &Path, files: &[String]) -> Option<String> {
    crate::proposals::activity(root, 200).into_iter().find(|a| files.iter().any(|f| a["file"].as_str() == Some(f.as_str()))).and_then(|a| a["who"].as_str().map(crate::trust::who_said))
}

/// The space as it would be with the proposal, for the map: entities and links with a status.
pub fn graph(ont: &Ontology, p: &Proposal) -> Json {
    let after = match prepare(ont, p) {
        Prepared::Fresh { files, .. } | Prepared::Replayed { files, .. } if p.status == "pending" => overlay(&ont.root, &files.into_iter().map(|(f, t)| (f, Some(t))).collect::<Vec<_>>()).map(|x| x.0),
        _ => own_texts(ont, p).map(|(_, a)| a),
    };
    let before = if p.status == "pending" { None } else { own_texts(ont, p).map(|(b, _)| b) };
    let before = before.as_ref().unwrap_or(ont);
    let Some(after) = after else { return json!({ "error": "the proposed files do not load" }) };
    let changes = compare(before, &after);
    let status = |kind: &str, name: &str| -> &'static str {
        match changes.iter().find(|c| c["kind"] == kind && c["name"] == name).and_then(|c| c["op"].as_str()) {
            Some("add") => "new",
            Some("remove") => "removed",
            Some(_) => "changed",
            None => "same",
        }
    };
    let domain_of = |o: &Ontology, t: &TypeDef| t.domain.clone().or_else(|| o.manifest.domains.iter().find(|d| d.types.contains(&t.name)).map(|d| d.name.clone())).unwrap_or_else(|| "Other".into());
    let mut nodes: Vec<Json> = after.types.values().map(|t| json!({ "name": t.name, "domain": domain_of(&after, t), "status": status("entity", &t.name), "description": t.description, "fields": t.properties.len() })).collect();
    for t in before.types.values().filter(|t| after.type_(&t.name).is_none()) {
        nodes.push(json!({ "name": t.name, "domain": domain_of(before, t), "status": "removed", "description": t.description, "fields": t.properties.len() }));
    }
    let mut edges: Vec<Json> = after.relationships.iter().map(|r| json!({ "name": r.name, "from": r.from, "to": r.to, "status": status("link", &r.qualified()) })).collect();
    for r in before.relationships.iter().filter(|r| !after.relationships.iter().any(|a| a.qualified() == r.qualified())) {
        edges.push(json!({ "name": r.name, "from": r.from, "to": r.to, "status": "removed" }));
    }
    json!({ "id": p.id, "headline": headline(p, &changes), "nodes": nodes, "edges": edges })
}

/// "ChatGPT's promo_sales.csv file: Promo sale and Promo sale revenue": what a decision was about, for the activity log.
pub fn object_words(p: &Proposal, changes: &[Json]) -> String {
    let agent = agent_name(&p.via);
    let poss = if agent == "An agent" { "an agent's".to_string() } else { format!("{}'s", agent) };
    let thing = match (&p.learned, &p.check) {
        (Some(l), _) if !l.file_name.is_empty() => format!("{} file", l.file_name),
        (_, Some(c)) => format!("check of {} {}", c.kind, c.name),
        _ => "proposal".to_string(),
    };
    let labels: Vec<String> = changes.iter().filter(|c| c["kind"] != "source").take(3).map(|c| c["label"].as_str().unwrap_or("").to_string()).collect();
    let list = match labels.len() {
        0 => String::new(),
        1 => labels[0].clone(),
        n => format!("{} and {}", labels[..n - 1].join(", "), labels[n - 1]),
    };
    if list.is_empty() { format!("{} {}", poss, thing) } else { format!("{} {}: {}", poss, thing, list) }
}

/// What a proposal changes, from its own texts laid over today's space (any state).
pub fn own_changes(ont: &Ontology, p: &Proposal) -> Vec<Json> {
    own_texts(ont, p).map(|(b, a)| compare(&b, &a)).unwrap_or_default()
}

/// The short card in the list: headline, what it adds, how many questions, and one-word chips.
pub fn card(ont: &Ontology, report: &Report, p: &Proposal, ctx: &Ctx) -> Json {
    let r = review(ont, report, p, ctx);
    let mut chips: Vec<&str> = Vec::new();
    let notes = r["notes"].as_array().cloned().unwrap_or_default();
    if notes.iter().any(|n| n["kind"] == "stale") {
        chips.push("can't accept");
    } else if notes.iter().any(|n| n["kind"] == "replayed") {
        chips.push("changed since");
    }
    if r["errors"].as_array().map(|e| !e.is_empty()).unwrap_or(false) || (!p.ok && p.status == "pending") {
        chips.push("can't accept");
    }
    if r["changes"].as_array().map(|c| c.iter().any(|x| x["op"] == "remove")).unwrap_or(false) {
        chips.push("removes");
    }
    if p.status == "replaced" {
        chips.push("replaced");
    }
    chips.dedup();
    let c = &r["counts"];
    let mut line: Vec<String> = Vec::new();
    for (k, w) in [("adds", "Adds"), ("changes", "Changes"), ("removes", "Removes")] {
        if let Some(a) = c[k].as_array().filter(|a| !a.is_empty()) {
            line.push(format!("{} {}", w, a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")));
        }
    }
    let asks = r["questions"].as_array().map(|q| q.iter().filter(|x| x.get("ask").is_some()).count()).unwrap_or(0);
    json!({ "headline": r["headline"], "line": line.join(" · "), "questions": asks, "chips": chips, "agent": r["context"]["agent"] })
}

/// When an error names something only a waiting proposal adds, say so: the agent sends this
/// together with that one, or after a person accepts it.
pub fn waiting_hint(ont: &Ontology, text: &str) -> Option<String> {
    for p in crate::proposals::list(&ont.root).into_iter().filter(|p| p.status == "pending" && p.check.is_none()) {
        let files: Vec<(String, Option<String>)> = p.parts().into_iter().map(|(f, t, _)| (f, Some(t))).collect();
        let Some((after, _)) = overlay(&ont.root, &files) else { continue };
        let mut names: Vec<String> = after.types.keys().filter(|n| ont.type_(n).is_none()).cloned().collect();
        names.extend(after.metrics.keys().filter(|n| !ont.metrics.contains_key(*n)).cloned());
        names.extend(after.sources.keys().filter(|n| !ont.sources.contains_key(*n)).cloned());
        names.extend(after.relationships.iter().filter(|r| !ont.relationships.iter().any(|o| o.qualified() == r.qualified())).map(|r| r.name.clone()));
        if let Some(n) = names.iter().find(|n| word_in(text, n)) {
            return Some(format!("{} is waiting for review in {}; send this together with it (one propose_patch with all the ops), or after a person accepts it.", n, p.id));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::words::*;
    use super::*;

    fn src(filter: &str, measure: &str) -> MetricSource {
        MetricSource { connection: "c".into(), table: "t".into(), filter: filter.into(), measure: measure.into(), ..Default::default() }
    }

    #[test]
    fn area_changes_read_as_sentences() {
        let dir = std::env::temp_dir().join(format!("lorefile-review-areas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::write(dir.join("ontology.yaml"), "name: shop\nnamespace: https://shop.example/ns#\ndomains:\n  - name: Core\n    types: [Order]\n  - name: Old\n    types: []\n  - name: Pipeline\n    types: [Deal]\n").unwrap();
        std::fs::write(dir.join("types/order.yaml"), "type: Order\ndescription: x\nproperties:\n  order_id: { type: string, key: true, description: x }\n").unwrap();
        std::fs::write(dir.join("types/deal.yaml"), "type: Deal\ndescription: x\nproperties:\n  deal_id: { type: string, key: true, description: x }\n").unwrap();
        let ont = crate::load_ontology(&dir).unwrap().0;
        let said = |ops: Vec<Json>| -> Vec<String> {
            let plan = crate::patch::plan_many(&ont, &ops).unwrap();
            let ov: crate::loader::Overrides = plan.files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect();
            let (after, _) = crate::edit::check_many(&dir, &ov).unwrap();
            compare(&ont, &after).iter().filter_map(|c| c["sentence"].as_str().map(|s| s.to_string())).collect()
        };
        assert_eq!(said(vec![json!({ "op": "add_area", "name": "Sales" })]), vec!["Adds the area Sales."]);
        assert_eq!(said(vec![json!({ "op": "rename_area", "area": "Core", "to": "Sales" })]), vec!["Renames the area Core to Sales (its IRI becomes …/ns#domain.sales)."]);
        assert_eq!(said(vec![json!({ "op": "delete_area", "area": "Old" })]), vec!["Deletes the empty area Old."]);
        assert_eq!(said(vec![json!({ "op": "move_to_area", "type": "Order", "area": "Pipeline" })]), vec!["Moves Order from Core to Pipeline."]);
        assert_eq!(said(vec![json!({ "op": "order_areas", "order": ["Pipeline", "Core", "Old"] })]), vec!["Reorders the areas."]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metric_recipes_read_as_sentences() {
        assert_eq!(recipe_sentence(&src("refunded = false", "sum(revenue)"), "lines").0, "add up revenue, leaving out lines where refunded is true");
        assert_eq!(recipe_sentence(&src("", "count distinct order_id"), "orders").0, "count the different order_id");
        assert_eq!(recipe_sentence(&src("", "count(distinct order_id)"), "orders").0, "count the different order_id");
        assert_eq!(recipe_sentence(&src("", "count(*)"), "rows").0, "count rows");
        assert_eq!(recipe_sentence(&src("", "count rows"), "orders").0, "count orders");
        assert_eq!(recipe_sentence(&src("", "avg(price)"), "rows").0, "average price");
        assert_eq!(recipe_sentence(&src("region = 'EU'", "sum(x)"), "rows").0, "add up x, only rows where region is EU");
        assert_eq!(recipe_sentence(&src("status != 'void'", "sum(x)"), "rows").0, "add up x, leaving out rows where status is void");
        assert_eq!(recipe_sentence(&src("status in ('paid', 'shipped')", "sum(x)"), "rows").0, "add up x, only rows where status is one of paid, shipped");
        assert_eq!(recipe_sentence(&src("refunded = false and region = 'EU'", "sum(x)"), "lines").0, "add up x, leaving out lines where refunded is true, only lines where region is EU");
        let (s, w) = recipe_sentence(&src("payment_status = 'paid' or total > 5", "sum(x)"), "rows");
        assert!(s.contains("filter as written"), "{}", s);
        assert_eq!(w.as_deref(), Some("where payment_status = 'paid' or total > 5"));
        let (_, w) = recipe_sentence(&src("", "sum(x) / count(*)"), "rows");
        assert_eq!(w.as_deref(), Some("sum(x) / count(*)"), "a measure that is not plain is shown as written");
        assert_eq!(rows_noun(Some("line_id")), "lines");
        assert_eq!(rows_noun(Some("order_id")), "orders");
        assert_eq!(rows_noun(Some("sku")), "rows");
        assert_eq!(rows_noun(None), "rows");
    }

    #[test]
    fn types_rules_and_sources_read_in_words() {
        let p = |k: &str, u: Option<&str>| Property { kind: k.into(), unit: u.map(|s| s.to_string()), ..Default::default() };
        assert_eq!(type_word(&p("number", Some("USD"))), "money (USD)");
        assert_eq!(type_word(&p("number", Some("kg"))), "number (kg)");
        assert_eq!(type_word(&p("integer", None)), "whole number");
        assert_eq!(type_word(&p("boolean", None)), "yes / no");
        assert_eq!(type_word(&p("string", None)), "text");
        assert_eq!(rule_sentence("refunded == false").as_deref(), Some("refunded must be false"));
        assert_eq!(rule_sentence("order.total >= 10").as_deref(), Some("order.total must be at least 10"));
        assert_eq!(rule_sentence("size(x) > 0 && y"), None);
        assert_eq!(source_sentence("from_user", None), "files people give an agent");
        assert_eq!(source_sentence("mcp", Some("meta-ads")), "tools on the meta-ads MCP server");
        assert_eq!(label("PromoSale"), "Promo sale");
        assert_eq!(label("promo_sale_revenue"), "Promo sale revenue");
        assert_eq!(label("SKU"), "SKU");
        assert_eq!(agent_name("agent via MCP (connection \"ChatGPT\", approved 27 Nov)"), "ChatGPT");
        assert_eq!(agent_name("agent via MCP"), "An agent");
    }

    /// A space in a temp folder, loaded; `more` lays texts over it (`None` deletes).
    fn space(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lorefile-review-unit-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::write(dir.join("ontology.yaml"), "name: t\ndomains:\n  - name: Core\n    types: []\n").unwrap();
        for (f, t) in files {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        dir
    }

    fn load(dir: &Path, over: &[(&str, Option<&str>)]) -> (Ontology, Report) {
        overlay(dir, &over.iter().map(|(f, t)| (f.to_string(), t.map(|x| x.to_string()))).collect::<Vec<_>>()).unwrap()
    }

    const ORDER: &str = "# One checkout.\n# Kept by the shop team.\ntype: Order\ndescription: One checkout.\nproperties:\n  # the key\n  order_id: { type: string, key: true, description: The order number. }\n  total: { type: number, unit: USD, description: What was paid. }\n  coupon: { type: string, description: The coupon used. }\n";

    #[test]
    fn a_whole_file_rewrite_reads_as_three_column_changes_and_the_comments_it_drops() {
        let dir = space("rewrite", &[("types/order.yaml", ORDER)]);
        let after_text = "type: Order\ndescription: One checkout.\nproperties:\n  order_id: { type: string, key: true, description: The order number. }\n  total: { type: number, unit: USD, description: What the customer paid after discounts. }\n  gift_wrap: { type: boolean, description: True when gift-wrapped. }\n";
        let (before, _) = load(&dir, &[]);
        let (after, _) = load(&dir, &[("types/order.yaml", Some(after_text))]);
        let c = compare(&before, &after);
        assert_eq!(c.len(), 1, "{:?}", c);
        let fields = c[0]["fields"].as_array().unwrap();
        let ops: Vec<(&str, &str)> = fields.iter().map(|f| (f["op"].as_str().unwrap(), f["name"].as_str().unwrap())).collect();
        assert_eq!(ops.len(), 3, "{:?}", ops);
        assert!(ops.contains(&("add", "gift_wrap")) && ops.contains(&("remove", "coupon")) && ops.contains(&("change", "total")), "{:?}", ops);
        assert_eq!(c[0]["summary"], "1 field added, 1 field changed, 1 field removed");
        assert_eq!(comments_lost(ORDER, after_text), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renames_read_as_renames_and_removals_list_what_uses_them() {
        let link = "type: Customer\ndescription: Someone who buys.\nproperties:\n  customer_id: { type: string, key: true }\nrelationships:\n  places: { to: Order, cardinality: one-to-many }\n";
        let dir = space("rename", &[("types/order.yaml", ORDER), ("types/customer.yaml", link), ("glossary.yaml", "terms:\n  - term: basket\n    means: Order\n")]);
        let (before, _) = load(&dir, &[]);
        // Order renamed Purchase, with the link and the word following it
        let purchase = ORDER.replace("type: Order", "type: Purchase");
        let (after, _) = load(&dir, &[("types/order.yaml", None), ("types/purchase.yaml", Some(&purchase)), ("types/customer.yaml", Some(&link.replace("to: Order", "to: Purchase"))), ("glossary.yaml", Some("terms:\n  - term: basket\n    means: Purchase\n"))]);
        let c = compare(&before, &after);
        let renames: Vec<&Json> = c.iter().filter(|x| x["op"] == "rename").collect();
        assert_eq!(renames.len(), 1, "{:?}", c);
        assert_eq!((renames[0]["from"].as_str(), renames[0]["name"].as_str()), (Some("Order"), Some("Purchase")));
        assert!(!c.iter().any(|x| x["op"] == "remove" || (x["kind"] == "entity" && x["op"] == "add")), "not a removal and an addition: {:?}", c);
        assert_eq!(headline(&Proposal::default(), &c), "An agent wants to rename Order to Purchase");
        // a column with the same type and description under a new name
        let renamed_col = ORDER.replace("coupon: { type: string", "promo: { type: string");
        let (after, _) = load(&dir, &[("types/order.yaml", Some(&renamed_col))]);
        let c = compare(&before, &after);
        assert_eq!(c[0]["fields"][0]["sentence"], "looks like a rename of coupon → promo", "{:?}", c);
        // a removal lists the link, the word and a number whose recipe names it
        let metrics = "metrics:\n  - metric: coupon_orders\n    description: Orders with a coupon.\n    owner: growth\n    source: { connection: shop, table: orders, filter: \"coupon is not null\", measure: count(*) }\n";
        let dir2 = space("remove", &[("types/order.yaml", &ORDER.replace("properties:", "source: { connection: shop, table: orders, key: order_id }\nproperties:")), ("types/customer.yaml", link), ("glossary.yaml", "terms:\n  - term: basket\n    means: Order\n"), ("sources/shop.yaml", "connection: shop\nkind: bigquery\n"), ("logic/metrics.yaml", metrics)]);
        let (before, _) = load(&dir2, &[]);
        let (after, _) = load(&dir2, &[("types/order.yaml", None)]);
        let c = compare(&before, &after);
        let gone = c.iter().find(|x| x["kind"] == "entity" && x["op"] == "remove").unwrap();
        let uses: Vec<&str> = gone["uses"].as_array().unwrap().iter().filter_map(|u| u.as_str()).collect();
        assert!(uses.iter().any(|u| u.contains("link places")) && uses.iter().any(|u| u.contains("\"basket\"")) && uses.iter().any(|u| u.contains("metric coupon_orders")), "{:?}", uses);
        let (after, _) = load(&dir2, &[("types/order.yaml", Some(&ORDER.replace("properties:", "source: { connection: shop, table: orders, key: order_id }\nproperties:").replace("  coupon: { type: string, description: The coupon used. }\n", "")))]);
        let c = compare(&before, &after);
        let col = c[0]["fields"].as_array().unwrap().iter().find(|f| f["op"] == "remove").unwrap();
        assert!(col["uses"][0].as_str().unwrap().contains("coupon_orders"), "a metric whose filter names a removed column: {:?}", col);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn only_the_warnings_a_proposal_adds_are_asked() {
        let dir = space("warnings", &[("types/order.yaml", "type: Order\nproperties:\n  order_id: { type: string, key: true }\n"), ("sources/shop.yaml", "connection: shop\nkind: bigquery\n")]);
        let (before, rb) = load(&dir, &[]);
        assert!(rb.diags.iter().any(|d| d.code == "W101"), "the space already has a warning of its own");
        let m = "metrics:\n  - metric: paid_orders\n    description: Paid orders.\n    source: { connection: shop, table: orders, measure: count(*) }\n";
        let (after, ra) = load(&dir, &[("logic/metrics.yaml", Some(m))]);
        let p = Proposal::default();
        let (qs, errors, other) = questions(&rb, &after, &ra, &p);
        assert!(errors.is_empty());
        assert_eq!(qs.len(), 1, "{:?}", qs);
        assert_eq!((qs[0]["code"].as_str(), qs[0]["ask"].as_str()), (Some("W109"), Some("Who decides what Paid orders means?")));
        assert_eq!(qs[0]["fix"], json!({ "op": "set_metric", "metric": "paid_orders", "field": "owner" }));
        assert_eq!(other, 1, "the space's own warning is counted, not asked");
        let ok = fix_op(&json!({ "op": "set_metric", "metric": "paid_orders", "owner": "growth" }), &qs, &after);
        assert!(ok.is_ok(), "{:?}", ok);
        assert!(fix_op(&json!({ "op": "set_metric", "metric": "paid_orders", "description": "x" }), &qs, &after).is_err(), "only what was asked");
        assert!(fix_op(&json!({ "op": "set_type_description", "type": "Order", "description": "x" }), &qs, &after).is_err(), "W101 on Order was the space's own");
        let _ = before;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn comments_are_counted_and_fingerprints_ignore_order() {
        assert_eq!(comments_lost("# a\n# b\nx: 1\n  # c\n", "x: 1\n"), 3);
        assert_eq!(comments_lost("x: 1\n", "# new\nx: 1\n"), 0);
        let a = vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())];
        let b = vec![("b".to_string(), "2".to_string()), ("a".to_string(), "1".to_string())];
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&[("a".to_string(), "1 ".to_string())]));
    }
}
