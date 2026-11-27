//! People: who owns and decides things in a space (specs/2026-11-27-people-areas-graph.md, Part 1).
//!
//! Names only: no emails, no logins, no roles. The list is worked out on every call from what the
//! space already says (owner fields, `trust.yaml`'s `by`, the activity log, git authors), plus
//! `people.yaml` for what can't be worked out: other spellings, team or person, "left", and pairs
//! that are not the same person. Agents are never people.

use crate::ontology::Ontology;
use crate::schema::{PeopleFile, Person};
use indexmap::IndexMap;
use serde_json::{json, Value as Json};

/// The people file, at the root of the space. Written by Lorefile only.
pub const FILE: &str = "people.yaml";

/// The kinds of thing that have an owner, in the order the page lists them.
pub const OWNED: [&str; 4] = ["metric", "guardrail", "entity", "function"];

/// The owner name rule (decision 13 and Part 1): the page's name rule plus `&` and `/`, up to 60
/// characters, never "agent" or the word "via", and no `@` (addresses keep working where they are
/// written, but a new owner is a name). Returns the trimmed name.
pub fn owner_ok(name: &str) -> Result<String, String> {
    let n = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() {
        return Err("type a name: a person (\"Priya\") or a team (\"Growth team\")".into());
    }
    if n.chars().count() > 60 {
        return Err("a name is at most 60 characters".into());
    }
    if n.contains('@') {
        return Err("owners are names, not addresses: write \"Finance team\", not finance@…".into());
    }
    if !n.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '\'' | '_' | '&' | '/')) {
        return Err(format!("`{}`: a name uses letters, digits, spaces and . - ' _ & / only", n));
    }
    let low = n.to_lowercase();
    if low.contains("agent") || low.split_whitespace().any(|w| w == "via") {
        return Err("an owner is a person or a team, never an agent".into());
    }
    Ok(n)
}

fn deaccent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e',
        'ğ' => 'g',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => 'i',
        'ł' | 'ľ' => 'l',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ř' => 'r',
        'ś' | 'š' | 'ş' => 's',
        'ť' | 'ţ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        'ß' => 's',
        other => other,
    }
}

/// The key two spellings of one person share: trimmed, spaces folded, lower case, no accents.
/// "Vinay ", "vinay" and "Vínay" are one person.
pub fn fold(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase().chars().map(deaccent).collect()
}

/// `fold` with spaces and punctuation gone: "vinaykarode" (a machine's git name) and "Vinay Karode".
fn squash(name: &str) -> String {
    fold(name).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// "finance@northstar.example" → "Finance team", as the page has always shown addresses.
pub fn shown(name: &str) -> String {
    match name.split_once('@') {
        Some((local, _)) => {
            let w = local.replace('_', " ");
            let mut c = w.chars();
            let first: String = c.next().map(|f| f.to_uppercase().collect()).unwrap_or_default();
            format!("{}{} team", first, c.as_str())
        }
        None => name.trim().to_string(),
    }
}

/// The `people.yaml` entry a name belongs to: by its name or one of its other spellings.
pub fn entry_for<'a>(file: Option<&'a PeopleFile>, name: &str) -> Option<&'a Person> {
    let k = fold(name);
    file?.people.iter().find(|p| fold(&p.name) == k || p.also.iter().any(|a| fold(a) == k))
}

/// Team or person, in order: `people.yaml` says so; an address; the whole word "team"; else a person.
pub fn is_team(file: Option<&PeopleFile>, name: &str) -> bool {
    if let Some(t) = entry_for(file, name).and_then(|p| p.team) {
        return t;
    }
    name.contains('@') || fold(name).split(|c: char| !c.is_alphanumeric()).any(|w| w == "team")
}

/// A `by` from trust.yaml, history or the activity log, as a person's name: `None` for agents,
/// connections, anyone said "via MCP" or "via API", the anonymous "person via page", and the
/// machine author "Lorefile".
pub fn person_from_by(by: &str) -> Option<String> {
    let low = by.to_lowercase();
    if low.contains("agent") || low.contains("via mcp") || low.contains("via api") || low.contains("connection") {
        return None;
    }
    let who = crate::trust::who_said(by);
    if who == "a person" || who == "an agent" || who.eq_ignore_ascii_case("lorefile") || who.trim().is_empty() {
        return None;
    }
    Some(who)
}

/// Every owner field the space has: (kind, item name, owner as written, file).
pub fn owners(ont: &Ontology) -> Vec<(&'static str, String, String, String)> {
    let mut out = Vec::new();
    for m in ont.metrics.values().filter(|m| !m.owner.trim().is_empty()) {
        out.push(("metric", m.name.clone(), m.owner.trim().to_string(), m.file.clone()));
    }
    for c in ont.constraints.values().filter(|c| !c.owner.trim().is_empty()) {
        out.push(("guardrail", c.id.clone(), c.owner.trim().to_string(), c.file.clone()));
    }
    for t in ont.types.values().filter(|t| !t.owner.trim().is_empty()) {
        out.push(("entity", t.name.clone(), t.owner.trim().to_string(), t.file.clone()));
    }
    for f in ont.functions.values().filter(|f| !f.owner.trim().is_empty()) {
        out.push(("function", f.name.clone(), f.owner.trim().to_string(), f.file.clone()));
    }
    out
}

/// One person (or team) as the page and `describe` show them.
#[derive(Debug, Clone, Default)]
pub struct One {
    pub name: String,
    pub team: bool,
    /// Other spellings: from `people.yaml` and every spelling seen that isn't the main one.
    pub also: Vec<String>,
    pub left: Option<String>,
    pub owns: IndexMap<&'static str, Vec<String>>,
    pub decided: usize,
    pub last_seen: Option<String>,
    pub seen_in: Vec<&'static str>,
    /// How often each spelling was seen, for the main name.
    spellings: IndexMap<String, (usize, bool)>,
    pub notes: Vec<String>,
}

impl One {
    pub fn owns_count(&self) -> usize {
        self.owns.values().map(|v| v.len()).sum()
    }

    /// "1 metric · 2 guardrails", or "nothing".
    pub fn owns_words(&self) -> String {
        let parts: Vec<String> = OWNED.iter().filter_map(|k| self.owns.get(k).filter(|v| !v.is_empty()).map(|v| plural(v.len(), k))).collect();
        if parts.is_empty() { "nothing".into() } else { parts.join(" · ") }
    }

    pub fn to_json(&self) -> Json {
        let owns: serde_json::Map<String, Json> = OWNED.iter().map(|k| (k.to_string(), json!(self.owns.get(k).cloned().unwrap_or_default()))).collect();
        json!({
            "name": self.name, "kind": if self.team { "team" } else { "person" }, "also": self.also, "left": self.left,
            "owns": owns, "owns_words": self.owns_words(), "owns_count": self.owns_count(), "decided": self.decided,
            "last_seen": self.last_seen, "seen_in": self.seen_in, "notes": self.notes, "shown": shown(&self.name),
        })
    }
}

pub fn plural(n: usize, word: &str) -> String {
    format!("{} {}{}", n, word, if n == 1 { "" } else { "s" })
}

/// Everything the People screen shows, worked out from the space.
#[derive(Debug, Clone, Default)]
pub struct Derived {
    pub people: Vec<One>,
    pub unowned: IndexMap<&'static str, Vec<String>>,
    pub same_person: Vec<[String; 2]>,
}

impl Derived {
    /// The person a written name is: their entry, by any of their spellings.
    pub fn find(&self, name: &str) -> Option<&One> {
        let k = fold(name);
        self.people.iter().find(|p| fold(&p.name) == k || p.also.iter().any(|a| fold(a) == k))
    }

    pub fn to_json(&self) -> Json {
        let unowned: serde_json::Map<String, Json> = self.unowned.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        json!({ "people": self.people.iter().map(|p| p.to_json()).collect::<Vec<_>>(), "unowned": unowned, "same_person": self.same_person })
    }
}

/// Where else a name was seen than in owner fields, for `derive`: (name as written, where, date).
pub struct Seen {
    pub name: String,
    pub source: &'static str,
    pub at: Option<String>,
}

/// The list for one space. `extra` holds the names from the activity log and git history, each
/// already turned into a person's name (see `person_from_by`); trust.yaml is read here.
pub fn derive(ont: &Ontology, extra: &[Seen]) -> Derived {
    let file = ont.people.as_ref();
    // key → person; the key is the people.yaml entry's fold when there is one, else the name's fold
    let mut map: IndexMap<String, One> = IndexMap::new();
    let key_of = |name: &str| -> String { entry_for(file, name).map(|p| fold(&p.name)).unwrap_or_else(|| fold(name)) };
    let see = |map: &mut IndexMap<String, One>, name: &str, source: &'static str, is_owner: bool, at: Option<&str>| {
        let k = key_of(name);
        let e = map.entry(k).or_default();
        let s = e.spellings.entry(name.trim().to_string()).or_insert((0, false));
        s.0 += 1;
        s.1 |= is_owner;
        if !e.seen_in.contains(&source) {
            e.seen_in.push(source);
        }
        if let Some(at) = at.map(|a| a.get(..10).unwrap_or(a).to_string()) {
            if e.last_seen.as_deref().map(|l| at.as_str() > l).unwrap_or(true) {
                e.last_seen = Some(at);
            }
        }
    };
    // people.yaml first, so someone nobody mentions is still listed ("owns nothing")
    if let Some(f) = file {
        for p in &f.people {
            let e = map.entry(fold(&p.name)).or_default();
            e.spellings.entry(p.name.trim().to_string()).or_insert((0, false));
            if !e.seen_in.contains(&"people.yaml") {
                e.seen_in.push("people.yaml");
            }
        }
    }
    let mut unowned: IndexMap<&'static str, Vec<String>> = IndexMap::new();
    let all_owners = owners(ont);
    for (kind, item, who, _) in &all_owners {
        see(&mut map, who, "owner", true, None);
        let k = key_of(who);
        if let Some(e) = map.get_mut(&k) {
            e.owns.entry(kind).or_default().push(item.clone());
        }
    }
    for m in ont.metrics.values().filter(|m| m.owner.trim().is_empty()) {
        unowned.entry("metric").or_default().push(m.name.clone());
    }
    for c in ont.constraints.values().filter(|c| c.owner.trim().is_empty()) {
        unowned.entry("guardrail").or_default().push(c.id.clone());
    }
    // trust.yaml: who stands behind each grade, and who said a pair differs on purpose
    if let Some(book) = &ont.trust {
        for k in crate::trust::Kind::ALL {
            for r in crate::trust::entries(book, k).values() {
                if let Some(n) = person_from_by(&r.by) {
                    see(&mut map, &n, "trust", false, Some(&r.at));
                    if let Some(e) = map.get_mut(&key_of(&n)) {
                        e.decided += 1;
                    }
                }
            }
        }
        for d in &book.distinct {
            if let Some(n) = person_from_by(&d.by) {
                see(&mut map, &n, "trust", false, Some(&d.at));
                if let Some(e) = map.get_mut(&key_of(&n)) {
                    e.decided += 1;
                }
            }
        }
    }
    for s in extra {
        see(&mut map, &s.name, s.source, false, s.at.as_deref());
    }
    let mut people: Vec<One> = map
        .into_values()
        .map(|mut e| {
            let entry = e.spellings.keys().find_map(|s| entry_for(file, s)).cloned();
            e.name = match &entry {
                Some(p) => p.name.trim().to_string(),
                // the spelling used most, then one written in an owner field, then alphabetical
                None => {
                    let mut sp: Vec<(&String, &(usize, bool))> = e.spellings.iter().collect();
                    sp.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(b.1 .1.cmp(&a.1 .1)).then(a.0.cmp(b.0)));
                    sp.first().map(|(s, _)| s.to_string()).unwrap_or_default()
                }
            };
            let mut also: Vec<String> = entry.as_ref().map(|p| p.also.clone()).unwrap_or_default();
            for s in e.spellings.keys() {
                if s != &e.name && !also.iter().any(|a| a == s) {
                    also.push(s.clone());
                }
            }
            e.also = also;
            e.left = entry.as_ref().and_then(|p| p.left.clone());
            e.team = is_team(file, &e.name);
            let f = fold(&e.name);
            if matches!(f.as_str(), "tbd" | "none" | "?" | "n/a" | "nobody" | "todo" | "-") {
                e.notes.push("this isn't a person: name a real owner".into());
            } else if e.name.contains(',') || fold(&e.name).split_whitespace().any(|w| w == "and") && !e.team {
                e.notes.push("looks like two names: owners are one person or one team".into());
            }
            e
        })
        .filter(|e| !e.name.is_empty())
        .collect();
    people.sort_by(|a, b| b.owns_count().cmp(&a.owns_count()).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let same_person = same_person(&people, file);
    Derived { people, unowned, same_person }
}

/// "Same person?" pairs (decision 12): one name is the first word(s) of the other, the two match
/// once spaces and punctuation go, or two names of 5+ letters differ by one letter. Never a team
/// with a person, never a pair `people.yaml` says are two people.
fn same_person(people: &[One], file: Option<&PeopleFile>) -> Vec<[String; 2]> {
    let not_same = |a: &str, b: &str| -> bool {
        file.map(|f| f.not_same.iter().any(|p| (fold(&p[0]) == fold(a) && fold(&p[1]) == fold(b)) || (fold(&p[0]) == fold(b) && fold(&p[1]) == fold(a)))).unwrap_or(false)
    };
    let mut out = Vec::new();
    for (i, a) in people.iter().enumerate() {
        for b in people.iter().skip(i + 1) {
            if a.team != b.team || not_same(&a.name, &b.name) {
                continue;
            }
            let (fa, fb) = (fold(&shown(&a.name)), fold(&shown(&b.name)));
            let (wa, wb): (Vec<&str>, Vec<&str>) = (fa.split_whitespace().collect(), fb.split_whitespace().collect());
            let prefix = |s: &[&str], l: &[&str]| s.len() < l.len() && l[..s.len()] == *s;
            let squashed = squash(&a.name) == squash(&b.name);
            let letters = |s: &str| s.chars().filter(|c| c.is_alphabetic()).count();
            let one_off = letters(&fa) >= 5 && letters(&fb) >= 5 && edit_distance(&fa, &fb) == 1;
            if prefix(&wa, &wb) || prefix(&wb, &wa) || squashed || one_off {
                // the shorter name first: "Priya" and "Priya Shah"
                if a.name.len() <= b.name.len() {
                    out.push([a.name.clone(), b.name.clone()]);
                } else {
                    out.push([b.name.clone(), a.name.clone()]);
                }
            }
        }
    }
    out
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The main name and kind of a written owner, from the owner fields and `people.yaml` only (what
/// `describe` uses): `(main, team)`. The main name is the people.yaml entry, else the spelling the
/// owner fields use most for that person.
pub fn person_of(ont: &Ontology, name: &str) -> (String, bool) {
    let file = ont.people.as_ref();
    if let Some(p) = entry_for(file, name) {
        return (p.name.trim().to_string(), is_team(file, &p.name));
    }
    let k = fold(name);
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for (_, _, who, _) in owners(ont) {
        if fold(&who) == k {
            *counts.entry(who).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let main = v.first().map(|(s, _)| s.clone()).unwrap_or_else(|| name.trim().to_string());
    let team = is_team(file, &main);
    (main, team)
}

/// `owner_kind` and, when the written name is another spelling, `owner_is`, for `describe`.
pub fn describe_owner(ont: &Ontology, j: &mut Json, owner: &str) {
    if owner.trim().is_empty() {
        return;
    }
    let (main, team) = person_of(ont, owner);
    j["owner_kind"] = json!(if team { "team" } else { "person" });
    if main != owner.trim() {
        j["owner_is"] = json!(main);
    }
}

/// The one "Owners" line of `summary`: up to 12 names, then "and N more".
pub fn owners_line(ont: &Ontology) -> Option<String> {
    let mut by: IndexMap<String, (bool, IndexMap<&'static str, usize>)> = IndexMap::new();
    for (kind, _, who, _) in owners(ont) {
        let (main, team) = person_of(ont, &who);
        let e = by.entry(main).or_insert((team, IndexMap::new()));
        *e.1.entry(kind).or_default() += 1;
    }
    if by.is_empty() {
        return None;
    }
    let mut v: Vec<(String, usize, String)> = by
        .into_iter()
        .map(|(name, (_, kinds))| {
            let n: usize = kinds.values().sum();
            let words: Vec<String> = OWNED.iter().filter_map(|k| kinds.get(k).map(|c| plural(*c, k))).collect();
            (shown(&name), n, words.join(", "))
        })
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let more = v.len().saturating_sub(12);
    let mut line = format!("Owners: {}", v.iter().take(12).map(|(n, _, w)| format!("{} ({})", n, w)).collect::<Vec<_>>().join(", "));
    if more > 0 {
        line.push_str(&format!(", and {} more", more));
    }
    Some(line)
}

/// The text of `people.yaml` for a file: the header comment, then the entries.
pub fn render(f: &PeopleFile) -> String {
    let mut out = String::from("# people.yaml: who is who in this space. Written by Lorefile from the People screen.\n# Names only: no emails, no logins.\n");
    let body = serde_yaml::to_string(f).unwrap_or_default();
    out.push_str(&body);
    out
}

/// What a People-screen action does to `people.yaml`, as the new file (or `None` when nothing
/// changes). `op` is team, person, left, back or not_same.
pub fn edit_file(ont: &Ontology, op: &str, name: &str, other: &str) -> Result<(PeopleFile, String), String> {
    let mut f = ont.people.clone().unwrap_or_default();
    let d = derive(ont, &[]);
    let main = d.find(name).map(|p| p.name.clone()).unwrap_or_else(|| name.trim().to_string());
    if main.is_empty() {
        return Err("say whose entry to change".into());
    }
    let entry = |f: &mut PeopleFile| -> usize {
        let k = fold(&main);
        match f.people.iter().position(|p| fold(&p.name) == k || p.also.iter().any(|a| fold(a) == k)) {
            Some(i) => i,
            None => {
                f.people.push(Person { name: main.clone(), ..Default::default() });
                f.people.len() - 1
            }
        }
    };
    let what = match op {
        "team" => {
            let i = entry(&mut f);
            f.people[i].team = Some(true);
            format!("marked {} as a team", shown(&main))
        }
        "person" => {
            let i = entry(&mut f);
            f.people[i].team = Some(false);
            format!("marked {} as a person", shown(&main))
        }
        "left" => {
            let i = entry(&mut f);
            f.people[i].left = Some(crate::trust::today());
            format!("marked {} as having left", shown(&main))
        }
        "back" => {
            let i = entry(&mut f);
            f.people[i].left = None;
            format!("marked {} as back", shown(&main))
        }
        "not_same" => {
            let o = d.find(other).map(|p| p.name.clone()).unwrap_or_else(|| other.trim().to_string());
            if o.is_empty() || fold(&o) == fold(&main) {
                return Err("not_same names two different people".into());
            }
            if !f.not_same.iter().any(|p| (fold(&p[0]) == fold(&main) && fold(&p[1]) == fold(&o)) || (fold(&p[0]) == fold(&o) && fold(&p[1]) == fold(&main))) {
                f.not_same.push([main.clone(), o.clone()]);
            }
            format!("said {} and {} are two people", shown(&main), shown(&o))
        }
        other => return Err(format!("op is rename, team, person, left, back or not_same, not `{}`", other)),
    };
    // an entry that says nothing beyond its name is dropped: the file holds only what can't be worked out
    f.people.retain(|p| !p.also.is_empty() || p.team.is_some() || p.left.is_some());
    Ok((f, what))
}

/// What a rename or merge writes (Part 1, "Rename and merge").
#[derive(Debug, Default)]
pub struct RenamePlan {
    /// Every file with its new text, `people.yaml` last.
    pub files: Vec<(String, String)>,
    /// Files rewritten through the mapping (flow-style owners): they lose their comments.
    pub lossy: Vec<String>,
    /// How many owner fields of each kind change.
    pub counts: IndexMap<&'static str, usize>,
    /// `to` was already someone: this is a merge.
    pub merge: bool,
    pub from: String,
    pub to: String,
}

impl RenamePlan {
    /// "1 metric, 2 guardrails", or "no owner fields".
    pub fn counts_words(&self) -> String {
        let parts: Vec<String> = OWNED.iter().filter_map(|k| self.counts.get(k).filter(|n| **n > 0).map(|n| plural(*n, k))).collect();
        if parts.is_empty() { "no owner fields".into() } else { parts.join(", ") }
    }
}

/// The value of an `owner:` line, unquoted, or `None` when the line isn't a plain owner line.
fn owner_line_value(line: &str) -> Option<(usize, String)> {
    let indent = line.len() - line.trim_start().len();
    let rest = line.trim_start();
    let rest = rest.strip_prefix("- ").unwrap_or(rest);
    let v = rest.strip_prefix("owner:")?;
    let v = match v.find(" #") {
        Some(i) => &v[..i],
        None => v,
    };
    let v = v.trim();
    if v.is_empty() || v.starts_with('{') || v.starts_with('[') || v.starts_with('|') || v.starts_with('>') {
        return None;
    }
    let v = v.trim_matches('"').trim_matches('\'');
    Some((indent, v.to_string()))
}

/// A name as a YAML scalar: plain when it is safe, quoted otherwise.
pub fn yaml_scalar(s: &str) -> String {
    let plain = s.chars().next().map(|c| c.is_alphanumeric()).unwrap_or(false)
        && !s.contains(": ")
        && !s.contains(" #")
        && !s.ends_with(':')
        && !matches!(s.to_ascii_lowercase().as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n" | "~")
        && s.parse::<f64>().is_err();
    if plain { s.to_string() } else { serde_json::to_string(s).unwrap_or_default() }
}

/// Set every owner line of `text` whose value is one of `names` (by `fold`) to `to`, keeping the
/// rest of each line. Returns the new text and how many lines changed.
fn rewrite_owner_lines(text: &str, names: &[String], to: &str) -> (String, usize) {
    let keys: Vec<String> = names.iter().map(|n| fold(n)).collect();
    let mut n = 0;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match owner_line_value(line) {
            Some((_, v)) if keys.contains(&fold(&v)) => {
                let head_len = line.find("owner:").unwrap_or(0) + "owner:".len();
                let comment = line.find(" #").filter(|i| *i > head_len).map(|i| line[line[..i].trim_end().len()..].to_string()).unwrap_or_default();
                out.push(format!("{} {}{}", &line[..head_len], yaml_scalar(to), comment));
                n += 1;
            }
            _ => out.push(line.to_string()),
        }
    }
    let mut s = out.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    (s, n)
}

/// Set `owner` on every item of a parsed file whose owner is one of `names`: the fallback for
/// flow-style items (`{ id: X, owner: Y }`), which loses the file's comments.
fn rewrite_owner_mapping(text: &str, names: &[String], to: &str) -> Result<(String, usize), String> {
    let keys: Vec<String> = names.iter().map(|n| fold(n)).collect();
    let mut doc: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    let mut n = 0;
    let mut fix = |m: &mut serde_yaml::Mapping| {
        if let Some(serde_yaml::Value::String(v)) = m.get(serde_yaml::Value::String("owner".into())) {
            if keys.contains(&fold(v)) {
                m.insert(serde_yaml::Value::String("owner".into()), serde_yaml::Value::String(to.to_string()));
                n += 1;
            }
        }
    };
    if let Some(root) = doc.as_mapping_mut() {
        let single = ["type", "metric", "function"].iter().any(|k| root.contains_key(serde_yaml::Value::String((*k).into())));
        if single {
            fix(root);
        }
        for list in ["types", "metrics", "functions", "constraints"] {
            if let Some(serde_yaml::Value::Sequence(items)) = root.get_mut(serde_yaml::Value::String(list.into())) {
                for it in items.iter_mut().filter_map(|i| i.as_mapping_mut()) {
                    fix(it);
                }
            }
        }
    }
    Ok((serde_yaml::to_string(&doc).map_err(|e| e.to_string())?, n))
}

/// Plan a rename or merge: every owner field that says `from` or one of its spellings becomes
/// `to`, by line edits where they can be made, `people.yaml` gains the old spellings under `to`.
pub fn rename_plan(ont: &Ontology, from: &str, to: &str) -> Result<RenamePlan, String> {
    let d = derive(ont, &[]);
    let person = d.find(from).ok_or_else(|| format!("nobody here is called {}", from))?;
    let to = owner_ok(to)?;
    if to == person.name {
        return Err(format!("{} is already called that", person.name));
    }
    let target = d.find(&to).filter(|p| fold(&p.name) != fold(&person.name)).cloned();
    let merge = target.is_some();
    let mut names: Vec<String> = vec![person.name.clone()];
    names.extend(person.also.iter().cloned());
    if let Some(e) = entry_for(ont.people.as_ref(), &person.name) {
        names.extend(e.also.iter().cloned());
    }
    let keys: Vec<String> = names.iter().map(|n| fold(n)).collect();
    // which items change, by file
    let mut by_file: IndexMap<String, usize> = IndexMap::new();
    let mut counts: IndexMap<&'static str, usize> = IndexMap::new();
    for (kind, _, who, file) in owners(ont) {
        if keys.contains(&fold(&who)) {
            *by_file.entry(file).or_default() += 1;
            *counts.entry(kind).or_default() += 1;
        }
    }
    let mut files = Vec::new();
    let mut lossy = Vec::new();
    for (file, want) in &by_file {
        let text = std::fs::read_to_string(ont.root.join(file)).map_err(|e| format!("reading {}: {}", file, e))?;
        if serde_yaml::from_str::<serde_yaml::Value>(&text).is_err() {
            return Err(format!("{} doesn't parse; fix it first, then rename", file));
        }
        let (new, n) = rewrite_owner_lines(&text, &names, &to);
        // the line edit has to have changed exactly the owner fields, and nothing else
        let lines_ok = n == *want && owners_after(&new, &keys) == 0;
        if lines_ok {
            files.push((file.clone(), new));
        } else {
            let (new, _) = rewrite_owner_mapping(&text, &names, &to).map_err(|e| format!("{}: {}", file, e))?;
            lossy.push(file.clone());
            files.push((file.clone(), new));
        }
    }
    // people.yaml: `to` gains `from` and its spellings; `from`'s own entry goes
    let mut f = ont.people.clone().unwrap_or_default();
    let fk = fold(&person.name);
    let old = f.people.iter().position(|p| fold(&p.name) == fk || p.also.iter().any(|a| fold(a) == fk)).map(|i| f.people.remove(i));
    let tk = fold(&to);
    let idx = match f.people.iter().position(|p| fold(&p.name) == tk || p.also.iter().any(|a| fold(a) == tk)) {
        Some(i) => i,
        None => {
            f.people.push(Person { name: target.as_ref().map(|t| t.name.clone()).unwrap_or_else(|| to.clone()), ..Default::default() });
            f.people.len() - 1
        }
    };
    let mut also: Vec<String> = f.people[idx].also.clone();
    for n in names.iter().chain(old.iter().flat_map(|o| o.also.iter())) {
        if fold(n) != fold(&f.people[idx].name) && !also.iter().any(|a| fold(a) == fold(n)) {
            also.push(n.clone());
        }
    }
    f.people[idx].also = also;
    if let Some(o) = &old {
        if f.people[idx].team.is_none() {
            f.people[idx].team = o.team;
        }
    }
    files.push((FILE.to_string(), render(&f)));
    Ok(RenamePlan { files, lossy, counts, merge, from: person.name.clone(), to: f.people[idx].name.clone() })
}

/// How many owner fields of a file's text still say one of `keys`, by parsing it.
fn owners_after(text: &str, keys: &[String]) -> usize {
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(text) else { return usize::MAX };
    let mut n = 0;
    let mut count = |v: &serde_yaml::Value| {
        if let Some(o) = v.get("owner").and_then(|o| o.as_str()) {
            if keys.contains(&fold(o)) {
                n += 1;
            }
        }
    };
    if ["type", "metric", "function"].iter().any(|k| doc.get(*k).is_some()) {
        count(&doc);
    }
    for list in ["types", "metrics", "functions", "constraints"] {
        for it in doc.get(list).and_then(|l| l.as_sequence()).into_iter().flatten() {
            count(it);
        }
    }
    n
}

/// W121: contradictions inside `people.yaml` itself.
pub fn problems(f: &PeopleFile) -> Vec<String> {
    let mut out = Vec::new();
    let mut owner_of: IndexMap<String, String> = IndexMap::new();
    for p in &f.people {
        for a in &p.also {
            match owner_of.get(&fold(a)) {
                Some(first) if fold(first) != fold(&p.name) => out.push(format!("people.yaml: {} is a spelling of both {} and {}", a, first, p.name)),
                _ => {
                    owner_of.insert(fold(a), p.name.clone());
                }
            }
            if let Some(other) = f.people.iter().find(|q| fold(&q.name) == fold(a) && fold(&q.name) != fold(&p.name)) {
                out.push(format!("people.yaml: {} is listed as another spelling of {}, but also has its own entry", other.name, p.name));
            }
        }
        if let Some(l) = &p.left {
            if chrono::NaiveDate::parse_from_str(l.trim(), "%Y-%m-%d").is_err() {
                out.push(format!("people.yaml: {}'s left `{}` isn't a date like 2026-11-27", p.name, l));
            }
        }
    }
    for pair in &f.not_same {
        let a = entry_for(Some(f), &pair[0]).map(|p| fold(&p.name)).unwrap_or_else(|| fold(&pair[0]));
        let b = entry_for(Some(f), &pair[1]).map(|p| fold(&p.name)).unwrap_or_else(|| fold(&pair[1]));
        if a == b {
            out.push(format!("people.yaml: not_same names {} twice (\"{}\" and \"{}\" are one entry)", pair[0], pair[0], pair[1]));
        }
    }
    out
}

/// W119: owner fields written as another spelling of someone in `people.yaml`:
/// (kind, item, written, main, file).
pub fn other_spellings(ont: &Ontology) -> Vec<(&'static str, String, String, String, String)> {
    let Some(f) = ont.people.as_ref() else { return vec![] };
    owners(ont)
        .into_iter()
        .filter_map(|(kind, item, who, file)| {
            let p = f.people.iter().find(|p| p.also.iter().any(|a| fold(a) == fold(&who)) && fold(&p.name) != fold(&who))?;
            Some((kind, item, who, p.name.clone(), file))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lorefile-people-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, text) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    const ONT: &str = "name: shop\n";
    const ORDER: &str = "type: Order\ndescription: One checkout.\nowner: Vinay\nproperties:\n  order_id: { type: string, key: true, description: The id. }\n";
    const METRICS: &str = "metrics:\n  - metric: revenue\n    description: Money in. # what we run on\n    sql: select 1\n    owner: vinay   # set in the trial\n  - metric: refunds\n    description: Money back.\n    sql: select 2\n  - { metric: orders, description: Orders., sql: select 3, owner: Vinay Karode }\n";
    const RULES: &str = "constraints:\n  - id: OR-001\n    name: No negative totals\n    applies_to: Order\n    rule: \"true\"\n    owner: finance@shop.example\n  - id: OR-002\n    name: Growth rule\n    applies_to: Order\n    rule: \"true\"\n    owner: Growth team\n";

    #[test]
    fn names_come_from_owners_and_grades_and_spellings_are_one_person() {
        let dir = space("derive", &[("ontology.yaml", ONT), ("types/order.yaml", ORDER), ("logic/metrics.yaml", METRICS), ("constraints/order.yaml", RULES),
            ("trust.yaml", "trust:\n  metrics:\n    revenue: { grade: accepted, by: Priya via page, at: 2026-11-03, of: abc }\n    refunds: { grade: proposed, by: agent via MCP, at: 2026-11-04, of: abc }\n  guardrails:\n    OR-001: { grade: accepted, by: person via page, at: 2026-11-05, of: abc }\n")]);
        let ont = crate::load_ontology(&dir).unwrap().0;
        let d = derive(&ont, &[Seen { name: "vinaykarode".into(), source: "history", at: Some("2026-11-27T10:00:00Z".into()) }]);
        let names: Vec<&str> = d.people.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Vinay") && names.contains(&"Vinay Karode") && names.contains(&"Priya"), "{:?}", names);
        assert!(!names.iter().any(|n| n.contains("agent") || *n == "a person"), "agents and anonymous saves are never people: {:?}", names);
        let v = d.find("vinay").unwrap();
        assert_eq!(v.name, "Vinay", "Vinay and vinay are one person, shown in a spelling an owner field uses");
        assert_eq!(v.owns["metric"], vec!["revenue"]);
        assert_eq!(v.owns["entity"], vec!["Order"]);
        assert_eq!(d.find("Priya").unwrap().decided, 1);
        assert_eq!(d.find("Priya").unwrap().last_seen.as_deref(), Some("2026-11-03"));
        assert!(d.find("finance@shop.example").unwrap().team, "an address is a team");
        assert!(d.find("Growth team").unwrap().team, "the word team makes a team");
        assert!(!d.find("Priya").unwrap().team);
        assert_eq!(d.unowned["metric"], vec!["refunds"]);
        assert!(d.same_person.iter().any(|p| p[0] == "Vinay" && p[1] == "Vinay Karode"), "a first word: {:?}", d.same_person);
        assert!(d.same_person.iter().any(|p| p.contains(&"vinaykarode".to_string()) && p.contains(&"Vinay Karode".to_string())), "spaces gone: {:?}", d.same_person);
        assert_eq!(shown("finance@shop.example"), "Finance team");
        // people.yaml: other spellings, teams, left, and pairs that are two people
        std::fs::write(dir.join("people.yaml"), "people:\n  - name: Vinay Karode\n    also: [Vinay, vinaykarode]\n  - name: Priya\n    left: 2026-11-27\nnot_same:\n  - [Growth team, finance@shop.example]\n").unwrap();
        let ont = crate::load_ontology(&dir).unwrap().0;
        let d = derive(&ont, &[]);
        let v = d.find("vinay").unwrap();
        assert_eq!(v.name, "Vinay Karode", "people.yaml's name is the main one");
        assert_eq!(v.owns["metric"], vec!["revenue", "orders"]);
        assert!(d.same_person.is_empty(), "{:?}", d.same_person);
        assert_eq!(d.find("Priya").unwrap().left.as_deref(), Some("2026-11-27"));
        let (main, team) = person_of(&ont, "vinay");
        assert_eq!((main.as_str(), team), ("Vinay Karode", false));
        let w = other_spellings(&ont);
        assert!(w.iter().any(|x| x.0 == "entity" && x.2 == "Vinay"), "W119 for Order: {:?}", w);
        assert!(owners_line(&ont).unwrap().starts_with("Owners: Vinay Karode (2 metrics, 1 entity)"), "{}", owners_line(&ont).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_keeps_comments_and_falls_back_for_flow_items() {
        let dir = space("rename", &[("ontology.yaml", ONT), ("types/order.yaml", ORDER), ("logic/metrics.yaml", METRICS), ("constraints/order.yaml", RULES)]);
        let ont = crate::load_ontology(&dir).unwrap().0;
        let p = rename_plan(&ont, "Vinay", "Vinay K").unwrap();
        assert!(!p.merge);
        assert_eq!(p.counts_words(), "1 metric, 1 entity");
        let order = &p.files.iter().find(|(f, _)| f == "types/order.yaml").unwrap().1;
        assert!(order.contains("owner: Vinay K\n"), "{}", order);
        let m = &p.files.iter().find(|(f, _)| f == "logic/metrics.yaml").unwrap().1;
        assert!(m.contains("    owner: Vinay K   # set in the trial\n") && m.contains("# what we run on"), "the comment stays: {}", m);
        assert!(m.contains("owner: Vinay Karode }"), "another person is untouched: {}", m);
        assert!(p.lossy.is_empty());
        let people = &p.files.last().unwrap();
        assert_eq!(people.0, "people.yaml");
        assert!(people.1.contains("name: Vinay K") && people.1.contains("- Vinay"), "{}", people.1);
        assert!(!people.1.contains("- vinay"), "spellings differing only in capitals are one: {}", people.1);
        // a merge into an existing person, and a flow-style owner rewritten through the mapping
        let m2 = rename_plan(&ont, "Vinay Karode", "Vinay").unwrap();
        assert!(m2.merge, "Vinay is already someone");
        assert_eq!(m2.lossy, vec!["logic/metrics.yaml"], "the flow item can't be line-edited");
        let mt = &m2.files.iter().find(|(f, _)| f == "logic/metrics.yaml").unwrap().1;
        let parsed: serde_yaml::Value = serde_yaml::from_str(mt).unwrap();
        assert_eq!(parsed["metrics"][2]["owner"], "Vinay");
        assert!(rename_plan(&ont, "Vinay", "Vinay").is_err());
        assert!(rename_plan(&ont, "Vinay", "x@y.z").is_err(), "no addresses");
        assert!(rename_plan(&ont, "Vinay", "an agent").is_err());
        assert!(rename_plan(&ont, "Nobody", "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_owner_rule_and_the_people_file_problems() {
        assert_eq!(owner_ok("  R&D  ").unwrap(), "R&D");
        assert!(owner_ok("Sales/Ops").is_ok());
        assert!(owner_ok("x@y.example").is_err());
        assert!(owner_ok("Sales agent").is_err());
        assert!(owner_ok("Vinay via page").is_err());
        assert!(owner_ok(&"a".repeat(61)).is_err());
        assert!(owner_ok("Robert'); DROP").is_err());
        let f: PeopleFile = serde_yaml::from_str("people:\n  - name: Vinay Karode\n    also: [Vinay]\n  - name: Vinay Kumar\n    also: [vinay]\n  - name: Sam\n    left: soon\n  - name: Priya\n    also: [Sam]\nnot_same:\n  - [Vinay, Vinay Karode]\n").unwrap();
        let p = problems(&f);
        assert!(p.iter().any(|x| x.to_lowercase().contains("vinay is a spelling of both")), "{:?}", p);
        assert!(p.iter().any(|x| x.contains("isn't a date")), "{:?}", p);
        assert!(p.iter().any(|x| x.contains("Sam is listed as another spelling of Priya")), "{:?}", p);
        assert!(p.iter().any(|x| x.contains("not_same names Vinay twice")), "{:?}", p);
        assert!(serde_yaml::from_str::<PeopleFile>("people:\n  - name: A\n    email: a@b.c\n").is_err(), "no emails");
    }
}
