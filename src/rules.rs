//! Rules are CEL expressions. This module compiles them, evaluates them against
//! records, and runs the full `validate(action, inputs)` check an agent calls
//! before doing anything.

use crate::data::{DataLayer, Record};
use crate::ontology::{snake, Kind, Ontology};
use crate::schema::*;
use anyhow::{anyhow, Result};
use cel::{Context, Program, Value};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::sync::Arc;

/// `A implies B` → `!(A) || (B)`. Everything else is plain CEL. The word is
/// only special outside string literals, so `name != "a implies b"` is left alone.
pub fn desugar(rule: &str) -> String {
    let r = rule.trim();
    if let Some(idx) = find_outside_strings(r, " implies ") {
        let (a, b) = (&r[..idx], &r[idx + " implies ".len()..]);
        return format!("!({}) || ({})", a.trim(), desugar(b));
    }
    r.to_string()
}

/// Byte index of `needle` in `s`, ignoring anything inside "…" or '…' (with backslash escapes).
fn find_outside_strings(s: &str, needle: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if s[i..].starts_with(needle) {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

pub fn compile(rule: &str) -> Result<Program, String> {
    Program::compile(&desugar(rule)).map_err(|e| e.to_string())
}

/// Top-level variable names a rule references.
pub fn variables(rule: &str) -> Vec<String> {
    match compile(rule) {
        Ok(p) => {
            let mut v: Vec<String> = p.references().variables().into_iter().map(|s| s.to_string()).collect();
            v.sort();
            v.dedup();
            v
        }
        Err(_) => vec![],
    }
}

pub fn json_to_cel(v: &Json) -> Value {
    match v {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::Int(u as i64)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Json::String(s) => Value::String(Arc::new(s.clone())),
        Json::Array(a) => Value::List(Arc::new(a.iter().map(json_to_cel).collect())),
        Json::Object(o) => {
            let map: HashMap<String, Value> = o.iter().map(|(k, v)| (k.clone(), json_to_cel(v))).collect();
            Value::from(map)
        }
    }
}

/// Evaluate a compiled rule with the given variables. Ok(true) means the rule holds.
pub fn eval(program: &Program, vars: &IndexMap<String, Json>) -> Result<bool, String> {
    let mut ctx = Context::default();
    for (k, v) in vars {
        ctx.add_variable_from_value(k.clone(), json_to_cel(v));
    }
    match program.execute(&ctx) {
        Ok(Value::Bool(b)) => Ok(b),
        Ok(other) => Err(format!("rule returned {:?}, expected true/false", other)),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------- action checks ----------------

#[derive(Debug, Clone, Serialize)]
pub struct RuleResult {
    pub id: String,
    pub name: String,
    pub severity: Severity,
    pub passed: bool,
    pub applies_to: String,
    /// Which action inputs this evaluation was bound to.
    pub bound_to: Vec<String>,
    pub rule: String,
    pub message: String,
    pub rationale: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The rule's grade in trust.yaml (proposed, accepted, checked, ungraded); left out when the space has no trust.yaml.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub input: String,
    pub replace_with: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedInput {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub key: String,
    pub resolved: bool,
    pub record: Record,
    /// The kinds of the action's type this input was checked as (the row matched their `where`,
    /// or the agent said so with `kinds`): their guardrails apply too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checked_as: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    pub action: String,
    /// pass | warn | fail
    pub verdict: String,
    pub summary: String,
    pub inputs: Vec<ResolvedInput>,
    pub results: Vec<RuleResult>,
    pub requires_approval_from: String,
    pub side_effects: String,
    pub suggestions: Vec<Suggestion>,
    pub notes: Vec<String>,
    /// What this answer relied on that no person has reviewed: rules an agent
    /// proposed (or that changed since a person graded them), and the data
    /// bindings rows were read through. The verdict is the same either way.
    pub unverified: Vec<String>,
}

/// A plausible inline record for a type, used in hints: `{"status": "open", "qty": 1}`.
fn example_record(t: &TypeDef) -> String {
    let mut parts = Vec::new();
    let rank = |k: &str| match k { "enum" => 0, "boolean" | "bool" => 1, "integer" | "int" | "number" | "float" | "decimal" => 2, _ => 3 };
    let mut props: Vec<(&String, Property)> = t.properties.iter().map(|(n, p)| (n, p.normalize())).filter(|(_, p)| !p.key).collect();
    props.sort_by_key(|(_, p)| rank(&p.kind));
    for (name, p) in props {
        let v = match p.kind.as_str() {
            "enum" => p.values.first().map(|v| serde_json::to_string(&serde_json::to_value(v).unwrap_or(Json::Null)).unwrap_or_default()).unwrap_or_else(|| "\"…\"".into()),
            "integer" | "int" => "1".into(),
            "number" | "float" | "decimal" => "1.0".into(),
            "boolean" | "bool" => "true".into(),
            _ => "\"…\"".into(),
        };
        parts.push(format!("\"{}\": {}", name, v));
        if parts.len() >= 2 {
            break;
        }
    }
    format!("{{{}}}", parts.join(", "))
}

fn key_of(rec: &Record, t: &TypeDef) -> String {
    let kp = t.key_property().unwrap_or_else(|| "id".into());
    rec.get(&kp)
        .or_else(|| rec.get("id"))
        .or_else(|| rec.get("name"))
        .map(|v| match v {
            Json::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn label_of(rec: &Record, t: &TypeDef) -> String {
    let key = key_of(rec, t);
    match rec.get("name").and_then(|v| v.as_str()) {
        Some(n) if n != key => format!("{} ({})", n, key),
        _ => key,
    }
}

/// Turn raw inputs (key strings or inline records) into records.
/// The `where` kinds below `declared` that a record matches, deepest first (siblings both, when their `where`s overlap).
fn where_kinds_matching(ont: &Ontology, declared: &str, rec: &Record) -> Vec<String> {
    let mut hits: Vec<String> = ont
        .all_kinds_of(declared)
        .into_iter()
        .filter(|k| {
            let Some(t) = ont.types.get(k) else { return false };
            if !t.is_where_kind() || !t.rows_of.as_ref().map(|b| ont.is_a(declared, b) || ont.is_a(b, declared)).unwrap_or(false) {
                return false;
            }
            let props: IndexMap<String, crate::schema::Property> = t.properties.iter().map(|(k, v)| (k.clone(), v.normalize())).collect();
            DataLayer::where_matches(rec, &t.where_all, &props)
        })
        .collect();
    // keep the deepest: a kind whose own kind also matched says less
    let all = hits.clone();
    hits.retain(|k| !all.iter().any(|o| o != k && ont.is_a(o, k)));
    hits.sort();
    hits
}

fn resolve_inputs(ont: &Ontology, data: &DataLayer, action: &ActionDef, raw: &IndexMap<String, Json>, kinds: &IndexMap<String, String>, notes: &mut Vec<String>, read: &mut Vec<String>) -> Result<Vec<ResolvedInput>> {
    let mut out = Vec::new();
    for (input, type_name) in &action.inputs {
        let declared = ont.type_(type_name).ok_or_else(|| anyhow!("action input `{}` has unknown type `{}`", input, type_name))?;
        // producer extends: the agent may say this input is a kind of the type the action takes
        let mut said: Option<String> = None;
        if let Some(k) = kinds.get(input) {
            let kt = ont.type_(k).ok_or_else(|| anyhow!("`kinds.{}` names {}, which is no entity here", input, k))?;
            if !ont.is_a(&kt.name, &declared.name) {
                return Err(anyhow!("{} is not a kind of {}, so it can't be passed as `{}`", kt.name, declared.name, input));
            }
            said = Some(kt.name.clone());
        }
        let t = said.as_ref().and_then(|k| ont.types.get(k)).unwrap_or(declared);
        let Some(v) = raw.get(input) else {
            return Err(anyhow!("missing input `{}` ({}). Inputs: {}", input, type_name,
                action.inputs.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<_>>().join(", ")));
        };
        match v {
            Json::Object(o) => {
                let rec: Record = o.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                let checked_as = checked_kinds(ont, &declared.name, said.as_deref(), &rec, input, notes);
                out.push(ResolvedInput { name: input.clone(), type_name: declared.name.clone(), key: key_of(&rec, t), resolved: true, record: rec, checked_as });
            }
            other => {
                let key = match other {
                    Json::String(s) => s.clone(),
                    o => o.to_string(),
                };
                match data.find(&t.name, &key)? {
                    Some(rec) => {
                        let reads = if t.is_where_kind() { t.rows_of.clone().unwrap_or(t.name.clone()) } else { t.name.clone() };
                        if ont.types.get(&reads).map(|x| x.source.is_some()).unwrap_or(false) && !read.contains(&reads) {
                            read.push(reads);
                        }
                        let checked_as = checked_kinds(ont, &declared.name, said.as_deref(), &rec, input, notes);
                        out.push(ResolvedInput { name: input.clone(), type_name: declared.name.clone(), key, resolved: true, record: rec, checked_as })
                    }
                    None if t.is_where_kind() && t.rows_of.as_ref().map(|b| data.find(b, &key).ok().flatten().is_some()).unwrap_or(false) => {
                        // consumer super: a row of the parent that is not this kind
                        let base = t.rows_of.clone().unwrap_or_default();
                        let rec = data.find(&base, &key)?.unwrap_or_default();
                        let what: Vec<String> = t.where_all.keys().map(|f| format!("{} is {}", f, rec.get(f).map(|v| v.as_str().map(|x| x.to_string()).unwrap_or_else(|| v.to_string())).unwrap_or_else(|| "empty".into()))).collect();
                        return Err(anyhow!("`{}` needs a {}; {} is a {} whose {}", input, t.name, key, base, what.join(" and ")));
                    }
                    None => {
                        let kp = t.key_property().unwrap_or_else(|| "id".into());
                        let mut rec = Record::new();
                        rec.insert(kp, json!(key));
                        if data.is_local(t) {
                            notes.push(format!("`{}` = {} was not found in {} data; rules that need its fields will error", input, key, t.name));
                        } else {
                            notes.push(format!("`{}` = {} could not be loaded ({} data is not readable here); fetch the row with your own connector and pass it inline, e.g. {}", input, key, t.name, example_record(t)));
                        }
                        out.push(ResolvedInput { name: input.clone(), type_name: declared.name.clone(), key, resolved: false, record: rec, checked_as: said.clone().into_iter().collect() });
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The kinds an input is checked as: the one the agent named, else the `where` kinds its record matches.
fn checked_kinds(ont: &Ontology, declared: &str, said: Option<&str>, rec: &Record, input: &str, notes: &mut Vec<String>) -> Vec<String> {
    if let Some(k) = said {
        if k != declared {
            notes.push(format!("`{}` was checked as a {} (a kind of {}), as `kinds` says", input, k, declared));
            return vec![k.to_string()];
        }
        return vec![];
    }
    let hits = where_kinds_matching(ont, declared, rec);
    for k in &hits {
        let why = ont.types.get(k).map(|t| crate::ontology::Ontology::where_words(&t.where_all)).unwrap_or_default();
        notes.push(format!("`{}` was checked as a {} (a kind of {}), because {}", input, k, declared, why));
    }
    hits
}

fn evaluate_all(ont: &Ontology, action: &ActionDef, inputs: &[ResolvedInput], notes: &mut Vec<String>) -> Vec<RuleResult> {
    let recs: IndexMap<String, &ResolvedInput> = inputs.iter().map(|i| (i.name.clone(), i)).collect();
    let mut results = Vec::new();
    // the action's rules, plus the guardrails of every kind an input was checked as (and of that
    // kind's parents below the type the action takes)
    let mut rules: Vec<&crate::schema::Constraint> = ont.constraints_for(&action.name);
    for ri in inputs {
        for k in &ri.checked_as {
            for n in ont.self_and_ancestors(k).into_iter().take_while(|n| n != &ri.type_name) {
                for c in ont.constraints.values().filter(|c| c.applies_to.eq_ignore_ascii_case(&n)) {
                    if !rules.iter().any(|x| x.id == c.id) {
                        rules.push(c);
                    }
                }
            }
        }
    }

    for c in rules {
        let program = match compile(&c.rule) {
            Ok(p) => p,
            Err(e) => {
                results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: false, applies_to: c.applies_to.clone(), bound_to: vec![], rule: c.rule.clone(), message: "rule does not parse".into(), rationale: c.rationale.clone(), error: Some(e), grade: None });
                continue;
            }
        };
        let mut bindings: Vec<(Vec<String>, IndexMap<String, Json>)> = Vec::new();

        match ont.kind_of(&c.applies_to) {
            Some(Kind::Type) => {
                let t = ont.type_(&c.applies_to).unwrap();
                for (name, ri) in &recs {
                    // a rule on Sale binds to an input that is a Sale, a kind of Sale, or was checked as one
                    let is = ri.type_name == t.name || ont.is_a(&ri.type_name, &t.name) || ri.checked_as.iter().any(|k| ont.is_a(k, &t.name));
                    if is {
                        let mut vars: IndexMap<String, Json> = ri.record.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                        let rec_json = Json::Object(ri.record.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                        let deepest = ri.checked_as.first().cloned().unwrap_or(ri.type_name.clone());
                        for n in ont.self_and_ancestors(&deepest) {
                            vars.insert(snake(&n), rec_json.clone());
                        }
                        vars.insert(snake(&t.name), rec_json.clone());
                        vars.insert("self".into(), rec_json);
                        bindings.push((vec![name.clone()], vars));
                    }
                }
            }
            Some(Kind::Relationship) => {
                let rels = ont.relationship(&c.applies_to);
                for e in &action.effects {
                    if let Some(edge) = &e.create {
                        let matches = rels.iter().any(|r| r.name.eq_ignore_ascii_case(&edge.relationship) || r.qualified().eq_ignore_ascii_case(&edge.relationship));
                        if !matches {
                            continue;
                        }
                        if let (Some(f), Some(t)) = (recs.get(&edge.from), recs.get(&edge.to)) {
                            let rel = rels[0];
                            let fj = Json::Object(f.record.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                            let tj = Json::Object(t.record.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                            let mut vars = IndexMap::new();
                            vars.insert(snake(&rel.from), fj.clone());
                            vars.insert(snake(&rel.to), tj.clone());
                            vars.insert("from".into(), fj);
                            vars.insert("to".into(), tj);
                            vars.insert("edge".into(), json!({}));
                            bindings.push((vec![edge.from.clone(), edge.to.clone()], vars));
                        }
                    }
                }
                if bindings.is_empty() {
                    notes.push(format!("{} applies to relationship `{}` but `{}` creates no such edge; evaluated with action inputs instead", c.id, c.applies_to, action.name));
                }
            }
            _ => {}
        }
        if bindings.is_empty() {
            // action-level: every input is a variable
            let mut vars = IndexMap::new();
            for (name, ri) in &recs {
                vars.insert(name.clone(), Json::Object(ri.record.iter().map(|(k, v)| (k.clone(), v.clone())).collect()));
            }
            vars.insert("action".into(), json!(action.name));
            bindings.push((recs.keys().cloned().collect(), vars));
        }

        for (bound_to, vars) in bindings {
            let labels: Vec<String> = bound_to
                .iter()
                .map(|b| recs.get(b).map(|ri| format!("{}={}", b, if ri.key.is_empty() { "?".into() } else { ri.key.clone() })).unwrap_or(b.clone()))
                .collect();
            match eval(&program, &vars) {
                Ok(true) => results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: true, applies_to: c.applies_to.clone(), bound_to, rule: c.rule.clone(), message: format!("holds for {}", labels.join(", ")), rationale: c.rationale.clone(), error: None, grade: None }),
                Ok(false) => results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: false, applies_to: c.applies_to.clone(), bound_to, rule: c.rule.clone(), message: format!("violated for {}", labels.join(", ")), rationale: c.rationale.clone(), error: None, grade: None }),
                Err(e) => results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: false, applies_to: c.applies_to.clone(), bound_to, rule: c.rule.clone(), message: format!("could not evaluate for {}", labels.join(", ")), rationale: c.rationale.clone(), error: Some(e), grade: None }),
            }
        }
    }
    results.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.passed.cmp(&b.passed)).then(a.id.cmp(&b.id)));
    results
}

/// A rule that could not be evaluated (a field the record does not have, a
/// row that was not found) never counts as a violation: the answer is
/// `unknown` unless some other rule is plainly violated.
fn verdict_of(results: &[RuleResult]) -> &'static str {
    let violated = |sev: Severity| results.iter().any(|r| !r.passed && r.error.is_none() && r.severity == sev);
    let errored = results.iter().any(|r| !r.passed && r.error.is_some() && r.severity != Severity::Info);
    if violated(Severity::Block) {
        "fail"
    } else if errored {
        "unknown"
    } else if violated(Severity::Warn) {
        "warn"
    } else {
        "pass"
    }
}

/// The main entry point: can this action be taken with these inputs?
pub fn check_action(ont: &Ontology, data: &DataLayer, action_name: &str, raw: &IndexMap<String, Json>) -> Result<CheckReport> {
    check_action_with(ont, data, action_name, raw, &IndexMap::new())
}

/// `check_action` with `kinds`: input name → the kind of entity it is, when that is a kind of the type the action takes.
pub fn check_action_with(ont: &Ontology, data: &DataLayer, action_name: &str, raw: &IndexMap<String, Json>, kinds: &IndexMap<String, String>) -> Result<CheckReport> {
    let action = ont.action(action_name).ok_or_else(|| {
        anyhow!("unknown action `{}`. Known actions: {}", action_name, ont.actions.keys().cloned().collect::<Vec<_>>().join(", "))
    })?;
    let mut notes = Vec::new();
    let mut read = Vec::new();
    let inputs = resolve_inputs(ont, data, action, raw, kinds, &mut notes, &mut read)?;
    let mut results = evaluate_all(ont, action, &inputs, &mut notes);
    // what a person stands behind: each result's grade, and what was relied on unreviewed
    let mut unverified: Vec<String> = Vec::new();
    let mut unreviewed_rules: Vec<String> = Vec::new();
    if ont.graded.has_file {
        use crate::trust::Kind as TK;
        for r in results.iter_mut() {
            let st = ont.grade_of(TK::Guardrail, &r.id);
            r.grade = Some(st.word().to_string());
            let why = match (&st.grade, &st.record) {
                (Some(Grade::Proposed), Some(rec)) => Some(format!("rule {}: an agent proposed it ({}) and no person has accepted it", r.id, crate::trust::short_date(&rec.at))),
                (None, Some(rec)) if st.changed => Some(format!("rule {}: changed since it was {} ({}, {}); no person has reviewed the new version", r.id, rec.grade.word(), crate::trust::who_said(&rec.by), crate::trust::short_date(&rec.at))),
                _ => None,
            };
            if let Some(w) = why {
                if !unreviewed_rules.contains(&r.id) {
                    unreviewed_rules.push(r.id.clone());
                    unverified.push(w);
                }
            }
        }
        for t in &read {
            let st = ont.grade_of(TK::Binding, t);
            match (&st.grade, &st.record) {
                (Some(Grade::Proposed), Some(rec)) => unverified.push(format!("{} rows were read through a data binding nobody has reviewed (proposed by an agent, {})", t, crate::trust::short_date(&rec.at))),
                (None, Some(rec)) if st.changed => unverified.push(format!("{} rows were read through a data binding that changed since it was {} ({}); nobody has reviewed the new version", t, rec.grade.word(), crate::trust::short_date(&rec.at))),
                _ => {}
            }
        }
    }
    // a key no row matched: rules that needed its fields could not run, so the
    // answer is "unknown", never "blocked" (an agent would repeat a wrong reason)
    let missing: Vec<&ResolvedInput> = inputs.iter().filter(|i| !i.resolved).collect();
    let unknown = !missing.is_empty() && results.iter().any(|r| r.error.is_some() && r.bound_to.iter().any(|b| missing.iter().any(|m| &m.name == b)));
    let verdict = if unknown { "unknown" } else { verdict_of(&results) };

    // suggestions: for inputs involved in blocking failures, try other rows
    let mut suggestions = Vec::new();
    if verdict == "fail" {
        let mut culprit_inputs: Vec<String> = results
            .iter()
            .filter(|r| !r.passed && r.severity == Severity::Block)
            .flat_map(|r| r.bound_to.clone())
            .collect();
        culprit_inputs.sort();
        culprit_inputs.dedup();
        for input in culprit_inputs {
            let Some(ri) = inputs.iter().find(|i| i.name == input) else { continue };
            let Some(t) = ont.type_(&ri.type_name) else { continue };
            if !data.is_local(t) {
                continue;
            }
            // only suggest swaps for inputs that are "the new thing" (not the part being acted on):
            // heuristic — the input participates as `to` of a created edge, or is the only input of its type.
            let is_target = action.effects.iter().any(|e| e.create.as_ref().map(|c| c.to == input || c.from == input).unwrap_or(false));
            let is_part_like = action.effects.iter().any(|e| e.remove.as_ref().map(|c| c.to == input || c.from == input).unwrap_or(false))
                && action.effects.iter().any(|e| e.create.as_ref().map(|c| c.to == input || c.from == input).unwrap_or(false));
            if !is_target || is_part_like {
                continue;
            }
            let candidates = data.instances(&t.name).unwrap_or_default();
            let mut found = 0;
            for cand in candidates {
                let ck = key_of(&cand, t);
                if ck.is_empty() || ck.eq_ignore_ascii_case(&ri.key) {
                    continue;
                }
                let mut alt: Vec<ResolvedInput> = inputs.clone();
                if let Some(slot) = alt.iter_mut().find(|i| i.name == input) {
                    slot.key = ck.clone();
                    slot.record = cand.clone();
                    slot.resolved = true;
                }
                let mut scratch = Vec::new();
                let alt_results = evaluate_all(ont, action, &alt, &mut scratch);
                if verdict_of(&alt_results) != "fail" {
                    suggestions.push(Suggestion { input: input.clone(), replace_with: ck, label: label_of(&cand, t) });
                    found += 1;
                    if found >= 5 {
                        break;
                    }
                }
            }
        }
    }

    let failed_at = |sev: Severity| -> Vec<String> {
        results
            .iter()
            .filter(|r| !r.passed && r.severity == sev)
            .map(|r| if unreviewed_rules.contains(&r.id) { format!("{} (unreviewed: an agent proposed this rule and no person has accepted it)", r.id) } else { r.id.clone() })
            .collect()
    };
    let (blocks, warns, infos) = (failed_at(Severity::Block), failed_at(Severity::Warn), failed_at(Severity::Info));
    let tail = |label: &str, ids: &Vec<String>| if ids.is_empty() { String::new() } else { format!("; {} {}: {}", ids.len(), label, ids.join(", ")) };
    let summary = match verdict {
        "unknown" => {
            let mut parts: Vec<String> = missing.iter().map(|m| format!("`{}` = {} was not found in {} data", m.name, m.key, m.type_name)).collect();
            for r in results.iter().filter(|r| !r.passed && r.error.is_some() && r.severity != Severity::Info) {
                parts.push(format!("{} could not be evaluated ({})", r.id, r.error.clone().unwrap_or_default()));
            }
            format!("{} could not be checked: {}. Fetch the missing rows or fields with your own connector and pass them inline.", action.name, parts.join("; "))
        }
        "pass" if infos.is_empty() => format!("{} may proceed: all {} rules hold.", action.name, results.len()),
        "pass" => format!("{} may proceed{}.", action.name, tail("note(s)", &infos)),
        "warn" => format!("{} may proceed with warnings: {}{}.", action.name, warns.join(", "), tail("note(s)", &infos)),
        _ => format!("{} is blocked by {}{}{}.", action.name, blocks.join(", "), tail("warning(s)", &warns), tail("note(s)", &infos)),
    };
    let summary = if unverified.is_empty() { summary } else { format!("{} Not reviewed by a person: {}.", summary, unverified.join("; ")) };

    Ok(CheckReport {
        action: action.name.clone(),
        verdict: verdict.to_string(),
        summary,
        inputs,
        results,
        requires_approval_from: action.requires_approval_from.clone(),
        side_effects: action.side_effects.clone(),
        suggestions,
        notes,
        unverified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implies_is_only_special_outside_strings() {
        assert_eq!(desugar("tier == 1 implies audited"), "!(tier == 1) || (audited)");
        assert_eq!(desugar("a implies b implies c"), "!(a) || (!(b) || (c))");
        assert_eq!(desugar(r#"order_id != "a implies b""#), r#"order_id != "a implies b""#);
        assert_eq!(desugar(r#"note == 'x implies y' implies ok"#), r#"!(note == 'x implies y') || (ok)"#);
    }

    #[test]
    fn a_rule_that_cannot_run_is_unknown_unless_another_is_plainly_violated() {
        let errored = RuleResult { id: "A".into(), name: "a".into(), severity: Severity::Block, passed: false, error: Some("no such key".into()), message: String::new(), rationale: String::new(), bound_to: vec![], applies_to: String::new(), rule: String::new(), grade: None };
        let violated = RuleResult { id: "B".into(), name: "b".into(), severity: Severity::Block, passed: false, error: None, message: String::new(), rationale: String::new(), bound_to: vec![], applies_to: String::new(), rule: String::new(), grade: None };
        let fine = RuleResult { id: "C".into(), name: "c".into(), severity: Severity::Warn, passed: true, error: None, message: String::new(), rationale: String::new(), bound_to: vec![], applies_to: String::new(), rule: String::new(), grade: None };
        assert_eq!(verdict_of(&[errored.clone(), fine.clone()]), "unknown");
        assert_eq!(verdict_of(&[errored, violated]), "fail");
        assert_eq!(verdict_of(&[fine]), "pass");
    }

    #[test]
    fn an_unreviewed_rule_still_blocks_and_the_answer_says_it_is_unreviewed() {
        use crate::trust::{fingerprint, things, Kind as TK};
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ecommerce");
        let (ont, _) = crate::load_ontology(&src).unwrap();
        let data = DataLayer::new(&ont);
        let inputs: IndexMap<String, Json> = [("order".to_string(), json!("ORD-1001"))].into_iter().collect();
        let plain = check_action(&ont, &data, "cancel_order", &inputs).unwrap();
        assert_eq!(plain.verdict, "fail");
        assert!(plain.unverified.is_empty() && plain.results.iter().all(|r| r.grade.is_none()), "no trust.yaml, no grades");
        // the same space with EC-001 and the Order binding proposed by an agent
        let mut graded = crate::load_ontology(&src).unwrap().0;
        let rec = |v: &Json| crate::schema::TrustRecord { grade: crate::schema::Grade::Proposed, by: "agent via MCP".into(), at: "2026-11-24".into(), of: fingerprint(v), against: None, note: None };
        let mut book = crate::schema::TrustBook::default();
        book.guardrails.insert("EC-001".into(), rec(&things(&graded, TK::Guardrail)["EC-001"]));
        book.bindings.insert("Order".into(), rec(&things(&graded, TK::Binding)["Order"]));
        graded.set_trust(Some(book));
        let data = DataLayer::new(&graded);
        let rep = check_action(&graded, &data, "cancel_order", &inputs).unwrap();
        assert_eq!(rep.verdict, plain.verdict, "the verdict does not change");
        let ec = rep.results.iter().find(|r| r.id == "EC-001").unwrap();
        assert_eq!(ec.grade.as_deref(), Some("proposed"));
        assert!(rep.unverified.iter().any(|u| u.starts_with("rule EC-001")), "{:?}", rep.unverified);
        assert!(rep.unverified.iter().any(|u| u.starts_with("Order rows were read through a data binding nobody has reviewed")), "{:?}", rep.unverified);
        assert!(rep.summary.contains("blocked by EC-001 (unreviewed: an agent proposed this rule"), "{}", rep.summary);
        // a pass over rows read through the proposed binding is still a pass
        let ok: IndexMap<String, Json> = [("order".to_string(), json!("ORD-1002"))].into_iter().collect();
        let rep = check_action(&graded, &data, "cancel_order", &ok).unwrap();
        let before = check_action(&ont, &DataLayer::new(&ont), "cancel_order", &ok).unwrap();
        assert_eq!(rep.verdict, before.verdict);
    }
}
