//! Apache Ossie import and export.
//!
//! Apache Ossie (github.com/apache/ossie, draft `0.3.0.dev0`) is the interchange
//! format of the semantic-layer tools: a **semantic model** (datasets, fields,
//! metrics, joins), an **ontology** (concepts, and relationships with a
//! verbalization) and a **mapping** between the two. `export` writes all three
//! from a space; `import` builds a space from them.
//!
//! The round trip is lossless by construction. Whatever Ossie has a home for
//! goes to that home. For each object, export then rebuilds it the way import
//! will, from the portable parts alone, and keeps only the difference in
//! `nonportable_extensions` under `vendor_name: lorefile` (Ossie readers must
//! preserve those). Import rebuilds and lays the difference back on. So an edit
//! another tool makes to a portable part (a description, a field) still wins,
//! and guardrails, actions, functions, sources, owners and metric recipes, which
//! Ossie has no words for, come back exactly as they were.

use crate::loader::{self, Overrides};
use crate::ontology::{snake, Ontology, Relationship};
use crate::schema::*;
use crate::validate;
use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{json, Map, Value as Json};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// The Ossie draft this module reads and writes.
pub const VERSION: &str = "0.3.0.dev0";
/// Our `vendor_name` in `nonportable_extensions`.
pub const VENDOR: &str = "lorefile";
/// Bumped if the shape of our extension data ever changes.
const FORMAT: u64 = 1;

// ===========================================================================
// YAML out. Ossie readers are often PyYAML (YAML 1.1), which reads `yes`,
// `on`, `2024-01-01` and `010` as something other than a string, so every
// string that could be misread is double-quoted.
// ===========================================================================

pub fn to_yaml(v: &Json) -> String {
    let mut out = String::new();
    match v {
        Json::Object(m) if !m.is_empty() => emit_map(&mut out, m, 0, false),
        Json::Array(a) if !a.is_empty() => emit_seq(&mut out, a, 0),
        other => {
            out.push_str(&scalar_inline(other));
            out.push('\n');
        }
    }
    out
}

fn emit_map(out: &mut String, m: &Map<String, Json>, indent: usize, first_inline: bool) {
    for (i, (k, v)) in m.iter().enumerate() {
        if !(first_inline && i == 0) {
            out.push_str(&" ".repeat(indent));
        }
        out.push_str(&yaml_string(k));
        out.push(':');
        match v {
            Json::Object(x) if !x.is_empty() => {
                out.push('\n');
                emit_map(out, x, indent + 2, false);
            }
            Json::Array(x) if !x.is_empty() => {
                out.push('\n');
                emit_seq(out, x, indent + 2);
            }
            Json::String(s) if block_ok(s) => emit_block(out, s, indent + 2),
            other => {
                out.push(' ');
                out.push_str(&scalar_inline(other));
                out.push('\n');
            }
        }
    }
}

fn emit_seq(out: &mut String, a: &[Json], indent: usize) {
    for item in a {
        out.push_str(&" ".repeat(indent));
        out.push('-');
        match item {
            Json::Object(m) if !m.is_empty() => {
                out.push(' ');
                emit_map(out, m, indent + 2, true);
            }
            Json::Array(x) if !x.is_empty() => {
                out.push('\n');
                emit_seq(out, x, indent + 2);
            }
            Json::String(s) if block_ok(s) => emit_block(out, s, indent + 2),
            other => {
                out.push(' ');
                out.push_str(&scalar_inline(other));
                out.push('\n');
            }
        }
    }
}

/// A multi-line string as a literal block (`|` keeps one final newline, `|-` none).
fn emit_block(out: &mut String, s: &str, indent: usize) {
    let (header, body) = match s.strip_suffix('\n') {
        Some(b) => (" |", b),
        None => (" |-", s),
    };
    out.push_str(header);
    out.push('\n');
    for line in body.split('\n') {
        if !line.is_empty() {
            out.push_str(&" ".repeat(indent));
            out.push_str(line);
        }
        out.push('\n');
    }
}

fn block_ok(s: &str) -> bool {
    if !s.contains('\n') || s.ends_with("\n\n") || s.trim().is_empty() {
        return false;
    }
    if s.chars().any(|c| c == '\t' || c == '\r' || (c.is_control() && c != '\n')) {
        return false;
    }
    let body = s.strip_suffix('\n').unwrap_or(s);
    if body.split('\n').any(|l| l.ends_with(' ')) {
        return false;
    }
    // the first line with text sets the indentation; it must not start with a space
    !body.split('\n').find(|l| !l.is_empty()).map(|l| l.starts_with(' ')).unwrap_or(true)
}

fn scalar_inline(v: &Json) -> String {
    match v {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => n.to_string(),
        Json::String(s) => yaml_string(s),
        Json::Array(_) => "[]".into(),
        Json::Object(_) => "{}".into(),
    }
}

fn yaml_string(s: &str) -> String {
    if plain_ok(s) {
        s.to_string()
    } else {
        // a JSON string is a valid YAML double-quoted scalar
        serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
    }
}

fn plain_ok(s: &str) -> bool {
    let Some(first) = s.chars().next() else { return false };
    if s.trim() != s || !(first.is_alphabetic() || first == '_' || first == '/' || first == '(') {
        return false;
    }
    if s.chars().any(|c| c.is_control() || matches!(c, '{' | '}' | '[' | ']' | '"' | '\\' | '`')) {
        return false;
    }
    if s.contains(": ") || s.contains(" #") || s.ends_with(':') {
        return false;
    }
    !matches!(s.to_ascii_lowercase().as_str(), "y" | "yes" | "n" | "no" | "true" | "false" | "on" | "off" | "null" | "~")
}

// ===========================================================================
// Small helpers shared by both directions
// ===========================================================================

fn to_json<T: serde::Serialize>(v: &T) -> Result<Json> {
    serde_json::to_value(v).map_err(|e| anyhow!("could not convert to JSON: {}", e))
}

fn s(v: &Json) -> &str {
    v.as_str().unwrap_or("")
}

fn arr(v: &Json) -> &[Json] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

/// The keys of `orig` whose values `rebuilt` does not reproduce. A key only
/// `rebuilt` has comes back as `null`, which `overlay` reads as "remove".
fn diff(orig: &Json, rebuilt: &Json, skip: &[&str]) -> Map<String, Json> {
    let mut out = Map::new();
    let (Some(o), Some(r)) = (orig.as_object(), rebuilt.as_object()) else { return out };
    for (k, v) in o {
        if !skip.contains(&k.as_str()) && r.get(k) != Some(v) {
            out.insert(k.clone(), v.clone());
        }
    }
    for k in r.keys() {
        if !skip.contains(&k.as_str()) && !o.contains_key(k) {
            out.insert(k.clone(), Json::Null);
        }
    }
    out
}

fn overlay(base: &mut Json, ext: &Map<String, Json>) {
    if let Some(b) = base.as_object_mut() {
        for (k, v) in ext {
            if v.is_null() {
                b.shift_remove(k);
            } else {
                b.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Our entry for a node's `nonportable_extensions`.
fn ext_list(data: Map<String, Json>) -> Json {
    json!([{ "vendor_name": VENDOR, "data": Json::Object(data) }])
}

/// The data of our entry on a node, if it has one.
fn our_ext(node: &Json) -> Option<Map<String, Json>> {
    arr(&node["nonportable_extensions"])
        .iter()
        .find(|e| s(&e["vendor_name"]).eq_ignore_ascii_case(VENDOR))
        .and_then(|e| e["data"].as_object().cloned())
}

/// Other vendors' entries on a node, by vendor name.
fn other_exts(node: &Json) -> Vec<String> {
    arr(&node["nonportable_extensions"])
        .iter()
        .map(|e| s(&e["vendor_name"]).to_string())
        .filter(|v| !v.eq_ignore_ascii_case(VENDOR))
        .collect()
}

fn insert_if(m: &mut Map<String, Json>, k: &str, v: Json) {
    let empty = match &v {
        Json::Null => true,
        Json::String(x) => x.trim().is_empty(),
        Json::Array(x) => x.is_empty(),
        Json::Object(x) => x.is_empty(),
        _ => false,
    };
    if !empty {
        m.insert(k.to_string(), v);
    }
}

/// `lumen-ecommerce` → `lumen-ecommerce`; `My Shop!` → `my-shop`.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if (c == '-' || c == '_' || c == ' ' || c == '.') && !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "space".into() } else { out }
}

/// `placed_by` → `placed by`; `PaymentMethod` → `payment method`.
fn human(name: &str) -> String {
    snake(name).replace('_', " ")
}

/// The IRI an entity named `name` gets in a space named `space` that sets no
/// `namespace:` (what an import compares a concept's `iri` against).
pub fn default_iri(space: &str, name: &str) -> String {
    crate::iri::derive(&crate::iri::default_namespace(space), &crate::iri::Key::Entity(name.to_string()), &[])
}

// ---- datatypes -------------------------------------------------------------

fn datatype_of(kind: &str) -> Option<&'static str> {
    match kind {
        "string" | "enum" => Some("String"),
        "integer" => Some("Integer"),
        "number" => Some("Decimal"),
        "boolean" => Some("Boolean"),
        "date" => Some("Date"),
        "datetime" => Some("DateTime"),
        "json" => Some("Opaque"),
        _ => None,
    }
}

fn kind_of_datatype(dt: &str) -> &'static str {
    match dt {
        "Integer" => "integer",
        "Decimal" | "Float" => "number",
        "Boolean" => "boolean",
        "Date" => "date",
        "DateTime" | "DateTimeTz" => "datetime",
        "Opaque" => "json",
        _ => "string",
    }
}

/// The ontology's built-in value type for a property kind.
fn value_concept(kind: &str) -> &'static str {
    match kind {
        "integer" => "Integer",
        "number" => "Decimal",
        "boolean" => "Boolean",
        "date" => "Date",
        "datetime" => "DateTime",
        _ => "String",
    }
}

const BUILTIN_VALUES: &[&str] = &["String", "Integer", "Decimal", "Float", "Boolean", "Date", "DateTime", "Time", "DateTimeTz"];

// ---- SQL text ----------------------------------------------------------------

/// Words that cannot stand bare as a column or dataset name in an expression.
const RESERVED: &[&str] = &[
    "all", "and", "any", "as", "asc", "between", "by", "case", "cast", "collate", "create", "cross", "cube", "delete", "desc", "distinct", "drop", "else",
    "end", "escape", "except", "exists", "false", "filter", "for", "from", "full", "group", "having", "in", "inner", "insert", "intersect", "interval", "into",
    "is", "join", "lateral", "left", "like", "limit", "natural", "not", "null", "offset", "on", "or", "order", "outer", "over", "partition", "qualify",
    "right", "rollup", "select", "table", "then", "true", "union", "update", "using", "values", "when", "where", "window", "with",
];

fn bare_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// A column or dataset name as it must be written in SQL.
fn sql_ident(name: &str) -> String {
    if bare_ident(name) && !RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// A name after or before a dot (`Order.order_id`): there even keywords read as
/// names, and quoting would make the name case-sensitive (`"Order"` is not `ORDER`),
/// so only a name that is not a plain identifier is quoted.
fn qual_ident(name: &str) -> String {
    if bare_ident(name) { name.to_string() } else { format!("\"{}\"", name.replace('"', "\"\"")) }
}

/// The column a field expression names, when it names exactly one.
fn column_of(sql: &str) -> Option<String> {
    let t = sql.trim();
    if bare_ident(t) {
        return Some(t.to_string());
    }
    let inner = t.strip_prefix('"')?.strip_suffix('"')?;
    if inner.replace("\"\"", "").contains('"') {
        return None;
    }
    Some(inner.replace("\"\"", "\""))
}

fn balanced(sql: &str) -> bool {
    let (mut depth, mut quote) = (0i32, None::<char>);
    for c in sql.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => {}
            },
        }
    }
    depth == 0 && quote.is_none()
}

const SQL_WORDS: &[&str] = &[
    "all", "and", "any", "as", "asc", "between", "by", "case", "current", "desc", "distinct", "else", "end", "escape", "exists", "false", "filter",
    "first", "following", "group", "ilike", "in", "interval", "is", "last", "like", "not", "null", "nulls", "or", "order", "over", "partition",
    "preceding", "range", "row", "rows", "some", "then", "true", "unbounded", "when", "where", "within",
];

/// Calls `f` on every bare column name in an expression and splices in what it
/// returns. Function names, keywords, literals, qualified names (`a.b`) and type
/// names after `AS` are left as they are.
fn rewrite_columns(sql: &str, f: &mut dyn FnMut(&str) -> String) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut prev_word = String::new();
    let next_non_space = |from: usize| chars[from..].iter().find(|c| !c.is_whitespace()).copied();
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == c {
                    if i + 1 < chars.len() && chars[i + 1] == c {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            out.extend(&chars[start..i]);
            prev_word.clear();
            continue;
        }
        if c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_') {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let before = out.trim_end().chars().last();
            let after = next_non_space(i);
            let lower = word.to_ascii_lowercase();
            let keep = before == Some('.')
                || matches!(after, Some('.') | Some('(') | Some('\''))
                || SQL_WORDS.contains(&lower.as_str())
                || prev_word == "as";
            out.push_str(&if keep { word.clone() } else { f(&word) });
            prev_word = lower;
            continue;
        }
        if !c.is_whitespace() {
            prev_word.clear();
        }
        out.push(c);
        i += 1;
    }
    out
}

/// `count distinct order_id, by date(placed_on)` → `COUNT(DISTINCT order_id)`.
/// The grouping hint after `by` has no place in an Ossie metric (grouping is
/// chosen when the metric is queried), so it is left out of the expression.
fn measure_to_sql(measure: &str) -> Option<String> {
    let base = measure.trim().to_string();
    let mut m = base.clone();
    let by = Regex::new(r"(?i)(,\s*|\s+)(by|per)\s").unwrap();
    for hit in by.find_iter(&base) {
        if balanced(&m[..hit.start()]) {
            m.truncate(hit.start());
            break;
        }
    }
    let m = m.trim();
    let distinct = Regex::new(r"(?i)^(count|sum|avg|min|max)\s+distinct\s+(.+)$").unwrap();
    let plain = Regex::new(r"(?i)^(count|sum|avg|min|max|median)\s+([^(\s].*)$").unwrap();
    let call = Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*\s*\(.*\)$").unwrap();
    let out = if m.eq_ignore_ascii_case("count") || m.eq_ignore_ascii_case("rows") {
        "COUNT(*)".to_string()
    } else if let Some(c) = distinct.captures(m) {
        format!("{}(DISTINCT {})", c[1].to_uppercase(), c[2].trim())
    } else if let Some(c) = plain.captures(m) {
        format!("{}({})", c[1].to_uppercase(), c[2].trim())
    } else if call.is_match(m) {
        m.to_string()
    } else {
        return None;
    };
    balanced(&out).then_some(out)
}

/// One function call at the top level, so a FILTER clause can follow it.
fn single_call(sql: &str) -> bool {
    let Some(open) = sql.find('(') else { return false };
    if !sql.ends_with(')') || !bare_ident(sql[..open].trim()) {
        return false;
    }
    let mut depth = 0;
    for (i, c) in sql.char_indices().skip(open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i != sql.len() - 1 {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

// ===========================================================================
// Rebuilding Lorefile objects from the portable parts. Export and import both
// call these, which is what makes the extension a true difference.
// ===========================================================================

/// `shop_files.orders` → (`shop_files`, `orders`); the connection is everything before the first dot.
fn split_source(source: &str) -> (String, String) {
    match source.split_once('.') {
        Some((c, t)) if !c.trim().is_empty() && !t.trim().is_empty() && !source.contains(char::is_whitespace) => (c.trim().to_string(), t.trim().to_string()),
        _ => ("ossie".to_string(), source.trim().to_string()),
    }
}

/// A Lorefile property (as JSON) from the portable parts of an Ossie field.
fn property_from_field(f: &Json, key: bool) -> Json {
    let mut desc = s(&f["description"]).to_string();
    let sql = s(&f["expression"]["sql"]);
    if column_of(sql).is_none() && !sql.is_empty() {
        desc = format!("{}{}Computed as `{}`.", desc, if desc.is_empty() { "" } else { " " }, sql);
    }
    json!({
        "type": kind_of_datatype(s(&f["datatype"])),
        "key": key,
        "required": false,
        "description": desc,
        "values": [],
    })
}

/// A Lorefile entity (as JSON, in `TypeDef`'s shape) from the portable parts of an Ossie dataset.
fn type_from_dataset(ds: &Json, key_hint: Option<&str>) -> Json {
    let (connection, table) = split_source(s(&ds["source"]));
    let mut pk: Vec<String> = arr(&ds["primary_key"]).iter().map(|k| s(k).to_string()).collect();
    if pk.is_empty() {
        if let Some(k) = key_hint {
            pk.push(k.to_string());
        }
    }
    let mut properties = Map::new();
    let mut columns = Map::new();
    for f in arr(&ds["fields"]) {
        let name = s(&f["name"]).to_string();
        if name.is_empty() {
            continue;
        }
        let key = pk.iter().any(|k| k.eq_ignore_ascii_case(&name));
        properties.insert(name.clone(), property_from_field(f, key));
        if let Some(col) = column_of(s(&f["expression"]["sql"])) {
            if col != name {
                columns.insert(name.clone(), json!(col));
            }
        }
    }
    let key_column = pk
        .first()
        .and_then(|k| arr(&ds["fields"]).iter().find(|f| s(&f["name"]).eq_ignore_ascii_case(k)))
        .and_then(|f| column_of(s(&f["expression"]["sql"])))
        .unwrap_or_default();
    let mut description = s(&ds["description"]).to_string();
    // the sentence our own export writes for a proposed definition is not a
    // description: it is dropped wherever it leads the text, also when another
    // tool appended to it or flattened `ai_context` to a string
    let for_ai = match &ds["ai_context"] {
        Json::String(t) => crate::trust::without_ai_sentence(t),
        Json::Object(o) => o.get("instructions").and_then(|x| x.as_str()).map(crate::trust::without_ai_sentence).unwrap_or_default(),
        _ => String::new(),
    };
    if !for_ai.is_empty() {
        description = format!("{}{}For AI tools: {}", description, if description.is_empty() { "" } else { "\n\n" }, for_ai);
    }
    json!({
        "type": s(&ds["name"]),
        "description": description,
        "properties": properties,
        "relationships": {},
        "source": { "connection": connection, "table": table, "key": key_column, "columns": columns },
        "owner": "",
        "tags": [],
        "instances": [],
        "file": "",
    })
}

/// A Lorefile metric (as JSON, in `MetricDef`'s shape) from the portable parts
/// of an Ossie metric. `scope` is the dataset's connection and table for a
/// dataset-scoped metric.
fn metric_from_ossie(m: &Json, scope: Option<(&str, &str)>) -> Json {
    let mut sql = s(&m["expression"]["sql"]).to_string();
    if sql.is_empty() {
        sql = arr(&m["expression"]["dialects"]).first().map(|d| s(&d["expression"]).to_string()).unwrap_or_default();
    }
    let mut out = json!({ "metric": s(&m["name"]), "description": s(&m["description"]) });
    match scope {
        Some((connection, table)) => {
            let strip = Regex::new(r"(?i)\bsource\s*\.\s*").unwrap();
            let mut src = Map::new();
            src.insert("connection".into(), json!(connection));
            insert_if(&mut src, "table", json!(table));
            insert_if(&mut src, "measure", json!(strip.replace_all(&sql, "").to_string()));
            out["source"] = Json::Object(src);
        }
        None => out["sql"] = json!(sql),
    }
    out["file"] = json!("");
    out
}

// ===========================================================================
// Export
// ===========================================================================

/// The three Ossie documents for one space, and what the export had to guess.
pub struct Exported {
    /// File-name stem: `<base>.semantic_model.yaml` and so on.
    pub base: String,
    pub semantic_model: Json,
    pub ontology: Json,
    pub mapping: Json,
    pub notes: Vec<String>,
}

impl Exported {
    /// The three documents as one JSON object (what `GET /api/export?format=ossie` returns).
    pub fn bundle(&self) -> Json {
        json!({
            "format": "apache-ossie",
            "version": VERSION,
            "name": self.base,
            "files": {
                "semantic_model": format!("{}.semantic_model.yaml", self.base),
                "ontology": format!("{}.ontology.yaml", self.base),
                "mapping": format!("{}.mapping.yaml", self.base),
            },
            "semantic_model": self.semantic_model,
            "ontology": self.ontology,
            "mapping": self.mapping,
            "notes": self.notes,
        })
    }

    /// `(file name, document)` for each of the three, with `ext` `yaml` or `json`;
    /// the mapping's references point at the other two files by those names.
    pub fn documents(&self, ext: &str) -> Vec<(String, Json)> {
        let mut mapping = self.mapping.clone();
        mapping["ontology_ref"]["iri"] = json!(format!("./{}.ontology.{}", self.base, ext));
        mapping["semantic_model_ref"]["iri"] = json!(format!("./{}.semantic_model.{}", self.base, ext));
        vec![
            (format!("{}.semantic_model.{}", self.base, ext), self.semantic_model.clone()),
            (format!("{}.ontology.{}", self.base, ext), self.ontology.clone()),
            (format!("{}.mapping.{}", self.base, ext), mapping),
        ]
    }
}

/// How one link becomes a join: the many side, the one side and the key fields.
struct Join {
    many: String,
    one: String,
    from_columns: Vec<String>,
    to_columns: Vec<String>,
}

/// A link joins two tables when the one side's key also names a property on the
/// many side (`Order.customer_id` → `Customer.customer_id`). Lorefile links name
/// no columns, so this is the only evidence there is.
fn infer_join(ont: &Ontology, r: &Relationship, datasets: &HashSet<String>) -> Option<Join> {
    let card = r.cardinality.to_ascii_lowercase().replace('_', "-");
    let try_side = |many: &str, one: &str| -> Option<Join> {
        if many == one || !datasets.contains(many) || !datasets.contains(one) {
            return None;
        }
        let key = ont.types.get(one)?.key_property()?;
        ont.types.get(many)?.properties.contains_key(&key).then(|| Join {
            many: many.to_string(),
            one: one.to_string(),
            from_columns: vec![key.clone()],
            to_columns: vec![key],
        })
    };
    match card.as_str() {
        "many-to-one" | "" => try_side(&r.from, &r.to),
        "one-to-many" => try_side(&r.to, &r.from),
        "one-to-one" => try_side(&r.from, &r.to).or_else(|| try_side(&r.to, &r.from)),
        _ => None,
    }
}

/// The first sentence of a description, on one line, without its full stop.
fn first_sentence(desc: &str) -> String {
    let one = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = one.find(". ").map(|i| &one[..i]).unwrap_or(&one);
    cut.trim().trim_end_matches('.').trim().to_string()
}

/// Where an entity is named in a sentence, as its name or in words (`payment method`).
fn find_word(text: &str, name: &str, from: usize) -> Option<(usize, usize)> {
    let lower = text.to_lowercase();
    let mut best: Option<(usize, usize)> = None;
    for cand in [name.to_lowercase(), human(name)] {
        let mut start = from;
        while let Some(pos) = lower[start..].find(&cand) {
            let a = start + pos;
            let b = a + cand.len();
            let ok_before = a == 0 || !lower[..a].chars().last().map(|c| c.is_alphanumeric()).unwrap_or(false);
            let ok_after = b >= lower.len() || !lower[b..].chars().next().map(|c| c.is_alphanumeric()).unwrap_or(false);
            if ok_before && ok_after {
                if best.map(|(x, _)| a < x).unwrap_or(true) {
                    best = Some((a, b));
                }
                break;
            }
            start = a + 1;
        }
    }
    best
}

/// Ossie's `verbalizes` pattern for a link: its sentence with the two entities
/// turned into `{Order}` / `{Customer}` placeholders, or, when the sentence does
/// not name both, one made from the link's name. Returns `(pattern, generated)`.
fn verbalize(from: &str, to: &str, name: &str, desc: &str) -> (String, bool) {
    let to_ph = if from == to { format!("{{{}:{}}}", to, name) } else { format!("{{{}}}", to) };
    let sentence = first_sentence(desc);
    if let Some((a1, b1)) = find_word(&sentence, from, 0) {
        if let Some((a2, b2)) = find_word(&sentence, to, b1) {
            // only when lowercasing kept the byte positions (plain ASCII around the names)
            if sentence.to_lowercase().len() == sentence.len() {
                let p = format!("{}{{{}}}{}{}{}", &sentence[..a1], from, &sentence[b1..a2], to_ph, &sentence[b2..]);
                return (p, false);
            }
        }
    }
    (format!("{{{}}} {} {}", from, human(name), to_ph), true)
}

/// Writes `ont` as the three Ossie documents.
pub fn export(ont: &Ontology) -> Result<Exported> {
    let space = if ont.manifest.name.trim().is_empty() { "space".to_string() } else { ont.manifest.name.clone() };
    let base = slug(&space);
    let mut notes: Vec<String> = Vec::new();

    // which entities are datasets: bound to a table of a source that is not an MCP server
    let is_table_bound = |t: &TypeDef| -> bool {
        match &t.source {
            Some(src) if !src.table.trim().is_empty() => !ont.sources.get(&src.connection).map(|x| x.kind.eq_ignore_ascii_case("mcp")).unwrap_or(false),
            _ => false,
        }
    };
    let dataset_names: HashSet<String> = ont.types.values().filter(|t| is_table_bound(t)).map(|t| t.name.clone()).collect();
    let mut used_names: HashSet<String> = dataset_names.iter().map(|n| n.to_lowercase()).collect();

    // synonyms per entity, from the glossary
    let mut synonyms: HashMap<&str, Vec<String>> = HashMap::new();
    for g in &ont.glossary {
        synonyms.entry(g.means.as_str()).or_default().push(g.term.clone());
    }

    // ---- datasets
    let mut datasets: IndexMap<String, Json> = IndexMap::new();
    for t in ont.types.values().filter(|t| is_table_bound(t)) {
        let src = t.source.as_ref().unwrap();
        let key_props: Vec<String> = t.properties.iter().filter(|(_, p)| p.normalize().key).map(|(n, _)| n.clone()).collect();
        let mut fields = Vec::new();
        for (pn, ps) in &t.properties {
            let p = ps.normalize();
            let col = src.columns.get(pn).cloned().unwrap_or_else(|| pn.clone());
            let mut f = Map::new();
            f.insert("name".into(), json!(pn));
            f.insert("expression".into(), json!({ "sql": sql_ident(&col) }));
            if let Some(dt) = datatype_of(&p.kind) {
                f.insert("datatype".into(), json!(dt));
            }
            insert_if(&mut f, "description", json!(p.description));
            let field = Json::Object(f.clone());
            let d = diff(&to_json(&p)?, &property_from_field(&field, p.key), &[]);
            if !d.is_empty() {
                f.insert("nonportable_extensions".into(), ext_list(d));
            }
            fields.push(Json::Object(f));
        }
        let mut ds = Map::new();
        ds.insert("name".into(), json!(t.name));
        ds.insert("source".into(), json!(format!("{}.{}", src.connection, src.table)));
        insert_if(&mut ds, "primary_key", json!(key_props));
        insert_if(&mut ds, "description", json!(t.description));
        if let Some(syn) = synonyms.get(t.name.as_str()) {
            ds.insert("ai_context".into(), json!({ "synonyms": syn }));
        }
        ds.insert("fields".into(), json!(fields));
        let portable = Json::Object(ds.clone());
        let d = diff(&to_json(t)?, &type_from_dataset(&portable, None), &["properties"]);
        if !d.is_empty() {
            ds.insert("nonportable_extensions".into(), ext_list(d));
        }
        datasets.insert(t.name.clone(), Json::Object(ds));
    }

    // ---- metrics: dataset-scoped when their table is one dataset, else carried whole
    let mut model_metrics: Vec<Json> = Vec::new();
    let mut ext_metrics: Vec<Json> = Vec::new();
    let mut metric_tables: IndexMap<(String, String), String> = IndexMap::new();
    for m in ont.metrics.values() {
        let mut placed = false;
        let sql_def = m.sql.as_deref().map(str::trim).filter(|x| !x.is_empty());
        let is_query = sql_def.map(|x| {
            let l = x.to_ascii_lowercase();
            l.starts_with("select") || l.starts_with("with") || l.contains(';')
        });
        if let (Some(sql), Some(false)) = (sql_def, is_query) {
            // a plain expression: model-scoped, as written
            let metric = json!({ "name": m.name, "expression": { "sql": sql } });
            model_metrics.push(finish_metric(metric, m, None)?);
            placed = true;
        } else if let Some(src) = m.source.as_ref().filter(|x| !x.table.is_empty() && x.tool.is_empty()) {
            let mcp = ont.sources.get(&src.connection).map(|x| x.kind.eq_ignore_ascii_case("mcp")).unwrap_or(false);
            let agg = measure_to_sql(&src.measure);
            let filter_ok = src.filter.is_empty() || (balanced(&src.filter) && !src.filter.contains("==") && !src.filter.contains("&&") && !src.filter.contains("||"));
            if let (false, Some(agg), true) = (mcp, agg, filter_ok) {
                if src.filter.is_empty() || single_call(&agg) {
                    let expr = if src.filter.is_empty() { agg } else { format!("{} FILTER (WHERE {})", agg, src.filter.trim()) };
                    // the dataset this table is: an entity's, or one made for the metric
                    let owner = ont
                        .types
                        .values()
                        .find(|t| dataset_names.contains(&t.name) && t.source.as_ref().map(|b| b.connection == src.connection && b.table == src.table).unwrap_or(false))
                        .map(|t| t.name.clone());
                    let ds_name = match owner {
                        Some(n) => n,
                        None => {
                            let key = (src.connection.clone(), src.table.clone());
                            if let Some(n) = metric_tables.get(&key) {
                                n.clone()
                            } else {
                                let mut n = format!("{}_{}", src.connection, src.table.replace('.', "_"));
                                while used_names.contains(&n.to_lowercase()) {
                                    n.push('_');
                                }
                                used_names.insert(n.to_lowercase());
                                let ds = json!({
                                    "name": n,
                                    "source": format!("{}.{}", src.connection, src.table),
                                    "description": format!("The `{}` table in `{}`, which the metric {} reads. It is not an entity in Lorefile.", src.table, src.connection, m.name),
                                    "fields": [],
                                    "nonportable_extensions": ext_list(Map::from_iter([("metric_table".to_string(), json!(true))])),
                                });
                                datasets.insert(n.clone(), ds);
                                metric_tables.insert(key, n.clone());
                                n
                            }
                        }
                    };
                    let synthetic = metric_tables.values().any(|n| n == &ds_name);
                    let ds = datasets.get_mut(&ds_name).unwrap();
                    let expr = rewrite_columns(&expr, &mut |col: &str| {
                        let fields = ds["fields"].as_array_mut().unwrap();
                        if let Some(f) = fields.iter().find(|f| column_of(s(&f["expression"]["sql"])).as_deref() == Some(col)) {
                            return sql_ident(s(&f["name"]));
                        }
                        if synthetic {
                            fields.push(json!({ "name": col, "expression": { "sql": sql_ident(col) } }));
                            sql_ident(col)
                        } else {
                            format!("source.{}", qual_ident(col))
                        }
                    });
                    let mut metric = json!({ "name": m.name, "expression": { "sql": expr } });
                    if expr.to_ascii_uppercase().starts_with("COUNT(") {
                        metric["datatype"] = json!("Integer");
                    }
                    let (c, t) = split_source(s(&ds["source"]));
                    let metric = finish_metric(metric, m, Some((&c, &t)))?;
                    let list = ds.as_object_mut().unwrap().entry("metrics").or_insert(json!([]));
                    list.as_array_mut().unwrap().push(metric);
                    placed = true;
                }
            }
        }
        if !placed {
            ext_metrics.push(to_json(m)?);
            notes.push(format!(
                "metric {} has no Ossie form ({}); it travels in the lorefile extension only",
                m.name,
                if !m.variants.is_empty() { "one definition per variant" } else if m.source.as_ref().map(|x| !x.tool.is_empty()).unwrap_or(false) { "it comes from an MCP tool" } else if m.sql.is_some() { "a full SQL query" } else { "no measure that reads as SQL" }
            ));
        }
    }

    // ---- joins
    let mut joins: Vec<Json> = Vec::new();
    let mut joined: HashMap<String, Join> = HashMap::new();
    let link_names: Vec<&str> = ont.relationships.iter().map(|r| r.name.as_str()).collect();
    let mut rel_names: HashSet<String> = HashSet::new();
    for r in &ont.relationships {
        let Some(j) = infer_join(ont, r, &dataset_names) else { continue };
        let mut name = if link_names.iter().filter(|n| **n == r.name).count() == 1 { r.name.clone() } else { format!("{}_{}", r.from, r.name) };
        while !rel_names.insert(name.to_lowercase()) {
            name.push('_');
        }
        joins.push(json!({
            "name": name,
            "from": j.many,
            "to": j.one,
            "from_columns": j.from_columns,
            "to_columns": j.to_columns,
            "nonportable_extensions": ext_list(Map::from_iter([("link".to_string(), json!(r.qualified()))])),
        }));
        joined.insert(r.qualified(), j);
    }
    let unjoined = ont.relationships.iter().filter(|r| !joined.contains_key(&r.qualified())).count();
    if unjoined > 0 && !dataset_names.is_empty() {
        notes.push(format!(
            "{} of {} links are not Ossie joins (one end has no table, the link is many-to-many, or no key name matches); the ontology document has them all",
            unjoined,
            ont.relationships.len()
        ));
    }

    // ---- what has no Ossie home, verbatim
    let mut root_ext = Map::new();
    root_ext.insert("format".into(), json!(FORMAT));
    let mut manifest = Map::new();
    insert_if(&mut manifest, "version", json!(ont.manifest.version));
    insert_if(&mut manifest, "domains", to_json(&ont.manifest.domains)?);
    insert_if(&mut manifest, "namespace", json!(ont.manifest.namespace));
    insert_if(&mut manifest, "prefixes", to_json(&ont.manifest.prefixes)?);
    insert_if(&mut root_ext, "manifest", Json::Object(manifest));
    insert_if(&mut root_ext, "sources", Json::Array(ont.sources.values().map(to_json).collect::<Result<_>>()?));
    insert_if(&mut root_ext, "entities", Json::Array(ont.types.values().filter(|t| !dataset_names.contains(&t.name)).map(to_json).collect::<Result<_>>()?));
    let standalone: Vec<Json> = ont
        .relationships
        .iter()
        .filter(|r| !ont.types.get(&r.from).map(|t| t.relationships.contains_key(&r.name) && t.file == r.file).unwrap_or(false))
        .map(|r| {
            let mut j = json!({ "relationship": r.name, "from": r.from, "to": r.to, "cardinality": r.cardinality, "description": r.description, "properties": r.properties, "file": r.file });
            if let Some(src) = &r.source {
                j["source"] = to_json(src).unwrap_or(Json::Null);
            }
            j
        })
        .collect();
    insert_if(&mut root_ext, "relationships", Json::Array(standalone));
    insert_if(&mut root_ext, "constraints", Json::Array(ont.constraints.values().map(to_json).collect::<Result<_>>()?));
    insert_if(&mut root_ext, "actions", Json::Array(ont.actions.values().map(to_json).collect::<Result<_>>()?));
    insert_if(&mut root_ext, "functions", Json::Array(ont.functions.values().map(to_json).collect::<Result<_>>()?));
    insert_if(&mut root_ext, "metrics", Json::Array(ext_metrics));
    insert_if(&mut root_ext, "glossary", to_json(&ont.glossary)?);
    // who stands behind what: the whole trust.yaml, read back exactly by Lorefile
    if let Some(book) = ont.trust.as_ref().filter(|b| !crate::trust::is_empty(b)) {
        root_ext.insert("trust".into(), to_json(book)?);
    }
    root_ext.insert("type_order".into(), json!(ont.types.keys().collect::<Vec<_>>()));
    root_ext.insert("metric_order".into(), json!(ont.metrics.keys().collect::<Vec<_>>()));

    let mut sm = Map::new();
    sm.insert("version".into(), json!(VERSION));
    sm.insert("name".into(), json!(space));
    insert_if(&mut sm, "description", json!(ont.manifest.description));
    sm.insert("datasets".into(), Json::Array(datasets.values().cloned().collect()));
    insert_if(&mut sm, "relationships", json!(joins));
    insert_if(&mut sm, "metrics", json!(model_metrics));
    sm.insert("nonportable_extensions".into(), ext_list(root_ext));
    // a proposed definition also says so in ai_context, the note Ossie keeps for AI tools,
    // so tools that ignore our extension still tell their users it is unverified
    let proposed = |k: crate::trust::Kind, n: &str| ont.graded.has_file && ont.grade_of(k, n).grade == Some(Grade::Proposed);
    let flag = |node: &mut Json| match node.get_mut("ai_context") {
        Some(Json::Object(o)) => {
            let t = o.get("instructions").and_then(|x| x.as_str()).map(|x| format!("{} {}", crate::trust::AI_UNVERIFIED, x)).unwrap_or_else(|| crate::trust::AI_UNVERIFIED.to_string());
            o.insert("instructions".into(), json!(t));
        }
        _ => node["ai_context"] = json!({ "instructions": crate::trust::AI_UNVERIFIED }),
    };
    let mut flagged = 0;
    if let Some(dss) = sm.get_mut("datasets").and_then(|d| d.as_array_mut()) {
        for ds in dss.iter_mut() {
            let name = s(&ds["name"]).to_string();
            if proposed(crate::trust::Kind::Binding, &name) {
                flag(ds);
                flagged += 1;
            }
            if let Some(ms) = ds.get_mut("metrics").and_then(|m| m.as_array_mut()) {
                for m in ms.iter_mut() {
                    let n = s(&m["name"]).to_string();
                    if proposed(crate::trust::Kind::Metric, &n) {
                        flag(m);
                        flagged += 1;
                    }
                }
            }
        }
    }
    if let Some(ms) = sm.get_mut("metrics").and_then(|m| m.as_array_mut()) {
        for m in ms.iter_mut() {
            let n = s(&m["name"]).to_string();
            if proposed(crate::trust::Kind::Metric, &n) {
                flag(m);
                flagged += 1;
            }
        }
    }
    if flagged > 0 {
        notes.push(format!("{} proposed definitions say so in ai_context.instructions (\"{}\")", flagged, crate::trust::AI_UNVERIFIED));
    }
    if datasets.is_empty() {
        notes.push("no entity reads from a table (they come from MCP tools or are written inline), so the semantic model has no datasets; Ossie asks for at least one, so other tools may refuse that file. The ontology document still has every entity".into());
    }

    // ---- ontology and mapping
    let (ontology, generated, from_name) = ontology_doc(ont, &space, &mut notes);
    if !generated.is_empty() {
        notes.push(format!("{} links have no sentence, so their verbalization was made from the link name: {}", generated.len(), generated.join(", ")));
    }
    if !from_name.is_empty() {
        notes.push(format!(
            "{} links have a sentence that does not name both entities, so their verbalization was made from the link name and the sentence kept as the description: {}",
            from_name.len(),
            from_name.join(", ")
        ));
    }
    let mapping = mapping_doc(ont, &space, &base, &dataset_names, &joined);
    notes.push(format!("each concept's iri is its entity's IRI: {}<Name>, or the entity's own `iri:`", ont.iris.namespace));

    Ok(Exported { base, semantic_model: Json::Object(sm), ontology, mapping, notes })
}

/// The Ossie metric with its description, and our extension holding what the
/// expression cannot say.
fn finish_metric(mut metric: Json, m: &MetricDef, scope: Option<(&str, &str)>) -> Result<Json> {
    if !m.description.trim().is_empty() {
        metric["description"] = json!(m.description);
    }
    let d = diff(&to_json(m)?, &metric_from_ossie(&metric, scope), &[]);
    if !d.is_empty() {
        metric["nonportable_extensions"] = ext_list(d);
    }
    Ok(metric)
}

/// Every entity as an `EntityType` concept; each property and link as a relationship.
fn ontology_doc(ont: &Ontology, space: &str, notes: &mut Vec<String>) -> (Json, Vec<String>, Vec<String>) {
    let mut generated = Vec::new();
    let mut from_name = Vec::new();
    let mut concepts = Vec::new();
    for t in ont.types.values() {
        let mut rels: Vec<Json> = Vec::new();
        // a kind lists its own links and fields: the rest come through `extends`
        let links: Vec<&Relationship> = ont.relationships.iter().filter(|r| r.from == t.name).collect();
        let link_names: HashSet<&str> = links.iter().map(|r| r.name.as_str()).collect();
        let key = t.key_property();
        for (pn, ps) in t.properties.iter().filter(|(n, _)| !t.inherited.contains_key(*n)) {
            if link_names.contains(pn.as_str()) {
                notes.push(format!("{}.{} is both a property and a link; the ontology lists the link", t.name, pn));
                continue;
            }
            let p = ps.normalize();
            let is_key = key.as_deref() == Some(pn.as_str());
            let mut r = Map::new();
            r.insert("name".into(), json!(pn));
            insert_if(&mut r, "description", json!(p.description.trim()));
            r.insert("multiplicity".into(), json!(if is_key { "OneToOne" } else { "ManyToOne" }));
            r.insert("roles".into(), json!([{ "concept": value_concept(&p.kind) }]));
            let v = value_concept(&p.kind);
            r.insert("verbalizes".into(), json!([format!("{{{}}} {} {} {{{}}}", t.name, if is_key { "is identified by" } else { "has" }, human(pn), v)]));
            rels.push(Json::Object(r));
        }
        for l in &links {
            if !ont.types.contains_key(&l.to) {
                notes.push(format!("link {} points at `{}`, which is not an entity; left out of the ontology", l.qualified(), l.to));
                continue;
            }
            let (pattern, gen) = verbalize(&l.from, &l.to, &l.name, &l.description);
            if gen && l.description.trim().is_empty() {
                generated.push(l.qualified());
            } else if gen {
                from_name.push(l.qualified());
            }
            let mut role = json!({ "concept": l.to });
            if l.from == l.to {
                role["name"] = json!(l.name);
            }
            let mut r = Map::new();
            r.insert("name".into(), json!(l.name));
            insert_if(&mut r, "description", json!(l.description.split_whitespace().collect::<Vec<_>>().join(" ")));
            match l.cardinality.to_ascii_lowercase().replace('_', "-").as_str() {
                "many-to-one" => {
                    r.insert("multiplicity".into(), json!("ManyToOne"));
                }
                "one-to-one" => {
                    r.insert("multiplicity".into(), json!("OneToOne"));
                }
                other => {
                    if !other.is_empty() {
                        r.insert("annotations".into(), json!({ "lorefile:cardinality": other }));
                    }
                }
            }
            r.insert("roles".into(), json!([role]));
            r.insert("verbalizes".into(), json!([pattern]));
            rels.push(Json::Object(r));
        }
        let mut c = Map::new();
        c.insert("concept".into(), json!(t.name));
        c.insert("type".into(), json!("EntityType"));
        if let Some(p) = &t.parent {
            c.insert("extends".into(), json!([p]));
        }
        insert_if(&mut c, "description", json!(t.description.trim()));
        if let Some(k) = key.as_ref().filter(|k| !link_names.contains(k.as_str())) {
            c.insert("identify_by".into(), json!([k]));
        }
        insert_if(&mut c, "relationships", json!(rels));
        c.insert("iri".into(), json!(ont.iri_type(&t.name)));
        let mut ann = Map::new();
        insert_if(&mut ann, "lorefile:domain", json!(ont.domain_of(&t.name)));
        insert_if(&mut ann, "lorefile:owner", json!(t.owner));
        insert_if(&mut ann, "lorefile:tags", json!(t.tags));
        insert_if(&mut ann, "lorefile:synonyms", json!(ont.glossary.iter().filter(|g| g.means == t.name).map(|g| g.term.clone()).collect::<Vec<_>>()));
        insert_if(&mut c, "annotations", Json::Object(ann));
        concepts.push(Json::Object(c));
    }
    let mut ann = Map::new();
    ann.insert(
        "lorefile:note".into(),
        json!("Written by Lorefile. Guardrails, actions and functions are listed here to read; their full definitions travel in the semantic model's nonportable_extensions (vendor_name: lorefile)."),
    );
    insert_if(
        &mut ann,
        "lorefile:guardrails",
        Json::Array(ont.constraints.values().map(|c| json!({ "id": c.id, "name": c.name, "applies_to": c.applies_to, "severity": c.severity.to_string(), "rule": c.rule })).collect()),
    );
    insert_if(
        &mut ann,
        "lorefile:actions",
        Json::Array(ont.actions.values().map(|a| json!({ "name": a.name, "description": first_sentence(&a.description), "inputs": a.inputs, "requires_approval_from": a.requires_approval_from })).collect()),
    );
    insert_if(&mut ann, "lorefile:functions", Json::Array(ont.functions.values().map(|f| json!({ "name": f.name, "description": first_sentence(&f.description) })).collect()));
    let mut o = Map::new();
    o.insert("version".into(), json!(VERSION));
    o.insert("name".into(), json!(space));
    insert_if(&mut o, "description", json!(ont.manifest.description.trim()));
    o.insert("ontology".into(), json!(concepts));
    o.insert("annotations".into(), Json::Object(ann));
    (Json::Object(o), generated, from_name)
}

/// How each table-bound entity's objects and links come from its dataset's fields.
fn mapping_doc(ont: &Ontology, space: &str, base: &str, datasets: &HashSet<String>, joined: &HashMap<String, Join>) -> Json {
    let mut cms = Vec::new();
    for t in ont.types.values().filter(|t| datasets.contains(&t.name)) {
        let links: Vec<&Relationship> = ont.outgoing(&t.name);
        let link_names: HashSet<&str> = links.iter().map(|r| r.name.as_str()).collect();
        let Some(key) = t.key_property().filter(|k| !link_names.contains(k.as_str())) else { continue };
        let ds = qual_ident(&t.name);
        let me = json!({ "referent_mappings": [{ "relationship": key, "expression": format!("{}.{}", ds, qual_ident(&key)) }] });
        let mut children = Vec::new();
        for (pn, ps) in &t.properties {
            if pn == &key || link_names.contains(pn.as_str()) {
                continue;
            }
            children.push(json!({
                "object_mapping": { "concept": value_concept(&ps.normalize().kind), "expression": format!("{}.{}", ds, qual_ident(pn)) },
                "relationship": pn,
            }));
        }
        for l in &links {
            let Some(j) = joined.get(&l.qualified()) else { continue };
            // only a link this entity holds the key of (the many side)
            if j.many != t.name || l.from != t.name {
                continue;
            }
            let Some(to_key) = ont.types.get(&l.to).and_then(|x| x.key_property()) else { continue };
            if ont.outgoing(&l.to).iter().any(|r| r.name == to_key) {
                continue; // that key is not a relationship of its concept
            }
            children.push(json!({
                "object_mapping": { "concept": l.to, "referent_mappings": [{ "relationship": to_key, "expression": format!("{}.{}", ds, qual_ident(&j.from_columns[0])) }] },
                "relationship": l.name,
            }));
        }
        let mut cm = json!({ "concept": t.name, "object_mappings": [me.clone()] });
        if !children.is_empty() {
            cm["link_mappings"] = json!([{ "object_mapping": me, "children": children }]);
        }
        cms.push(cm);
    }
    json!({
        "version": VERSION,
        "name": format!("{}-mapping", space),
        "description": format!("Maps the {} ontology onto the {} semantic model, both written by Lorefile.", space, space),
        "ontology_ref": { "name": space, "iri": format!("./{}.ontology.yaml", base) },
        "semantic_model_ref": { "name": space, "iri": format!("./{}.semantic_model.yaml", base) },
        "concept_mappings": cms,
    })
}

// ===========================================================================
// Import
// ===========================================================================

/// What an import produced: the files (relative path → text), and the notes on
/// what was guessed or dropped.
pub struct Imported {
    pub files: Vec<(String, String)>,
    /// The grades the file claims (our extension's `trust`); imports start ungraded unless `--keep-grades`.
    pub trust: Option<TrustBook>,
    pub notes: Vec<String>,
    pub entities: usize,
    pub links: usize,
    pub metrics: usize,
    pub terms: usize,
}

/// Reads a YAML or JSON document.
pub fn read_doc(path: &Path) -> Result<Json> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let doc: Json = if path.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
        serde_json::from_str(&text).with_context(|| format!("{} is not valid JSON", path.display()))?
    } else {
        serde_yaml::from_str(&text).with_context(|| format!("{} is not valid YAML", path.display()))?
    };
    if !doc.is_object() {
        bail!("{} is not an Ossie document (expected a mapping at the top)", path.display());
    }
    Ok(doc)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FileKind {
    Manifest,
    Types,
    Relationships,
    Constraints,
    Actions,
    Sources,
    Metrics,
    Functions,
    Glossary,
}

/// Objects grouped into files, the way the original space had them when the
/// extension says so, else a plain default layout.
#[derive(Default)]
struct Layout {
    files: IndexMap<String, (Option<FileKind>, Vec<Json>)>,
}

/// A file path from an extension, if it is safe to write and the loader will read it.
fn safe_rel(p: &str) -> Option<String> {
    let path = Path::new(p);
    if p.is_empty() || p.contains('\\') || path.is_absolute() {
        return None;
    }
    // the grades file is never a place an imported object may ask to be written:
    // a file from outside could otherwise carry a ready-made trust.yaml in
    if crate::edit::is_trust_file(p) {
        return None;
    }
    let ext = path.extension()?.to_str()?;
    if ext != "yaml" && ext != "yml" {
        return None;
    }
    for c in path.components() {
        match c {
            Component::Normal(x) => {
                let x = x.to_string_lossy();
                if x.starts_with('.') || x == "data" || x == "target" {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(p.to_string())
}

impl Layout {
    fn add(&mut self, kind: FileKind, wanted: Option<&str>, default: &str, obj: Json) {
        let fits = |l: &Layout, p: &str| l.files.get(p).map(|(k, _)| *k == Some(kind)).unwrap_or(true);
        let mut path = wanted.and_then(safe_rel).filter(|p| fits(self, p)).unwrap_or_else(|| default.to_string());
        let mut n = 2;
        while !fits(self, &path) {
            path = format!("{}-{}.yaml", default.trim_end_matches(".yaml"), n);
            n += 1;
        }
        let e = self.files.entry(path).or_insert((Some(kind), Vec::new()));
        e.1.push(obj);
    }
}

fn prune(v: &mut Json, keys: &[&str]) {
    if let Some(m) = v.as_object_mut() {
        m.shift_remove("file");
        for k in keys {
            let empty = match m.get(*k) {
                Some(Json::Null) | Some(Json::Bool(false)) => true,
                Some(Json::String(x)) => x.is_empty(),
                Some(Json::Array(x)) => x.is_empty(),
                Some(Json::Object(x)) => x.is_empty(),
                _ => false,
            };
            if empty {
                m.shift_remove(*k);
            }
        }
    }
}

/// `{type: string}` with nothing else set reads better as `string`.
fn prune_properties(props: &mut Json) {
    if let Some(m) = props.as_object_mut() {
        for (_, p) in m.iter_mut() {
            if p.is_object() {
                prune(p, &["key", "required", "description", "values", "unit", "iri"]);
                if p.as_object().map(|o| o.len() == 1 && o.contains_key("type")).unwrap_or(false) {
                    *p = p["type"].clone();
                }
            }
        }
    }
}

fn tidy(kind: FileKind, mut v: Json) -> Json {
    match kind {
        FileKind::Types => {
            prune_properties(&mut v["properties"]);
            if let Some(rels) = v.get_mut("relationships").and_then(|r| r.as_object_mut()) {
                for (_, r) in rels.iter_mut() {
                    prune_properties(&mut r["properties"]);
                    prune(r, &["cardinality", "description", "properties", "source"]);
                }
            }
            if let Some(src) = v.get_mut("source") {
                prune(src, &["table", "key", "columns", "tool", "params"]);
            }
            prune(&mut v, &["description", "domain", "properties", "relationships", "source", "owner", "tags", "instances"]);
        }
        FileKind::Relationships => {
            prune_properties(&mut v["properties"]);
            prune(&mut v, &["cardinality", "description", "properties", "source"]);
        }
        FileKind::Constraints => prune(&mut v, &["description", "sql", "rationale", "owner", "tags"]),
        FileKind::Actions => prune(&mut v, &["description", "inputs", "effects", "checks", "requires_approval_from", "side_effects", "tags"]),
        FileKind::Sources => prune(&mut v, &["description", "server", "path"]),
        FileKind::Metrics => prune(&mut v, &[]),
        FileKind::Functions => prune(&mut v, &["description", "inputs", "outputs"]),
        FileKind::Glossary => prune(&mut v, &["description"]),
        FileKind::Manifest => prune(&mut v, &["version", "description", "domains", "namespace", "prefixes"]),
    }
    v
}

fn render(kind: FileKind, objs: Vec<Json>, header: &str) -> String {
    let objs: Vec<Json> = objs.into_iter().map(|o| tidy(kind, o)).collect();
    let single = objs.len() == 1;
    let doc = match kind {
        FileKind::Manifest => objs.into_iter().next().unwrap_or(json!({})),
        FileKind::Types if single => objs.into_iter().next().unwrap(),
        FileKind::Types => json!({ "types": objs }),
        FileKind::Relationships if single => objs.into_iter().next().unwrap(),
        FileKind::Relationships => json!({ "relationships": objs }),
        FileKind::Actions if single => objs.into_iter().next().unwrap(),
        FileKind::Actions => json!({ "actions": objs }),
        FileKind::Sources if single => objs.into_iter().next().unwrap(),
        FileKind::Sources => json!({ "sources": objs }),
        FileKind::Constraints => json!({ "constraints": objs }),
        FileKind::Metrics => json!({ "metrics": objs }),
        FileKind::Functions => json!({ "functions": objs }),
        FileKind::Glossary => json!({ "terms": objs }),
    };
    format!("{}{}", header, to_yaml(&doc))
}

/// `DATASET.field` (optionally quoted, optionally followed by `WHERE …`) in a mapping expression.
fn dataset_field(expr: &str) -> Option<(String, String)> {
    let re = Regex::new(r#"(?is)^\s*"?([A-Za-z_][A-Za-z0-9_$]*)"?\s*\.\s*"?([A-Za-z_][A-Za-z0-9_$]*)"?\s*(where\b.*)?$"#).unwrap();
    re.captures(expr).map(|c| (c[1].to_string(), c[2].to_string()))
}

/// The mapping document, read for what an import can use: which dataset field
/// identifies a concept, and which field holds each of its relationships.
#[derive(Default)]
struct MappingIndex {
    /// concept → (dataset, field) of its simple identifier
    key: HashMap<String, (String, String)>,
    /// (concept, relationship) → (dataset, field, object concept)
    links: HashMap<(String, String), (String, String, String)>,
}

impl MappingIndex {
    fn from(doc: Option<&Json>) -> MappingIndex {
        let mut ix = MappingIndex::default();
        let Some(doc) = doc else { return ix };
        let mappings: Vec<&Json> = arr(&doc["concept_mappings"]).iter().collect();
        let one = |om: &Json| -> Option<(String, String)> {
            if let Some(e) = om["expression"].as_str() {
                return dataset_field(e);
            }
            let refs = arr(&om["referent_mappings"]);
            if refs.len() == 1 {
                refs[0]["expression"].as_str().and_then(dataset_field)
            } else {
                None
            }
        };
        for cm in mappings {
            let concept = s(&cm["concept"]).to_string();
            if let Some(om) = arr(&cm["object_mappings"]).first() {
                if let Some(df) = one(om) {
                    ix.key.entry(concept.clone()).or_insert(df);
                }
            }
            for lm in arr(&cm["link_mappings"]) {
                for child in arr(&lm["children"]) {
                    let rel = s(&child["relationship"]).to_string();
                    if let Some((d, f)) = one(&child["object_mapping"]) {
                        ix.links.entry((concept.clone(), rel)).or_insert((d, f, s(&child["object_mapping"]["concept"]).to_string()));
                    }
                }
            }
        }
        ix
    }
}

/// Folds `RUNWAY` and `Runway`, `order_line` and `OrderLine` together.
fn fold(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// `{Flight} has departure- {Delay}` → `Flight has departure Delay.`
fn render_verbalization(pattern: &str) -> String {
    let ph = Regex::new(r"\{([^}:]+)(?::([^}]+))?\}").unwrap();
    let out = ph.replace_all(pattern, |c: &regex::Captures| c.get(2).map(|r| r.as_str().replace('_', " ")).unwrap_or_else(|| c[1].to_string())).to_string();
    let hyphen = Regex::new(r"(\w)- ").unwrap();
    let out = hyphen.replace_all(&out, "$1 ").trim().to_string();
    if out.ends_with('.') { out } else { format!("{}.", out) }
}

/// Builds the space's files from Ossie documents. Nothing is written here.
pub fn import(sm_path: &Path, ontology_path: Option<&Path>, mapping_path: Option<&Path>, has_manifest: bool) -> Result<Imported> {
    let mut sm = read_doc(sm_path)?;
    let mut notes: Vec<String> = Vec::new();
    if sm.get("ontology").is_some() && sm.get("datasets").is_none() {
        bail!("{} is an Ossie ontology document; pass the semantic model to --from-ossie and this file to --ontology", sm_path.display());
    }
    if sm.get("concept_mappings").is_some() {
        bail!("{} is an Ossie mapping document; pass the semantic model to --from-ossie and this file to --mapping", sm_path.display());
    }
    // the older shape: one model inside a `semantic_model` list
    if let Some(list) = sm.get("semantic_model").and_then(|x| x.as_array()).cloned() {
        if list.len() != 1 {
            bail!("{} holds {} models in the older `semantic_model` list; Ossie {} has one model per document, so split it first", sm_path.display(), list.len(), VERSION);
        }
        notes.push(format!("read the older `semantic_model` list shape; Ossie {} puts the model at the root", VERSION));
        sm = list[0].clone();
    }
    let version = s(&sm["version"]);
    if !version.is_empty() && version != VERSION {
        notes.push(format!("the model says Ossie {}; read as {}", version, VERSION));
    }
    let ontology = ontology_path.map(read_doc).transpose()?;
    let mapping = mapping_path.map(read_doc).transpose()?;
    if let Some(o) = &ontology {
        if o.get("ontology").is_none() {
            bail!("{} is not an Ossie ontology document (no `ontology` list)", ontology_path.unwrap().display());
        }
    }
    let mx = MappingIndex::from(mapping.as_ref());
    let root = our_ext(&sm).unwrap_or_default();
    let model_name = s(&sm["name"]).to_string();
    let mut layout = Layout::default();
    let mut dropped_ext: Vec<String> = other_exts(&sm);

    // ---- concepts from the ontology document
    let mut concepts: IndexMap<String, Json> = IndexMap::new();
    let mut value_types: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(o) = &ontology {
        for c in arr(&o["ontology"]) {
            let name = s(&c["concept"]).to_string();
            if s(&c["type"]) == "ValueType" {
                value_types.insert(name, arr(&c["extends"]).iter().map(|x| s(x).to_string()).collect());
            } else if !name.is_empty() {
                concepts.insert(name, c.clone());
            }
        }
    }
    // a value type's kind: follow `extends` down to a built-in
    let kind_of_concept = |name: &str| -> Option<&'static str> {
        let mut cur = name.to_string();
        for _ in 0..32 {
            if BUILTIN_VALUES.contains(&cur.as_str()) {
                return Some(kind_of_datatype(&cur));
            }
            cur = value_types.get(&cur)?.first()?.clone();
        }
        None
    };

    // ---- entities: datasets first
    struct Entity {
        json: Json,
        file: Option<String>,
        foreign: bool,
        dataset: Option<String>,
    }
    let mut entities: IndexMap<String, Entity> = IndexMap::new();
    let mut ds_entity: HashMap<String, String> = HashMap::new(); // dataset name (folded) → entity
    let mut ds_scope: HashMap<String, (String, String)> = HashMap::new(); // dataset name → (connection, table)
    let mut connections: IndexMap<String, Vec<String>> = IndexMap::new(); // connection → datasets using it
    let datasets: Vec<Json> = arr(&sm["datasets"]).to_vec();
    // which concept each foreign dataset is: same name, or the only concept the mapping draws from it
    let mut claimed: HashSet<String> = HashSet::new();
    let by_name: HashMap<String, String> = concepts.keys().map(|c| (fold(c), c.clone())).collect();
    for ds in &datasets {
        let name = s(&ds["name"]).to_string();
        let (c, t) = split_source(s(&ds["source"]));
        ds_scope.insert(name.clone(), (c.clone(), t.clone()));
        connections.entry(c).or_default().push(name.clone());
        dropped_ext.extend(other_exts(ds));
        for f in arr(&ds["fields"]) {
            dropped_ext.extend(other_exts(f));
        }
        let ext = our_ext(ds);
        if ext.as_ref().map(|x| x.get("metric_table").and_then(|v| v.as_bool()).unwrap_or(false)).unwrap_or(false) {
            continue; // a table a metric reads, not an entity
        }
        let concept = if ext.is_some() {
            None
        } else {
            by_name.get(&fold(&name)).cloned().filter(|c| !claimed.contains(c)).or_else(|| {
                let drawn: Vec<&String> = mx.key.iter().filter(|(_, (d, _))| fold(d) == fold(&name)).map(|(c, _)| c).collect();
                (drawn.len() == 1 && concepts.contains_key(drawn[0]) && !claimed.contains(drawn[0]) && !datasets.iter().any(|o| fold(s(&o["name"])) == fold(drawn[0]))).then(|| drawn[0].clone())
            })
        };
        if let Some(c) = &concept {
            claimed.insert(c.clone());
        }
        let key_hint = concept.as_ref().and_then(|c| mx.key.get(c)).filter(|(d, _)| fold(d) == fold(&name)).map(|(_, f)| f.clone());
        if let Some(key) = &key_hint {
            if arr(&ds["primary_key"]).is_empty() {
                notes.push(format!("dataset {}: key field {} taken from the mapping", name, key));
            }
        }
        let mut tj = type_from_dataset(ds, key_hint.as_deref());
        for f in arr(&ds["fields"]) {
            if let Some(fx) = our_ext(f) {
                let fname = s(&f["name"]);
                if let Some(p) = tj["properties"].get_mut(fname) {
                    overlay(p, &fx);
                }
            }
        }
        let foreign = ext.is_none();
        let mut file = None;
        if let Some(x) = &ext {
            file = x.get("file").and_then(|f| f.as_str()).map(String::from);
            overlay(&mut tj, x);
        } else {
            if let Some(c) = &concept {
                tj["type"] = json!(c);
                if c != &name {
                    notes.push(format!("dataset {} is the entity {} (the ontology concept it maps to)", name, c));
                }
                let cd = s(&concepts[c]["description"]);
                let dd = s(&tj["description"]).to_string();
                if !cd.trim().is_empty() && !dd.contains(cd.trim()) {
                    tj["description"] = json!(if dd.is_empty() { cd.trim().to_string() } else { format!("{} {}", cd.trim(), dd) });
                }
            }
            let untyped = arr(&ds["fields"]).iter().filter(|f| f["datatype"].is_null()).count();
            if untyped > 0 {
                notes.push(format!("dataset {}: {} field(s) had no datatype; read as string unless the ontology says otherwise", name, untyped));
            }
            for f in arr(&ds["fields"]) {
                if let Some(sql) = f["expression"]["sql"].as_str() {
                    if column_of(sql).is_none() {
                        notes.push(format!("{}.{} is computed (`{}`); Lorefile properties name a column, so the expression is kept in its description", name, s(&f["name"]), sql));
                    }
                }
                if f["ai_context"].is_object() || f["ai_context"].is_string() {
                    notes.push(format!("{}.{}: ai_context on a field has no place in Lorefile (a glossary term means an entity, not a property); dropped", name, s(&f["name"])));
                }
            }
            if let Some(ex) = ds["ai_context"].get("examples") {
                if !arr(ex).is_empty() {
                    notes.push(format!("dataset {}: ai_context examples dropped", name));
                }
            }
            if !arr(&ds["unique_keys"]).is_empty() {
                notes.push(format!("dataset {}: unique_keys dropped (Lorefile marks one key)", name));
            }
        }
        let ename = s(&tj["type"]).to_string();
        ds_entity.insert(fold(&name), ename.clone());
        if entities.contains_key(&ename) {
            notes.push(format!("dataset {} would be a second entity named {}; skipped", name, ename));
            continue;
        }
        entities.insert(ename, Entity { json: tj, file, foreign, dataset: Some(name) });
    }
    // entities that are not datasets, carried whole
    for e in arr(&root.get("entities").cloned().unwrap_or(json!([]))) {
        let name = s(&e["type"]).to_string();
        if name.is_empty() || entities.contains_key(&name) {
            continue;
        }
        entities.insert(name, Entity { json: e.clone(), file: e["file"].as_str().map(String::from), foreign: false, dataset: None });
    }
    // concepts nobody bound: entities without data
    let known: HashSet<String> = entities.keys().map(|k| fold(k)).collect();
    for (c, cj) in &concepts {
        if claimed.contains(c) || known.contains(&fold(c)) {
            continue;
        }
        let mut desc = s(&cj["description"]).trim().to_string();
        if let Some((d, f)) = mx.key.get(c) {
            let shown = ds_entity.get(&fold(d)).cloned().unwrap_or_else(|| d.clone());
            desc = format!("{}{}Its values appear in {}.{} (from the Ossie mapping).", desc, if desc.is_empty() { "" } else { " " }, shown, f);
        }
        let tj = json!({ "type": c, "description": desc, "properties": {}, "relationships": {}, "owner": "", "tags": [], "instances": [], "file": "" });
        entities.insert(c.clone(), Entity { json: tj, file: None, foreign: true, dataset: None });
    }

    // ---- what the ontology says about each foreign entity's properties and links
    let entity_by_fold: HashMap<String, String> = entities.keys().map(|k| (fold(k), k.clone())).collect();
    let mut iris = 0;
    let mut rules_dropped = 0;
    let mut wide_dropped: Vec<String> = Vec::new();
    for (c, cj) in &concepts {
        let own_iri = cj.get("iri").map(|_| s(&cj["iri"]).trim().to_string()).filter(|i| !i.is_empty() && i != &default_iri(&model_name, c));
        if own_iri.as_deref().map(|i| !crate::iri::is_absolute(i)).unwrap_or(false) {
            iris += 1;
        }
        rules_dropped += arr(&cj["requires"]).len() + arr(&cj["derived_by"]).len();
        let Some(ename) = entity_by_fold.get(&fold(c)).cloned() else { continue };
        // a concept's `extends`: the first entity it names is the parent; the rest stay tags
        let parents: Vec<String> = arr(&cj["extends"]).iter().map(|x| s(x).to_string()).filter(|p| !value_types.contains_key(p) && !BUILTIN_VALUES.contains(&p.as_str())).collect();
        let parent_entities: Vec<String> = parents.iter().filter_map(|p| entity_by_fold.get(&fold(p)).cloned()).collect();
        let e = entities.get_mut(&ename).unwrap();
        if let Some(first) = parent_entities.first() {
            if e.json.get("extends").is_none() {
                e.json["extends"] = json!(first);
            }
            for other in parent_entities.iter().skip(1) {
                if let Some(tags) = e.json["tags"].as_array_mut() {
                    tags.push(json!(format!("subclass-of:{}", other)));
                }
                notes.push(format!("{} also extends {} in Ossie: an entity extends one other, so it is kept as the tag subclass-of:{}", ename, other, other));
            }
        }
        if !e.foreign {
            continue;
        }
        if let Some(i) = own_iri.filter(|i| crate::iri::is_absolute(i)) {
            e.json["iri"] = json!(i);
        }
        let bound = e.dataset.clone();
        let identify: Vec<String> = arr(&cj["identify_by"]).iter().map(|x| s(x).to_string()).collect();
        for r in arr(&cj["relationships"]) {
            let rname = s(&r["name"]).to_string();
            rules_dropped += arr(&r["requires"]).len() + arr(&r["derived_by"]).len();
            let rel_iri = r.get("iri").map(|_| s(&r["iri"]).trim().to_string()).filter(|i| crate::iri::is_absolute(i));
            if r.get("iri").is_some() && rel_iri.is_none() {
                iris += 1;
            }
            let roles = arr(&r["roles"]);
            let sentence = if !s(&r["description"]).trim().is_empty() {
                s(&r["description"]).trim().to_string()
            } else {
                arr(&r["verbalizes"]).first().map(|v| render_verbalization(s(v))).unwrap_or_default()
            };
            match roles.len() {
                0 => {
                    // unary: a yes/no fact
                    if bound.is_none() && e.json["properties"].get(&rname).is_none() {
                        e.json["properties"][&rname] = json!({ "type": "boolean", "key": false, "required": false, "description": sentence, "values": [] });
                    } else if let Some(p) = e.json["properties"].get_mut(&rname) {
                        if s(&p["description"]).is_empty() {
                            p["description"] = json!(sentence);
                        }
                    } else {
                        notes.push(format!("{}.{}: a yes/no fact with no field of that name in its dataset; dropped", c, rname));
                    }
                }
                1 => {
                    let target = s(&roles[0]["concept"]).to_string();
                    if let Some(to) = entity_by_fold.get(&fold(&target)).filter(|_| !value_types.contains_key(&target) && !BUILTIN_VALUES.contains(&target.as_str())) {
                        let card = match s(&r["multiplicity"]) {
                            "ManyToOne" => "many-to-one",
                            "OneToOne" => "one-to-one",
                            _ => "many-to-many",
                        };
                        if e.json["relationships"].get(&rname).is_none() {
                            e.json["relationships"][&rname] = json!({ "to": to, "cardinality": card, "description": sentence, "properties": {} });
                            if let Some(i) = &rel_iri {
                                e.json["relationships"][&rname]["iri"] = json!(i);
                            }
                        }
                    } else if let Some(kind) = kind_of_concept(&target) {
                        // the field: from the mapping (a link, or the concept's identifier), else one of the same name
                        let by_key = (identify.len() == 1 && identify[0] == rname).then(|| mx.key.get(c)).flatten();
                        let field = match (&bound, mx.links.get(&(c.clone(), rname.clone())).map(|(d, f, _)| (d, f)).or(by_key.map(|(d, f)| (d, f)))) {
                            (Some(d), Some((md, f))) if fold(md) == fold(d) => Some(f.clone()),
                            (Some(_), _) => e.json["properties"].get(&rname).map(|_| rname.clone()),
                            (None, _) => None,
                        };
                        match (&bound, field) {
                            (Some(d), Some(f)) => {
                                if let Some(p) = e.json["properties"].get_mut(&f) {
                                    let untyped = datasets
                                        .iter()
                                        .find(|x| s(&x["name"]) == d)
                                        .and_then(|x| arr(&x["fields"]).iter().find(|y| s(&y["name"]) == f))
                                        .map(|y| y["datatype"].is_null())
                                        .unwrap_or(false);
                                    if untyped {
                                        p["type"] = json!(kind);
                                    }
                                    if s(&p["description"]).is_empty() {
                                        p["description"] = json!(sentence);
                                    }
                                }
                            }
                            (Some(_), None) => notes.push(format!(
                                "{}.{}: no field of its dataset holds it{}; dropped",
                                c,
                                rname,
                                if arr(&r["derived_by"]).is_empty() { "" } else { " (Ossie derives it)" }
                            )),
                            (None, _) => {
                                if e.json["properties"].get(&rname).is_none() {
                                    let key = identify.len() == 1 && identify[0] == rname;
                                    e.json["properties"][&rname] = json!({ "type": kind, "key": key, "required": false, "description": sentence, "values": [] });
                                    if let Some(i) = &rel_iri {
                                        e.json["properties"][&rname]["iri"] = json!(i);
                                    }
                                }
                            }
                        }
                    } else {
                        notes.push(format!("{}.{}: its role `{}` is not a concept in the ontology; dropped", c, rname, target));
                    }
                }
                _ => wide_dropped.push(format!("{}.{}", c, rname)),
            }
        }
        // an unbound entity identified by one value relationship: that property is its key
        if bound.is_none() && identify.len() == 1 {
            if let Some(p) = e.json["properties"].get_mut(&identify[0]) {
                p["key"] = json!(true);
            }
        }
    }
    if iris > 0 {
        notes.push(format!("{} IRIs not kept: they are not absolute IRIs", iris));
    }
    if rules_dropped > 0 {
        notes.push(format!("{} `requires`/`derived_by` expressions in Ossie's ontology language were not converted to guardrails", rules_dropped));
    }
    if !wide_dropped.is_empty() {
        notes.push(format!("{} relationships with three or more roles dropped (Lorefile links join two entities): {}", wide_dropped.len(), wide_dropped.join(", ")));
    }
    if let Some(o) = &ontology {
        let ann = o["annotations"].as_object().map(|a| a.keys().filter(|k| !k.starts_with("lorefile:")).count()).unwrap_or(0);
        if ann > 0 {
            notes.push(format!("{} ontology annotations not kept", ann));
        }
    }

    // ---- links Ossie draws as joins
    let mut existing: HashSet<String> = HashSet::new();
    for (name, e) in &entities {
        for r in e.json["relationships"].as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            existing.insert(format!("{}.{}", name, r));
        }
    }
    let mut standalone: Vec<Json> = Vec::new();
    for r in arr(&root.get("relationships").cloned().unwrap_or(json!([]))) {
        existing.insert(format!("{}.{}", s(&r["from"]), s(&r["relationship"])));
        standalone.push(r.clone());
    }
    let mut foreign_links: Vec<Json> = Vec::new();
    for j in arr(&sm["relationships"]) {
        dropped_ext.extend(other_exts(j));
        if let Some(link) = our_ext(j).and_then(|x| x.get("link").and_then(|l| l.as_str()).map(String::from)) {
            if existing.contains(&link) {
                continue;
            }
        }
        let (Some(from), Some(to)) = (ds_entity.get(&fold(s(&j["from"]))), ds_entity.get(&fold(s(&j["to"])))) else {
            notes.push(format!("join {} names a dataset that is not here; dropped", s(&j["name"])));
            continue;
        };
        let cols = |k: &str| arr(&j[k]).iter().map(|c| s(c).to_string()).collect::<Vec<_>>();
        let on = cols("from_columns")
            .iter()
            .zip(cols("to_columns").iter())
            .map(|(a, b)| format!("{}.{} = {}.{}", from, a, to, b))
            .collect::<Vec<_>>()
            .join(" and ");
        // the ontology already links these two: add the join to its sentence
        let same = entities.get_mut(from.as_str()).and_then(|e| {
            if !e.foreign {
                return None;
            }
            let rels = e.json["relationships"].as_object_mut()?;
            rels.values_mut().find(|r| s(&r["to"]) == to.as_str())
        });
        if let Some(r) = same {
            let d = s(&r["description"]).to_string();
            r["description"] = json!(format!("{}{}Joined on {}.", d, if d.is_empty() { "" } else { " " }, on));
            continue;
        }
        let name = s(&j["name"]).to_string();
        if existing.contains(&format!("{}.{}", from, name)) {
            continue;
        }
        existing.insert(format!("{}.{}", from, name));
        foreign_links.push(json!({
            "relationship": name,
            "from": from,
            "to": to,
            "cardinality": "many-to-one",
            "description": format!("Each {} points at one {}, joined on {}.", from, to, on),
            "properties": {},
            "file": "",
        }));
    }

    // ---- metrics
    let mut metrics: Vec<(Json, Option<String>)> = Vec::new();
    let mut take_metric = |m: &Json, scope: Option<(&str, &str)>, notes: &mut Vec<String>, dropped_ext: &mut Vec<String>| {
        dropped_ext.extend(other_exts(m));
        let mut mj = metric_from_ossie(m, scope);
        let ext = our_ext(m);
        if ext.is_none() {
            if s(&m["expression"]["sql"]).is_empty() {
                match arr(&m["expression"]["dialects"]).first() {
                    Some(d) => notes.push(format!("metric {}: no portable sql; used its {} form", s(&m["name"]), s(&d["dialect"]))),
                    None => {
                        notes.push(format!("metric {} has no expression; dropped", s(&m["name"])));
                        return;
                    }
                }
            }
            if m.get("datatype").is_some() {
                notes.push(format!("metric {}: datatype {} not kept", s(&m["name"]), s(&m["datatype"])));
            }
        }
        let file = ext.as_ref().and_then(|x| x.get("file").and_then(|f| f.as_str()).map(String::from));
        if let Some(x) = &ext {
            overlay(&mut mj, x);
        }
        metrics.push((mj, file));
    };
    for ds in &datasets {
        let (c, t) = ds_scope.get(s(&ds["name"])).cloned().unwrap_or_default();
        for m in arr(&ds["metrics"]) {
            take_metric(m, Some((&c, &t)), &mut notes, &mut dropped_ext);
        }
    }
    for m in arr(&sm["metrics"]) {
        take_metric(m, None, &mut notes, &mut dropped_ext);
    }
    for m in arr(&root.get("metrics").cloned().unwrap_or(json!([]))) {
        metrics.push((m.clone(), m["file"].as_str().map(String::from)));
    }
    if let Some(order) = root.get("metric_order").and_then(|o| o.as_array()) {
        let pos = |n: &str| order.iter().position(|x| s(x) == n).unwrap_or(usize::MAX);
        metrics.sort_by_key(|(m, _)| pos(s(&m["metric"])));
    }

    // ---- glossary: carried whole, plus synonyms other tools added
    let mut terms: Vec<Json> = arr(&root.get("glossary").cloned().unwrap_or(json!([]))).to_vec();
    let mut have: HashSet<String> = terms.iter().map(|t| s(&t["term"]).to_lowercase()).collect();
    let mut add_synonyms = |ctx: &Json, means: &str, terms: &mut Vec<Json>| {
        for syn in arr(&ctx["synonyms"]) {
            let t = s(syn).trim().to_string();
            if !t.is_empty() && have.insert(t.to_lowercase()) {
                terms.push(json!({ "term": t, "means": means, "description": "" }));
            }
        }
    };
    for ds in &datasets {
        if let Some(e) = ds_entity.get(&fold(s(&ds["name"]))) {
            add_synonyms(&ds["ai_context"], e, &mut terms);
        }
        for m in arr(&ds["metrics"]) {
            add_synonyms(&m["ai_context"], s(&m["name"]), &mut terms);
        }
    }
    for m in arr(&sm["metrics"]) {
        add_synonyms(&m["ai_context"], s(&m["name"]), &mut terms);
    }

    // ---- sources: carried whole, plus one per connection a dataset names
    let mut sources: Vec<Json> = arr(&root.get("sources").cloned().unwrap_or(json!([]))).to_vec();
    for x in &sources {
        let k = s(&x["kind"]).to_ascii_lowercase();
        if matches!(k.as_str(), "csv" | "parquet" | "duckdb") {
            notes.push(format!(
                "source {} reads local {} files ({}); an Ossie document carries no rows, so copy them into the new space",
                s(&x["connection"]),
                k,
                x["path"].as_str().unwrap_or("data")
            ));
        }
    }
    let declared: HashSet<String> = sources.iter().map(|x| s(&x["connection"]).to_string()).collect();
    for (conn, users) in &connections {
        if declared.contains(conn) {
            continue;
        }
        let l = conn.to_lowercase();
        let kind = if l.contains("bigquery") || l.starts_with("bq") || l.contains("gcp") {
            Some("bigquery")
        } else if l.contains("snowflake") || l.starts_with("snow") {
            Some("snowflake")
        } else if l.contains("postgres") || l.starts_with("pg") || l.contains("psql") {
            Some("postgres")
        } else {
            None
        };
        let desc = format!(
            "Imported from Apache Ossie: the database the datasets {} name as `{}.…`.{} Agents query it with their own connector; no credentials live here.",
            users.join(", "),
            conn,
            if kind.is_none() { " Ossie does not say what kind of database it is: set `kind:` (bigquery, snowflake, postgres, …)." } else { "" }
        );
        notes.push(match kind {
            Some(k) => format!("source {}: kind {} guessed from its name", conn, k),
            None => format!("source {}: kind not stated by Ossie; declared as `sql`, set it to the real one", conn),
        });
        sources.push(json!({ "connection": conn, "kind": kind.unwrap_or("sql"), "description": desc, "file": "" }));
    }

    // ---- manifest
    if !has_manifest {
        let mut manifest = json!({ "name": if model_name.is_empty() { "imported".to_string() } else { model_name.clone() }, "description": s(&sm["description"]) });
        match &sm["ai_context"] {
            Json::String(t) if !t.trim().is_empty() => manifest["description"] = json!(format!("{}\n\nFor AI tools: {}", s(&manifest["description"]), t.trim()).trim().to_string()),
            Json::Object(o) => {
                if let Some(t) = o.get("instructions").and_then(|x| x.as_str()) {
                    manifest["description"] = json!(format!("{}\n\nFor AI tools: {}", s(&manifest["description"]), t.trim()).trim().to_string());
                }
            }
            _ => {}
        }
        if let Some(m) = root.get("manifest").and_then(|m| m.as_object()) {
            overlay(&mut manifest, m);
        }
        if entities.values().any(|e| e.json.get("extends").is_some()) {
            manifest["format"] = json!(2);
        }
        layout.add(FileKind::Manifest, None, "ontology.yaml", manifest);
    }

    // ---- lay the files out
    let mut ents: Vec<(String, Entity)> = entities.into_iter().collect();
    if let Some(order) = root.get("type_order").and_then(|o| o.as_array()) {
        let pos = |n: &str| order.iter().position(|x| s(x) == n).unwrap_or(usize::MAX);
        ents.sort_by_key(|(n, _)| pos(n));
    }
    let n_entities = ents.len();
    let mut n_links = 0;
    for (name, e) in ents {
        n_links += e.json["relationships"].as_object().map(|m| m.len()).unwrap_or(0);
        let default = format!("types/{}.yaml", snake(&name).replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_"));
        layout.add(FileKind::Types, e.file.as_deref(), &default, e.json);
    }
    n_links += standalone.len() + foreign_links.len();
    for r in standalone {
        let f = r["file"].as_str().map(String::from);
        layout.add(FileKind::Relationships, f.as_deref(), "relationships/links.yaml", r);
    }
    for r in foreign_links {
        layout.add(FileKind::Relationships, None, "relationships/from_ossie.yaml", r);
    }
    for (kind, key, default) in [
        (FileKind::Sources, "sources", "sources/sources.yaml"),
        (FileKind::Constraints, "constraints", "constraints/guardrails.yaml"),
        (FileKind::Actions, "actions", "actions/actions.yaml"),
        (FileKind::Functions, "functions", "logic/functions.yaml"),
    ] {
        let list: Vec<Json> = if key == "sources" { sources.clone() } else { arr(&root.get(key).cloned().unwrap_or(json!([]))).to_vec() };
        for o in list {
            let f = o["file"].as_str().map(String::from);
            let default = if key == "sources" && f.as_deref().unwrap_or("").is_empty() { format!("sources/{}.yaml", slug(s(&o["connection"])).replace('-', "_")) } else { default.to_string() };
            layout.add(kind, f.as_deref(), &default, o);
        }
    }
    let n_metrics = metrics.len();
    for (m, f) in metrics {
        layout.add(FileKind::Metrics, f.as_deref(), "logic/metrics.yaml", m);
    }
    let n_terms = terms.len();
    for t in terms {
        layout.add(FileKind::Glossary, None, "glossary.yaml", t);
    }

    dropped_ext.sort();
    dropped_ext.dedup();
    if !dropped_ext.is_empty() {
        notes.push(format!("nonportable_extensions from {} not kept: Lorefile has nowhere to store another tool's data", dropped_ext.join(", ")));
    }

    let header = format!("# Imported from Apache Ossie{} by `lorefile import`.\n", if model_name.is_empty() { String::new() } else { format!(" ({})", model_name) });
    let files = layout.files.into_iter().filter_map(|(path, (kind, objs))| kind.map(|k| (path, render(k, objs, &header)))).collect();
    let trust: Option<TrustBook> = root.get("trust").map(|t| crate::trust::claims_from_json(t, &mut notes));
    Ok(Imported { files, trust, notes, entities: n_entities, links: n_links, metrics: n_metrics, terms: n_terms })
}

/// The line of `text` an error is most likely about: the last `name` it quotes
/// that appears in the file.
fn line_of(text: &str, message: &str) -> Option<usize> {
    let quoted: Vec<&str> = message.split('`').skip(1).step_by(2).collect();
    for q in quoted.iter().rev() {
        if q.is_empty() {
            continue;
        }
        if let Some(i) = text.lines().position(|l| l.contains(q)) {
            return Some(i + 1);
        }
    }
    None
}

/// What `lorefile import` did.
pub struct Written {
    pub dir: PathBuf,
    pub files: Vec<String>,
    pub imported: Imported,
    pub warnings: usize,
}

/// Builds the files, compiles the space with them laid over it, and writes them
/// only when that compiles with no new errors.
pub fn import_into(sm: &Path, ontology: Option<&Path>, mapping: Option<&Path>, into: &Path, keep_grades: bool) -> Result<Written> {
    let exists = into.exists();
    if exists && !into.is_dir() {
        bail!("{} is a file; --into takes a folder", into.display());
    }
    let has_manifest = exists && (into.join("ontology.yaml").exists() || into.join("ontology.yml").exists());
    if exists && !has_manifest {
        let holds_spaces = std::fs::read_dir(into)?.filter_map(|e| e.ok()).any(|e| e.path().is_dir() && e.path().join("ontology.yaml").exists());
        if holds_spaces {
            bail!("{} is a folder of spaces; import into a new folder inside it, e.g. --into {}", into.display(), into.join("imported").display());
        }
    }
    let mut imported = import(sm, ontology, mapping, has_manifest)?;
    // never overwrite: a file already there gets a sibling name
    let mut files: Vec<(String, String)> = Vec::new();
    for (rel, text) in std::mem::take(&mut imported.files) {
        let mut path = rel.clone();
        let mut n = 1;
        while into.join(&path).exists() || files.iter().any(|(p, _)| p == &path) {
            let stem = rel.trim_end_matches(".yaml").trim_end_matches(".yml");
            path = if n == 1 { format!("{}-ossie.yaml", stem) } else { format!("{}-ossie-{}.yaml", stem, n) };
            n += 1;
        }
        files.push((path, text));
    }
    if has_manifest {
        imported.notes.push(format!("kept the existing ontology.yaml in {}", into.display()));
    }

    // compile: the folder as it is, then with the new files over it
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let scratch = std::env::temp_dir().join(format!("lorefile-import-{}-{}", std::process::id(), nanos));
    let root = if exists { into.to_path_buf() } else {
        std::fs::create_dir_all(&scratch)?;
        scratch.clone()
    };
    let compile = |ov: &Overrides| -> Result<(Ontology, validate::Report)> {
        let ont = Ontology::from_raw(loader::load_with(&root, ov)?);
        let rep = validate::validate(&ont);
        Ok((ont, rep))
    };
    let before: HashSet<(String, String)> = if exists {
        compile(&Overrides::new())?.1.errors().iter().map(|d| (d.code.to_string(), d.message.clone())).collect()
    } else {
        HashSet::new()
    };
    let ov: Overrides = files.iter().map(|(p, t)| (p.clone(), Some(t.clone()))).collect();
    let result = compile(&ov);
    if !exists {
        let _ = std::fs::remove_dir_all(&scratch);
    }
    let (ont, rep) = result?;
    let new_errors: Vec<_> = rep.errors().into_iter().filter(|d| !before.contains(&(d.code.to_string(), d.message.clone()))).collect();
    if !new_errors.is_empty() {
        let lines: Vec<String> = new_errors
            .iter()
            .map(|d| {
                let file = if d.file.is_empty() { d.message.split(':').next().unwrap_or("").to_string() } else { d.file.clone() };
                let line = files.iter().find(|(p, _)| p == &file).and_then(|(_, t)| line_of(t, &d.message));
                match line {
                    Some(n) => format!("error {} {}:{}: {}", d.code, file, n, d.message),
                    None if !d.file.is_empty() => format!("error {} {}: {}", d.code, d.file, d.message),
                    None => format!("error {}: {}", d.code, d.message),
                }
            })
            .collect();
        bail!("the imported space does not compile, so nothing was written:\n{}", lines.join("\n"));
    }

    if let Some(t) = crate::trust::after_import(&ont, imported.trust.as_ref(), keep_grades, into, &mut imported.notes) {
        files.push(t);
    }
    for (rel, text) in &files {
        let p = into.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&p, text).with_context(|| format!("writing {}", p.display()))?;
    }
    Ok(Written { dir: into.to_path_buf(), files: files.into_iter().map(|(p, _)| p).collect(), imported, warnings: rep.warnings().len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_quotes_what_pyyaml_would_misread() {
        let y = to_yaml(&json!({ "a": "yes", "b": "2024-01-01", "c": "010", "d": "plain words, fine", "e": "x: y", "f": "line one\nline two\n", "g": "" }));
        let back: Json = serde_yaml::from_str(&y).unwrap();
        assert_eq!(back["a"], "yes");
        assert_eq!(back["b"], "2024-01-01");
        assert_eq!(back["c"], "010");
        assert_eq!(back["f"], "line one\nline two\n");
        assert!(y.contains("a: \"yes\"") && y.contains("b: \"2024-01-01\"") && y.contains("d: plain words, fine"), "{}", y);
    }

    #[test]
    fn measures_become_sql() {
        assert_eq!(measure_to_sql("count distinct order_id, by date(placed_on, 'Europe/London')").as_deref(), Some("COUNT(DISTINCT order_id)"));
        assert_eq!(measure_to_sql("sum(revenue)").as_deref(), Some("sum(revenue)"));
        assert_eq!(measure_to_sql("sum revenue_usd").as_deref(), Some("SUM(revenue_usd)"));
        assert_eq!(measure_to_sql("the number of happy customers"), None);
    }

    #[test]
    fn column_names_are_rewritten_and_nothing_else() {
        let out = rewrite_columns("COUNT(DISTINCT order_id) FILTER (WHERE payment_status = 'paid' and status not in ('cancelled', 'returned') and x.y > 1)", &mut |c| format!("<{}>", c));
        assert_eq!(out, "COUNT(DISTINCT <order_id>) FILTER (WHERE <payment_status> = 'paid' and <status> not in ('cancelled', 'returned') and x.y > 1)");
    }

    #[test]
    fn a_sentence_becomes_a_pattern() {
        assert_eq!(verbalize("Order", "Customer", "placed_by", "An Order is placed by one Customer.").0, "An {Order} is placed by one {Customer}");
        assert_eq!(verbalize("OrderLine", "Order", "part_of", "An order line belongs to one Order.").0, "An {OrderLine} belongs to one {Order}");
        assert_eq!(verbalize("Order", "PaymentMethod", "paid_with", "The card charged at checkout.").0, "{Order} paid with {PaymentMethod}");
        assert_eq!(verbalize("Part", "Part", "replaces", "").0, "{Part} replaces {Part:replaces}");
        assert_eq!(render_verbalization("{Flight} has departure- {Delay}"), "Flight has departure Delay.");
    }
}
