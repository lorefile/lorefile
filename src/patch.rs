//! Friendly edits. The page sends a small operation (describe a field, add a
//! link, add a guardrail, create a thing); this turns it into the full text of
//! one YAML file, which then goes through `edit::apply` like any other write,
//! so it is compiled and checked before anything touches disk.
//!
//! Type files are rewritten through serde_yaml, which keeps key order but
//! drops comments in that one file. Guardrails are appended as text and keep
//! the file as it was.

use crate::edit;
use crate::loader::Overrides;
use crate::ontology::{snake, Ontology};
use serde_json::Value as Json;
use serde_yaml::{Mapping, Value as Yaml};
use std::collections::HashMap;

/// What an operation plans against: the compiled ontology plus the files a
/// batch has already changed but not written. A second op on the same file
/// reads the first op's text, not the disk.
pub struct Workspace<'a> {
    pub ont: &'a Ontology,
    pub pending: &'a HashMap<String, String>,
}

impl Workspace<'_> {
    fn exists(&self, rel: &str) -> bool {
        self.pending.contains_key(rel) || self.ont.root.join(rel).is_file()
    }

    fn read(&self, rel: &str) -> Result<String, String> {
        if let Some(t) = self.pending.get(rel) {
            return Ok(t.clone());
        }
        std::fs::read_to_string(self.ont.root.join(rel)).map_err(|e| format!("reading {}: {}", rel, e))
    }
}

/// Several ops as one change. Each op is planned against the ontology as it
/// would be after the ones before it, so "add the source, then the entity,
/// then bind it, then the metric" works in one sitting. Returns the final text
/// of every touched file plus one sentence per op.
pub struct BatchPlan {
    pub files: Vec<(String, String)>,
    pub summaries: Vec<String>,
    pub kinds: Vec<(String, String)>,
}

pub fn plan_many(ont: &Ontology, ops: &[Json]) -> Result<BatchPlan, String> {
    if ops.is_empty() {
        return Err("`ops` is empty".into());
    }
    if ops.len() > 40 {
        return Err("at most 40 ops in one batch".into());
    }
    let mut pending: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut summaries = Vec::new();
    let mut kinds = Vec::new();
    let mut current: Option<Ontology> = None;
    for (i, op) in ops.iter().enumerate() {
        let p = {
            let ws = Workspace { ont: current.as_ref().unwrap_or(ont), pending: &pending };
            plan_in(&ws, op).map_err(|e| format!("op {} ({}): {}", i + 1, field(op, "op"), e))?
        };
        if !pending.contains_key(&p.file) {
            order.push(p.file.clone());
        }
        pending.insert(p.file.clone(), p.text.clone());
        summaries.push(p.summary.clone());
        kinds.push((p.kind.clone(), p.name.clone()));
        // the next op sees the ontology with everything so far in place
        let ov: Overrides = pending.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect();
        let (o, _) = edit::check_many(&ont.root, &ov).map_err(|e| format!("op {}: {:#}", i + 1, e))?;
        current = Some(o);
    }
    let files = order.into_iter().map(|f| { let t = pending.remove(&f).unwrap_or_default(); (f, t) }).collect();
    Ok(BatchPlan { files, summaries, kinds })
}

/// Append one item to the top-level list `list_key` of `rel` as text, matching
/// the indentation the file already uses so comments and order survive. A
/// missing file is created with just that list.
fn append_list_item(ws: &Workspace, rel: &str, list_key: &str, item: Yaml) -> Result<(String, String), String> {
    let rel = edit::safe_rel_path(rel)?;
    edit::resolve_in_root(&ws.ont.root, &rel)?;
    let rendered = serde_yaml::to_string(&Yaml::Sequence(vec![item])).map_err(|e| e.to_string())?;
    if !ws.exists(&rel) {
        let body: String = rendered.lines().map(|l| format!("  {}\n", l)).collect();
        return Ok((rel, format!("{}:\n{}", list_key, body)));
    }
    let existing = ws.read(&rel)?;
    let doc: Yaml = serde_yaml::from_str(&existing).map_err(|e| format!("{} does not parse: {}", rel, e))?;
    let has_list = doc.as_mapping().map(|m| m.contains_key(key(list_key))).unwrap_or(false);
    if !has_list {
        return Err(format!("{} has no top-level `{}:` list to add to", rel, list_key));
    }
    // `terms: []` (a blank space) or `terms: [a, b]`: turn the flow list into a block list first
    let flow_line = existing.lines().enumerate().find(|(_, l)| l.starts_with(&format!("{}:", list_key)) && l.trim_end().ends_with(']')).map(|(i, l)| (i, l.to_string()));
    let (existing, indent) = match flow_line {
        Some((i, line)) => {
            let is_empty = line[list_key.len() + 1..].trim() == "[]";
            if !is_empty {
                return Err(format!("{} keeps `{}:` on one line; put it on several lines first", rel, list_key));
            }
            let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
            lines[i] = format!("{}:", list_key);
            (lines.join("\n") + "\n", 2)
        }
        None => {
            let indent = existing.lines().find(|l| l.trim_start().starts_with("- ") && l.contains(':')).map(|l| l.len() - l.trim_start().len()).unwrap_or(2);
            (existing, indent)
        }
    };
    let pad = " ".repeat(indent);
    let body: String = rendered.lines().map(|l| format!("{}{}\n", pad, l)).collect();
    let mut out = existing;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&body);
    Ok((rel, out))
}

/// A metric source from the flat fields of a request: connection + table or tool, params, filter, measure.
fn metric_source(ont: &Ontology, body: &Json, what: &str) -> Result<Mapping, String> {
    let connection = field(body, "connection");
    if connection.is_empty() {
        return Err(format!("{} needs a connection (a source under sources/)", what));
    }
    let src = ont.sources.get(connection).ok_or_else(|| format!("no connection named `{}`; add it under sources/ first", connection))?;
    let is_mcp = src.kind.eq_ignore_ascii_case("mcp");
    let table = field(body, "table");
    let tool = field(body, "tool");
    if is_mcp && tool.is_empty() {
        return Err(format!("`{}` is an MCP source, so {} needs the tool to call", connection, what));
    }
    if !is_mcp && table.is_empty() {
        return Err(format!("{} needs the table in `{}`", what, connection));
    }
    let mut m = Mapping::new();
    m.insert(key("connection"), Yaml::String(connection.to_string()));
    if !table.is_empty() {
        m.insert(key("table"), Yaml::String(table.to_string()));
    }
    if !tool.is_empty() {
        m.insert(key("tool"), Yaml::String(tool.to_string()));
    }
    if let Some(p) = body.get("params").filter(|p| p.as_object().map(|o| !o.is_empty()).unwrap_or(false)) {
        m.insert(key("params"), serde_yaml::to_value(p).map_err(|e| e.to_string())?);
    }
    for k in ["filter", "measure"] {
        let v = field(body, k);
        if !v.is_empty() {
            m.insert(key(k), Yaml::String(v.to_string()));
        }
    }
    Ok(m)
}

/// `basis` from a request: transactional, attributed, modelled or experimental.
fn basis(body: &Json) -> Result<Option<String>, String> {
    match field(body, "basis") {
        "" => Ok(None),
        b @ ("transactional" | "attributed" | "modelled" | "experimental") => Ok(Some(b.to_string())),
        other => Err(format!("basis must be transactional, attributed, modelled or experimental, not `{}`", other)),
    }
}

/// `record_check`: an agent reports that a definition gave the right number
/// against real rows. Planned here (the thing exists, the source exists, a
/// person has accepted it), returned as a claim for a person to confirm.
pub fn plan_check(ont: &Ontology, body: &Json) -> Result<crate::proposals::CheckClaim, String> {
    let kind = match field(body, "kind") {
        "" => None,
        k => Some(crate::trust::Kind::parse(k).ok_or_else(|| format!("kind `{}` cannot be checked; one of metric, binding, guardrail", k))?),
    };
    let name = { let n = field(body, "name"); if n.is_empty() { field(body, "metric") } else { n } };
    let (kind, name) = edit::find_graded(ont, kind, name)?;
    if !kind.can_be_checked() {
        return Err("a word has no rows to check against".into());
    }
    let against = field(body, "against");
    if against.is_empty() || !ont.sources.contains_key(against) {
        return Err(format!("`against` must name the source whose rows were used (one of {})", ont.sources.keys().cloned().collect::<Vec<_>>().join(", ")));
    }
    let note = crate::trust::clean_note(field(body, "note"));
    if note.is_empty() {
        return Err("`note` says what was compared with what, e.g. \"412 paid orders for 16–22 Nov, the same as Shopify's own report\"".into());
    }
    let st = ont.grade_of(kind, &name);
    if st.grade.is_none() || st.grade == Some(crate::schema::Grade::Proposed) {
        return Err(format!("{} {} is {}; a person has to accept it first, then a check can be recorded", kind.word(), name, if st.changed { "changed since it was graded".to_string() } else { st.word().to_string() }));
    }
    Ok(crate::proposals::CheckClaim { kind: match kind { crate::trust::Kind::Metric => "metric", crate::trust::Kind::Binding => "binding", crate::trust::Kind::Guardrail => "guardrail", crate::trust::Kind::Term => "term" }.into(), of: crate::trust::things(ont, kind).get(&name).map(crate::trust::fingerprint).unwrap_or_default(), name, against: against.to_string(), note })
}

pub struct Patch {
    pub file: String,
    pub text: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
}

/// Names go into file names, URLs and the page: letters, digits, spaces and `_ . -`, starting with a letter.
fn plain_name(s: &str, what: &str) -> Result<(), String> {
    let ok = s.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ' ')
        && s.len() <= 80;
    if ok { Ok(()) } else { Err(format!("{} must start with a letter and use only letters, digits, spaces, `_`, `-` and `.` (got `{}`)", what, s)) }
}

fn field<'a>(body: &'a Json, k: &str) -> &'a str {
    body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim()
}

fn key(s: &str) -> Yaml {
    Yaml::String(s.to_string())
}

/// Insert `k: v` right after `after` (or at the end), replacing an existing `k` in place.
fn insert_after(m: &mut Mapping, after: &str, k: &str, v: Yaml) {
    if m.contains_key(key(k)) {
        m.insert(key(k), v);
        return;
    }
    let mut out = Mapping::new();
    let mut placed = false;
    for (kk, vv) in m.iter() {
        out.insert(kk.clone(), vv.clone());
        if kk.as_str() == Some(after) {
            out.insert(key(k), v.clone());
            placed = true;
        }
    }
    if !placed {
        out.insert(key(k), v);
    }
    *m = out;
}

/// Read a type's file, find its mapping (single `type:` file or an item of a
/// `types:` list), let `f` change it, and return (file, new text).
fn edit_type_file(ws: &Workspace, type_name: &str, f: impl FnOnce(&mut Mapping) -> Result<(), String>) -> Result<(String, String), String> {
    let ont = ws.ont;
    let t = ont.type_(type_name).ok_or_else(|| format!("no type named `{}`", type_name))?;
    let text = ws.read(&t.file)?;
    let mut doc: Yaml = serde_yaml::from_str(&text).map_err(|e| format!("{} does not parse: {}", t.file, e))?;
    let root = doc.as_mapping_mut().ok_or_else(|| format!("{} is not a mapping", t.file))?;
    let found = if let Some(Yaml::Sequence(items)) = root.get_mut(key("types")) {
        items.iter_mut().filter_map(|i| i.as_mapping_mut()).find(|m| m.get(key("type")).and_then(|v| v.as_str()) == Some(&t.name))
    } else if root.get(key("type")).and_then(|v| v.as_str()) == Some(&t.name) {
        Some(root)
    } else {
        None
    };
    let m = found.ok_or_else(|| format!("could not find `{}` inside {}", t.name, t.file))?;
    f(m)?;
    let out = serde_yaml::to_string(&doc).map_err(|e| e.to_string())?;
    Ok((t.file.clone(), out))
}

fn severity_ok(s: &str) -> Result<String, String> {
    match s {
        "" => Ok("warn".into()),
        "block" | "warn" | "info" => Ok(s.into()),
        other => Err(format!("severity must be block, warn or info, not `{}`", other)),
    }
}

/// A rule id like `OR-003`: initials of the thing it applies to, then the next free number.
fn next_rule_id(ont: &Ontology, applies_to: &str) -> String {
    let parts: Vec<String> = snake(applies_to).split('_').filter(|p| !p.is_empty()).map(|p| p.to_string()).collect();
    let mut prefix: String = parts.iter().filter_map(|p| p.chars().next()).take(3).collect::<String>().to_uppercase();
    if prefix.len() < 2 {
        prefix = applies_to.chars().filter(|c| c.is_ascii_alphabetic()).take(2).collect::<String>().to_uppercase();
    }
    let n = ont.constraints.keys().filter(|k| k.starts_with(&format!("{}-", prefix))).count() + 1;
    format!("{}-{:03}", prefix, n)
}

pub fn plan(ont: &Ontology, body: &Json) -> Result<Patch, String> {
    let none = HashMap::new();
    plan_in(&Workspace { ont, pending: &none }, body)
}

pub fn plan_in(ws: &Workspace, body: &Json) -> Result<Patch, String> {
    let ont = ws.ont;
    let op = field(body, "op");
    match op {
        "set_type_description" => {
            let t = field(body, "type").to_string();
            let text = field(body, "text").to_string();
            let (file, out) = edit_type_file(ws, &t, |m| {
                insert_after(m, "type", "description", Yaml::String(text.clone()));
                Ok(())
            })?;
            Ok(Patch { file, text: out, kind: "type".into(), name: t.clone(), summary: format!("described {}", t) })
        }
        "set_property_description" => {
            let t = field(body, "type").to_string();
            let prop = field(body, "property").to_string();
            let text = field(body, "text").to_string();
            let (file, out) = edit_type_file(ws, &t, |m| {
                let props = m.get_mut(key("properties")).and_then(|p| p.as_mapping_mut()).ok_or_else(|| format!("{} has no properties block", t))?;
                let entry = props.get_mut(key(&prop)).ok_or_else(|| format!("{} has no field `{}`", t, prop))?;
                match entry {
                    Yaml::String(kind) => {
                        let mut mm = Mapping::new();
                        mm.insert(key("type"), Yaml::String(kind.clone()));
                        mm.insert(key("description"), Yaml::String(text.clone()));
                        *entry = Yaml::Mapping(mm);
                    }
                    Yaml::Mapping(mm) => {
                        mm.insert(key("description"), Yaml::String(text.clone()));
                    }
                    _ => return Err(format!("field `{}` has an unexpected shape", prop)),
                }
                Ok(())
            })?;
            Ok(Patch { file, text: out, kind: "type".into(), name: t.clone(), summary: format!("described {}.{}", t, prop) })
        }
        "set_relationship_description" => {
            let from = field(body, "type").to_string();
            let name = field(body, "relationship").to_string();
            let text = field(body, "text").to_string();
            let rel = ont.relationships.iter().find(|r| r.from == from && r.name == name).ok_or_else(|| format!("no link `{}` from {}", name, from))?;
            let t = ont.type_(&from).ok_or_else(|| format!("no type named `{}`", from))?;
            if rel.file == t.file {
                let (file, out) = edit_type_file(ws, &from, |m| {
                    let rels = m.get_mut(key("relationships")).and_then(|p| p.as_mapping_mut()).ok_or_else(|| format!("{} has no relationships block", from))?;
                    let entry = rels.get_mut(key(&name)).and_then(|e| e.as_mapping_mut()).ok_or_else(|| format!("{} has no link `{}`", from, name))?;
                    entry.insert(key("description"), Yaml::String(text.clone()));
                    Ok(())
                })?;
                return Ok(Patch { file, text: out, kind: "relationship".into(), name: format!("{}.{}", from, name), summary: format!("described the link {} {}", from, name) });
            }
            // standalone relationship file
            let full = ont.root.join(&rel.file);
            let src = std::fs::read_to_string(&full).map_err(|e| format!("reading {}: {}", rel.file, e))?;
            let mut doc: Yaml = serde_yaml::from_str(&src).map_err(|e| format!("{} does not parse: {}", rel.file, e))?;
            let root = doc.as_mapping_mut().ok_or_else(|| format!("{} is not a mapping", rel.file))?;
            if let Some(Yaml::Sequence(items)) = root.get_mut(key("relationships")) {
                let item = items.iter_mut().filter_map(|i| i.as_mapping_mut()).find(|m| m.get(key("relationship")).and_then(|v| v.as_str()) == Some(&name) && m.get(key("from")).and_then(|v| v.as_str()) == Some(&from)).ok_or_else(|| format!("could not find `{}` in {}", name, rel.file))?;
                item.insert(key("description"), Yaml::String(text));
            } else {
                root.insert(key("description"), Yaml::String(text));
            }
            let out = serde_yaml::to_string(&doc).map_err(|e| e.to_string())?;
            Ok(Patch { file: rel.file.clone(), text: out, kind: "relationship".into(), name: format!("{}.{}", from, name), summary: format!("described the link {} {}", from, name) })
        }
        "add_relationship" => {
            let from = field(body, "type").to_string();
            let name = snake(field(body, "name"));
            let to = field(body, "to").to_string();
            let card = { let c = field(body, "cardinality"); if c.is_empty() { "many-to-one".to_string() } else { c.to_string() } };
            let desc = field(body, "description").to_string();
            if name.is_empty() {
                return Err("the link needs a name, like `placed_by`".into());
            }
            plain_name(&name, "a link name")?;
            if ont.type_(&to).is_none() {
                return Err(format!("no type named `{}` to link to", to));
            }
            if ont.relationships.iter().any(|r| r.from == from && r.name == name) {
                return Err(format!("{} already has a link called `{}`", from, name));
            }
            let (file, out) = edit_type_file(ws, &from, |m| {
                if !m.contains_key(key("relationships")) {
                    insert_after(m, "properties", "relationships", Yaml::Mapping(Mapping::new()));
                }
                let rels = m.get_mut(key("relationships")).and_then(|p| p.as_mapping_mut()).ok_or_else(|| "relationships is not a mapping".to_string())?;
                let mut e = Mapping::new();
                e.insert(key("to"), Yaml::String(to.clone()));
                e.insert(key("cardinality"), Yaml::String(card.clone()));
                if !desc.is_empty() {
                    e.insert(key("description"), Yaml::String(desc.clone()));
                }
                rels.insert(key(&name), Yaml::Mapping(e));
                Ok(())
            })?;
            Ok(Patch { file, text: out, kind: "relationship".into(), name: format!("{}.{}", from, name), summary: format!("linked {} {} {}", from, name, to) })
        }
        "add_constraint" => {
            let applies_to = field(body, "applies_to").to_string();
            let name = field(body, "name").to_string();
            let rule = field(body, "rule").to_string();
            if applies_to.is_empty() || name.is_empty() || rule.is_empty() {
                return Err("a guardrail needs applies_to, name and rule".into());
            }
            plain_name(&name, "a guardrail name")?;
            if !field(body, "id").is_empty() {
                plain_name(field(body, "id"), "a guardrail id")?;
            }
            if ont.kind_of(&applies_to).is_none() {
                return Err(format!("`{}` is not a type, link or action", applies_to));
            }
            let id = { let i = field(body, "id"); if i.is_empty() { next_rule_id(ont, &applies_to) } else { i.to_string() } };
            if ont.constraint(&id).is_some() {
                return Err(format!("a guardrail with id {} already exists", id));
            }
            let severity = severity_ok(field(body, "severity"))?;
            let mut m = Mapping::new();
            m.insert(key("id"), Yaml::String(id.clone()));
            m.insert(key("name"), Yaml::String(name.clone()));
            m.insert(key("applies_to"), Yaml::String(applies_to.clone()));
            m.insert(key("rule"), Yaml::String(rule));
            m.insert(key("severity"), Yaml::String(severity));
            let rationale = field(body, "rationale");
            if !rationale.is_empty() {
                m.insert(key("rationale"), Yaml::String(rationale.to_string()));
            }
            let owner = field(body, "owner");
            if !owner.is_empty() {
                m.insert(key("owner"), Yaml::String(owner.to_string()));
            }
            let content = serde_yaml::to_string(&Yaml::Mapping(m)).map_err(|e| e.to_string())?;
            // put it with its siblings: the file other rules on the same thing live in
            let explicit = field(body, "file");
            let sibling = ont.constraints.values().find(|c| c.applies_to == applies_to).map(|c| c.file.clone());
            let file = if !explicit.is_empty() { explicit.to_string() } else { sibling.unwrap_or_else(|| format!("constraints/{}.yaml", snake(&applies_to))) };
            let (rel, text) = edit::plan_create(&ont.root, "constraint", &id, &content, Some(&file))?;
            Ok(Patch { file: rel, text, kind: "constraint".into(), name: id.clone(), summary: format!("added guardrail {}: {}", id, name) })
        }
        "set_source" => {
            let t = field(body, "type").to_string();
            let connection = field(body, "connection").to_string();
            let table = field(body, "table").to_string();
            let tool = field(body, "tool").to_string();
            let keyp = field(body, "key").to_string();
            if connection.is_empty() {
                return Err("a binding needs a connection".into());
            }
            let src_def = ont.sources.get(&connection).ok_or_else(|| format!("no connection named `{}`; add it under sources/ first", connection))?;
            let is_mcp = src_def.kind.eq_ignore_ascii_case("mcp");
            if is_mcp && tool.is_empty() {
                return Err(format!("`{}` is an MCP source, so the binding needs the tool that returns the rows", connection));
            }
            if !is_mcp && table.is_empty() {
                return Err("a binding needs a connection and a table".into());
            }
            let params = body.get("params").filter(|p| p.as_object().map(|o| !o.is_empty()).unwrap_or(false)).cloned();
            let (file, out) = edit_type_file(ws, &t, |m| {
                let mut src = Mapping::new();
                src.insert(key("connection"), Yaml::String(connection.clone()));
                if !table.is_empty() {
                    src.insert(key("table"), Yaml::String(table.clone()));
                }
                if !tool.is_empty() {
                    src.insert(key("tool"), Yaml::String(tool.clone()));
                }
                if let Some(p) = &params {
                    src.insert(key("params"), serde_yaml::to_value(p).map_err(|e| e.to_string())?);
                }
                if !keyp.is_empty() {
                    src.insert(key("key"), Yaml::String(keyp.clone()));
                }
                insert_after(m, "relationships", "source", Yaml::Mapping(src));
                Ok(())
            })?;
            let where_ = if is_mcp { format!("{} tool {}", connection, tool) } else { format!("{}.{}", connection, table) };
            Ok(Patch { file, text: out, kind: "type".into(), name: t.clone(), summary: format!("bound {} to {}", t, where_) })
        }
        "add_source" => {
            let connection = field(body, "connection").to_string();
            let kind = field(body, "kind").to_string().to_ascii_lowercase();
            if connection.is_empty() || kind.is_empty() {
                return Err("a source needs `connection` (its name) and `kind` (bigquery, snowflake, postgres, csv, parquet, s3, duckdb, mcp…)".into());
            }
            if !connection.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("a connection name is letters, digits and underscores, like `shop_bigquery` or `meta_ads`".into());
            }
            if ont.sources.contains_key(&connection) {
                return Err(format!("a source named `{}` already exists", connection));
            }
            let mut m = Mapping::new();
            m.insert(key("connection"), Yaml::String(connection.clone()));
            m.insert(key("kind"), Yaml::String(kind.clone()));
            if kind == "mcp" {
                let server = field(body, "server");
                if server.is_empty() {
                    return Err("a `kind: mcp` source needs `server`: the name of the MCP server agents call".into());
                }
                m.insert(key("server"), Yaml::String(server.to_string()));
            }
            m.insert(key("description"), Yaml::String(field(body, "description").to_string()));
            for k in ["path", "project", "dataset", "database", "schema", "account", "warehouse", "env", "url"] {
                let v = field(body, k);
                if !v.is_empty() {
                    if v.contains("password") || v.contains("secret") || v.contains("token") {
                        return Err(format!("`{}` looks like a credential; sources name an environment variable instead, never the value", k));
                    }
                    m.insert(key(k), Yaml::String(v.to_string()));
                }
            }
            let content = serde_yaml::to_string(&Yaml::Mapping(m)).map_err(|e| e.to_string())?;
            let (rel, text) = edit::plan_create(&ont.root, "source", &connection, &content, None)?;
            Ok(Patch { file: rel, text, kind: "source".into(), name: connection.clone(), summary: format!("added the {} source {}", kind, connection) })
        }
        "add_term" => {
            let term = field(body, "term").to_string();
            let means = field(body, "means").to_string();
            if term.is_empty() || means.is_empty() {
                return Err("a glossary term needs `term` (the word people use) and `means` (the entity it refers to)".into());
            }
            plain_name(&term, "a glossary word")?;
            if ont.glossary.iter().any(|g| g.term.eq_ignore_ascii_case(&term)) {
                return Err(format!("the glossary already has `{}`", term));
            }
            if ont.type_(&means).is_none() {
                return Err(format!("`{}` is not an entity here; add it first or name the entity the word refers to", means));
            }
            let mut m = Mapping::new();
            m.insert(key("term"), Yaml::String(term.clone()));
            m.insert(key("means"), Yaml::String(means.clone()));
            let d = field(body, "description");
            if !d.is_empty() {
                m.insert(key("description"), Yaml::String(d.to_string()));
            }
            let file = { let f = field(body, "file"); if f.is_empty() { "glossary.yaml".to_string() } else { f.to_string() } };
            let (rel, text) = append_list_item(ws, &file, "terms", Yaml::Mapping(m))?;
            Ok(Patch { file: rel, text, kind: "term".into(), name: term.clone(), summary: format!("added the word \"{}\" for {}", term, means) })
        }
        "add_metric" => {
            let name = field(body, "metric").to_string();
            if name.is_empty() {
                return Err("a metric needs a name; `success` is the conventional name for the one number the business is run on".into());
            }
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("a metric name is letters, digits and underscores, like `success` or `blended_cac`".into());
            }
            if ont.metrics.contains_key(&name) {
                return Err(format!("a metric named `{}` already exists; edit it in the Logic step instead", name));
            }
            let mut m = Mapping::new();
            m.insert(key("metric"), Yaml::String(name.clone()));
            m.insert(key("description"), Yaml::String(field(body, "description").to_string()));
            let variants = body.get("variants").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let sql = field(body, "sql");
            if !variants.is_empty() {
                let mut list = Vec::new();
                for v in &variants {
                    let when = field(v, "when");
                    if when.is_empty() {
                        return Err("each variant needs `when`: the condition it applies to, like product_line == \"saas\"".into());
                    }
                    let mut vm = Mapping::new();
                    vm.insert(key("when"), Yaml::String(when.to_string()));
                    let d = field(v, "description");
                    if !d.is_empty() {
                        vm.insert(key("description"), Yaml::String(d.to_string()));
                    }
                    vm.insert(key("source"), Yaml::Mapping(metric_source(ont, v, &format!("variant `{}`", when))?));
                    if let Some(b) = basis(v)? {
                        vm.insert(key("basis"), Yaml::String(b));
                    }
                    list.push(Yaml::Mapping(vm));
                }
                m.insert(key("variants"), Yaml::Sequence(list));
            } else if !field(body, "connection").is_empty() {
                m.insert(key("source"), Yaml::Mapping(metric_source(ont, body, "the metric")?));
            } else if sql.is_empty() {
                return Err("a metric needs where it comes from: a connection with a table or tool, variants, or sql".into());
            }
            for k in ["grain", "sql", "owner", "caveats"] {
                let v = field(body, k);
                if !v.is_empty() {
                    m.insert(key(k), Yaml::String(v.to_string()));
                }
            }
            if let Some(b) = basis(body)? {
                m.insert(key("basis"), Yaml::String(b));
            }
            // join the file the other metrics already live in, else logic/metrics.yaml
            let explicit = field(body, "file");
            let sibling = ont.metrics.values().next().map(|x| x.file.clone());
            let file = if !explicit.is_empty() { explicit.to_string() } else { sibling.unwrap_or_else(|| "logic/metrics.yaml".to_string()) };
            let (rel, text) = append_list_item(ws, &file, "metrics", Yaml::Mapping(m))?;
            let where_ = if !variants.is_empty() { format!("{} variants", variants.len()) } else if !sql.is_empty() { "sql".to_string() } else { let t = field(body, "tool"); if t.is_empty() { format!("{}.{}", field(body, "connection"), field(body, "table")) } else { format!("{} tool {}", field(body, "connection"), t) } };
            Ok(Patch { file: rel, text, kind: "metric".into(), name: name.clone(), summary: format!("added metric {} ({})", name, where_) })
        }
        "add_type" => {
            let name = field(body, "name").to_string();
            if name.is_empty() {
                return Err("the entity needs a name".into());
            }
            plain_name(&name, "an entity name")?;
            if ont.type_(&name).is_some() {
                return Err(format!("a type named `{}` already exists", name));
            }
            let mut m = Mapping::new();
            m.insert(key("type"), Yaml::String(name.clone()));
            let domain = field(body, "domain");
            if !domain.is_empty() {
                m.insert(key("domain"), Yaml::String(domain.to_string()));
            }
            m.insert(key("description"), Yaml::String(field(body, "description").to_string()));
            let mut props = Mapping::new();
            let mut idp = Mapping::new();
            idp.insert(key("type"), Yaml::String("string".into()));
            idp.insert(key("key"), Yaml::Bool(true));
            idp.insert(key("description"), Yaml::String(format!("Identifier of one {}", name)));
            props.insert(key(&format!("{}_id", snake(&name))), Yaml::Mapping(idp));
            m.insert(key("properties"), Yaml::Mapping(props));
            let content = serde_yaml::to_string(&Yaml::Mapping(m)).map_err(|e| e.to_string())?;
            let (rel, text) = edit::plan_create(&ont.root, "type", &name, &content, None)?;
            Ok(Patch { file: rel, text, kind: "type".into(), name: name.clone(), summary: format!("created {}", name) })
        }
        "add_function" => {
            let name = field(body, "function").to_string();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("a function needs a name of letters, digits and underscores, like `forecast_success`".into());
            }
            if ont.functions.contains_key(&name) {
                return Err(format!("a function named `{}` already exists", name));
            }
            let kind = field(body, "kind");
            let server = field(body, "server");
            let tool = field(body, "tool");
            let url = field(body, "url");
            let runs = match kind {
                "" => return Err("say where it runs: kind mcp (with server and tool), http (with url), or manual".into()),
                "mcp" => {
                    if server.is_empty() || tool.is_empty() {
                        return Err("kind mcp needs `server` (a kind: mcp source) and `tool`".into());
                    }
                    if !ont.sources.values().any(|s| s.kind.eq_ignore_ascii_case("mcp") && (s.connection == server || s.server.as_deref() == Some(server))) {
                        return Err(format!("no `kind: mcp` source declares the server `{}`; add it under sources/ first", server));
                    }
                    let mut r = Mapping::new();
                    r.insert(key("kind"), Yaml::String("mcp".into()));
                    r.insert(key("server"), Yaml::String(server.to_string()));
                    r.insert(key("tool"), Yaml::String(tool.to_string()));
                    if let Some(a) = body.get("args").filter(|a| a.as_object().map(|o| !o.is_empty()).unwrap_or(false)) {
                        r.insert(key("args"), serde_yaml::to_value(a).map_err(|e| e.to_string())?);
                    }
                    r
                }
                "http" => {
                    if url.is_empty() {
                        return Err("kind http needs `url`".into());
                    }
                    let mut r = Mapping::new();
                    r.insert(key("kind"), Yaml::String("http".into()));
                    r.insert(key("url"), Yaml::String(url.to_string()));
                    let method = field(body, "method");
                    if !method.is_empty() {
                        r.insert(key("method"), Yaml::String(method.to_string()));
                    }
                    r
                }
                "manual" => {
                    let mut r = Mapping::new();
                    r.insert(key("kind"), Yaml::String("manual".into()));
                    r
                }
                other => return Err(format!("runs.kind `{}` is not mcp, http or manual", other)),
            };
            let io = |k: &str| -> Result<Mapping, String> {
                let mut m = Mapping::new();
                if let Some(o) = body.get(k).and_then(|v| v.as_object()) {
                    for (n, t) in o {
                        let t = t.as_str().unwrap_or("").trim();
                        if t.is_empty() {
                            return Err(format!("{} `{}` needs a type: an entity name or string/integer/number/boolean/date/json", k, n));
                        }
                        m.insert(key(n), Yaml::String(t.to_string()));
                    }
                }
                Ok(m)
            };
            let mut m = Mapping::new();
            m.insert(key("function"), Yaml::String(name.clone()));
            m.insert(key("description"), Yaml::String(field(body, "description").to_string()));
            let inputs = io("inputs")?;
            let outputs = io("outputs")?;
            if !inputs.is_empty() {
                m.insert(key("inputs"), Yaml::Mapping(inputs));
            }
            if !outputs.is_empty() {
                m.insert(key("outputs"), Yaml::Mapping(outputs));
            }
            m.insert(key("runs"), Yaml::Mapping(runs));
            if let Some(list) = body.get("may_be_called_by").and_then(|v| v.as_array()) {
                let roles: Vec<Yaml> = list.iter().filter_map(|x| x.as_str()).map(|x| Yaml::String(x.to_string())).collect();
                if !roles.is_empty() {
                    m.insert(key("may_be_called_by"), Yaml::Sequence(roles));
                }
            }
            for k in ["owner", "caveats"] {
                let v = field(body, k);
                if !v.is_empty() {
                    m.insert(key(k), Yaml::String(v.to_string()));
                }
            }
            let explicit = field(body, "file");
            let sibling = ont.functions.values().next().map(|x| x.file.clone());
            let file = if !explicit.is_empty() { explicit.to_string() } else { sibling.unwrap_or_else(|| "logic/functions.yaml".to_string()) };
            let (rel, text) = append_list_item(ws, &file, "functions", Yaml::Mapping(m))?;
            let how = match kind { "mcp" => format!("tool {} on {}", tool, server), "http" => url.to_string(), _ => "done by a person".to_string() };
            Ok(Patch { file: rel, text, kind: "function".into(), name: name.clone(), summary: format!("added function {} ({})", name, how) })
        }
        "record_check" => Err("record_check reports a check, it changes no file; send it on its own, not in `ops`".into()),
        other => Err(format!("unknown op `{}`; one of add_type, set_type_description, set_property_description, add_relationship, set_relationship_description, add_constraint, set_source, add_source, add_term, add_metric, add_function, record_check", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_ontology;
    use serde_json::json;
    use std::path::Path;

    fn ecommerce() -> Ontology {
        load_ontology(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ecommerce")).unwrap().0
    }

    #[test]
    fn add_term_appends_to_the_glossary_with_its_indentation() {
        let ont = ecommerce();
        let p = plan(&ont, &json!({ "op": "add_term", "term": "cart", "means": "Order", "description": "What the app calls an unpaid order." })).unwrap();
        assert_eq!(p.file, "glossary.yaml");
        assert!(p.text.starts_with("terms:\n"), "existing file kept");
        assert!(p.text.ends_with("  - term: cart\n    means: Order\n    description: What the app calls an unpaid order.\n"), "{}", p.text);
        assert!(p.text.contains("- term: basket"), "existing terms kept");
        assert!(plan(&ont, &json!({ "op": "add_term", "term": "SKU", "means": "Variant" })).is_err(), "duplicates refused");
        assert!(plan(&ont, &json!({ "op": "add_term", "term": "x", "means": "Nope" })).is_err(), "unknown entity refused");
    }

    #[test]
    fn the_first_word_in_a_blank_space_turns_the_empty_list_into_a_block() {
        let dir = std::env::temp_dir().join(format!("lorefile-patch-blank-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::write(dir.join("ontology.yaml"), "name: blank\n").unwrap();
        std::fs::write(dir.join("glossary.yaml"), "# words\nterms: []\n").unwrap();
        std::fs::write(dir.join("types/order.yaml"), "type: Order\nproperties: { order_id: { type: string, key: true } }\n").unwrap();
        let ont = load_ontology(&dir).unwrap().0;
        let p = plan(&ont, &json!({ "op": "add_term", "term": "checkout", "means": "Order" })).unwrap();
        assert_eq!(p.text, "# words\nterms:\n  - term: checkout\n    means: Order\n", "{}", p.text);
        let parsed: serde_yaml::Value = serde_yaml::from_str(&p.text).unwrap();
        assert_eq!(parsed["terms"][0]["term"], "checkout");
        let s = plan(&ont, &json!({ "op": "add_source", "connection": "shop_warehouse", "kind": "bigquery", "project": "acme", "dataset": "shop", "description": "The warehouse." })).unwrap();
        assert_eq!(s.file, "sources/shop_warehouse.yaml");
        assert!(s.text.contains("kind: bigquery") && s.text.contains("dataset: shop"), "{}", s.text);
        assert!(plan(&ont, &json!({ "op": "add_source", "connection": "x", "kind": "mcp" })).is_err(), "mcp needs a server");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blank_space_is_taught_in_one_batch() {
        let dir = std::env::temp_dir().join(format!("lorefile-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::write(dir.join("ontology.yaml"), "name: blank\ndomains:\n  - name: Core\n    types: []\n").unwrap();
        std::fs::write(dir.join("glossary.yaml"), "terms: []\n").unwrap();
        let ont = load_ontology(&dir).unwrap().0;
        let ops = vec![
            json!({ "op": "add_source", "connection": "shop", "kind": "bigquery", "project": "acme", "dataset": "shop", "description": "The warehouse." }),
            json!({ "op": "add_type", "name": "Order", "domain": "Core", "description": "One checkout." }),
            json!({ "op": "set_source", "type": "Order", "connection": "shop", "table": "orders", "key": "order_id" }),
            json!({ "op": "add_metric", "metric": "success", "description": "Paid orders.", "connection": "shop", "table": "orders", "filter": "payment_status = 'paid'", "measure": "count distinct order_id", "grain": "day", "owner": "growth" }),
            json!({ "op": "add_term", "term": "basket", "means": "Order" }),
        ];
        let plan = plan_many(&ont, &ops).unwrap();
        let files: Vec<&str> = plan.files.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(files, vec!["sources/shop.yaml", "types/order.yaml", "logic/metrics.yaml", "glossary.yaml"], "{:?}", files);
        let order = &plan.files[1].1;
        assert!(order.contains("type: Order") && order.contains("connection: shop") && order.contains("table: orders"), "the binding landed on the new entity:\n{}", order);
        assert_eq!(plan.summaries.len(), 5);
        // the whole batch compiles clean together
        let ov: Overrides = plan.files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect();
        let (o, report) = edit::check_many(&dir, &ov).unwrap();
        assert!(report.ok(), "{:?}", report.diags);
        assert!(o.metrics.contains_key("success") && o.glossary.iter().any(|g| g.term == "basket"));
        // and the single-sitting failure of the trial is gone: the metric's source was only pending
        assert!(plan_many(&ont, &ops[3..4]).is_err(), "alone, the metric still needs its source first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_metric_builds_a_recipe_and_refuses_half_a_binding() {
        let ont = ecommerce();
        let p = plan(&ont, &json!({ "op": "add_metric", "metric": "returns_rate", "description": "Returned orders over orders.", "connection": "shop_bigquery", "table": "orders", "filter": "returned", "measure": "count distinct order_id", "grain": "week", "owner": "ops@lumenhome" })).unwrap();
        assert_eq!(p.file, "logic/measurement.yaml", "joins the file the other metrics live in");
        assert!(p.text.contains("  - metric: success"), "existing metrics kept");
        assert!(p.text.ends_with("      measure: count distinct order_id\n    grain: week\n    owner: ops@lumenhome\n"), "{}", p.text);
        assert_eq!(p.summary, "added metric returns_rate (shop_bigquery.orders)");
        let v = plan(&ont, &json!({ "op": "add_metric", "metric": "spend", "description": "x", "variants": [{ "when": "channel == 'meta'", "connection": "meta_ads", "tool": "get_insights", "params": { "level": "campaign" } }] })).unwrap();
        assert!(v.text.contains("tool: get_insights") && v.text.contains("level: campaign"), "{}", v.text);
        assert!(plan(&ont, &json!({ "op": "add_metric", "metric": "bad", "connection": "meta_ads" })).is_err(), "an MCP source needs a tool");
        assert!(plan(&ont, &json!({ "op": "add_metric", "metric": "success", "connection": "shop_bigquery", "table": "orders" })).is_err(), "exists already");
        assert!(plan(&ont, &json!({ "op": "add_metric", "metric": "lonely" })).is_err(), "needs a source");
    }
}
