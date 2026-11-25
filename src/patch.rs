//! Friendly edits. The page sends a small operation (describe a field, add a
//! link, add a guardrail, create a thing); this turns it into the full text of
//! one YAML file, which then goes through `edit::apply` like any other write,
//! so it is compiled and checked before anything touches disk.
//!
//! Type files are rewritten through serde_yaml, which keeps key order but
//! drops comments in that one file. Guardrails are appended as text and keep
//! the file as it was.

use crate::edit;
use crate::ontology::{snake, Ontology};
use serde_json::Value as Json;
use serde_yaml::{Mapping, Value as Yaml};

pub struct Patch {
    pub file: String,
    pub text: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
}

fn field<'a>(body: &'a Json, k: &str) -> &'a str {
    body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim()
}

fn key(s: &str) -> Yaml {
    Yaml::String(s.to_string())
}

/// Insert `k: v` right after `after` (or at the end), replacing an existing `k` in place.
fn insert_after(m: &mut Mapping, after: &str, k: &str, v: Yaml) {
    if m.contains_key(&key(k)) {
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
fn edit_type_file(ont: &Ontology, type_name: &str, f: impl FnOnce(&mut Mapping) -> Result<(), String>) -> Result<(String, String), String> {
    let t = ont.type_(type_name).ok_or_else(|| format!("no type named `{}`", type_name))?;
    let full = ont.root.join(&t.file);
    let text = std::fs::read_to_string(&full).map_err(|e| format!("reading {}: {}", t.file, e))?;
    let mut doc: Yaml = serde_yaml::from_str(&text).map_err(|e| format!("{} does not parse: {}", t.file, e))?;
    let root = doc.as_mapping_mut().ok_or_else(|| format!("{} is not a mapping", t.file))?;
    let found = if let Some(Yaml::Sequence(items)) = root.get_mut(&key("types")) {
        items.iter_mut().filter_map(|i| i.as_mapping_mut()).find(|m| m.get(&key("type")).and_then(|v| v.as_str()) == Some(&t.name))
    } else if root.get(&key("type")).and_then(|v| v.as_str()) == Some(&t.name) {
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
    let op = field(body, "op");
    match op {
        "set_type_description" => {
            let t = field(body, "type").to_string();
            let text = field(body, "text").to_string();
            let (file, out) = edit_type_file(ont, &t, |m| {
                insert_after(m, "type", "description", Yaml::String(text.clone()));
                Ok(())
            })?;
            Ok(Patch { file, text: out, kind: "type".into(), name: t.clone(), summary: format!("described {}", t) })
        }
        "set_property_description" => {
            let t = field(body, "type").to_string();
            let prop = field(body, "property").to_string();
            let text = field(body, "text").to_string();
            let (file, out) = edit_type_file(ont, &t, |m| {
                let props = m.get_mut(&key("properties")).and_then(|p| p.as_mapping_mut()).ok_or_else(|| format!("{} has no properties block", t))?;
                let entry = props.get_mut(&key(&prop)).ok_or_else(|| format!("{} has no field `{}`", t, prop))?;
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
                let (file, out) = edit_type_file(ont, &from, |m| {
                    let rels = m.get_mut(&key("relationships")).and_then(|p| p.as_mapping_mut()).ok_or_else(|| format!("{} has no relationships block", from))?;
                    let entry = rels.get_mut(&key(&name)).and_then(|e| e.as_mapping_mut()).ok_or_else(|| format!("{} has no link `{}`", from, name))?;
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
            if let Some(Yaml::Sequence(items)) = root.get_mut(&key("relationships")) {
                let item = items.iter_mut().filter_map(|i| i.as_mapping_mut()).find(|m| m.get(&key("relationship")).and_then(|v| v.as_str()) == Some(&name) && m.get(&key("from")).and_then(|v| v.as_str()) == Some(&from)).ok_or_else(|| format!("could not find `{}` in {}", name, rel.file))?;
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
            if ont.type_(&to).is_none() {
                return Err(format!("no type named `{}` to link to", to));
            }
            if ont.relationships.iter().any(|r| r.from == from && r.name == name) {
                return Err(format!("{} already has a link called `{}`", from, name));
            }
            let (file, out) = edit_type_file(ont, &from, |m| {
                if !m.contains_key(&key("relationships")) {
                    insert_after(m, "properties", "relationships", Yaml::Mapping(Mapping::new()));
                }
                let rels = m.get_mut(&key("relationships")).and_then(|p| p.as_mapping_mut()).ok_or_else(|| "relationships is not a mapping".to_string())?;
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
            let keyp = field(body, "key").to_string();
            if connection.is_empty() || table.is_empty() {
                return Err("a binding needs a connection and a table".into());
            }
            if !ont.sources.contains_key(&connection) {
                return Err(format!("no connection named `{}`; add it under sources/ first", connection));
            }
            let (file, out) = edit_type_file(ont, &t, |m| {
                let mut src = Mapping::new();
                src.insert(key("connection"), Yaml::String(connection.clone()));
                src.insert(key("table"), Yaml::String(table.clone()));
                if !keyp.is_empty() {
                    src.insert(key("key"), Yaml::String(keyp.clone()));
                }
                insert_after(m, "relationships", "source", Yaml::Mapping(src));
                Ok(())
            })?;
            Ok(Patch { file, text: out, kind: "type".into(), name: t.clone(), summary: format!("bound {} to {}.{}", t, connection, table) })
        }
        "add_type" => {
            let name = field(body, "name").to_string();
            if name.is_empty() {
                return Err("the thing needs a name".into());
            }
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
        other => Err(format!("unknown op `{}`", other)),
    }
}
