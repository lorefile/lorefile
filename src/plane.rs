//! The home page and the Map: one picture of a space (specs/2026-11-27-home-plane.md).
//!
//! `GET /api/plane` answers with everything both views draw: each entity with its traffic
//! light and the reasons for it in words, how important it is and why, a glyph guessed from
//! its name, the metrics that count it and the fields they use, the sources underneath, the
//! agents above, and the waiting proposals as ghosts. Fields stay lazy (`/api/describe`).
//!
//! Never a row value and never a metric's value: names, words and grades only.

use crate::ontology::Ontology;
use crate::proposals::Proposal;
use crate::review::{self, words};
use crate::schema::{Grade, MetricDef, MetricSource};
use crate::trust::{self, Kind as TKind};
use crate::validate::{Diag, Level, Report};
use regex::Regex;
use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// Lights
// ---------------------------------------------------------------------------

/// A traffic light. `None` is no badge: nothing to fix, and nobody has graded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Light {
    None,
    Green,
    Amber,
    Red,
}

impl Light {
    pub fn word(self) -> &'static str {
        match self {
            Light::None => "none",
            Light::Green => "green",
            Light::Amber => "amber",
            Light::Red => "red",
        }
    }
}

/// Why something has its light, in words a person reads in the card and the list.
#[derive(Debug, Clone)]
pub struct Reason {
    pub light: Light,
    /// Where it came from when it rolled up: "metric promo_sale_revenue".
    pub from: Option<String>,
    pub code: String,
    pub text: String,
    /// The one-field fix the card offers (Decision 22), as the op the page sends.
    pub fix: Option<Json>,
    /// A waiting proposal's id, for the card's Review button.
    pub review: Option<String>,
}

impl Reason {
    fn new(light: Light, code: &str, text: impl Into<String>) -> Reason {
        Reason { light, from: None, code: code.to_string(), text: text.into(), fix: None, review: None }
    }
    fn fix(mut self, fix: Json) -> Reason {
        self.fix = Some(fix);
        self
    }
    fn to_json(&self) -> Json {
        let mut j = json!({ "light": self.light.word(), "code": self.code, "text": self.text });
        if let Some(f) = &self.from {
            j["from"] = json!(f);
        }
        if let Some(f) = &self.fix {
            j["fix"] = f.clone();
        }
        if let Some(r) = &self.review {
            j["review"] = json!(r);
        }
        j
    }
}

/// The worst light among the reasons; `base` (green or none) when there are none.
pub fn light_of(reasons: &[Reason], base: Light) -> Light {
    reasons.iter().map(|r| r.light).max().map(|l| l.max(base)).unwrap_or(base)
}

/// A message for people, without the backticks it was written with.
fn clean(msg: &str) -> String {
    msg.replace('`', "")
}

/// "ChatGPT" or "an agent", from a trust record's or a proposal's `by` / `via`.
fn agent_words(by: &str) -> String {
    let a = review::agent_name(by);
    if a == "An agent" {
        "an agent".into()
    } else {
        a
    }
}

/// The reasons a grade gives: unverified (proposed) or changed since a person graded it.
fn grade_reasons(ont: &Ontology, k: TKind, name: &str) -> Vec<Reason> {
    let st = ont.grade_of(k, name);
    let Some(r) = &st.record else { return vec![] };
    if st.changed {
        return vec![Reason::new(Light::Amber, "W113", format!("changed since {} {} it on {}", trust::who_said(&r.by), r.grade.word(), trust::short_date(&r.at)))];
    }
    if r.grade == Grade::Proposed {
        return vec![Reason::new(Light::Amber, "proposed", format!("unverified: {} proposed it on {}, no person has looked", agent_words(&r.by), trust::short_date(&r.at)))];
    }
    vec![]
}

/// Accepted or checked, and not changed since: a person stands behind it.
fn stands_behind(ont: &Ontology, k: TKind, name: &str) -> bool {
    let st = ont.grade_of(k, name);
    !st.changed && matches!(st.grade, Some(Grade::Accepted) | Some(Grade::Checked))
}

/// One diagnostic as a reason, in the review screen's words where it has them.
fn diag_reason(d: &Diag, ont: &Ontology) -> Option<Reason> {
    let subj = d.subject.as_ref()?;
    if d.level == Level::Error {
        return Some(Reason::new(Light::Red, d.code, clean(&d.message)));
    }
    let n = subj.name.as_str();
    Some(match (d.code, subj.kind) {
        // W113 is said from the grade itself (who and when), not from the message
        ("W113", _) | ("W114", _) => return None,
        ("W109", "metric") => Reason::new(Light::Amber, "W109", "no owner. Who decides what it means?").fix(json!({ "op": "set_metric", "metric": n, "field": "owner" })),
        ("W101", "entity") => Reason::new(Light::Amber, "W101", "no description: agents have only its name to go on").fix(json!({ "op": "set_type_description", "type": n, "field": "description" })),
        ("W101", "metric") => Reason::new(Light::Amber, "W101", "no description: agents have only its name to go on").fix(json!({ "op": "set_metric", "metric": n, "field": "description" })),
        ("W103", _) => Reason::new(Light::Amber, "W103", "no field says which row is which"),
        ("W105", _) => {
            let field = d.message.split('`').nth(1).and_then(|f| f.split('.').nth(1)).unwrap_or("a field");
            Reason::new(Light::Amber, "W105", format!("{} is \"one of\" but lists no values", field))
        }
        ("W106", _) => {
            let path = d.message.rsplit(" at ").next().unwrap_or("its file");
            let file = std::path::Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
            Reason::new(Light::Amber, "W106", format!("Lorefile can't find {}", file))
        }
        ("W102", _) => Reason::new(Light::Amber, "W102", "stops actions but doesn't say why"),
        (_, "entity") | (_, "metric") | (_, "guardrail") | (_, "source") => Reason::new(Light::Amber, d.code, clean(&d.message)),
        _ => {
            let _ = ont;
            return None;
        }
    })
}

// ---------------------------------------------------------------------------
// Icons (Part 5)
// ---------------------------------------------------------------------------

/// The glyph table, in order: the first row with a matching word wins.
const ICONS: [(&str, &[&str]); 16] = [
    ("receipt", &["order", "sale", "sales", "invoice", "payment", "transaction", "purchase", "checkout", "refund", "charge"]),
    ("person", &["customer", "user", "member", "contact", "person", "lead", "buyer", "shopper", "subscriber", "patient", "student"]),
    ("people", &["team", "employee", "staff", "agent_group", "owner_group"]),
    ("building", &["company", "organization", "organisation", "org", "account", "vendor", "supplier", "carrier", "partner", "merchant", "brand"]),
    ("box", &["product", "item", "sku", "variant", "inventory", "stock", "part", "material"]),
    ("tag", &["promo", "promotion", "coupon", "discount", "offer", "price", "pricing", "tier", "plan"]),
    ("megaphone", &["campaign", "ad", "ads", "creative", "channel", "audience"]),
    ("truck", &["shipment", "parcel", "delivery", "package", "fulfilment", "fulfillment", "route"]),
    ("pin", &["warehouse", "location", "store", "region", "site", "address", "country", "city", "zone"]),
    ("calendar", &["date", "day", "week", "month", "period", "calendar", "season", "event_date"]),
    ("bolt", &["event", "session", "visit", "click", "pageview", "impression", "signal"]),
    ("repeat", &["subscription", "renewal", "contract", "membership"]),
    ("chat", &["ticket", "case", "issue", "conversation", "message", "review", "feedback"]),
    ("chart", &["metric", "kpi", "score", "forecast", "budget", "target"]),
    ("document", &["file", "document", "report", "export", "upload", "sheet"]),
    ("link", &["relationship", "mapping", "assignment", "association"]),
];

/// An entity's glyph, guessed from the words of its name, then its source, else a cube; and why.
pub fn icon_for(name: &str, source_kind: Option<&str>) -> (&'static str, String) {
    let snake = crate::ontology::snake(name);
    let words: Vec<&str> = snake.split('_').filter(|w| !w.is_empty()).collect();
    for (icon, list) in ICONS {
        for w in list {
            let hit = if w.contains('_') {
                snake == *w || snake.starts_with(&format!("{}_", w)) || snake.ends_with(&format!("_{}", w)) || snake.contains(&format!("_{}_", w))
            } else {
                words.iter().any(|x| x == w || *x == format!("{}s", w))
            };
            if hit {
                return (icon, format!("name: {}", w));
            }
        }
    }
    match source_kind {
        Some(k) if k.eq_ignore_ascii_case("from_user") => ("document", "source: files people give agents".into()),
        _ => ("cube", "no word in its name matched".into()),
    }
}

/// A source's glyph by its kind.
pub fn source_icon(kind: &str) -> &'static str {
    match kind.to_ascii_lowercase().as_str() {
        "csv" | "parquet" => "sheet",
        "s3" => "bucket",
        "mcp" => "plug",
        "from_user" => "paperclip",
        "http" => "globe",
        _ => "cylinder",
    }
}

/// What a source is, in a few words.
fn source_words(kind: &str) -> &'static str {
    match kind.to_ascii_lowercase().as_str() {
        "from_user" => "files people give agents",
        "mcp" => "agents fetch rows with tools",
        "csv" | "parquet" | "duckdb" => "readable here",
        "bigquery" | "snowflake" | "postgres" | "postgresql" => "agents fetch rows",
        "s3" => "files in a bucket",
        "http" => "an HTTP API",
        _ => "agents fetch rows",
    }
}

// ---------------------------------------------------------------------------
// Metrics and the entities they count
// ---------------------------------------------------------------------------

fn sources_of(m: &MetricDef) -> Vec<&MetricSource> {
    m.source.iter().chain(m.variants.iter().map(|v| &v.source)).collect()
}

/// The entities a metric counts: its source (and each variant's) matched on connection and
/// table (or tool) against each entity's source. SQL only, or no match: none.
pub fn metric_entities(m: &MetricDef, ont: &Ontology) -> Vec<String> {
    let mut out = Vec::new();
    for s in sources_of(m) {
        for t in ont.types.values() {
            if let Some(b) = &t.source {
                let same = b.connection == s.connection && ((!s.table.is_empty() && b.table == s.table) || (s.table.is_empty() && !s.tool.is_empty() && b.tool == s.tool));
                if same && !out.contains(&t.name) {
                    out.push(t.name.clone());
                }
            }
        }
    }
    out
}

const SQL_WORDS: [&str; 24] = ["sum", "count", "avg", "min", "max", "distinct", "and", "or", "not", "is", "in", "like", "between", "null", "true", "false", "case", "when", "then", "else", "end", "as", "coalesce", "round"];

/// The identifiers of an expression that are not SQL words, with quoted strings left out.
fn identifiers(expr: &str) -> Vec<String> {
    let unquoted = Regex::new(r#"'[^']*'|"[^"]*""#).map(|r| r.replace_all(expr, " ").to_string()).unwrap_or_default();
    let re = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap();
    let mut out: Vec<String> = Vec::new();
    for m in re.find_iter(&unquoted) {
        let w = m.as_str();
        if !SQL_WORDS.contains(&w.to_ascii_lowercase().as_str()) && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
        }
    }
    out
}

/// The fields a metric uses, each with how: "adds up" (its measure), "leaves out" or "keeps"
/// (its filter, in `filter_words`' direction). Only fields of the entities it counts.
pub fn metric_uses(m: &MetricDef, ont: &Ontology, entities: &[String]) -> Vec<(String, &'static str)> {
    let fields: Vec<&String> = entities.iter().filter_map(|e| ont.type_(e)).flat_map(|t| t.properties.keys()).collect();
    let mut out: Vec<(String, &'static str)> = Vec::new();
    let and = Regex::new(r"(?i)\s+and\s+").unwrap();
    let mut push = |f: String, how: &'static str| {
        if fields.iter().any(|x| **x == f) && !out.iter().any(|(o, _)| *o == f) {
            out.push((f, how));
        }
    };
    for s in sources_of(m) {
        let counting = s.measure.to_ascii_lowercase().contains("count");
        for f in identifiers(&s.measure) {
            push(f, if counting { "counts" } else { "adds up" });
        }
        let parts: Vec<&str> = and.split(&s.filter).collect();
        for part in parts.into_iter().filter(|p| !p.trim().is_empty()) {
            let how = match words::filter_words(part, "rows") {
                Some(w) if w.starts_with("leaving out") => "leaves out",
                _ => "keeps",
            };
            for f in identifiers(part) {
                push(f, how);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Importance (Part 3)
// ---------------------------------------------------------------------------

pub struct Signals {
    pub metrics: usize,
    pub links: usize,
    pub light: Light,
    pub waiting: bool,
    pub has_data: bool,
    pub changed: bool,
}

/// The score, and why, in the words the card shows.
pub fn importance(s: &Signals) -> (i64, Vec<String>) {
    let mut score = 0;
    let mut why = Vec::new();
    if s.metrics > 0 {
        score += 4;
        why.push("counted by a metric (+4)".to_string());
    }
    if s.links > 0 {
        let n = s.links.min(6) as i64;
        score += n;
        why.push(format!("{} (+{})", review::plural(s.links, "link"), n));
    }
    match s.light {
        Light::Red => {
            score += 6;
            why.push("something is wrong (+6)".into());
        }
        Light::Amber => {
            score += 3;
            why.push("needs a look (+3)".into());
        }
        _ => {}
    }
    if s.waiting {
        score += 4;
        why.push("a waiting proposal changes it (+4)".into());
    }
    if s.has_data {
        score += 1;
        why.push("has data (+1)".into());
    }
    if s.changed {
        score += 2;
        why.push("changed this week (+2)".into());
    }
    (score, why)
}

// ---------------------------------------------------------------------------
// Agents (Decision 16)
// ---------------------------------------------------------------------------

/// The first letter or digit of a name, in capitals; "A" when it has none.
pub fn monogram(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "A".into())
}

/// A name shown as text, cut at 40 characters.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>().trim_end())
    } else {
        s.to_string()
    }
}

/// An agent that wrote here without an OAuth connection, named by its channel.
fn activity_agent(via: &str) -> Option<(String, String)> {
    let v = via.trim();
    if !v.starts_with("agent") || v.contains("connection \"") {
        return None;
    }
    let name = if v.contains("via API") { "the page's tool runner" } else { "an agent (MCP)" };
    Some((format!("activity:{}", v), name.to_string()))
}

fn days_ago(at: &str, today: chrono::NaiveDate) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(at.get(..10)?, "%Y-%m-%d").ok()?;
    Some((today - d).num_days())
}

// ---------------------------------------------------------------------------
// The whole picture
// ---------------------------------------------------------------------------

/// The space as the page knows it.
pub struct SpaceInfo<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub builtin: bool,
    pub editable: bool,
}

/// Everything `plane` reads. `ghost` gives a waiting proposal's graph and changes (cached by the server).
pub struct Input<'a> {
    pub ont: &'a Ontology,
    pub report: &'a Report,
    pub space: SpaceInfo<'a>,
    pub proposals: &'a [Proposal],
    pub activity: &'a [Json],
    /// OAuth connections (`connections_json`'s list); empty when OAuth is off.
    pub connections: &'a [Json],
    /// Files committed in the last 7 days, with the day (history on); `None` with history off.
    pub git_changed: Option<&'a HashMap<String, String>>,
    pub today: chrono::NaiveDate,
    pub version: String,
}

/// The ghost of a waiting proposal: `review::graph` plus the kinds and names it changes. One whose
/// files no longer load against today's space (stale, or with errors it would add) has no ghost.
pub fn ghost(ont: &Ontology, report: &Report, p: &Proposal) -> Json {
    let agent = review::agent_name(&p.via);
    let cant = |why: String| json!({ "id": p.id, "error": why, "agent": agent, "monogram": monogram(&agent) });
    match review::prepare(ont, p) {
        review::Prepared::Stale { why } => return cant(why),
        review::Prepared::Fresh { files, .. } | review::Prepared::Replayed { files, .. } => {
            let laid: Vec<(String, Option<String>)> = files.into_iter().map(|(f, t)| (f, Some(t))).collect();
            match review::overlay(&ont.root, &laid) {
                None => return cant("its files do not load".into()),
                Some((_, after)) if after.errors().len() > report.errors().len() => return cant("its files have errors against today's space".into()),
                _ => {}
            }
        }
    }
    let mut g = review::graph(ont, p);
    if g.get("error").is_none() {
        let changes: Vec<Json> = review::own_changes(ont, p).iter().map(|c| json!({ "kind": c["kind"], "name": c["name"], "op": c["op"] })).collect();
        g["changes"] = json!(changes);
    }
    g["agent"] = json!(review::agent_name(&p.via));
    g["monogram"] = json!(monogram(&review::agent_name(&p.via)));
    g
}

/// The answer of `GET /api/plane`.
pub fn plane(inp: &Input, ghost_of: &mut dyn FnMut(&Proposal) -> Json) -> Json {
    let ont = inp.ont;
    // --- diagnostics by subject ---
    let mut own: HashMap<(String, String), Vec<Reason>> = HashMap::new();
    let mut space_reasons: Vec<Reason> = Vec::new();
    for d in &inp.report.diags {
        if d.code == "E000" {
            continue; // a blank space: the page says "empty", with no error look
        }
        let known = d.subject.as_ref().map(|s| match s.kind {
            "entity" => ont.types.contains_key(&s.name),
            "metric" => ont.metrics.contains_key(&s.name),
            "source" => ont.sources.contains_key(&s.name),
            "guardrail" => ont.constraints.contains_key(&s.name),
            _ => true,
        });
        match (known, diag_reason(d, ont)) {
            (Some(true), Some(r)) => {
                let s = d.subject.as_ref().unwrap();
                own.entry((s.kind.to_string(), s.name.clone())).or_default().push(r);
            }
            (Some(true), None) => {}
            _ if d.level == Level::Error => space_reasons.push(Reason::new(Light::Red, d.code, clean(&d.message))),
            (None, _) => space_reasons.push(Reason::new(Light::Amber, d.code, clean(&d.message))),
            _ => {}
        }
    }
    let take = |own: &mut HashMap<(String, String), Vec<Reason>>, k: &str, n: &str| own.remove(&(k.to_string(), n.to_string())).unwrap_or_default();

    // --- waiting proposals: the 5 newest, drawn as ghosts ---
    let pending: Vec<&Proposal> = inp.proposals.iter().filter(|p| p.status == "pending" && p.check.is_none()).collect();
    let mut waiting: Vec<Json> = Vec::new();
    let mut touched: HashMap<(String, String), (String, String)> = HashMap::new(); // (kind, name) → (proposal id, agent)
    for p in pending.iter().take(5) {
        let mut g = ghost_of(p);
        let agent = review::agent_name(&p.via);
        if g.get("error").is_some() {
            waiting.push(json!({ "id": p.id, "agent": agent, "monogram": monogram(&agent), "drawn": false, "why": g["error"] }));
            continue;
        }
        for c in g["changes"].as_array().cloned().unwrap_or_default() {
            let (k, n, op) = (c["kind"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""), c["op"].as_str().unwrap_or(""));
            if op != "add" {
                touched.entry((k.to_string(), n.to_string())).or_insert((p.id.clone(), agent.clone()));
            }
        }
        g["drawn"] = json!(true);
        g["agent"] = json!(agent);
        waiting.push(g);
    }
    let waiting_reason = |touched: &HashMap<(String, String), (String, String)>, k: &str, n: &str| -> Option<Reason> {
        touched.get(&(k.to_string(), n.to_string())).map(|(id, agent)| Reason { review: Some(id.clone()), ..Reason::new(Light::Amber, "waiting", format!("{} wants to change it (waiting for review)", if agent == "An agent" { "an agent" } else { agent.as_str() })) })
    };

    // --- metrics ---
    let mut metric_json: Vec<Json> = Vec::new();
    let mut metric_light: HashMap<String, (Light, Vec<Reason>)> = HashMap::new();
    let mut counted_by: HashMap<String, Vec<String>> = HashMap::new();
    for m in ont.metrics.values() {
        let ents = metric_entities(m, ont);
        let uses = metric_uses(m, ont, &ents);
        for e in &ents {
            counted_by.entry(e.clone()).or_default().push(m.name.clone());
        }
        let mut rs = take(&mut own, "metric", &m.name);
        rs.extend(grade_reasons(ont, TKind::Metric, &m.name));
        rs.extend(waiting_reason(&touched, "metric", &m.name));
        let base = if stands_behind(ont, TKind::Metric, &m.name) { Light::Green } else { Light::None };
        let light = light_of(&rs, base);
        let (sentence, as_written) = words::metric_sentence(m, ont);
        metric_json.push(json!({
            "name": m.name, "label": words::label(&m.name),
            "sentence": if as_written.is_none() { json!(crate::edit::sentence(&sentence)) } else { Json::Null },
            "entities": ents, "uses": uses.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>(),
            "uses_how": uses.iter().map(|(f, h)| json!({ "field": f, "how": h })).collect::<Vec<_>>(),
            "grade": ont.grade_of(TKind::Metric, &m.name).word(), "owner": if m.owner.trim().is_empty() { Json::Null } else { json!(m.owner) },
            "caveats": m.caveats, "basis": m.basis.map(|b| b.word()),
            "light": light.word(), "reasons": rs.iter().map(Reason::to_json).collect::<Vec<_>>(),
        }));
        metric_light.insert(m.name.clone(), (light, rs));
    }

    // --- guardrails, counted on their entity ---
    let mut guard_light: HashMap<String, (Light, Vec<Reason>)> = HashMap::new();
    for c in ont.constraints.values() {
        let mut rs = take(&mut own, "guardrail", &c.id);
        if c.owner.trim().is_empty() {
            rs.push(Reason::new(Light::Amber, "no_owner", "no sign-off: nobody owns it"));
        }
        rs.extend(grade_reasons(ont, TKind::Guardrail, &c.id));
        let base = if stands_behind(ont, TKind::Guardrail, &c.id) { Light::Green } else { Light::None };
        guard_light.insert(c.id.clone(), (light_of(&rs, base), rs));
    }
    let guards_of = |name: &str| -> Vec<String> {
        ont.constraints
            .values()
            .filter(|c| c.applies_to == name || ont.relationships.iter().any(|r| (c.applies_to == r.name || c.applies_to == r.qualified()) && (r.from == name || r.to == name)))
            .map(|c| c.id.clone())
            .collect()
    };

    // the W106 of each entity, found once (not once per source and entity)
    let mut w106: HashMap<&str, Vec<&Diag>> = HashMap::new();
    for d in inp.report.diags.iter().filter(|d| d.code == "W106") {
        if let Some(sj) = d.subject.as_ref().filter(|x| x.kind == "entity") {
            w106.entry(sj.name.as_str()).or_default().push(d);
        }
    }
    // --- sources ---
    let mut source_json: Vec<Json> = Vec::new();
    let mut source_light: HashMap<String, (Light, Vec<Reason>)> = HashMap::new();
    for s in ont.sources.values() {
        let mut rs = take(&mut own, "source", &s.connection);
        let ents: Vec<String> = ont.types.values().filter(|t| t.source.as_ref().map(|b| b.connection == s.connection).unwrap_or(false)).map(|t| t.name.clone()).collect();
        for e in &ents {
            for d in w106.get(e.as_str()).cloned().unwrap_or_default() {
                if let Some(r) = diag_reason(d, ont) {
                    rs.push(Reason { text: format!("{} (for {})", r.text, words::label(e)), ..r });
                }
            }
        }
        let metrics: Vec<String> = ont.metrics.values().filter(|m| sources_of(m).iter().any(|x| x.connection == s.connection)).map(|m| m.name.clone()).collect();
        let light = light_of(&rs, Light::None);
        source_json.push(json!({
            "name": s.connection, "kind": s.kind, "words": source_words(&s.kind), "icon": source_icon(&s.kind),
            "description": s.description, "entities": ents, "metrics": metrics,
            "light": light.word(), "reasons": rs.iter().map(Reason::to_json).collect::<Vec<_>>(),
        }));
        source_light.insert(s.connection.clone(), (light, rs));
    }

    // --- what changed this week ---
    let recent_day = |at: &str| days_ago(at, inp.today).map(|d| (0..7).contains(&d)).unwrap_or(false);
    let changed_of = |t: &crate::schema::TypeDef| -> Option<String> {
        if let Some(g) = inp.git_changed {
            return g.get(&t.file).cloned();
        }
        let mut days: Vec<String> = Vec::new();
        if let Some(r) = ont.grade_of(TKind::Binding, &t.name).record {
            days.push(r.at.clone());
        }
        for p in inp.proposals.iter().filter(|p| p.status == "accepted") {
            let names_it = p.learned.as_ref().map(|l| l.entity == t.name).unwrap_or(false) || p.parts().iter().any(|(f, _, _)| *f == t.file);
            if names_it {
                days.push(p.resolved_at.get(..10).unwrap_or("").to_string());
            }
        }
        days.into_iter().filter(|d| recent_day(d)).max()
    };

    // --- entities ---
    let mut entity_json: Vec<Json> = Vec::new();
    let mut lights: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut area_of: Vec<(String, Light)> = Vec::new();
    let labels: HashMap<String, usize> = ont.types.values().fold(HashMap::new(), |mut m, t| {
        *m.entry(words::label(&t.name).to_lowercase()).or_insert(0) += 1;
        m
    });
    // each entity's area, from the manifest's lists once (not a walk of every list per entity)
    let mut listed: HashMap<&str, &str> = HashMap::new();
    for d in &ont.manifest.domains {
        for t in &d.types {
            listed.entry(t.as_str()).or_insert(d.name.as_str());
        }
    }
    // links in or out of each entity, counted once (not once per entity)
    let mut link_count: HashMap<&str, usize> = HashMap::new();
    for r in &ont.relationships {
        *link_count.entry(r.from.as_str()).or_insert(0) += 1;
        if r.to != r.from {
            *link_count.entry(r.to.as_str()).or_insert(0) += 1;
        }
    }
    for t in ont.types.values() {
        let mut rs = take(&mut own, "entity", &t.name);
        let undescribed = t.properties.values().filter(|p| p.normalize().description.trim().is_empty()).count();
        if undescribed > 0 {
            rs.push(Reason::new(Light::Amber, "fields", format!("{} {} no plain words", review::plural(undescribed, "field"), if undescribed == 1 { "has" } else { "have" })));
        }
        if t.source.is_some() {
            rs.extend(grade_reasons(ont, TKind::Binding, &t.name).into_iter().map(|r| Reason { text: format!("its source is {}", r.text), ..r }));
        }
        rs.extend(waiting_reason(&touched, "entity", &t.name));
        let source_conn = t.source.as_ref().map(|s| s.connection.clone());
        if let Some((l, srs)) = source_conn.as_ref().and_then(|c| source_light.get(c)) {
            if *l == Light::Red {
                for r in srs.iter().filter(|r| r.light == Light::Red) {
                    rs.push(Reason { from: Some(format!("source {}", source_conn.as_deref().unwrap_or(""))), ..r.clone() });
                }
            }
        }
        let metrics = counted_by.get(&t.name).cloned().unwrap_or_default();
        for m in &metrics {
            if let Some((l, mrs)) = metric_light.get(m) {
                if *l >= Light::Amber {
                    for r in mrs.iter().filter(|r| r.light >= Light::Amber) {
                        rs.push(Reason { from: Some(format!("metric {}", m)), text: format!("{}: {}", words::label(m), r.text), ..r.clone() });
                    }
                }
            }
        }
        let guards = guards_of(&t.name);
        for g in &guards {
            if let Some((l, grs)) = guard_light.get(g) {
                if *l >= Light::Amber {
                    for r in grs.iter().filter(|r| r.light >= Light::Amber) {
                        rs.push(Reason { from: Some(format!("guardrail {}", g)), text: format!("Guardrail {}: {}", g, r.text), ..r.clone() });
                    }
                }
            }
        }
        let base = if t.source.is_some() && stands_behind(ont, TKind::Binding, &t.name) { Light::Green } else { Light::None };
        let light = light_of(&rs, base);
        *lights.entry(light.word()).or_insert(0) += 1;
        let links = link_count.get(t.name.as_str()).copied().unwrap_or(0);
        let changed = changed_of(t);
        let has_data = t.source.is_some() || !t.instances.is_empty();
        let (score, why) = importance(&Signals { metrics: metrics.len(), links, light, waiting: touched.contains_key(&("entity".to_string(), t.name.clone())), has_data, changed: changed.is_some() });
        let source_kind = source_conn.as_ref().and_then(|c| ont.sources.get(c)).map(|s| s.kind.clone());
        let (icon, icon_why) = icon_for(&t.name, source_kind.as_deref());
        let units: Vec<String> = t.properties.values().filter_map(|p| p.normalize().unit.clone()).filter(|u| !u.trim().is_empty()).fold(Vec::new(), |mut v, u| {
            if !v.contains(&u) {
                v.push(u);
            }
            v
        });
        let area = t.domain.clone().or_else(|| listed.get(t.name.as_str()).map(|d| d.to_string())).unwrap_or_else(|| "Other".into());
        area_of.push((area.clone(), light));
        let src = t.source.as_ref().map(|s| {
            let st = ont.grade_of(TKind::Binding, &t.name);
            let mut j = json!({
                "name": s.connection, "kind": source_kind, "table": s.table,
                "grade": if st.changed { "changed".to_string() } else { st.word().to_string() },
            });
            if let Some(r) = &st.record {
                j["by"] = json!(trust::who_said(&r.by));
            }
            j
        });
        let label = words::label(&t.name);
        // what is empty or zero is left out: a space of thousands stays small
        let mut j = json!({
            "name": t.name, "label": label, "area": area, "icon": icon, "icon_why": icon_why,
            "fields": t.properties.len(), "described": t.properties.len() - undescribed,
            "light": light.word(), "importance": score, "importance_why": why,
        });
        let short: String = t.description.chars().take(160).collect();
        if !t.description.trim().is_empty() {
            j["description"] = json!(if short.len() < t.description.len() { format!("{}…", short.trim_end()) } else { short });
        }
        if let Some(k) = t.key_property() {
            j["key"] = json!(k);
        }
        if !units.is_empty() {
            j["units"] = json!(units);
        }
        if let Some(sv) = src {
            j["source"] = sv;
        }
        if has_data {
            j["has_data"] = json!(true);
        }
        if labels.get(&label.to_lowercase()).copied().unwrap_or(0) > 1 {
            j["same_label"] = json!(true);
        }
        if !metrics.is_empty() {
            j["metrics"] = json!(metrics);
        }
        if !guards.is_empty() {
            j["guardrails"] = json!(guards.len());
            j["guardrail_names"] = json!(guards.iter().take(3).collect::<Vec<_>>());
        }
        let actions = ont.actions.values().filter(|a| a.inputs.values().any(|ty| *ty == t.name)).count();
        if actions > 0 {
            j["actions"] = json!(actions);
        }
        let also: Vec<String> = ont.glossary.iter().filter(|g| g.means == t.name).map(|g| g.term.clone()).collect();
        if !also.is_empty() {
            j["also_called"] = json!(also);
        }
        if links > 0 {
            j["links"] = json!(links);
        }
        if !rs.is_empty() {
            j["reasons"] = json!(rs.iter().map(Reason::to_json).collect::<Vec<_>>());
        }
        if let Some(c) = changed {
            j["changed"] = json!(c);
        }
        if let Some((id, _)) = touched.get(&("entity".to_string(), t.name.clone())) {
            j["waiting"] = json!(id);
        }
        entity_json.push(j);
    }
    // highest first, ties by name: the order the layout and the levels use
    entity_json.sort_by(|a, b| b["importance"].as_i64().cmp(&a["importance"].as_i64()).then(a["name"].as_str().cmp(&b["name"].as_str())));

    // --- areas, in the manifest's order, then Other ---
    let mut area_names: Vec<String> = ont.manifest.domains.iter().map(|d| d.name.clone()).collect();
    for (a, _) in &area_of {
        if !area_names.contains(a) {
            area_names.push(a.clone());
        }
    }
    let areas: Vec<Json> = area_names
        .iter()
        .filter_map(|a| {
            let ls: Vec<Light> = area_of.iter().filter(|(x, _)| x == a).map(|(_, l)| *l).collect();
            if ls.is_empty() {
                return None;
            }
            let mut counts = serde_json::Map::new();
            for l in &ls {
                let e = counts.entry(l.word()).or_insert(json!(0));
                *e = json!(e.as_u64().unwrap_or(0) + 1);
            }
            Some(json!({ "name": a, "entities": ls.len(), "light": ls.iter().max().unwrap_or(&Light::None).word(), "lights": counts }))
        })
        .collect();

    // --- links, as rows: [from, to, name, cardinality, has a sentence]. From and to are places in
    // `entities`; a link to an entity that does not exist carries the missing name instead ---
    let index: HashMap<&str, usize> = entity_json.iter().enumerate().filter_map(|(i, e)| e["name"].as_str().map(|n| (n, i))).collect();
    let links: Vec<Json> = ont
        .relationships
        .iter()
        .filter_map(|r| {
            let from = *index.get(r.from.as_str())?;
            let to = index.get(r.to.as_str()).map(|i| json!(i)).unwrap_or_else(|| json!(r.to));
            Some(json!([from, to, r.name, r.cardinality, !r.description.trim().is_empty()]))
        })
        .collect();

    // --- agents ---
    let mut agents: Vec<Json> = Vec::new();
    let waiting_of = |name: &str, key: Option<&str>| -> usize {
        pending.iter().filter(|p| { let a = review::agent_name(&p.via); if a != "An agent" { a == name } else { key == Some(&format!("activity:{}", p.via.trim())[..]) } }).count()
    };
    let recent_of = |name: Option<&str>, via: Option<&str>| -> Vec<Json> {
        let mut out: Vec<Json> = Vec::new();
        let mut add = |entity: String, how: &str| {
            if ont.types.contains_key(&entity) && !out.iter().any(|x| x["entity"] == entity.as_str()) {
                out.push(json!({ "entity": entity, "how": how }));
            }
        };
        for p in inp.proposals.iter().filter(|p| recent_day(&p.created_at)) {
            let mine = match name {
                Some(n) => review::agent_name(&p.via) == n,
                None => via.map(|v| p.via.trim() == v).unwrap_or(false),
            };
            let how = match p.status.as_str() {
                "accepted" => "saved",
                "pending" => "proposed",
                _ => continue,
            };
            if !mine {
                continue;
            }
            if let Some(l) = &p.learned {
                add(l.entity.clone(), how);
            }
            for (f, _, _) in p.parts() {
                if let Some(t) = ont.types.values().find(|t| t.file == f) {
                    add(t.name.clone(), how);
                }
            }
        }
        for a in inp.activity.iter().filter(|a| recent_day(a["at"].as_str().unwrap_or(""))) {
            let who = a["who"].as_str().unwrap_or("");
            let mine = match name {
                Some(n) => who.contains(&format!("connection \"{}\"", n)),
                None => via.map(|v| who.trim() == v).unwrap_or(false),
            };
            if mine {
                if let Some(t) = ont.types.values().find(|t| a["file"].as_str() == Some(t.file.as_str())) {
                    add(t.name.clone(), "saved");
                }
            }
        }
        out
    };
    let mut seen_names: Vec<String> = Vec::new();
    for c in inp.connections.iter().filter(|c| c["space"].is_null() || c["space"].as_str() == Some(inp.space.id)) {
        let name = c["name"].as_str().unwrap_or("").to_string();
        let shown = cut(&name, 40);
        let w = waiting_of(&name, None);
        seen_names.push(name.clone());
        agents.push(json!({
            "id": c["id"], "name": shown, "monogram": monogram(&name), "seen": "connection",
            "last_used": c["last_used"], "approved": c["approved"], "writes": c["writes"], "scopes": c["scopes"],
            "waiting": w, "light": if w > 0 { "amber" } else { "none" },
            "recent": recent_of(Some(&name), None),
        }));
    }
    let mut extra: Vec<(String, String, String)> = Vec::new(); // (key, name, via)
    let vias = inp.activity.iter().filter(|a| days_ago(a["at"].as_str().unwrap_or(""), inp.today).map(|d| d < 30).unwrap_or(false)).map(|a| a["who"].as_str().unwrap_or("").to_string()).chain(inp.proposals.iter().filter(|p| days_ago(&p.created_at, inp.today).map(|d| d < 30).unwrap_or(false)).map(|p| p.via.clone()));
    for via in vias {
        let conn = review::agent_name(&via);
        if conn != "An agent" {
            // a connection that has since gone, or one bound elsewhere, shows only in activity
            continue;
        }
        if let Some((key, name)) = activity_agent(&via) {
            if !extra.iter().any(|(k, _, _)| *k == key) {
                extra.push((key, name, via.trim().to_string()));
            }
        }
    }
    for (key, name, via) in extra {
        let w = waiting_of("", Some(&key));
        agents.push(json!({
            "id": key, "name": name, "monogram": "A", "seen": "activity", "last_used": Json::Null,
            "waiting": w, "light": if w > 0 { "amber" } else { "none" }, "recent": recent_of(None, Some(&via)),
        }));
    }

    // --- the space itself ---
    let empty = ont.types.is_empty();
    let mut c = serde_json::Map::new();
    for (k, v) in &lights {
        c.insert(k.to_string(), json!(v));
    }
    for k in ["red", "amber", "green", "none"] {
        c.entry(k.to_string()).or_insert(json!(0));
    }
    json!({
        "version": inp.version,
        "space": {
            "id": inp.space.id, "name": inp.space.name, "builtin": inp.space.builtin, "editable": inp.space.editable,
            "ok": inp.report.ok(), "empty": empty, "light": light_of(&space_reasons, Light::None).word(),
            "reasons": space_reasons.iter().map(Reason::to_json).collect::<Vec<_>>(),
        },
        "counts": { "entities": ont.types.len(), "metrics": ont.metrics.len(), "sources": ont.sources.len(), "agents": agents.len(), "waiting": pending.len(), "lights": c },
        "areas": areas,
        "entities": entity_json,
        "links": links,
        "metrics": metric_json,
        "sources": source_json,
        "agents": agents,
        "waiting": waiting,
        "waiting_more": pending.len().saturating_sub(5),
    })
}

/// What `?version=` compares: the space's files, the proposals folder, the activity log and when
/// each connection was last used (to the minute). Unchanged means nothing on the picture moved.
pub fn version(dir: &std::path::Path, fp: &impl std::fmt::Debug, connections: &[Json]) -> String {
    let mut s = format!("{:?}", fp);
    let lf = dir.join(".lorefile");
    if let Ok(rd) = std::fs::read_dir(lf.join("proposals")) {
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| format!("{}:{:?}", e.file_name().to_string_lossy(), e.metadata().and_then(|m| m.modified()).ok()))
            .collect();
        names.sort();
        s.push_str(&names.join(","));
    }
    s.push_str(&format!("|{}", std::fs::metadata(lf.join("activity.jsonl")).map(|m| m.len()).unwrap_or(0)));
    for c in connections {
        s.push_str(&format!("|{}:{}", c["id"].as_str().unwrap_or(""), c["last_used"].as_str().unwrap_or("").get(..16).unwrap_or("")));
    }
    trust::fingerprint(&json!(s))
}

/// The files committed in the last 7 days, each with the day of its newest commit: one `git log`.
pub fn git_changed(dir: &std::path::Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(o) = std::process::Command::new("git").arg("-C").arg(dir).args(["log", "--since=7.days", "--name-only", "--format=%ct", "--relative"]).output() else { return out };
    let text = String::from_utf8_lossy(&o.stdout);
    let mut day = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Ok(ts) = line.parse::<i64>() {
            day = chrono::DateTime::from_timestamp(ts, 0).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
        } else if !out.contains_key(line) {
            out.insert(line.to_string(), day.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_come_from_the_first_word_that_matches() {
        assert_eq!(icon_for("PromoSale", None), ("receipt", "name: sale".to_string()));
        assert_eq!(icon_for("Customer", None).0, "person");
        assert_eq!(icon_for("Warehouse", None).0, "pin");
        assert_eq!(icon_for("Carrier", None).0, "building");
        assert_eq!(icon_for("Thing", None).0, "cube");
        assert_eq!(icon_for("CampaignOrder", None).0, "receipt", "the table's order decides, not the name's");
        assert_eq!(icon_for("Widget", Some("from_user")), ("document", "source: files people give agents".to_string()));
        assert_eq!(icon_for("Orders", None).0, "receipt", "a plural counts");
        assert_eq!(source_icon("from_user"), "paperclip");
        assert_eq!(source_icon("bigquery"), "cylinder");
    }

    #[test]
    fn identifiers_leave_out_sql_words_and_strings() {
        assert_eq!(identifiers("count(distinct order_id)"), vec!["order_id"]);
        assert_eq!(identifiers("status = 'paid' and refunded = false"), vec!["status", "refunded"]);
    }

    #[test]
    fn importance_adds_up_with_reasons() {
        let (score, why) = importance(&Signals { metrics: 1, links: 0, light: Light::Amber, waiting: false, has_data: true, changed: true });
        assert_eq!(score, 10);
        assert_eq!(why, vec!["counted by a metric (+4)", "needs a look (+3)", "has data (+1)", "changed this week (+2)"]);
        assert_eq!(importance(&Signals { metrics: 0, links: 9, light: Light::None, waiting: false, has_data: false, changed: false }).0, 6, "links count up to 6");
    }

    #[test]
    fn monograms_are_a_letter_never_a_logo() {
        assert_eq!(monogram("ChatGPT"), "C");
        assert_eq!(monogram("  9lives"), "9");
        assert_eq!(monogram("—"), "A");
    }

    fn space(tag: &str, files: &[(&str, &str)]) -> (std::path::PathBuf, Ontology) {
        let d = std::env::temp_dir().join(format!("lorefile-plane-unit-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (rel, text) in files {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let ont = Ontology::from_raw(crate::loader::load(&d).unwrap());
        (d, ont)
    }

    const TWO: [(&str, &str); 4] = [
        ("ontology.yaml", "name: t\n"),
        ("sources/wh.yaml", "connection: wh\nkind: bigquery\ndescription: x\n"),
        ("types/order.yaml", "type: Order\ndescription: o\nproperties: { order_id: { type: string, key: true, description: id }, total: { type: number, description: t }, channel: { type: string, description: c } }\nsource: { connection: wh, table: orders, key: order_id }\n"),
        ("types/payout.yaml", "type: Payout\ndescription: p\nproperties: { payout_id: { type: string, key: true, description: id }, commission: { type: number, description: c } }\nsource: { connection: wh, table: payouts, key: payout_id }\n"),
    ];

    #[test]
    fn metrics_count_the_entities_their_sources_bind() {
        let mut files = TWO.to_vec();
        files.push(("logic/m.yaml", "metrics:\n  - metric: orders\n    description: o\n    owner: x\n    source: { connection: wh, table: orders, measure: count(distinct order_id) }\n  - metric: earned\n    description: e\n    owner: x\n    variants:\n      - { when: channel = 'shop', source: { connection: wh, table: orders, measure: sum(total) } }\n      - { when: channel = 'affiliate', source: { connection: wh, table: payouts, measure: sum(commission) } }\n  - metric: margin\n    description: m\n    owner: x\n    sql: select 1\n"));
        let (d, ont) = space("metrics", &files);
        let m = |n: &str| ont.metrics.get(n).unwrap();
        assert_eq!(metric_entities(m("orders"), &ont), vec!["Order"]);
        assert_eq!(metric_uses(m("orders"), &ont, &["Order".to_string()]), vec![("order_id".to_string(), "counts")]);
        assert_eq!(metric_entities(m("earned"), &ont), vec!["Order", "Payout"], "a variant on each source: both");
        assert!(metric_entities(m("margin"), &ont).is_empty(), "SQL only: tied to no entity");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn agents_are_the_connections_that_reach_this_space_and_channels_seen_in_activity() {
        let (d, ont) = space("agents", &TWO);
        let report = crate::validate::validate(&ont);
        let conns = vec![
            json!({ "id": "c1", "name": "ChatGPT", "space": null, "last_used": "2026-11-27T10:00:00Z", "writes": "wait", "scopes": ["lorefile:read"] }),
            json!({ "id": "c2", "name": "Codex", "space": "here", "last_used": null, "writes": "wait" }),
            json!({ "id": "c3", "name": "Elsewhere", "space": "other", "last_used": null, "writes": "wait" }),
            json!({ "id": "c4", "name": "<img src=x onerror=alert(1)>", "space": null }),
        ];
        let activity = vec![
            json!({ "at": "2026-11-27T09:00:00Z", "who": "agent via MCP (connection \"ChatGPT\", approved 27 Nov)", "text": "saved", "file": "types/order.yaml" }),
            json!({ "at": "2026-11-26T09:00:00Z", "who": "agent via MCP", "text": "saved", "file": "types/payout.yaml" }),
            json!({ "at": "2026-11-26T09:00:00Z", "who": "Vinay via page", "text": "accepted", "file": "trust.yaml" }),
        ];
        let inp = Input {
            ont: &ont, report: &report, space: SpaceInfo { id: "here", name: "here", builtin: false, editable: true },
            proposals: &[], activity: &activity, connections: &conns, git_changed: None,
            today: chrono::NaiveDate::from_ymd_opt(2026, 11, 27).unwrap(), version: "v".into(),
        };
        let p = plane(&inp, &mut |_| json!({}));
        let names: Vec<&str> = p["agents"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["ChatGPT", "Codex", "<img src=x onerror=alert(1)>", "an agent (MCP)"], "bound elsewhere: not shown; a person: never an agent");
        let chat = &p["agents"][0];
        assert_eq!((chat["monogram"].as_str(), chat["seen"].as_str()), (Some("C"), Some("connection")));
        assert_eq!(chat["recent"], json!([{ "entity": "Order", "how": "saved" }]));
        let stdio = &p["agents"][3];
        assert_eq!((stdio["seen"].as_str(), stdio["recent"][0]["entity"].as_str()), (Some("activity"), Some("Payout")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_worst_light_wins() {
        assert_eq!(light_of(&[], Light::Green), Light::Green);
        assert_eq!(light_of(&[Reason::new(Light::Amber, "W109", "x")], Light::Green), Light::Amber);
        assert_eq!(light_of(&[Reason::new(Light::Amber, "a", "x"), Reason::new(Light::Red, "E", "y")], Light::None), Light::Red);
    }
}
