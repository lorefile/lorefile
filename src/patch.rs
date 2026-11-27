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
use serde_json::{json, Value as Json};
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
    /// The file each op wrote, in op order: `summaries[i]` belongs to `op_files[i]`.
    pub op_files: Vec<String>,
}

impl BatchPlan {
    /// The sentences of the ops that wrote `file`, joined: a file's own heading.
    pub fn summary_for(&self, file: &str) -> String {
        self.summaries.iter().zip(self.op_files.iter()).filter(|(_, f)| f.as_str() == file).map(|(s, _)| s.as_str()).collect::<Vec<_>>().join("; ")
    }
}

/// Ops that write more than one file, so they go through `plan_many` and are saved as one change.
pub const MULTI_FILE_OPS: [&str; 1] = ["make_parent"];

/// Does this one op write several files?
pub fn multi_file(op: &Json) -> bool {
    MULTI_FILE_OPS.contains(&field(op, "op"))
}

pub fn plan_many(ont: &Ontology, ops: &[Json]) -> Result<BatchPlan, String> {
    plan_many_over(ont, &[], ops)
}

/// `plan_many` on top of files not written yet (a proposal being accepted with a person's
/// fixes): the ops read those texts, and the result holds them plus whatever the ops changed.
pub fn plan_many_over(ont: &Ontology, start: &[(String, String)], ops: &[Json]) -> Result<BatchPlan, String> {
    if ops.is_empty() {
        return Err("`ops` is empty".into());
    }
    if ops.len() > 40 {
        return Err("at most 40 ops in one batch".into());
    }
    let mut pending: HashMap<String, String> = start.iter().cloned().collect();
    let mut order: Vec<String> = start.iter().map(|(f, _)| f.clone()).collect();
    let mut summaries = Vec::new();
    let mut kinds = Vec::new();
    let mut op_files = Vec::new();
    let mut current: Option<Ontology> = if start.is_empty() {
        None
    } else {
        let ov: Overrides = pending.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect();
        Some(edit::check_many(&ont.root, &ov).map_err(|e| format!("{:#}", e))?.0)
    };
    // `make_parent` is several files: the new parent, and each child made a kind of it
    let mut expanded: Vec<Json> = Vec::new();
    for op in ops {
        if field(op, "op") == "make_parent" {
            let ws = Workspace { ont: current.as_ref().unwrap_or(ont), pending: &pending };
            expanded.extend(expand_make_parent(&ws, op)?);
        } else {
            expanded.push(op.clone());
        }
    }
    let ops: &[Json] = &expanded;
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
        op_files.push(p.file.clone());
        // the next op sees the ontology with everything so far in place
        let ov: Overrides = pending.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect();
        let (o, _) = edit::check_many(&ont.root, &ov).map_err(|e| format!("op {}: {:#}", i + 1, e))?;
        current = Some(o);
    }
    let files = order.into_iter().map(|f| { let t = pending.remove(&f).unwrap_or_default(); (f, t) }).collect();
    Ok(BatchPlan { files, summaries, kinds, op_files })
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
        return Err(format!("{} is {}; a person has to accept it first, then a check can be recorded", kind.name_words(&name), if st.changed { "changed since it was graded".to_string() } else { st.word().to_string() }));
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

/// `where` as the op sent it, `{"kind": "promo"}` or `{"channel": ["email", "sms"]}`, as YAML.
fn where_yaml(body: &Json) -> Result<Option<Yaml>, String> {
    match body.get("where") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(o)) if o.is_empty() => Ok(None),
        Some(Json::Object(o)) => {
            let mut w = Mapping::new();
            for (f, v) in o {
                prop_name_ok(f)?;
                let y: Yaml = serde_yaml::to_value(v).map_err(|e| e.to_string())?;
                match &y {
                    Yaml::String(_) | Yaml::Number(_) | Yaml::Bool(_) => {}
                    Yaml::Sequence(vs) if !vs.is_empty() && vs.iter().all(|x| matches!(x, Yaml::String(_) | Yaml::Number(_) | Yaml::Bool(_))) => {}
                    _ => return Err(format!("`where.{}` is one value or a list of values, e.g. {{\"kind\": \"promo\"}}", f)),
                }
                w.insert(key(f), y);
            }
            Ok(Some(Yaml::Mapping(w)))
        }
        Some(_) => Err("`where` maps a parent's field to a value, e.g. {\"kind\": \"promo\"}: no SQL".into()),
    }
}

/// `make_parent {name, description, children, fields}` as the ops it stands for: `add_type` for the
/// parent with the shared fields (the most common words among the children, ties to the first),
/// then `set_kind_of` for each child. Refused in words on a type or unit clash, a taken name, or
/// a child that already extends something else.
fn expand_make_parent(ws: &Workspace, op: &Json) -> Result<Vec<Json>, String> {
    let ont = ws.ont;
    let name = field(op, "name").to_string();
    plain_name(&name, "an entity name")?;
    if ont.type_(&name).is_some() {
        return Err(format!("make_parent: the name {} is taken", name));
    }
    let children: Vec<String> = op.get("children").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    if children.len() < 2 {
        return Err("make_parent needs `children`: the entities that become kinds of the new parent".into());
    }
    let mut kids: Vec<crate::schema::TypeDef> = Vec::new();
    for c in &children {
        let t = ont.type_(c).ok_or_else(|| format!("make_parent: there is no entity called {}", c))?;
        if let Some(p) = &t.parent {
            return Err(format!("make_parent: {} is already a kind of {}", t.name, p));
        }
        kids.push(t.clone());
    }
    let names: Vec<String> = kids.iter().map(|k| k.name.clone()).collect();
    let fields: Vec<String> = match op.get("fields").and_then(|f| f.as_array()) {
        Some(list) if !list.is_empty() => list.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
        _ => crate::validate::shared_core(ont, &names),
    };
    if fields.is_empty() {
        return Err(format!("make_parent: {} share no field of the same name and type", names.join(", ")));
    }
    let mut props = serde_json::Map::new();
    for f in &fields {
        let specs: Vec<crate::schema::Property> = kids.iter().map(|k| k.properties.get(f).map(|p| p.normalize()).ok_or_else(|| format!("make_parent: {} has no field `{}`", k.name, f))).collect::<Result<_, _>>()?;
        if specs.iter().any(|p| p.kind != specs[0].kind) {
            return Err(format!("make_parent: `{}` is {} in {} but {} in {}; a shared field needs one type", f, specs[0].kind, kids[0].name, specs.iter().find(|p| p.kind != specs[0].kind).map(|p| p.kind.clone()).unwrap_or_default(), kids[specs.iter().position(|p| p.kind != specs[0].kind).unwrap_or(0)].name));
        }
        if specs.iter().any(|p| p.unit != specs[0].unit) {
            return Err(format!("make_parent: `{}` has different units in {}; a shared field needs one unit", f, names.join(", ")));
        }
        // the most common words, ties going to the first child
        let mut counts: Vec<(String, usize)> = Vec::new();
        for p in &specs {
            let d = p.description.trim().to_string();
            if d.is_empty() {
                continue;
            }
            match counts.iter_mut().find(|(x, _)| *x == d) {
                Some(e) => e.1 += 1,
                None => counts.push((d, 1)),
            }
        }
        let best = counts.iter().fold(None::<&(String, usize)>, |acc, x| match acc { Some(a) if a.1 >= x.1 => Some(a), _ => Some(x) }).map(|x| x.0.clone()).unwrap_or_default();
        let mut spec = serde_json::Map::new();
        spec.insert("type".into(), json!(specs[0].kind));
        if let Some(u) = &specs[0].unit {
            spec.insert("unit".into(), json!(u));
        }
        if !best.is_empty() {
            spec.insert("description".into(), json!(best));
        }
        props.insert(f.clone(), Json::Object(spec));
    }
    let keys: Vec<Option<String>> = kids.iter().map(|k| k.key_property()).collect();
    let mut add = json!({ "op": "add_type", "name": name, "description": field(op, "description"), "properties": props });
    if let Some(Some(k)) = keys.first() {
        if keys.iter().all(|x| x.as_deref() == Some(k.as_str())) && fields.contains(k) {
            add["key"] = json!(k);
        }
    }
    let mut out = vec![add];
    for c in &names {
        out.push(json!({ "op": "set_kind_of", "type": c, "extends": name }));
    }
    Ok(out)
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

/// A field name goes into YAML keys, SQL and the page: letters, digits and `_`, starting with a letter.
fn prop_name_ok(s: &str) -> Result<(), String> {
    let ok = s.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && s.len() <= 80;
    if ok { Ok(()) } else { Err(format!("a field name starts with a letter and uses only letters, digits and `_` (got `{}`); map a header like `Unit Price ($)` with `columns`", s)) }
}

/// One property as given (`"integer"` or `{type, key, unit, description}`), checked the way a
/// file is: an unknown key such as `descripton` is an error. Written as given, in its order.
fn property_yaml(name: &str, spec: &Json) -> Result<Yaml, String> {
    match spec {
        Json::String(kind) if !kind.trim().is_empty() => Ok(Yaml::String(kind.trim().to_string())),
        Json::Object(_) => {
            serde_json::from_value::<crate::schema::Property>(spec.clone()).map_err(|e| format!("property `{}`: {}", name, e))?;
            serde_yaml::to_value(spec).map_err(|e| e.to_string())
        }
        _ => Err(format!("property `{}` is a type name (`integer`) or a mapping ({{type, key, unit, description}})", name)),
    }
}

/// Mark a property as the key, turning the shorthand `id: string` into a mapping.
fn set_key(entry: &mut Yaml) {
    match entry {
        Yaml::String(kind) => {
            let mut mm = Mapping::new();
            mm.insert(key("type"), Yaml::String(kind.clone()));
            mm.insert(key("key"), Yaml::Bool(true));
            *entry = Yaml::Mapping(mm);
        }
        Yaml::Mapping(mm) => {
            if !mm.contains_key(key("key")) {
                let mut out = Mapping::new();
                for (k, v) in mm.iter() {
                    out.insert(k.clone(), v.clone());
                    if k.as_str() == Some("type") {
                        out.insert(key("key"), Yaml::Bool(true));
                    }
                }
                if !out.contains_key(key("key")) {
                    out.insert(key("key"), Yaml::Bool(true));
                }
                *mm = out;
            } else {
                mm.insert(key("key"), Yaml::Bool(true));
            }
        }
        _ => {}
    }
}

/// The lines of one item in a file's text: `(first, end, key indent)`, `end` exclusive. A file of
/// one item (`type: X` at the top) is the whole file; a list item is `- ` at the list's indent up
/// to the next item or top-level key. `None` for a flow-style item (`- { … }`) or none found.
fn item_block(text: &str, list: &str, id_key: &str, id: &str) -> Option<(usize, usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let value_is = |rest: &str| -> bool {
        let v = rest.split(" #").next().unwrap_or("").trim().trim_matches('"').trim_matches('\'');
        v == id
    };
    // a single-item file
    if lines.iter().any(|l| l.strip_prefix(&format!("{}:", id_key)).map(value_is).unwrap_or(false)) && !lines.iter().any(|l| l.starts_with(&format!("{}:", list))) {
        return Some((0, lines.len(), 0));
    }
    let start = lines.iter().position(|l| l.trim_end() == format!("{}:", list))?;
    let mut i = start + 1;
    let dash = lines.iter().skip(start + 1).find(|l| l.trim_start().starts_with("- ") || l.trim_start() == "-").map(|l| l.len() - l.trim_start().len())?;
    while i < lines.len() {
        let l = lines[i];
        let ind = l.len() - l.trim_start().len();
        let content = !l.trim().is_empty() && !l.trim_start().starts_with('#');
        if content && ind < dash.max(1) && !(dash == 0 && l.starts_with("- ")) {
            break; // the list ended
        }
        if ind == dash && l.trim_start().starts_with("- ") {
            let first = i;
            let mut j = i + 1;
            while j < lines.len() {
                let m = lines[j];
                let mi = m.len() - m.trim_start().len();
                if !m.trim().is_empty() && !m.trim_start().starts_with('#') && (mi < dash || (mi == dash && m.trim_start().starts_with("- ")) || (dash == 0 && mi == 0 && !m.starts_with("- "))) {
                    break;
                }
                j += 1;
            }
            let key = dash + 2;
            let head = l.trim_start()[2..].trim_start();
            if head.starts_with('{') {
                if head.contains(&format!("{}: {}", id_key, id)) {
                    return None;
                }
            } else {
                let mine = head.strip_prefix(&format!("{}:", id_key)).map(value_is).unwrap_or(false)
                    || lines[first + 1..j].iter().any(|m| m.len() - m.trim_start().len() == key && m.trim_start().strip_prefix(&format!("{}:", id_key)).map(value_is).unwrap_or(false));
                if mine {
                    return Some((first, j, key));
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    None
}

/// The item's text with its `owner:` line set to `owner` (or removed when empty), keeping every
/// other line and comment. `None` when the item can't be found as block text.
fn set_owner_line(text: &str, list: &str, id_key: &str, id: &str, owner: &str) -> Option<String> {
    let (first, end, key) = item_block(text, list, id_key, id)?;
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let is_owner_line = |idx: usize, l: &str| -> bool {
        let ind = l.len() - l.trim_start().len();
        let t = l.trim_start();
        (ind == key && t.starts_with("owner:")) || (idx == first && key >= 2 && t.strip_prefix("- ").map(|r| r.starts_with("owner:")).unwrap_or(false))
    };
    let at = (first..end).find(|i| is_owner_line(*i, &lines[*i]));
    match (at, owner.is_empty()) {
        (Some(i), true) => {
            if i == first && key >= 2 {
                return None; // `- owner: X` heads the item: leave that to the mapping
            }
            lines.remove(i);
        }
        (Some(i), false) => {
            let l = lines[i].clone();
            let head = l.find("owner:").map(|p| p + "owner:".len())?;
            let comment = l.find(" #").filter(|p| *p > head).map(|p| l[l[..p].trim_end().len()..].to_string()).unwrap_or_default();
            lines[i] = format!("{} {}{}", &l[..head], crate::people::yaml_scalar(owner), comment);
        }
        (None, true) => {}
        (None, false) => {
            // after the item's last line of content
            let last = (first..end).rev().find(|i| !lines[*i].trim().is_empty() && !lines[*i].trim_start().starts_with('#')).unwrap_or(first);
            lines.insert(last + 1, format!("{}owner: {}", " ".repeat(key), crate::people::yaml_scalar(owner)));
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') || !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// The item's mapping in a parsed file: a single-item file, or the list entry with `id_key: id`.
fn find_item<'a>(doc: &'a mut Yaml, list: &str, id_key: &str, id: &str) -> Option<&'a mut Mapping> {
    let root = doc.as_mapping_mut()?;
    if root.get(key(id_key)).and_then(|v| v.as_str()) == Some(id) {
        return Some(root);
    }
    root.get_mut(key(list))?.as_sequence_mut()?.iter_mut().filter_map(|i| i.as_mapping_mut()).find(|m| m.get(key(id_key)).and_then(|v| v.as_str()) == Some(id))
}

/// The owner the item has in `text`, by parsing it.
fn owner_in_text(text: &str, list: &str, id_key: &str, id: &str) -> Option<String> {
    let mut doc: Yaml = serde_yaml::from_str(text).ok()?;
    find_item(&mut doc, list, id_key, id)?.get(key("owner")).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// `set_owner` through the mapping: the fallback, which drops the file's comments.
fn set_owner_mapping(text: &str, list: &str, id_key: &str, id: &str, owner: &str) -> Result<String, String> {
    let mut doc: Yaml = serde_yaml::from_str(text).map_err(|e| format!("the file does not parse: {}", e))?;
    let m = find_item(&mut doc, list, id_key, id).ok_or_else(|| format!("could not find `{}` in its file", id))?;
    if owner.is_empty() {
        m.remove(key("owner"));
    } else {
        insert_after(m, "description", "owner", Yaml::String(owner.to_string()));
    }
    serde_yaml::to_string(&doc).map_err(|e| e.to_string())
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
            // field → the file's column, when they differ
            let mut columns = Mapping::new();
            if let Some(cols) = body.get("columns") {
                let o = cols.as_object().ok_or("`columns` maps a field to its column: {\"unit_price\": \"Unit Price ($)\"}")?;
                let ty = ont.type_(&t).ok_or_else(|| format!("no type named `{}`", t))?;
                for (f, c) in o {
                    let c = c.as_str().map(|x| x.trim()).filter(|x| !x.is_empty()).ok_or_else(|| format!("`columns`: the column for `{}` must be its name", f))?;
                    if !ty.properties.contains_key(f) {
                        return Err(format!("`columns`: {} has no field `{}`", t, f));
                    }
                    columns.insert(key(f), Yaml::String(c.to_string()));
                }
            }
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
                if !columns.is_empty() {
                    src.insert(key("columns"), Yaml::Mapping(columns.clone()));
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
        "set_metric" => {
            // words only: who owns it, what it is, its caveats, grain and basis. The recipe is what
            // the number is, so changing it is a new proposal of the metric, not a word fix.
            let name = field(body, "metric").to_string();
            let m = ont.metrics.get(&name).ok_or_else(|| format!("no metric named `{}`", name))?;
            let obj = body.as_object().cloned().unwrap_or_default();
            for k in obj.keys() {
                match k.as_str() {
                    "op" | "metric" | "reason" | "by" | "owner" | "description" | "caveats" | "grain" | "basis" => {}
                    "source" | "filter" | "measure" | "variants" | "sql" | "connection" | "table" | "tool" | "params" => {
                        return Err(format!("set_metric changes words only (owner, description, caveats, grain, basis); `{}` changes what the number is, so propose the metric again with propose_change", k))
                    }
                    other => return Err(format!("set_metric takes owner, description, caveats, grain and basis, not `{}`", other)),
                }
            }
            let b = basis(body)?;
            let mut set: Vec<(&str, String)> = Vec::new();
            for k in ["owner", "description", "caveats", "grain"] {
                if let Some(v) = body.get(k).and_then(|v| v.as_str()) {
                    let v = v.trim();
                    if v.is_empty() {
                        return Err(format!("`{}` is empty; leave it out to keep it as it is", k));
                    }
                    if v.chars().count() > 600 {
                        return Err(format!("`{}` is longer than 600 characters", k));
                    }
                    set.push((k, v.to_string()));
                }
            }
            if let Some(b) = b {
                set.push(("basis", b));
            }
            if set.is_empty() {
                return Err("set_metric needs at least one of owner, description, caveats, grain or basis".into());
            }
            let file = m.file.clone();
            let text = ws.read(&file)?;
            let mut doc: Yaml = serde_yaml::from_str(&text).map_err(|e| format!("{} does not parse: {}", file, e))?;
            let items = doc.get_mut("metrics").and_then(|v| v.as_sequence_mut()).ok_or_else(|| format!("{} has no `metrics:` list", file))?;
            let item = items.iter_mut().filter_map(|i| i.as_mapping_mut()).find(|i| i.get(key("metric")).and_then(|v| v.as_str()) == Some(name.as_str())).ok_or_else(|| format!("could not find `{}` inside {}", name, file))?;
            for (k, v) in &set {
                insert_after(item, "description", k, Yaml::String(v.clone()));
            }
            let out = serde_yaml::to_string(&doc).map_err(|e| e.to_string())?;
            let what: Vec<String> = set.iter().map(|(k, v)| if *k == "owner" { format!("owner {}", v) } else { (*k).to_string() }).collect();
            Ok(Patch { file, text: out, kind: "metric".into(), name: name.clone(), summary: format!("set the {} of metric {}", what.join(", "), name) })
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
            // a kind of another entity: its fields, links and guardrails come with it
            let extends = field(body, "extends").to_string();
            if !extends.is_empty() {
                let p = ont.type_(&extends).ok_or_else(|| format!("{} can't be a kind of {}: there is no entity called {}", name, extends, extends))?;
                m.insert(key("extends"), Yaml::String(p.name.clone()));
                if let Some(w) = where_yaml(body)? {
                    m.insert(key("where"), w);
                }
            } else if body.get("where").map(|w| !w.is_null()).unwrap_or(false) {
                return Err("`where` picks a parent's rows, so it needs `extends`".into());
            }
            let domain = field(body, "domain");
            if !domain.is_empty() {
                m.insert(key("domain"), Yaml::String(domain.to_string()));
            }
            m.insert(key("description"), Yaml::String(field(body, "description").to_string()));
            let keyp = field(body, "key").to_string();
            let mut props = Mapping::new();
            match body.get("properties") {
                // the fields as the YAML writes them: `units: integer`, or `{type, key, unit, description}`
                Some(Json::Object(given)) if !given.is_empty() => {
                    if given.len() > 300 {
                        return Err("at most 300 properties in one entity".into());
                    }
                    for (pname, spec) in given {
                        prop_name_ok(pname)?;
                        props.insert(key(pname), property_yaml(pname, spec)?);
                    }
                    let keyed: Vec<String> = props.iter().filter(|(_, v)| v.get("key").and_then(|k| k.as_bool()).unwrap_or(false)).filter_map(|(k, _)| k.as_str().map(|s| s.to_string())).collect();
                    if keyed.len() > 1 {
                        return Err(format!("only one field can be the key; {} all say `key: true`", keyed.join(", ")));
                    }
                    if !keyp.is_empty() {
                        if keyed.iter().any(|k| k != &keyp) {
                            return Err(format!("`key` names {}, but {} says `key: true`", keyp, keyed.join(", ")));
                        }
                        let entry = props.get_mut(key(&keyp)).ok_or_else(|| format!("`key` must be one of the properties; `{}` is not", keyp))?;
                        set_key(entry);
                    }
                }
                Some(Json::Object(_)) | None if !extends.is_empty() => {
                    // a kind gets its parent's fields and key: nothing of its own to add
                }
                Some(Json::Object(_)) | None => {
                    // no fields given: one identifier, the key, as before
                    let mut idp = Mapping::new();
                    idp.insert(key("type"), Yaml::String("string".into()));
                    idp.insert(key("key"), Yaml::Bool(true));
                    idp.insert(key("description"), Yaml::String(format!("Identifier of one {}", name)));
                    let id = if keyp.is_empty() { format!("{}_id", snake(&name)) } else { prop_name_ok(&keyp).map(|_| keyp.clone())? };
                    props.insert(key(&id), Yaml::Mapping(idp));
                }
                Some(_) => return Err("`properties` maps each field to its type, e.g. {\"units\": \"integer\", \"revenue\": {\"type\": \"number\", \"unit\": \"USD\"}}".into()),
            }
            m.insert(key("properties"), Yaml::Mapping(props));
            let content = serde_yaml::to_string(&Yaml::Mapping(m)).map_err(|e| e.to_string())?;
            let (rel, text) = edit::plan_create(&ont.root, "type", &name, &content, None)?;
            Ok(Patch { file: rel, text, kind: "type".into(), name: name.clone(), summary: if extends.is_empty() { format!("created {}", name) } else { format!("created {}, a kind of {}", name, extends) } })
        }
        "set_kind_of" => {
            // an existing entity becomes a kind of another: the fields it now gets unchanged go,
            // differing words stay as narrowings, and a real conflict refuses in words
            let t = field(body, "type").to_string();
            let p = field(body, "extends").to_string();
            let ty = ont.type_(&t).ok_or_else(|| format!("no type named `{}`", t))?.clone();
            let pt = ont.type_(&p).ok_or_else(|| format!("{} can't be a kind of {}: there is no entity called {}", t, p, p))?.clone();
            if ty.name == pt.name || ont.is_a(&pt.name, &ty.name) {
                return Err(format!("{} can't be a kind of {}: {} is already {} or a kind of it", ty.name, pt.name, pt.name, ty.name));
            }
            if let Some(cur) = &ty.parent {
                if cur != &pt.name {
                    return Err(format!("{} is already a kind of {}; an entity extends one other entity", ty.name, cur));
                }
            }
            let pkey = pt.key_property();
            let mut dropped: Vec<String> = Vec::new();
            let mut kept: Vec<String> = Vec::new();
            for (f, spec) in ty.own_properties() {
                let Some(pf) = pt.properties.get(f).map(|x| x.normalize()) else { continue };
                let mine = spec.normalize();
                if mine.kind != pf.kind {
                    return Err(format!("{}.{} is {} but {}'s is {}; a kind can't change a field's type", ty.name, f, mine.kind, pt.name, pf.kind));
                }
                if mine.unit.is_some() && pf.unit.is_some() && mine.unit != pf.unit {
                    return Err(format!("{}.{} is in {} but {}'s is in {}; a kind can't change a field's unit", ty.name, f, mine.unit.clone().unwrap_or_default(), pt.name, pf.unit.clone().unwrap_or_default()));
                }
                if mine.key && pkey.as_deref() != Some(f.as_str()) {
                    if let Some(k) = &pkey {
                        return Err(format!("{} is identified by {}; {} can't be identified by {}", pt.name, k, ty.name, f));
                    }
                }
                let same_words = mine.description.trim().is_empty() || mine.description.trim() == pf.description.trim();
                if same_words && mine.unit == pf.unit && mine.required == pf.required && mine.values == pf.values {
                    dropped.push(f.clone());
                } else {
                    kept.push(f.clone());
                }
            }
            let w = where_yaml(body)?;
            let pname = pt.name.clone();
            let (file, out) = edit_type_file(ws, &ty.name, |m| {
                insert_after(m, "type", "extends", Yaml::String(pname.clone()));
                if let Some(w) = w {
                    insert_after(m, "extends", "where", w);
                }
                if let Some(props) = m.get_mut(key("properties")).and_then(|x| x.as_mapping_mut()) {
                    for f in &dropped {
                        props.remove(key(f));
                    }
                }
                Ok(())
            })?;
            let mut summary = format!("made {} a kind of {}", ty.name, pt.name);
            if !dropped.is_empty() {
                summary.push_str(&format!(" ({} now from {})", dropped.join(", "), pt.name));
            }
            if !kept.is_empty() {
                summary.push_str(&format!("; keeps its own words for {}", kept.join(", ")));
            }
            Ok(Patch { file, text: out, kind: "type".into(), name: ty.name.clone(), summary })
        }
        "add_property" => {
            let t = field(body, "type").to_string();
            let prop = field(body, "property").to_string();
            if t.is_empty() || prop.is_empty() {
                return Err("add_property needs `type` (the entity) and `property` (the new field's name)".into());
            }
            prop_name_ok(&prop)?;
            let ty = ont.type_(&t).ok_or_else(|| format!("no type named `{}`", t))?;
            if let Some(have) = ty.properties.keys().find(|k| k.eq_ignore_ascii_case(&prop)) {
                return Err(format!("{} already has a field `{}`", t, have));
            }
            let mut pm = Mapping::new();
            let datatype = { let d = field(body, "datatype"); if d.is_empty() { "string" } else { d } };
            pm.insert(key("type"), Yaml::String(datatype.to_string()));
            let is_key = body.get("key").and_then(|k| k.as_bool()).unwrap_or(false);
            if is_key {
                if let Some(k) = ty.key_property() {
                    return Err(format!("{} already has a key, `{}`", t, k));
                }
                pm.insert(key("key"), Yaml::Bool(true));
            }
            let unit = field(body, "unit");
            if !unit.is_empty() {
                pm.insert(key("unit"), Yaml::String(unit.to_string()));
            }
            let desc = field(body, "description");
            if !desc.is_empty() {
                pm.insert(key("description"), Yaml::String(desc.to_string()));
            }
            serde_yaml::from_value::<crate::schema::Property>(Yaml::Mapping(pm.clone())).map_err(|e| format!("field `{}`: {}", prop, e))?;
            let column = field(body, "column").to_string();
            let mapped = !column.is_empty() && column != prop;
            if mapped && ty.source.is_none() {
                return Err(format!("`column` maps the field to a column of the binding, and {} has no binding", t));
            }
            let (file, out) = edit_type_file(ws, &t, |m| {
                if !m.contains_key(key("properties")) {
                    insert_after(m, "description", "properties", Yaml::Mapping(Mapping::new()));
                }
                let props = m.get_mut(key("properties")).and_then(|p| p.as_mapping_mut()).ok_or_else(|| "properties is not a mapping".to_string())?;
                props.insert(key(&prop), Yaml::Mapping(pm.clone()));
                if mapped {
                    let src = m.get_mut(key("source")).and_then(|s| s.as_mapping_mut()).ok_or_else(|| format!("{} has no binding", t))?;
                    if !src.contains_key(key("columns")) {
                        src.insert(key("columns"), Yaml::Mapping(Mapping::new()));
                    }
                    let cols = src.get_mut(key("columns")).and_then(|c| c.as_mapping_mut()).ok_or_else(|| "columns is not a mapping".to_string())?;
                    cols.insert(key(&prop), Yaml::String(column.clone()));
                }
                Ok(())
            })?;
            Ok(Patch { file, text: out, kind: "type".into(), name: t.clone(), summary: format!("added the field {}.{}", t, prop) })
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
        "set_owner" => {
            // who owns an entity, metric, function or guardrail (decision 8): one line where it can be
            let kind = field(body, "kind");
            let name = field(body, "name").to_string();
            let given = body.get("owner").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let (file, list, id_key, pk, item) = match kind {
                "entity" | "type" => {
                    let t = ont.type_(&name).ok_or_else(|| format!("no entity called `{}`", name))?;
                    (t.file.clone(), "types", "type", "type", t.name.clone())
                }
                "metric" => {
                    let m = ont.metrics.get(&name).ok_or_else(|| format!("no metric named `{}`", name))?;
                    (m.file.clone(), "metrics", "metric", "metric", m.name.clone())
                }
                "function" => {
                    let f = ont.functions.get(&name).ok_or_else(|| format!("no function named `{}`", name))?;
                    (f.file.clone(), "functions", "function", "function", f.name.clone())
                }
                "guardrail" | "constraint" => {
                    let c = ont.constraint(&name).ok_or_else(|| format!("no guardrail with id `{}`", name))?;
                    (c.file.clone(), "constraints", "id", "constraint", c.id.clone())
                }
                other => return Err(format!("set_owner's kind is entity, metric, function or guardrail, not `{}`", other)),
            };
            // a name already in the space (an existing team address included) is used as written;
            // a new one must pass the owner name rule
            let owner = if given.is_empty() {
                String::new()
            } else {
                let known = crate::people::derive(ont, &[]);
                match known.find(&given) {
                    Some(_) if given.contains('@') || given.to_lowercase().contains("agent") => given.clone(),
                    _ => crate::people::owner_ok(&given)?,
                }
            };
            let text = ws.read(&file)?;
            let noun = match pk { "type" => "entity", "constraint" => "guardrail", k => k };
            let new = match set_owner_line(&text, list, id_key, &item, &owner) {
                Some(t) if owner_in_text(&t, list, id_key, &item).as_deref() == Some(owner.as_str()) || (owner.is_empty() && owner_in_text(&t, list, id_key, &item).is_none()) => t,
                // a flow-style item, or a line edit that didn't land: the mapping (comments in this file go)
                _ => set_owner_mapping(&text, list, id_key, &item, &owner)?,
            };
            let summary = if owner.is_empty() { format!("removed the owner of {} {}", noun, item) } else { format!("made {} the owner of {} {}", crate::people::shown(&owner), noun, item) };
            Ok(Patch { file, text: new, kind: pk.into(), name: item, summary })
        }
        "record_check" => Err("record_check reports a check, it changes no file; send it on its own, not in `ops`".into()),
        other => Err(format!("unknown op `{}`; one of add_type, add_property, set_type_description, set_metric, set_owner, set_property_description, add_relationship, set_relationship_description, add_constraint, set_source, add_source, add_term, add_metric, add_function, record_check", other)),
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
    fn add_type_takes_the_fields_and_makes_no_generated_id() {
        let ont = ecommerce();
        let p = plan(&ont, &json!({ "op": "add_type", "name": "PromoSale", "description": "One order line.", "properties": {
            "line_id": { "type": "string", "key": true, "description": "Identifier of one order line." },
            "units": "integer",
            "revenue": { "type": "number", "unit": "USD", "description": "What the customer paid." }
        } })).unwrap();
        assert!(!p.text.contains("promo_sale_id"), "{}", p.text);
        assert!(p.text.contains("  line_id:\n    type: string\n    key: true\n    description: Identifier of one order line.\n"), "{}", p.text);
        assert!(p.text.contains("  units: integer\n"), "shorthand kept: {}", p.text);
        assert!(p.text.contains("    unit: USD\n"), "{}", p.text);
        // `key` names the key when no property says so
        let k = plan(&ont, &json!({ "op": "add_type", "name": "Sheet", "properties": { "code": "string", "n": "integer" }, "key": "code" })).unwrap();
        assert!(k.text.contains("  code:\n    type: string\n    key: true\n"), "{}", k.text);
        let e = plan(&ont, &json!({ "op": "add_type", "name": "Bad", "properties": { "x": { "type": "string", "descripton": "typo" } } })).err().unwrap();
        assert!(e.contains("descripton"), "an unknown key is an error, as in a file: {}", e);
        assert!(plan(&ont, &json!({ "op": "add_type", "name": "Bad", "properties": { "x": "string" }, "key": "y" })).is_err(), "the key must be a property");
        assert!(plan(&ont, &json!({ "op": "add_type", "name": "Bad", "properties": { "Unit Price": "number" } })).is_err(), "field names are plain");
        // without properties, the old shape: one generated key
        let old = plan(&ont, &json!({ "op": "add_type", "name": "Voucher" })).unwrap();
        assert!(old.text.contains("voucher_id:"), "{}", old.text);
    }

    #[test]
    fn add_property_adds_a_field_and_maps_its_column() {
        let ont = ecommerce();
        let p = plan(&ont, &json!({ "op": "add_property", "type": "Order", "property": "gift_wrap", "datatype": "boolean", "description": "True when the order is gift wrapped." })).unwrap();
        assert!(p.text.contains("  gift_wrap:\n    type: boolean\n    description: True when the order is gift wrapped.\n"), "{}", p.text);
        assert_eq!(p.summary, "added the field Order.gift_wrap");
        let c = plan(&ont, &json!({ "op": "add_property", "type": "Order", "property": "gift_wrap", "datatype": "boolean", "column": "Gift Wrap?" })).unwrap();
        assert!(c.text.contains("columns:\n    gift_wrap: Gift Wrap?"), "{}", c.text);
        let (o, report) = edit::check_many(&ont.root, &[(c.file.clone(), Some(c.text.clone()))].into_iter().collect()).unwrap();
        assert!(report.ok(), "{:?}", report.diags);
        assert_eq!(o.type_("Order").unwrap().source.as_ref().unwrap().columns["gift_wrap"], "Gift Wrap?");
        assert!(plan(&ont, &json!({ "op": "add_property", "type": "Order", "property": "order_id" })).err().unwrap().contains("already has a field"));
        assert!(plan(&ont, &json!({ "op": "add_property", "type": "Order", "property": "x", "key": true })).err().unwrap().contains("already has a key"));
        let s = plan(&ont, &json!({ "op": "set_source", "type": "Order", "connection": "shop_files", "table": "orders", "key": "order_id", "columns": { "order_id": "Order ID" } })).unwrap();
        assert!(s.text.contains("columns:\n    order_id: Order ID"), "set_source takes columns: {}", s.text);
        assert!(plan(&ont, &json!({ "op": "set_source", "type": "Order", "connection": "shop_files", "table": "orders", "columns": { "nope": "x" } })).is_err());
    }

    #[test]
    fn set_metric_changes_words_only() {
        let ont = ecommerce();
        let name = ont.metrics.keys().next().unwrap().clone();
        let p = plan(&ont, &json!({ "op": "set_metric", "metric": name, "owner": "Vinay", "caveats": "Refunds are not counted." })).unwrap();
        assert!(p.text.contains("owner: Vinay") && p.text.contains("caveats: Refunds are not counted."), "{}", p.text);
        assert!(p.summary.starts_with("set the owner Vinay, caveats of metric"), "{}", p.summary);
        let e = plan(&ont, &json!({ "op": "set_metric", "metric": name, "filter": "x = 1" })).err().unwrap();
        assert!(e.contains("changes what the number is"), "{}", e);
        assert!(plan(&ont, &json!({ "op": "set_metric", "metric": name })).is_err(), "needs one field");
        assert!(plan(&ont, &json!({ "op": "set_metric", "metric": "nope", "owner": "x" })).is_err());
        assert!(plan(&ont, &json!({ "op": "set_metric", "metric": name, "basis": "vibes" })).is_err());
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
    #[test]
    fn set_owner_edits_one_line_and_keeps_comments() {
        let dir = std::env::temp_dir().join(format!("lorefile-set-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::create_dir_all(dir.join("logic")).unwrap();
        std::fs::create_dir_all(dir.join("constraints")).unwrap();
        std::fs::write(dir.join("ontology.yaml"), "name: shop\n").unwrap();
        std::fs::write(dir.join("types/order.yaml"), "# orders\ntype: Order\ndescription: One checkout.\nproperties:\n  order_id: { type: string, key: true, description: The id. }\n").unwrap();
        std::fs::write(dir.join("logic/metrics.yaml"), "metrics:\n  - metric: revenue # the one number\n    description: Money in.\n    sql: select 1\n    owner: vinay  # set in the trial\n  - metric: refunds\n    description: Money back.\n    sql: select 2\n  - { metric: orders, description: Orders., sql: select 3 }\n").unwrap();
        std::fs::write(dir.join("constraints/order.yaml"), "constraints:\n  - id: OR-001\n    name: No negative totals\n    applies_to: Order\n    rule: |\n      true\n    severity: warn\n").unwrap();
        let ont = load_ontology(&dir).unwrap().0;
        let p = plan(&ont, &json!({ "op": "set_owner", "kind": "metric", "name": "revenue", "owner": "Vinay Karode" })).unwrap();
        assert!(p.text.contains("    owner: Vinay Karode  # set in the trial\n") && p.text.contains("# the one number"), "{}", p.text);
        assert_eq!(p.summary, "made Vinay Karode the owner of metric revenue");
        let r = plan(&ont, &json!({ "op": "set_owner", "kind": "metric", "name": "refunds", "owner": "Priya" })).unwrap();
        assert!(r.text.contains("    sql: select 2\n    owner: Priya\n  - { metric: orders"), "added after the item's last line: {}", r.text);
        assert!(r.text.contains("# the one number"), "{}", r.text);
        let f = plan(&ont, &json!({ "op": "set_owner", "kind": "metric", "name": "orders", "owner": "Priya" })).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&f.text).unwrap();
        assert_eq!(parsed["metrics"][2]["owner"], "Priya", "a flow item goes through the mapping: {}", f.text);
        let e = plan(&ont, &json!({ "op": "set_owner", "kind": "entity", "name": "Order", "owner": "Growth team" })).unwrap();
        assert!(e.text.starts_with("# orders\n") && e.text.ends_with("owner: Growth team\n"), "{}", e.text);
        let g = plan(&ont, &json!({ "op": "set_owner", "kind": "guardrail", "name": "OR-001", "owner": "R&D" })).unwrap();
        assert!(g.text.contains("      true\n    severity: warn\n    owner: R&D\n"), "after a block scalar: {}", g.text);
        let x = plan(&ont, &json!({ "op": "set_owner", "kind": "metric", "name": "revenue", "owner": "" })).unwrap();
        assert!(!x.text.contains("owner:") && x.text.contains("# the one number"), "removed: {}", x.text);
        assert!(plan(&ont, &json!({ "op": "set_owner", "kind": "metric", "name": "revenue", "owner": "x@y.example" })).is_err(), "a new name is a name");
        assert!(plan(&ont, &json!({ "op": "set_owner", "kind": "metric", "name": "revenue", "owner": "my agent" })).is_err());
        assert!(plan(&ont, &json!({ "op": "set_owner", "kind": "metric", "name": "nope", "owner": "Priya" })).is_err());
        assert!(plan(&ont, &json!({ "op": "set_owner", "kind": "action", "name": "revenue", "owner": "Priya" })).is_err());
        for t in [&p.text, &r.text, &e.text, &g.text, &x.text] {
            assert!(serde_yaml::from_str::<serde_yaml::Value>(t).is_ok(), "{}", t);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
