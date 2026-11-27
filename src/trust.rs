//! Evidence grades: who stands behind each definition. One file per space,
//! `trust.yaml`, written by Lorefile only, one line per graded thing:
//!
//! ```yaml
//! trust:
//!   metrics:
//!     success: { grade: checked, by: "Priya via page", at: 2026-11-28, of: 3f9a1c2e70b4, against: shop_bigquery, note: "…" }
//! ```
//!
//! Three grades, lowest first: proposed (an agent wrote it, no person has
//! looked), accepted (a person stands behind the words), checked (a person ran
//! it against real rows). No entry is ungraded. Every entry carries `of`, a
//! fingerprint of the thing as compiled, so a change made behind the grade's
//! back (a hand edit, a merge, another tool) makes the grade stop counting.
//!
//! A grade moves up only by a person's click and down by itself when the
//! definition changes. This module holds the rules; `edit` writes the file in
//! the same commit as the facts, and `mcp`, the CLI and the page read the
//! sentences from here so they all say the same thing.

use crate::ontology::Ontology;
use crate::schema::{Grade, TrustBook, TrustRecord};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Map, Value as Json};
use std::collections::HashMap;

/// The file's name, at the root of the space.
pub const FILE: &str = "trust.yaml";

/// What an agent or the page's raw editor is told when it tries to write the file.
pub const REFUSED: &str = "trust.yaml is written by Lorefile when a person accepts or checks something";

/// The kinds that carry a grade in the first version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Metric,
    Binding,
    Term,
    Guardrail,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Metric, Kind::Binding, Kind::Term, Kind::Guardrail];

    /// The mapping's key in `trust.yaml`.
    pub fn key(&self) -> &'static str {
        match self {
            Kind::Metric => "metrics",
            Kind::Binding => "bindings",
            Kind::Term => "terms",
            Kind::Guardrail => "guardrails",
        }
    }

    /// The word a person reads: "metric success", "word basket".
    pub fn word(&self) -> &'static str {
        match self {
            Kind::Metric => "metric",
            Kind::Binding => "data binding",
            Kind::Term => "word",
            Kind::Guardrail => "guardrail",
        }
    }

    /// What the thing is, in a sentence: "this definition", "this choice of source".
    fn noun(&self) -> &'static str {
        match self {
            Kind::Metric => "definition",
            Kind::Binding => "choice of source",
            Kind::Term => "also-called word",
            Kind::Guardrail => "guardrail",
        }
    }

    /// The kind and the name in the page's words: "metric promo_sale_revenue", "source of
    /// PromoSale", "also-called word \"promo line\"", "guardrail RET-001".
    pub fn name_words(&self, name: &str) -> String {
        match self {
            Kind::Metric => format!("metric {}", name),
            Kind::Binding => format!("source of {}", name),
            Kind::Term => format!("also-called word \"{}\"", name),
            Kind::Guardrail => format!("guardrail {}", name),
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "metric" | "metrics" => Some(Kind::Metric),
            "binding" | "bindings" | "data binding" | "source" => Some(Kind::Binding),
            "term" | "terms" | "word" | "words" | "glossary" => Some(Kind::Term),
            "guardrail" | "guardrails" | "rule" | "rules" | "constraint" | "constraints" => Some(Kind::Guardrail),
            _ => None,
        }
    }

    /// A word has no rows to be checked against.
    pub fn can_be_checked(&self) -> bool {
        *self != Kind::Term
    }
}

pub fn entries(book: &TrustBook, k: Kind) -> &IndexMap<String, TrustRecord> {
    match k {
        Kind::Metric => &book.metrics,
        Kind::Binding => &book.bindings,
        Kind::Term => &book.terms,
        Kind::Guardrail => &book.guardrails,
    }
}

pub fn entries_mut(book: &mut TrustBook, k: Kind) -> &mut IndexMap<String, TrustRecord> {
    match k {
        Kind::Metric => &mut book.metrics,
        Kind::Binding => &mut book.bindings,
        Kind::Term => &mut book.terms,
        Kind::Guardrail => &mut book.guardrails,
    }
}

// ---------------------------------------------------------------------------
// The fingerprint
// ---------------------------------------------------------------------------

/// FNV-1a, 64 bits: written here so the value never changes between Rust
/// versions (the standard library's hasher may).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// JSON with the keys of every object sorted, so key order never matters.
fn canonical(v: &Json, out: &mut String) {
    match v {
        Json::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                canonical(&m[*k], out);
            }
            out.push('}');
        }
        Json::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// The first 12 hex characters of the hash of the value's canonical JSON.
pub fn fingerprint(v: &Json) -> String {
    let mut s = String::new();
    canonical(v, &mut s);
    format!("{:016x}", fnv1a(s.as_bytes()))[..12].to_string()
}

/// The key under which a metric's or binding's fingerprint carries the
/// definitions of the sources it reads (never written to a file).
const READS: &str = "reads";

/// What a person typed as a check's note, made safe to keep: control
/// characters become spaces (YAML refuses them) and it is kept to a few lines.
pub fn clean_note(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > NOTE_MAX {
        let cut: String = flat.chars().take(NOTE_MAX).collect();
        format!("{}…", cut.trim_end())
    } else {
        flat
    }
}

/// The longest note a check keeps.
pub const NOTE_MAX: usize = 500;

fn without(mut v: Json, keys: &[&str]) -> Json {
    if let Some(m) = v.as_object_mut() {
        for k in keys {
            m.remove(*k);
        }
    }
    v
}

/// Everything that can carry a grade of one kind, as compiled: name → value.
/// Loader-set fields (`file`) are left out, so moving a thing to another
/// file is not a change.
pub fn things(ont: &Ontology, k: Kind) -> IndexMap<String, Json> {
    let mut out = IndexMap::new();
    // The sources a metric or binding reads are part of what was graded: the
    // same recipe over another project, dataset or MCP server is another
    // number. Each connection's definition rides along (its words left out),
    // so re-pointing a source stops the grades of everything that reads it.
    let reads = |conns: Vec<&str>| -> Json {
        let mut m = Map::new();
        for c in conns {
            if let Some(s) = ont.sources.get(c) {
                m.insert(c.to_string(), without(serde_json::to_value(s).unwrap_or(Json::Null), &["file", "description"]));
            }
        }
        Json::Object(m)
    };
    match k {
        Kind::Metric => {
            for m in ont.metrics.values() {
                let mut v = without(serde_json::to_value(m).unwrap_or(Json::Null), &["file"]);
                let conns: Vec<&str> = m.source.iter().map(|s| s.connection.as_str()).chain(m.variants.iter().map(|x| x.source.connection.as_str())).collect();
                if !conns.is_empty() {
                    v[READS] = reads(conns);
                }
                out.insert(m.name.clone(), v);
            }
        }
        Kind::Binding => {
            // the entity's name, its key field and its `source:` block: a field
            // description added to the entity does not disturb the binding's grade
            for t in ont.types.values() {
                if let Some(s) = &t.source {
                    out.insert(t.name.clone(), json!({ "entity": t.name, "key": t.key_property(), "source": s, READS: reads(vec![s.connection.as_str()]) }));
                } else if t.is_where_kind() {
                    // a kind that picks its parent's rows: graded on its own, and re-pointing the
                    // parent's source changes what was graded, so it reads "changed since" too
                    let base = t.rows_of.as_ref().and_then(|b| ont.types.get(b));
                    let src = base.and_then(|b| b.source.clone());
                    let conns: Vec<&str> = src.as_ref().map(|s| vec![s.connection.as_str()]).unwrap_or_default();
                    out.insert(t.name.clone(), json!({ "entity": t.name, "key": t.key_property(), "extends": t.parent, "where": t.where_all, "rows_of": t.rows_of, "source": src, READS: reads(conns) }));
                }
            }
        }
        Kind::Term => {
            // a word written twice (two meanings, two files) is graded as one
            // thing: every entry counts, so adding a second meaning is a change
            for g in &ont.glossary {
                let v = serde_json::to_value(g).unwrap_or(Json::Null);
                match out.get_mut(&g.term) {
                    None => {
                        out.insert(g.term.clone(), v);
                    }
                    Some(Json::Array(a)) => a.push(v),
                    Some(first) => {
                        let one = first.take();
                        *first = json!([one, v]);
                    }
                }
            }
        }
        Kind::Guardrail => {
            for c in ont.constraints.values() {
                out.insert(c.id.clone(), without(serde_json::to_value(c).unwrap_or(Json::Null), &["file"]));
            }
        }
    }
    out
}

/// The part of a thing a check depends on: its words (description,
/// rationale, caveats, owner, tags) left out. A person changing only these
/// keeps a checked grade; changing the rest makes it accepted.
fn recipe(k: Kind, v: &Json) -> Json {
    match k {
        Kind::Metric => {
            let mut r = without(v.clone(), &["description", "caveats", "owner", "tags"]);
            if let Some(vs) = r.get_mut("variants").and_then(|x| x.as_array_mut()) {
                for x in vs.iter_mut() {
                    if let Some(m) = x.as_object_mut() {
                        m.remove("description");
                    }
                }
            }
            r
        }
        Kind::Guardrail => without(v.clone(), &["name", "description", "rationale", "owner", "tags"]),
        Kind::Term => without(v.clone(), &["description"]),
        Kind::Binding => v.clone(),
    }
}

// ---------------------------------------------------------------------------
// Resolved grades
// ---------------------------------------------------------------------------

/// What a thing's grade is right now.
#[derive(Debug, Clone, Default)]
pub struct Status {
    /// The grade that counts; `None` is ungraded (no entry, or changed since).
    pub grade: Option<Grade>,
    /// The entry in `trust.yaml`, if any.
    pub record: Option<TrustRecord>,
    /// There is an entry, but the thing no longer matches its fingerprint.
    pub changed: bool,
}

/// "24 Nov" from "2026-11-24"; anything else as it is.
pub fn short_date(at: &str) -> String {
    chrono::NaiveDate::parse_from_str(at.trim(), "%Y-%m-%d").map(|d| d.format("%-d %b").to_string()).unwrap_or_else(|_| at.to_string())
}

/// "Priya" from "Priya via page"; "a person" from "person via page"; "an agent" from "agent via MCP".
pub fn who_said(by: &str) -> String {
    let by = by.trim();
    let name = by.split(" via ").next().unwrap_or(by).trim();
    match name.to_ascii_lowercase().as_str() {
        "" | "person" => "a person".into(),
        "agent" => "an agent".into(),
        _ => name.to_string(),
    }
}

impl Status {
    pub fn word(&self) -> &'static str {
        self.grade.map(|g| g.word()).unwrap_or("ungraded")
    }

    /// The sentence an agent can use as it is. `None` for an ungraded thing nobody ever graded.
    pub fn say(&self, k: Kind) -> Option<String> {
        let r = self.record.as_ref()?;
        let when = short_date(&r.at);
        let who = who_said(&r.by);
        if self.changed {
            return Some(format!(
                "Changed since it was {} on {} ({}); no person has reviewed the new version, so treat it as unverified.",
                r.grade.word(), when, who
            ));
        }
        Some(match r.grade {
            Grade::Proposed => format!("Unverified: an agent proposed this {} on {} and no person has reviewed it.", k.noun(), when),
            Grade::Accepted if k.can_be_checked() => format!("Accepted by {} on {}: a person stands behind this {}; nobody has recorded checking it against data.", who, when, k.noun()),
            Grade::Accepted => format!("Accepted by {} on {}: a person stands behind this {}.", who, when, k.noun()),
            Grade::Checked => format!("Checked against {} by {} on {}.", r.against.as_deref().unwrap_or("a source"), who, when),
        })
    }

    /// The `trust` object `describe` returns.
    pub fn to_json(&self, k: Kind) -> Json {
        let mut j = json!({ "grade": self.word() });
        if let Some(r) = &self.record {
            if self.changed {
                j["changed_since"] = json!(format!("{} {}", r.grade.word(), r.at));
                j["was_by"] = json!(r.by);
            } else {
                j["by"] = json!(r.by);
                j["at"] = json!(r.at);
                if let Some(a) = &r.against {
                    j["against"] = json!(a);
                }
                if let Some(n) = &r.note {
                    j["note"] = json!(n);
                }
            }
        }
        if let Some(s) = self.say(k) {
            j["say"] = json!(s);
        }
        j
    }

    /// The short mark `summary` puts on a line: only proposed and checked things get one.
    pub fn mark(&self) -> Option<String> {
        let r = self.record.as_ref()?;
        match self.grade? {
            Grade::Proposed => Some("UNVERIFIED: an agent proposed it, nobody has reviewed it".into()),
            Grade::Checked => Some(format!("checked {}{}", r.at, r.against.as_ref().map(|a| format!(" against {}", a)).unwrap_or_default())),
            Grade::Accepted => None,
        }
    }
}

/// Every graded thing's status, worked out once when the space loads.
#[derive(Debug, Clone, Default)]
pub struct Graded {
    /// The space has a `trust.yaml`.
    pub has_file: bool,
    map: HashMap<(Kind, String), Status>,
    /// Every thing that can carry a grade, in order, so counts and lists cover the ungraded too.
    pub all: Vec<(Kind, String)>,
    /// Entries naming something that does not exist (W114).
    pub orphans: Vec<(Kind, String, TrustRecord)>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Counts {
    pub checked: usize,
    pub accepted: usize,
    pub proposed: usize,
    pub ungraded: usize,
    /// Of the ungraded, how many were graded once and changed since.
    pub changed: usize,
}

impl Graded {
    pub fn get(&self, k: Kind, name: &str) -> Status {
        self.map.get(&(k, name.to_string())).cloned().unwrap_or_default()
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for (k, n) in &self.all {
            let s = self.get(*k, n);
            match s.grade {
                Some(Grade::Checked) => c.checked += 1,
                Some(Grade::Accepted) => c.accepted += 1,
                Some(Grade::Proposed) => c.proposed += 1,
                None => {
                    c.ungraded += 1;
                    if s.changed {
                        c.changed += 1;
                    }
                }
            }
        }
        c
    }

    /// One row per thing that can carry a grade, for the page and `GET /api/trust`.
    pub fn list(&self) -> Vec<Json> {
        self.all
            .iter()
            .map(|(k, n)| {
                let s = self.get(*k, n);
                let mut j = s.to_json(*k);
                j["kind"] = json!(k);
                j["name"] = json!(n);
                j
            })
            .collect()
    }

    /// The one Trust line at the top of `summary`.
    pub fn line(&self) -> String {
        let c = self.counts();
        let mut parts = vec![
            format!("{} checked against data", c.checked),
            format!("{} accepted by a person", c.accepted),
            format!("{} proposed by an agent and not reviewed", c.proposed),
            format!("{} ungraded", c.ungraded),
        ];
        if c.changed > 0 {
            parts.push(format!("({} of them changed since a person graded them)", c.changed));
        }
        format!(
            "Trust (metrics, data bindings, glossary words and guardrails): {}. Say \"unverified\" when you use anything marked UNVERIFIED below.",
            parts.join(" · ")
        )
    }
}

/// Work out every grade: an entry counts when the thing still matches its fingerprint.
pub fn resolve(ont: &Ontology) -> Graded {
    let mut g = Graded { has_file: ont.trust.is_some(), ..Default::default() };
    let empty = TrustBook::default();
    let book = ont.trust.as_ref().unwrap_or(&empty);
    for k in Kind::ALL {
        let things = things(ont, k);
        for (name, v) in &things {
            g.all.push((k, name.clone()));
            if let Some(r) = entries(book, k).get(name) {
                let changed = r.of.trim() != fingerprint(v);
                g.map.insert((k, name.clone()), Status { grade: if changed { None } else { Some(r.grade) }, record: Some(r.clone()), changed });
            }
        }
        for (name, r) in entries(book, k) {
            if !things.contains_key(name) {
                g.orphans.push((k, name.clone(), r.clone()));
            }
        }
    }
    g
}

// ---------------------------------------------------------------------------
// Stamping
// ---------------------------------------------------------------------------

/// Who is saving: the save gate gives an agent's write `proposed` and a person's `accepted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Agent,
    Person,
}

impl Who {
    /// From the words history uses: anything with "agent" in it is an agent.
    pub fn of(via: &str) -> Who {
        if via.to_ascii_lowercase().contains("agent") {
            Who::Agent
        } else {
            Who::Person
        }
    }
}

/// One change to `trust.yaml`.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Set(Kind, String, TrustRecord),
    Remove(Kind, String),
}

/// Today's UTC date, the `at` of a new grade.
pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// The entries a change makes: compare every graded kind before and after.
/// New or changed things get the grade for `who`; deleted ones lose their
/// entry; a person's words-only change to a checked thing keeps the check
/// with a fresh fingerprint. Things the change did not touch are left alone.
pub fn restamp(before: &Ontology, after: &Ontology, who: Who, by: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let at = today();
    for k in Kind::ALL {
        let was = things(before, k);
        let now = things(after, k);
        for (name, v) in &now {
            let of = fingerprint(v);
            let fresh = |grade: Grade| TrustRecord { grade, by: by.to_string(), at: at.clone(), of: of.clone(), against: None, note: None };
            match was.get(name) {
                Some(old) if fingerprint(old) == of => {}
                Some(old) => {
                    let st = before.graded.get(k, name);
                    let rec = match who {
                        Who::Agent => fresh(Grade::Proposed),
                        Who::Person => match (&st.grade, &st.record) {
                            // who stands behind a definition doesn't change because someone else named its owner
                            (Some(Grade::Checked | Grade::Accepted), Some(r)) if owner_only(old, v) => TrustRecord { of: of.clone(), ..r.clone() },
                            (Some(Grade::Checked), Some(r)) if fingerprint(&recipe(k, old)) == fingerprint(&recipe(k, v)) => TrustRecord { of: of.clone(), ..r.clone() },
                            _ => fresh(Grade::Accepted),
                        },
                    };
                    out.push(Entry::Set(k, name.clone(), rec));
                }
                None => out.push(Entry::Set(k, name.clone(), fresh(if who == Who::Agent { Grade::Proposed } else { Grade::Accepted }))),
            }
        }
        for name in was.keys() {
            if !now.contains_key(name) {
                out.push(Entry::Remove(k, name.clone()));
            }
        }
    }
    out
}

/// Two versions of a thing differ in their `owner` and nothing else.
pub fn owner_only(old: &Json, new: &Json) -> bool {
    fingerprint(&without(old.clone(), &["owner"])) == fingerprint(&without(new.clone(), &["owner"]))
}

/// The entries a rename or merge of a person makes (Part 1, rename mode): every item that changed
/// only in its owner and whose grade matched before keeps that record with a fresh fingerprint,
/// whatever the grade; items with no record get none; a record already changed since is left as it
/// is, so it stays changed since. A rename is a spelling fix, not a review.
pub fn restamp_keep(before: &Ontology, after: &Ontology) -> Vec<Entry> {
    let mut out = Vec::new();
    for k in Kind::ALL {
        let was = things(before, k);
        for (name, v) in &things(after, k) {
            let Some(old) = was.get(name) else { continue };
            let of = fingerprint(v);
            if fingerprint(old) == of || !owner_only(old, v) {
                continue;
            }
            let st = before.graded.get(k, name);
            if let (Some(_), Some(r)) = (st.grade, st.record) {
                out.push(Entry::Set(k, name.clone(), TrustRecord { of, ..r }));
            }
        }
    }
    out
}

/// Lay the entries over a book. Returns true when it changed.
pub fn apply(book: &mut TrustBook, list: &[Entry]) -> bool {
    let mut changed = false;
    for e in list {
        match e {
            Entry::Set(k, n, r) => {
                let m = entries_mut(book, *k);
                if m.get(n) != Some(r) {
                    m.insert(n.clone(), r.clone());
                    changed = true;
                }
            }
            Entry::Remove(k, n) => {
                if entries_mut(book, *k).shift_remove(n).is_some() {
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Accepted or checked things this change would alter or remove (decision 5:
/// an agent never overwrites them, its change waits for a person), each as a
/// sentence: "metric success is checked (Priya, 28 Nov)".
pub fn would_overwrite_graded(before: &Ontology, after: &Ontology) -> Vec<String> {
    let mut out = Vec::new();
    for k in Kind::ALL {
        let now = things(after, k);
        for (name, old) in things(before, k) {
            let st = before.graded.get(k, &name);
            let Some(grade) = st.grade.filter(|g| *g >= Grade::Accepted) else { continue };
            let touched = match now.get(&name) {
                None => true,
                Some(v) => fingerprint(v) != fingerprint(&old),
            };
            if touched {
                let r = st.record.unwrap_or_else(|| TrustRecord { grade, by: String::new(), at: String::new(), of: String::new(), against: None, note: None });
                out.push(format!("{} is {} ({}, {})", k.name_words(&name), grade.word(), who_said(&r.by), short_date(&r.at)));
            }
        }
    }
    out
}

/// The `by` words for a person: "Priya via page" when a usable name was
/// given, else "person via page". A name is what the person typed (there are
/// no accounts), so it is kept short and plain, and never says "agent".
pub fn by_words(name: Option<&str>, channel: &str) -> String {
    let n = name.unwrap_or("").trim();
    let ok = !n.is_empty()
        && n.chars().count() <= 60
        && n.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '\'' | '_'))
        && !n.to_lowercase().contains("agent")
        && !n.to_lowercase().split_whitespace().any(|w| w == "via");
    if ok {
        format!("{} via {}", n, channel)
    } else {
        format!("person via {}", channel)
    }
}

// ---------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------

/// A key or a name as YAML: plain when it is a simple word, quoted otherwise.
fn yaml_key(s: &str) -> String {
    let plain = s.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !matches!(s.to_ascii_lowercase().as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n" | "~");
    if plain {
        s.to_string()
    } else {
        serde_json::to_string(s).unwrap_or_default()
    }
}

/// The file's text: a header, then one line per graded thing (flow style, so
/// two people grading different things merge cleanly in git).
pub fn render(book: &TrustBook) -> String {
    let mut s = String::from(
        "# trust.yaml · who stands behind what in this space.\n# Written by Lorefile when a person accepts or checks something. Not edited by hand; agents cannot change it.\ntrust:\n",
    );
    let mut any = false;
    for k in Kind::ALL {
        let m = entries(book, k);
        if m.is_empty() {
            continue;
        }
        any = true;
        s.push_str(&format!("  {}:\n", k.key()));
        for (name, r) in m {
            let mut parts = vec![
                format!("grade: {}", r.grade.word()),
                format!("by: {}", serde_json::to_string(&r.by).unwrap_or_default()),
                format!("at: {}", serde_json::to_string(&r.at).unwrap_or_default()),
                format!("of: {}", serde_json::to_string(&r.of).unwrap_or_default()),
            ];
            if let Some(a) = &r.against {
                parts.push(format!("against: {}", serde_json::to_string(a).unwrap_or_default()));
            }
            if let Some(n) = &r.note {
                parts.push(format!("note: {}", serde_json::to_string(n).unwrap_or_default()));
            }
            s.push_str(&format!("    {}: {{ {} }}\n", yaml_key(name), parts.join(", ")));
        }
    }
    if !book.distinct.is_empty() {
        any = true;
        s.push_str("  distinct:\n");
        for d in &book.distinct {
            let q = |x: &str| serde_json::to_string(x).unwrap_or_default();
            s.push_str(&format!("    - {{ a: {}, b: {}, by: {}, at: {}, of_a: {}, of_b: {}, why: {} }}\n", q(&d.a), q(&d.b), q(&d.by), q(&d.at), q(&d.of_a), q(&d.of_b), q(&d.why)));
        }
    }
    if !any {
        s.push_str("  {}\n");
        // `trust:` followed by an indented `{}` is not valid; write the empty mapping inline
        s = s.replace("trust:\n  {}\n", "trust: {}\n");
    }
    s
}

/// The fingerprint of one side of a "different on purpose" pair: `metric:<name>` (as graded),
/// `entity:<Name>` (its name, own fields, source and `where`), `term:<word>=<Entity>`.
/// `None` when that side no longer exists.
pub fn pair_fingerprint(ont: &Ontology, key: &str) -> Option<String> {
    let (kind, name) = key.split_once(':')?;
    match kind {
        "metric" => things(ont, Kind::Metric).get(name).map(fingerprint),
        "entity" => {
            let t = ont.types.get(name)?;
            let own: Vec<Json> = t.own_properties().map(|(n, p)| { let p = p.normalize(); json!({ "name": n, "type": p.kind, "unit": p.unit, "key": p.key, "values": p.values }) }).collect();
            Some(fingerprint(&json!({ "name": t.name, "own": own, "source": t.source, "where": t.where_all })))
        }
        "term" => {
            let (w, means) = name.split_once('=')?;
            ont.glossary.iter().any(|g| g.term == w && g.means == means).then(|| fingerprint(&json!({ "term": w, "means": means })))
        }
        _ => None,
    }
}

/// One side of a pair in words: `metric:x` → "metric x", `entity:X` → "entity X",
/// `term:deal=X` → "\"deal\" for X".
pub fn side_words(k: &str) -> String {
    match k.split_once(':') {
        Some(("term", n)) => match n.split_once('=') {
            Some((w, e)) => format!("\"{}\" for {}", w, e),
            None => format!("\"{}\"", n),
        },
        Some((kind, n)) => format!("{} {}", kind, n),
        None => k.to_string(),
    }
}

/// The line that says a pair is different on purpose, either way round.
pub fn distinct_of<'a>(book: &'a TrustBook, a: &str, b: &str) -> Option<&'a crate::schema::Distinct> {
    book.distinct.iter().find(|d| (d.a == a && d.b == b) || (d.a == b && d.b == a))
}

/// Drop the "different on purpose" lines whose side no longer exists (like a W114 grade).
pub fn prune_distinct(book: &mut TrustBook, ont: &Ontology) {
    book.distinct.retain(|d| pair_fingerprint(ont, &d.a).is_some() && pair_fingerprint(ont, &d.b).is_some());
}

/// Parse a trust file's text. Empty text is an empty book.
pub fn parse(text: &str) -> Result<TrustBook, String> {
    if text.trim().is_empty() {
        return Ok(TrustBook::default());
    }
    serde_yaml::from_str::<crate::schema::TrustFile>(text).map(|f| f.trust).map_err(|e| format!("trust.yaml does not parse: {}", e))
}

/// True when a book holds no entry at all.
pub fn is_empty(book: &TrustBook) -> bool {
    Kind::ALL.iter().all(|k| entries(book, *k).is_empty())
}

/// Undo of a commit that touched `trust.yaml`: the file goes back with the
/// facts, entry by entry. `now` keeps what changed since; `theirs` (the
/// version before the undone commit) brings back what that commit replaced.
/// An entry both changed differently keeps `now`'s (the fingerprint check
/// flags it if it no longer fits). `None` (file absent) is an empty book.
pub fn merge_undo(now: Option<&str>, then: Option<&str>, was: Option<&str>) -> Option<String> {
    let book = |t: Option<&str>| t.map(parse).unwrap_or_else(|| Ok(TrustBook::default()));
    let (o, b, t) = (book(now).ok()?, book(then).ok()?, book(was).ok()?);
    let mut out = TrustBook::default();
    for k in Kind::ALL {
        let (om, bm, tm) = (entries(&o, k), entries(&b, k), entries(&t, k));
        let mut names: Vec<&String> = om.keys().collect();
        for n in tm.keys() {
            if !om.contains_key(n) {
                names.push(n);
            }
        }
        for n in names {
            let (ov, bv, tv) = (om.get(n), bm.get(n), tm.get(n));
            let pick = if ov == bv { tv } else { ov };
            if let Some(r) = pick {
                entries_mut(&mut out, k).insert(n.clone(), r.clone());
            }
        }
    }
    if is_empty(&out) && was.is_none() {
        return None;
    }
    Some(render(&out))
}

/// The sentence an Ossie export puts in `ai_context.instructions` of a metric
/// or dataset whose definition is proposed, where other tools' AI reads it.
/// Generated, so an import recognises it and drops it.
pub const AI_UNVERIFIED: &str = "Unverified: an agent proposed this definition and no person has reviewed it.";

/// Text from an `ai_context` with our generated sentence taken off its front.
pub fn without_ai_sentence(t: &str) -> String {
    let t = t.trim();
    t.strip_prefix(AI_UNVERIFIED).unwrap_or(t).trim().to_string()
}

/// The grades a file from outside states, read one line at a time: a line
/// that does not parse (an unknown grade, an extra key) is counted and said,
/// never silently dropped along with the rest.
pub fn claims_from_json(v: &Json, notes: &mut Vec<String>) -> TrustBook {
    let mut book = TrustBook::default();
    let mut unread: Vec<String> = Vec::new();
    match v.as_object() {
        Some(o) => {
            for (key, lines) in o {
                let kind = Kind::ALL.into_iter().find(|k| k.key() == key);
                match (kind, lines.as_object()) {
                    (Some(k), Some(m)) => {
                        for (name, rec) in m {
                            match serde_json::from_value::<TrustRecord>(rec.clone()) {
                                Ok(r) => {
                                    entries_mut(&mut book, k).insert(name.clone(), r);
                                }
                                Err(_) => unread.push(k.name_words(name)),
                            }
                        }
                    }
                    _ => unread.push(format!("`{}`", key)),
                }
            }
        }
        None if v.is_null() => {}
        None => unread.push("the trust block".into()),
    }
    if !unread.is_empty() {
        notes.push(format!("{} grade claim{} in the file could not be read and {} ignored: {}", unread.len(), if unread.len() == 1 { "" } else { "s" }, if unread.len() == 1 { "was" } else { "were" }, unread.join(", ")));
    }
    book
}

/// After an import compiled: imported facts start ungraded. Without
/// `keep_grades` what the file claimed is only listed; with it (a team
/// re-importing its own export) the grades whose fingerprint still matches
/// come back as `trust.yaml`, and the rest are dropped and listed. Returns the
/// file to write, if any.
pub fn after_import(ont: &Ontology, claimed: Option<&TrustBook>, keep_grades: bool, into: &std::path::Path, notes: &mut Vec<String>) -> Option<(String, String)> {
    let book = claimed.filter(|b| !is_empty(b))?;
    let counts: Vec<String> = claims(book).iter().map(|(g, n)| format!("{} {}", n, g)).collect();
    let total: u64 = claims(book).values().filter_map(|v| v.as_u64()).sum();
    if !keep_grades {
        notes.push(format!(
            "{} definition{} arrived with a grade ({}); they start ungraded here, because a grade is a statement by this team about this copy. Add --keep-grades when the file is your own export",
            total, if total == 1 { "" } else { "s" }, counts.join(", ")
        ));
        return None;
    }
    if into.join(FILE).exists() || into.join("trust.yml").exists() {
        notes.push(format!("{} grades in the file were not restored: {} already has a trust.yaml, which an import never overwrites", total, into.display()));
        return None;
    }
    let (kept, dropped) = keep_matching(ont, book);
    let n: usize = Kind::ALL.iter().map(|k| entries(&kept, *k).len()).sum();
    notes.push(format!("restored {} of {} grades with --keep-grades (each still matches what was graded)", n, total));
    for d in dropped {
        notes.push(format!("dropped grade: {}", d));
    }
    if n == 0 {
        return None;
    }
    Some((FILE.to_string(), render(&kept)))
}

/// Grades a file from outside claims, counted for an import's report.
pub fn claims(book: &TrustBook) -> Map<String, Json> {
    let mut m = Map::new();
    for k in Kind::ALL {
        for r in entries(book, k).values() {
            let e = m.entry(r.grade.word().to_string()).or_insert(json!(0));
            *e = json!(e.as_u64().unwrap_or(0) + 1);
        }
    }
    m
}

/// Keep only the grades whose fingerprint matches the imported thing (an
/// import with `--keep-grades`); the rest are dropped and listed.
pub fn keep_matching(ont: &Ontology, book: &TrustBook) -> (TrustBook, Vec<String>) {
    let mut kept = TrustBook::default();
    let mut dropped = Vec::new();
    for k in Kind::ALL {
        let things = things(ont, k);
        for (name, r) in entries(book, k) {
            let checked = r.grade == Grade::Checked;
            let bad = if checked && !k.can_be_checked() {
                Some("a word cannot be checked")
            } else if r.against.as_deref().map(|a| !a.trim().is_empty() && !ont.sources.contains_key(a)).unwrap_or(false) {
                Some("the source it was checked against is not in this space")
            } else if checked && (r.against.as_deref().unwrap_or("").trim().is_empty() || r.by.trim().is_empty() || r.at.trim().is_empty()) {
                Some("a check needs who, when and against which source")
            } else {
                None
            };
            if let Some(why) = bad {
                dropped.push(format!("{}: {} in the file, but {}; it starts ungraded", k.name_words(name), r.grade.word(), why));
                continue;
            }
            match things.get(name) {
                Some(v) if fingerprint(v) == r.of.trim() => {
                    entries_mut(&mut kept, k).insert(name.clone(), r.clone());
                }
                Some(_) => dropped.push(format!("{}: {} in the file, but the definition differs from what was graded; it starts ungraded", k.name_words(name), r.grade.word())),
                None => dropped.push(format!("{}: {} in the file, but there is no such thing here; dropped", k.name_words(name), r.grade.word())),
            }
        }
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_ignores_key_order_and_is_stable() {
        let a = json!({ "metric": "success", "source": { "table": "orders", "connection": "shop" } });
        let b = json!({ "source": { "connection": "shop", "table": "orders" }, "metric": "success" });
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_eq!(fingerprint(&a).len(), 12);
        // a fixed value: the hash is ours, so it may never move between Rust versions
        assert_eq!(fingerprint(&json!({})), format!("{:016x}", fnv1a(b"{}"))[..12].to_string());
        assert_eq!(&format!("{:016x}", fnv1a(b"")), "cbf29ce484222325");
        assert_ne!(fingerprint(&a), fingerprint(&json!({ "metric": "success", "source": { "connection": "shop", "table": "order" } })));
    }

    #[test]
    fn the_file_round_trips_and_merges_entry_by_entry() {
        let rec = |g: Grade, of: &str| TrustRecord { grade: g, by: "Priya via page".into(), at: "2026-11-28".into(), of: of.into(), against: None, note: None };
        let mut book = TrustBook::default();
        book.metrics.insert("success".into(), TrustRecord { against: Some("shop_bigquery".into()), note: Some("412 paid orders, \"same\" as Shopify".into()), ..rec(Grade::Checked, "3f9a1c2e70b4") });
        book.terms.insert("true".into(), rec(Grade::Accepted, "0c1d2e3f8a90"));
        let text = render(&book);
        assert!(text.contains("    success: { grade: checked, by: \"Priya via page\""), "{}", text);
        assert_eq!(parse(&text).unwrap(), book, "{}", text);
        assert_eq!(parse(&render(&TrustBook::default())).unwrap(), TrustBook::default());

        // undo of the check: then = checked, was = accepted, now = checked → accepted again
        let mut was = book.clone();
        was.metrics.insert("success".into(), rec(Grade::Accepted, "3f9a1c2e70b4"));
        let merged = merge_undo(Some(&text), Some(&text), Some(&render(&was))).unwrap();
        assert_eq!(parse(&merged).unwrap().metrics["success"].grade, Grade::Accepted);
        // undo of the commit that created the file, with a later entry kept
        let mut later = book.clone();
        later.guardrails.insert("EC-1".into(), rec(Grade::Accepted, "aaaaaaaaaaaa"));
        let merged = merge_undo(Some(&render(&later)), Some(&text), None).unwrap();
        let m = parse(&merged).unwrap();
        assert!(m.metrics.is_empty() && m.terms.is_empty() && m.guardrails.contains_key("EC-1"), "{}", merged);
        assert_eq!(merge_undo(Some(&text), Some(&text), None), None, "nothing left: the file goes");
    }

    #[test]
    fn names_and_dates_read_as_words() {
        assert_eq!(who_said("Priya via page"), "Priya");
        assert_eq!(who_said("person via page"), "a person");
        assert_eq!(who_said("agent via MCP"), "an agent");
        assert_eq!(short_date("2026-11-04"), "4 Nov");
        assert_eq!(Who::of("agent via API"), Who::Agent);
        assert_eq!(Who::of("Priya via page"), Who::Person);
        assert_eq!(Kind::parse("word"), Some(Kind::Term));
        assert_eq!(by_words(Some("Priya"), "page"), "Priya via page");
        assert_eq!(by_words(Some("an agent"), "page"), "person via page", "a typed name never makes it an agent");
        assert_eq!(by_words(None, "CLI"), "person via CLI");
    }
}
