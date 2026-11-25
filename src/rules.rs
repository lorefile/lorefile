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

/// `A implies B` → `!(A) || (B)`. Everything else is plain CEL.
pub fn desugar(rule: &str) -> String {
    let r = rule.trim();
    if let Some(idx) = r.find(" implies ") {
        let (a, b) = (&r[..idx], &r[idx + " implies ".len()..]);
        return format!("!({}) || ({})", a.trim(), desugar(b));
    }
    r.to_string()
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
fn resolve_inputs(ont: &Ontology, data: &DataLayer, action: &ActionDef, raw: &IndexMap<String, Json>, notes: &mut Vec<String>) -> Result<Vec<ResolvedInput>> {
    let mut out = Vec::new();
    for (input, type_name) in &action.inputs {
        let t = ont.type_(type_name).ok_or_else(|| anyhow!("action input `{}` has unknown type `{}`", input, type_name))?;
        let Some(v) = raw.get(input) else {
            return Err(anyhow!("missing input `{}` ({}). Inputs: {}", input, type_name,
                action.inputs.iter().map(|(k, v)| format!("{}: {}", k, v)).collect::<Vec<_>>().join(", ")));
        };
        match v {
            Json::Object(o) => {
                let rec: Record = o.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                out.push(ResolvedInput { name: input.clone(), type_name: t.name.clone(), key: key_of(&rec, t), resolved: true, record: rec });
            }
            other => {
                let key = match other {
                    Json::String(s) => s.clone(),
                    o => o.to_string(),
                };
                match data.find(&t.name, &key)? {
                    Some(rec) => out.push(ResolvedInput { name: input.clone(), type_name: t.name.clone(), key, resolved: true, record: rec }),
                    None => {
                        let kp = t.key_property().unwrap_or_else(|| "id".into());
                        let mut rec = Record::new();
                        rec.insert(kp, json!(key));
                        if data.is_local(t) {
                            notes.push(format!("`{}` = {} was not found in {} data; rules that need its fields will error", input, key, t.name));
                        } else {
                            notes.push(format!("`{}` = {} could not be loaded ({} data is not readable here); fetch the row with your own connector and pass it inline, e.g. {}", input, key, t.name, example_record(t)));
                        }
                        out.push(ResolvedInput { name: input.clone(), type_name: t.name.clone(), key, resolved: false, record: rec });
                    }
                }
            }
        }
    }
    Ok(out)
}

fn evaluate_all(ont: &Ontology, action: &ActionDef, inputs: &[ResolvedInput], notes: &mut Vec<String>) -> Vec<RuleResult> {
    let recs: IndexMap<String, &ResolvedInput> = inputs.iter().map(|i| (i.name.clone(), i)).collect();
    let mut results = Vec::new();

    for c in ont.constraints_for(&action.name) {
        let program = match compile(&c.rule) {
            Ok(p) => p,
            Err(e) => {
                results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: false, applies_to: c.applies_to.clone(), bound_to: vec![], rule: c.rule.clone(), message: "rule does not parse".into(), rationale: c.rationale.clone(), error: Some(e) });
                continue;
            }
        };
        let mut bindings: Vec<(Vec<String>, IndexMap<String, Json>)> = Vec::new();

        match ont.kind_of(&c.applies_to) {
            Some(Kind::Type) => {
                let t = ont.type_(&c.applies_to).unwrap();
                for (name, ri) in &recs {
                    if ri.type_name == t.name {
                        let mut vars: IndexMap<String, Json> = ri.record.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                        let rec_json = Json::Object(ri.record.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
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
                Ok(true) => results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: true, applies_to: c.applies_to.clone(), bound_to, rule: c.rule.clone(), message: format!("holds for {}", labels.join(", ")), rationale: c.rationale.clone(), error: None }),
                Ok(false) => results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: false, applies_to: c.applies_to.clone(), bound_to, rule: c.rule.clone(), message: format!("violated for {}", labels.join(", ")), rationale: c.rationale.clone(), error: None }),
                Err(e) => results.push(RuleResult { id: c.id.clone(), name: c.name.clone(), severity: c.severity, passed: false, applies_to: c.applies_to.clone(), bound_to, rule: c.rule.clone(), message: format!("could not evaluate for {}", labels.join(", ")), rationale: c.rationale.clone(), error: Some(e) }),
            }
        }
    }
    results.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.passed.cmp(&b.passed)).then(a.id.cmp(&b.id)));
    results
}

fn verdict_of(results: &[RuleResult]) -> &'static str {
    if results.iter().any(|r| !r.passed && r.severity == Severity::Block) {
        "fail"
    } else if results.iter().any(|r| !r.passed && r.severity == Severity::Warn) {
        "warn"
    } else {
        "pass"
    }
}

/// The main entry point: can this action be taken with these inputs?
pub fn check_action(ont: &Ontology, data: &DataLayer, action_name: &str, raw: &IndexMap<String, Json>) -> Result<CheckReport> {
    let action = ont.action(action_name).ok_or_else(|| {
        anyhow!("unknown action `{}`. Known actions: {}", action_name, ont.actions.keys().cloned().collect::<Vec<_>>().join(", "))
    })?;
    let mut notes = Vec::new();
    let inputs = resolve_inputs(ont, data, action, raw, &mut notes)?;
    let results = evaluate_all(ont, action, &inputs, &mut notes);
    let verdict = verdict_of(&results);

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

    let failed_at = |sev: Severity| -> Vec<String> { results.iter().filter(|r| !r.passed && r.severity == sev).map(|r| r.id.clone()).collect() };
    let (blocks, warns, infos) = (failed_at(Severity::Block), failed_at(Severity::Warn), failed_at(Severity::Info));
    let tail = |label: &str, ids: &Vec<String>| if ids.is_empty() { String::new() } else { format!("; {} {}: {}", ids.len(), label, ids.join(", ")) };
    let summary = match verdict {
        "pass" if infos.is_empty() => format!("{} may proceed: all {} rules hold.", action.name, results.len()),
        "pass" => format!("{} may proceed{}.", action.name, tail("note(s)", &infos)),
        "warn" => format!("{} may proceed with warnings: {}{}.", action.name, warns.join(", "), tail("note(s)", &infos)),
        _ => format!("{} is blocked by {}{}{}.", action.name, blocks.join(", "), tail("warning(s)", &warns), tail("note(s)", &infos)),
    };

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
    })
}
