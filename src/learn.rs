//! Learning a file: what a CSV, a spreadsheet sheet or an export *means*, so the
//! next file of the same shape is read the same way by any agent.
//!
//! The agent sends the file's name, its column names in order, up to five example
//! values per column and, if it can, some counts. Never rows. This module is pure:
//! no I/O. Example values live only in the input struct, are used here in memory to
//! guess types and the key, and are dropped with the input when the call ends.
//! Nothing here formats an example value into a string that leaves the module,
//! error messages included. What leaves is structure (names, types, the key, the
//! binding), the agent's descriptions (minus any that quote a value), and the recipe
//! built from the user's answers.

use crate::ontology::{snake, Ontology};
use crate::proposals::{Learned, Proposal};
use crate::schema::{MetricDef, TypeDef};
use crate::trust::{self, Kind as TKind};
use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Map, Value as Json};
use std::collections::HashSet;
use std::sync::LazyLock;

/// The source every learned file is bound to, made on first use, one per space.
pub const SOURCE: &str = "user_files";
/// The source kind: rows come from a file a person gives the agent. Never readable here.
pub const KIND: &str = "from_user";
pub const SOURCE_DESCRIPTION: &str = "Files people hand to an agent (a chat upload, an export, an attachment). The agent reads the rows itself; Lorefile keeps what the columns mean and which number counts.";

pub const MAX_COLUMNS: usize = 200;
pub const MAX_EXAMPLES: usize = 5;
pub const MAX_EXAMPLE_LEN: usize = 100;
pub const MAX_FILE_NAME: usize = 200;

pub fn is_from_user(kind: &str) -> bool {
    kind.eq_ignore_ascii_case(KIND)
}

/// The one sentence every surface uses for an entity whose rows come from a person's file.
pub fn hint(entity: &str, table: &str) -> String {
    let named = if table.is_empty() { String::new() } else { format!(" (usually named {}…)", table) };
    format!(
        "Rows of {} arrive as a file a person gives you{}. Read that file with your own tools; this ontology says what its columns mean and which number counts. Lorefile does not keep the rows.",
        entity, named
    )
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// What `match_file` and `learn_file` take. `learn_file` adds the descriptions,
/// `entity`, `key` and `answers`; `match_file` ignores them.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileInput {
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub row_count: Option<u64>,
    #[serde(default)]
    pub columns: Vec<ColumnIn>,
    #[serde(default)]
    pub entity: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub answers: Option<Answers>,
    #[serde(default)]
    pub reason: Option<String>,
    /// The user's own words asking to save the file without saying which number counts.
    #[serde(default)]
    pub skip_because: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnIn {
    pub name: String,
    /// Up to five values from the file, used only to guess the type. Strings, numbers or booleans.
    #[serde(default)]
    examples: Vec<Json>,
    #[serde(default)]
    pub distinct: Option<u64>,
    #[serde(default)]
    pub empty: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    /// The examples as trimmed text, cut to 100 characters. Private: never leaves the module.
    #[serde(skip)]
    values: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answers {
    /// Which number is the result: a column and how it adds up.
    #[serde(default)]
    pub success: Option<Success>,
    /// Rows that do not count, joined with `and`: `{column, equals}`.
    #[serde(default)]
    pub exclude: Vec<Exclude>,
    /// What does not count, in the user's words; becomes the metric's `caveats`.
    #[serde(default)]
    pub exclude_words: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    /// The metric's name. Defaults to `<entity>_<column>`; `success` only when the user says
    /// this is the one number the business runs on.
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub basis: Option<String>,
    /// Words the user uses for this file, each saved as a glossary word for the entity.
    #[serde(default)]
    pub also_called: Vec<String>,
    /// For a changed file: a known field → the file's new header for it.
    #[serde(default)]
    pub renamed: IndexMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Success {
    pub column: String,
    /// sum | count_distinct | count
    #[serde(default)]
    pub how: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exclude {
    pub column: String,
    pub equals: Json,
}

static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""(?:[^"\\]|\\.)*""#).unwrap());

/// A serde error without the quoted text in it: a wrong-typed field could hold a value from the file.
fn hide_values(msg: &str) -> String {
    QUOTED.replace_all(msg, "\"…\"").to_string()
}

fn nonempty(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

fn example_text(v: &Json) -> String {
    let s = match v {
        Json::String(s) => s.clone(),
        Json::Null => String::new(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => n.to_string(),
        other => other.to_string(),
    };
    s.trim().chars().take(MAX_EXAMPLE_LEN).collect()
}

/// Read the arguments of `match_file` or `learn_file`, applying the limits.
pub fn parse(args: &Json) -> Result<FileInput, String> {
    let mut input: FileInput = serde_json::from_value(args.clone()).map_err(|e| format!("the arguments do not fit: {}", hide_values(&e.to_string())))?;
    if input.columns.is_empty() {
        return Err("send `columns`: the file's column names in order, each with up to five example values".into());
    }
    if input.columns.len() > MAX_COLUMNS {
        return Err(format!("at most {} columns in one call; send a workbook one sheet at a time", MAX_COLUMNS));
    }
    input.file_name = input.file_name.trim().chars().take(MAX_FILE_NAME).collect();
    input.entity = nonempty(&input.entity);
    input.description = nonempty(&input.description);
    input.key = nonempty(&input.key);
    input.reason = nonempty(&input.reason);
    for (i, c) in input.columns.iter_mut().enumerate() {
        c.name = c.name.trim().to_string();
        if c.name.is_empty() {
            return Err(format!("column {} has no name", i + 1));
        }
        c.values = c.examples.iter().take(MAX_EXAMPLES).map(example_text).collect();
        c.examples.clear();
        c.description = nonempty(&c.description);
        c.kind = nonempty(&c.kind);
        c.unit = nonempty(&c.unit);
    }
    Ok(input)
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// Trimmed, lower case, with `_`, space and `-` treated alike. How files are recognised.
pub fn normalize_header(h: &str) -> String {
    let mut out = String::new();
    for c in h.trim().chars().flat_map(|c| c.to_lowercase()) {
        if c == ' ' || c == '-' || c == '_' {
            if !out.ends_with('_') {
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out.trim_matches('_').to_string()
}

/// Lower-case words of a name: split on anything not a letter or digit, and on camelCase.
fn words(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphanumeric() {
            let boundary = c.is_uppercase() && i > 0 && (chars[i - 1].is_lowercase() || (chars[i - 1].is_uppercase() && i + 1 < chars.len() && chars[i + 1].is_lowercase()));
            if boundary && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.extend(c.to_lowercase());
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

const MONTHS: [&str; 24] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec", "january", "february", "march", "april", "june", "july", "august", "september", "october", "november", "december",
];
static PERIOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(q[1-4]|h[12]|fy\d+|w\d+|wk\d+|\d+(st|nd|rd|th)|(19|20)\d\d(q[1-4]|h[12]|m\d+)?|q[1-4](19|20)?\d\d)$").unwrap());
static COPY_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(copy|final|latest|draft|backup|v\d+)$").unwrap());
static GENERIC_SHEET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(sheet|tab|page|data|export)\s*\d*$").unwrap());

/// A word of a file name that says when or which copy, not what.
fn is_noise(w: &str) -> bool {
    if w.chars().all(|c| c.is_ascii_digit()) || COPY_MARK.is_match(w) || PERIOD.is_match(w) {
        return true;
    }
    MONTHS.iter().any(|m| w == *m || (w.starts_with(m) && w[m.len()..].chars().all(|c| c.is_ascii_digit())) || (w.ends_with(m) && w[..w.len() - m.len()].chars().all(|c| c.is_ascii_digit()) && w.len() > m.len()))
}

fn singular(w: &str) -> String {
    match w {
        "people" => return "person".into(),
        "children" => return "child".into(),
        "data" | "series" | "news" => return w.into(),
        _ => {}
    }
    if w.len() < 3 || w.ends_with("ss") || w.ends_with("us") || w.ends_with("is") {
        return w.into();
    }
    if let Some(stem) = w.strip_suffix("ies") {
        return format!("{}y", stem);
    }
    for end in ["sses", "xes", "ches", "shes", "uses"] {
        if w.ends_with(end) {
            return w[..w.len() - 2].into();
        }
    }
    if let Some(stem) = w.strip_suffix('s') {
        return stem.into();
    }
    w.into()
}

fn pascal(ws: &[String]) -> String {
    ws.iter()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// (Entity, table) from a file name: `promo_sales_2026-12.csv` → (`PromoSale`, `promo_sales`).
/// A workbook sheet comes as `report.xlsx › Sales`; a sheet called Sheet1 says nothing, so the file name is used.
pub fn names_from_file(file_name: &str) -> Option<(String, String)> {
    let (file, sheet) = match file_name.split_once('›') {
        Some((f, s)) => (f.trim(), s.trim()),
        None => (file_name.trim(), ""),
    };
    let stem = match file.rsplit_once('.') {
        Some((s, ext)) if !s.is_empty() && (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) => s,
        _ => file,
    };
    let keep = |s: &str| -> Vec<String> { words(s).into_iter().filter(|w| !is_noise(w)).collect() };
    let file_words = keep(stem);
    let sheet_words = if sheet.is_empty() || GENERIC_SHEET.is_match(sheet) { vec![] } else { keep(sheet) };
    let base = if sheet_words.is_empty() { file_words.clone() } else { sheet_words.clone() };
    if base.is_empty() {
        return None;
    }
    let mut ws = base;
    if let Some(last) = ws.last_mut() {
        *last = singular(last);
    }
    let entity = pascal(&ws);
    if !entity.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
        return None;
    }
    let table = [file_words, sheet_words].concat().join("_");
    Some((entity, table))
}

static PLAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_]*$").unwrap());

/// Field names for the headers: a plain header is the field name; any other gets the snake
/// case of its words, and the binding maps it to the exact header. Clashes get `_2`.
pub fn field_names(headers: &[String], notes: &mut Vec<String>) -> Vec<String> {
    let mut taken: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for h in headers {
        let mut name = if PLAIN.is_match(h) {
            h.clone()
        } else {
            let w = words(h).join("_");
            if w.is_empty() {
                "column".to_string()
            } else if w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                format!("n_{}", w)
            } else {
                w
            }
        };
        if taken.contains(&name.to_lowercase()) {
            let base = name.clone();
            let mut n = 2;
            while taken.contains(&format!("{}_{}", base, n).to_lowercase()) {
                n += 1;
            }
            name = format!("{}_{}", base, n);
            notes.push(format!("two columns give the field name `{}`; the column `{}` is saved as `{}`", base, h, name));
        }
        taken.insert(name.to_lowercase());
        out.push(name);
    }
    out
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());
static WHOLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?(0|[1-9][0-9]*)$").unwrap());
static SLASH_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,2}[/.]\d{1,2}[/.]\d{2,4}$").unwrap());

fn is_empty_value(s: &str) -> bool {
    s.is_empty() || matches!(s.to_ascii_lowercase().as_str(), "null" | "na" | "n/a" | "none" | "nan")
}

fn is_bool_word(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "true" | "false" | "yes" | "no")
}

fn is_iso_date(s: &str) -> bool {
    ISO_DATE.is_match(s) && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn is_iso_datetime(s: &str) -> bool {
    if s.len() < 16 || !ISO_DATE.is_match(&s[..10]) {
        return false;
    }
    if chrono::DateTime::parse_from_rfc3339(s).is_ok() || chrono::DateTime::parse_from_rfc3339(&s.replacen(' ', "T", 1)).is_ok() {
        return true;
    }
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"].iter().any(|f| chrono::NaiveDateTime::parse_from_str(s, f).is_ok())
}

/// A decimal number, optionally with € £ $ and thousands separators. `00123` is not one.
fn decimal(s: &str) -> Option<f64> {
    let mut t: String = s.chars().filter(|c| !matches!(c, '€' | '£' | '$' | ' ' | '\u{a0}')).collect();
    if t.starts_with('+') {
        t.remove(0);
    }
    if t.is_empty() || !t.chars().any(|c| c.is_ascii_digit()) || !t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-')) {
        return None;
    }
    let norm = match (t.rfind('.'), t.rfind(',')) {
        (Some(d), Some(c)) if d > c => t.replace(',', ""),
        (Some(_), Some(_)) => t.replace('.', "").replace(',', "."),
        (None, Some(c)) if t.matches(',').count() == 1 && t.len() - c - 1 != 3 => t.replace(',', "."),
        (None, Some(_)) => t.replace(',', ""),
        _ => t,
    };
    let unsigned = norm.trim_start_matches('-');
    let int_part = unsigned.split('.').next().unwrap_or("");
    if int_part.len() > 1 && int_part.starts_with('0') {
        return None;
    }
    norm.parse::<f64>().ok()
}

fn money_symbol(s: &str) -> Option<&'static str> {
    if s.contains('€') {
        Some("EUR")
    } else if s.contains('£') {
        Some("GBP")
    } else if s.contains('$') {
        Some("$")
    } else {
        None
    }
}

const MONEY_WORDS: [&str; 12] = ["price", "revenue", "amount", "cost", "discount", "total", "spend", "value", "fee", "tax", "sales", "gmv"];
const NOT_MONEY_WORDS: [&str; 15] = ["count", "qty", "quantity", "units", "number", "num", "id", "rate", "pct", "percent", "ratio", "share", "code", "key", "no"];

fn money_name(field: &str) -> bool {
    let w = words(field);
    w.iter().any(|x| MONEY_WORDS.contains(&x.as_str())) && !w.iter().any(|x| NOT_MONEY_WORDS.contains(&x.as_str()))
}

fn flag_name(field: &str) -> bool {
    let n = normalize_header(field);
    n.starts_with("is_") || n.starts_with("has_") || n.ends_with("ed") || words(field).iter().any(|w| w == "flag" || w == "active")
}

/// The first rule that fits every non-empty example. Never `enum`: five examples cannot list
/// every value, and next month's new code would be refused.
pub fn guess_type(name: &str, vals: &[&str]) -> (&'static str, Option<String>) {
    if vals.is_empty() {
        return ("string", None);
    }
    if vals.iter().all(|v| is_bool_word(v)) || (vals.iter().all(|v| *v == "0" || *v == "1") && flag_name(name)) {
        return ("boolean", None);
    }
    if vals.iter().all(|v| is_iso_date(v)) {
        return ("date", None);
    }
    if vals.iter().all(|v| is_iso_datetime(v) || is_iso_date(v)) {
        return ("datetime", None);
    }
    if vals.iter().all(|v| WHOLE.is_match(v)) {
        return ("integer", None);
    }
    if vals.iter().all(|v| decimal(v).is_some()) {
        return ("number", None);
    }
    let note = vals.iter().all(|v| SLASH_DATE.is_match(v)).then(|| format!("`{}` looks like a date, saved as text: only dates written 2026-11-20 are read as dates", name));
    ("string", note)
}

fn canonical_kind(k: &str) -> Option<&'static str> {
    match k.trim().to_ascii_lowercase().as_str() {
        "string" | "text" | "str" | "varchar" => Some("string"),
        "integer" | "int" | "bigint" => Some("integer"),
        "number" | "float" | "decimal" | "double" | "numeric" | "money" | "currency" => Some("number"),
        "boolean" | "bool" => Some("boolean"),
        "date" => Some("date"),
        "datetime" | "timestamp" | "date-time" => Some("datetime"),
        _ => None,
    }
}

fn fits(kind: &str, vals: &[&str]) -> bool {
    match kind {
        "string" => true,
        "boolean" => vals.iter().all(|v| is_bool_word(v)) || vals.iter().all(|v| *v == "0" || *v == "1"),
        "date" => vals.iter().all(|v| is_iso_date(v)),
        "datetime" => vals.iter().all(|v| is_iso_datetime(v) || is_iso_date(v)),
        "integer" => vals.iter().all(|v| WHOLE.is_match(v)),
        "number" => vals.iter().all(|v| decimal(v).is_some()),
        _ => false,
    }
}

fn id_like(field: &str) -> bool {
    let w = words(field);
    w == ["id"] || (w.len() >= 2 && matches!(w.last().map(|s| s.as_str()), Some("id" | "key" | "number" | "no" | "code")))
}

// ---------------------------------------------------------------------------
// The draft
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Field {
    /// The file's exact header.
    pub header: String,
    pub name: String,
    pub kind: String,
    pub money: bool,
    pub key: bool,
    pub unit: Option<String>,
    /// A `$` was seen, which could be USD, CAD or AUD.
    pub dollar: bool,
    /// The agent's description, unless it quoted a value from the file.
    pub description: Option<String>,
    /// Whether the agent's own words for this column mention the key's grain ("line"). Wording only.
    grain_word: bool,
}

#[derive(Debug, Clone)]
pub struct Draft {
    pub entity: Option<String>,
    pub table: String,
    pub description: Option<String>,
    pub fields: Vec<Field>,
    pub key: Option<usize>,
    pub domain: Option<String>,
    pub notes: Vec<String>,
    pub left_out: Vec<String>,
}

impl Draft {
    pub fn key_name(&self) -> Option<&str> {
        self.key.map(|k| self.fields[k].name.as_str())
    }

    /// "one row per line (key line_id)".
    pub fn grain(&self) -> String {
        match self.key_name() {
            Some(k) => format!("one row per {} (key {})", grain_noun(k, self.entity.as_deref().unwrap_or("row")), k),
            None => "no column identifies one row yet".into(),
        }
    }
}

/// `line_id` → "line"; `id` → the entity's words.
fn grain_noun(key: &str, entity: &str) -> String {
    let mut w = words(key);
    if w.len() >= 2 && matches!(w.last().map(|s| s.as_str()), Some("id" | "key" | "number" | "no" | "code")) {
        w.pop();
        w.join(" ")
    } else {
        words(entity).join(" ")
    }
}

/// Example values a description must not quote: 3 characters or more, not a plain
/// true/false, not a column name, and not a value the user named in `exclude`.
fn sensitive_values(input: &FileInput, names: &[String], allow: &[String]) -> Vec<String> {
    let headers: HashSet<String> = input.columns.iter().map(|c| c.name.to_lowercase()).chain(names.iter().map(|n| n.to_lowercase())).collect();
    let mut out: Vec<String> = Vec::new();
    for c in &input.columns {
        for v in &c.values {
            if v.chars().count() >= 3 && !is_bool_word(v) && !headers.contains(&v.to_lowercase()) && !allow.iter().any(|a| a == v) && !out.contains(v) {
                out.push(v.clone());
            }
        }
    }
    out
}

fn quotes_a_value(text: &str, values: &[String]) -> bool {
    values.iter().any(|v| text.contains(v.as_str()))
}

/// What Lorefile would save for this file, by fixed rules.
pub fn draft(ont: &Ontology, input: &FileInput) -> Result<Draft, String> {
    let mut notes = Vec::new();
    let mut left_out = Vec::new();
    let from_file = names_from_file(&input.file_name);
    let entity = input.entity.clone().or_else(|| from_file.as_ref().map(|(e, _)| e.clone()));
    let table = match &from_file {
        Some((_, t)) if !t.is_empty() => t.clone(),
        _ => entity.as_deref().map(snake).unwrap_or_default(),
    };
    let headers: Vec<String> = input.columns.iter().map(|c| c.name.clone()).collect();
    let names = field_names(&headers, &mut notes);
    let answers = input.answers.as_ref();
    let currency = answers.and_then(|a| nonempty(&a.currency)).map(|c| c.to_uppercase());
    let allow: Vec<String> = answers.map(|a| a.exclude.iter().map(|x| example_text(&x.equals)).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let sensitive = sensitive_values(input, &names, &allow);
    // the key's grain word ("line" for line_id), to word the exclude question
    let key_hint = input.key.clone();

    let mut fields = Vec::new();
    for (i, c) in input.columns.iter().enumerate() {
        let name = &names[i];
        let vals: Vec<&str> = c.values.iter().map(|s| s.as_str()).filter(|s| !is_empty_value(s)).collect();
        let (guessed, gnote) = guess_type(name, &vals);
        let mut kind = guessed;
        let mut said_integer = false;
        if let Some(k) = &c.kind {
            match canonical_kind(k) {
                Some(ck) if fits(ck, &vals) => {
                    kind = ck;
                    said_integer = ck == "integer";
                }
                Some(ck) => notes.push(format!("`{}`: {} does not fit the examples, saved as {}", name, ck, guessed)),
                None => notes.push(format!("`{}`: only string, integer, number, boolean, date and datetime are used for file columns, saved as {}", name, guessed)),
            }
        }
        if kind == "string" {
            if let Some(n) = gnote {
                notes.push(n);
            }
        }
        let symbols: HashSet<&str> = vals.iter().filter_map(|v| money_symbol(v)).collect();
        let money = (kind == "number" || (kind == "integer" && !said_integer)) && (money_name(name) || !symbols.is_empty());
        if money {
            kind = "number";
        }
        let dollar = symbols.contains("$");
        let symbol_unit = if symbols.len() == 1 { symbols.iter().next().filter(|s| **s != "$").map(|s| s.to_string()) } else { None };
        let unit = c.unit.clone().or_else(|| if money { currency.clone().or(symbol_unit) } else { None });
        let description = match &c.description {
            Some(d) if quotes_a_value(d, &sensitive) => {
                left_out.push(format!("the description of `{}`: it quotes a value from the file (say what a column is, never its values)", name));
                None
            }
            other => other.clone(),
        };
        fields.push(Field { header: c.name.clone(), name: name.clone(), kind: kind.to_string(), money, key: false, unit, dollar, description, grain_word: false });
    }

    let description = match &input.description {
        Some(d) if quotes_a_value(d, &sensitive) => {
            left_out.push("the entity's description: it quotes a value from the file".into());
            None
        }
        other => other.clone(),
    };

    let key = pick_key(input, &fields, key_hint.as_deref())?;
    if let Some(k) = key {
        fields[k].key = true;
    }
    // does the agent's description of a flag column speak of the key's grain ("line")?
    if let Some(k) = key {
        let noun = grain_noun(&fields[k].name, entity.as_deref().unwrap_or(""));
        if !noun.is_empty() && noun != "row" {
            let re = Regex::new(&format!(r"(?i)\b{}s?\b", regex::escape(&noun))).ok();
            for (i, c) in input.columns.iter().enumerate() {
                if let (Some(re), Some(d)) = (&re, &c.description) {
                    fields[i].grain_word = re.is_match(d);
                }
            }
        }
    }
    let domain = if ont.manifest.domains.len() == 1 { Some(ont.manifest.domains[0].name.clone()) } else { None };
    Ok(Draft { entity, table, description, fields, key, domain, notes, left_out })
}

impl Field {
    pub fn numeric(&self) -> bool {
        self.kind == "number" || self.kind == "integer"
    }
}

/// The key: the agent's, else a column the counts prove unique, else an id-like column
/// whose examples all differ, else none (and the key question).
fn pick_key(input: &FileInput, fields: &[Field], agent: Option<&str>) -> Result<Option<usize>, String> {
    if let Some(k) = agent {
        return fields
            .iter()
            .position(|f| f.name == k || f.header == k)
            .or_else(|| fields.iter().position(|f| normalize_header(&f.header) == normalize_header(k)))
            .map(Some)
            .ok_or_else(|| format!("`key` must be one of the file's columns; `{}` is not", k));
    }
    let unique_by_counts = |i: usize| -> bool {
        let c = &input.columns[i];
        matches!((input.row_count, c.distinct, c.empty), (Some(n), Some(d), Some(0)) if n > 0 && d == n)
    };
    let counted: Vec<usize> = (0..fields.len()).filter(|&i| unique_by_counts(i)).collect();
    if let Some(&i) = counted.iter().find(|&&i| id_like(&fields[i].name)) {
        return Ok(Some(i));
    }
    if let Some(&i) = counted.first() {
        return Ok(Some(i));
    }
    for (i, f) in fields.iter().enumerate() {
        let c = &input.columns[i];
        if !id_like(&f.name) || c.values.len() < 2 || c.values.iter().any(|v| is_empty_value(v)) || matches!(c.distinct, Some(d) if Some(d) != input.row_count && input.row_count.is_some()) {
            continue;
        }
        let set: HashSet<&String> = c.values.iter().collect();
        if set.len() == c.values.len() {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Questions
// ---------------------------------------------------------------------------

const RESULT_WORDS: [&str; 8] = ["revenue", "sales", "gmv", "net", "amount", "total", "value", "spend"];
const EXCLUDE_WORDS: [&str; 16] = ["refund", "refunded", "cancel", "cancelled", "canceled", "return", "returned", "void", "voided", "test", "fraud", "chargeback", "duplicate", "deleted", "rejected", "failed"];

fn humanize(field: &str) -> String {
    let n = normalize_header(field);
    let n = n.strip_prefix("is_").or_else(|| n.strip_prefix("has_")).unwrap_or(&n).to_string();
    n.replace('_', " ")
}

fn suggested_result(d: &Draft) -> Option<&Field> {
    let money: Vec<&Field> = d.fields.iter().filter(|f| f.money).collect();
    for w in RESULT_WORDS {
        if let Some(f) = money.iter().find(|f| words(&f.name).iter().any(|x| x == w)) {
            return Some(f);
        }
    }
    money.last().copied().or_else(|| d.fields.iter().rfind(|f| f.numeric() && !f.key))
}

fn flag_column(d: &Draft) -> Option<&Field> {
    let flags: Vec<&Field> = d.fields.iter().filter(|f| f.kind == "boolean").collect();
    flags.iter().find(|f| words(&f.name).iter().any(|w| EXCLUDE_WORDS.contains(&w.as_str()))).copied().or_else(|| flags.first().copied())
}

/// At most three, only when needed, in this order: the result, what does not count, the
/// currency, the key. With all four, the currency waits: a number can be saved without a unit.
pub fn questions(d: &Draft) -> Vec<Json> {
    let mut q = Vec::new();
    match suggested_result(d) {
        Some(f) => q.push(json!({ "id": "success", "ask": format!("Which number in this file is the result you care about: {}?", f.name), "suggested": { "column": f.name, "how": "sum" } })),
        None => {
            let mut j = json!({ "id": "success", "ask": "Which number in this file is the result you care about, or is it how many rows there are?" });
            if let Some(k) = d.key_name() {
                j["suggested"] = json!({ "column": k, "how": "count_distinct" });
            }
            q.push(j);
        }
    }
    if let Some(f) = flag_column(d) {
        let rows = match d.key_name() {
            Some(k) if f.grain_word => {
                let noun = grain_noun(k, d.entity.as_deref().unwrap_or(""));
                if noun.is_empty() { "rows".to_string() } else { format!("{}s", noun) }
            }
            _ => "rows".to_string(),
        };
        q.push(json!({ "id": "exclude", "ask": format!("Do {} {} count?", humanize(&f.name), rows), "suggested": [{ "column": f.name, "equals": true }] }));
    }
    let unpriced: Vec<&Field> = d.fields.iter().filter(|f| f.money && f.unit.is_none()).collect();
    let currency = (!unpriced.is_empty()).then(|| {
        let names: Vec<&str> = unpriced.iter().map(|f| f.name.as_str()).collect();
        let list = match names.len() {
            1 => names[0].to_string(),
            n => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
        };
        let mut j = json!({ "id": "currency", "ask": format!("Which currency {} {} in?", if names.len() == 1 { "is" } else { "are" }, list) });
        if unpriced.iter().any(|f| f.dollar) {
            j["suggested"] = json!("USD");
            j["ask"] = json!(format!("Which currency {} {} in: US dollars?", if names.len() == 1 { "is" } else { "are" }, list));
        }
        j
    });
    let key = d.key.is_none().then(|| json!({ "id": "key", "ask": "Which column identifies one row? None of the columns looks like a unique identifier." }));
    match (currency, key) {
        (Some(c), None) => q.push(c),
        (None, Some(k)) => q.push(k),
        (Some(c), Some(k)) => {
            if q.len() < 2 {
                q.push(c);
            }
            q.push(k);
        }
        (None, None) => {}
    }
    q.truncate(3);
    q
}

// ---------------------------------------------------------------------------
// Recognising a file again
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Changed {
    pub entity: String,
    pub shared: usize,
    /// Indices of the file's columns the entity does not have.
    pub added: Vec<usize>,
    /// Fields of the entity the file does not have.
    pub missing: Vec<String>,
    /// (known field, index of the file column that may be it under a new name).
    pub renamed: Vec<(String, usize)>,
    /// Saved metrics that read a missing column: (metric, the missing columns it reads).
    pub at_risk: Vec<(String, Vec<String>)>,
    /// Other entities that share as many columns: the user picks.
    pub also: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Match {
    Same { entity: String, also: Vec<String> },
    Changed(Changed),
    Waiting { proposal: Box<Proposal>, exact: bool },
    New,
}

/// (field, normalized column) for a bound entity: the column is the binding's mapping, else the field's name.
fn entity_columns(t: &TypeDef) -> Vec<(String, String)> {
    let cols = t.source.as_ref().map(|s| &s.columns);
    t.properties.keys().map(|p| (p.clone(), normalize_header(cols.and_then(|m| m.get(p)).map(|s| s.as_str()).unwrap_or(p)))).collect()
}

fn qualifies(shared: usize, a: usize, b: usize) -> bool {
    shared >= 3 && shared * 5 >= 3 * a.max(b)
}

pub fn same_headers(a: &[String], b: &[String]) -> bool {
    let x: HashSet<String> = a.iter().map(|h| normalize_header(h)).collect();
    let y: HashSet<String> = b.iter().map(|h| normalize_header(h)).collect();
    x == y
}

/// The metrics whose recipe reads this entity's binding: the same connection and table (or tool).
pub fn bound_metrics<'o>(ont: &'o Ontology, t: &TypeDef) -> Vec<&'o MetricDef> {
    let Some(b) = &t.source else { return vec![] };
    ont.metrics
        .values()
        .filter(|m| m.source.as_ref().map(|s| s.connection == b.connection && ((!b.table.is_empty() && s.table == b.table) || (!b.tool.is_empty() && s.tool == b.tool))).unwrap_or(false))
        .collect()
}

static IDENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap());

fn reads_column(m: &MetricDef, col: &str) -> bool {
    let Some(s) = &m.source else { return false };
    let text = format!("{} {}", s.filter, s.measure);
    IDENT.find_iter(&text).any(|w| w.as_str().eq_ignore_ascii_case(col))
}

/// The normalized headers of the file, in order.
/// `learn_file`'s input as it can be stored for a replay: each column's name with the type,
/// unit and description the draft settled on, plus the entity, key and answers. Never the
/// examples, and never a description that quoted one (the draft already left those out).
pub fn replay_input(ont: &Ontology, input: &FileInput, args: &Json) -> Json {
    let d = draft(ont, input).ok();
    let columns: Vec<Json> = input
        .columns
        .iter()
        .map(|c| {
            let f = d.as_ref().and_then(|d| d.fields.iter().find(|f| f.header == c.name));
            let mut j = json!({ "name": c.name });
            if let Some(f) = f {
                j["type"] = json!(f.kind);
                if let Some(u) = &f.unit {
                    j["unit"] = json!(u);
                }
                if let Some(desc) = &f.description {
                    j["description"] = json!(desc);
                }
            }
            j
        })
        .collect();
    let mut out = json!({ "file_name": input.file_name, "columns": columns });
    let key = d.as_ref().and_then(|d| d.key_name().map(|k| k.to_string())).or_else(|| input.key.clone());
    if let Some(k) = key {
        out["key"] = json!(k);
    }
    if let Some(e) = &input.entity {
        out["entity"] = json!(e);
    }
    if let Some(desc) = d.as_ref().and_then(|d| d.description.clone()) {
        out["description"] = json!(desc);
    }
    // the answers are what the user said; the values they name (`exclude.equals`) are written into the metric anyway
    if let Some(a) = args.get("answers") {
        out["answers"] = a.clone();
    }
    out
}

pub fn file_headers(input: &FileInput) -> Vec<String> {
    input.columns.iter().map(|c| normalize_header(&c.name)).collect()
}

/// Same, changed, waiting or new: by the column names alone, never the file name.
/// Every bound entity is compared, not only learned ones; then the learned proposals nobody has accepted yet.
pub fn match_shape(ont: &Ontology, pending: &[Proposal], input: &FileInput) -> Match {
    let file = file_headers(input);
    let fset: HashSet<&str> = file.iter().map(|s| s.as_str()).collect();
    struct Cand {
        same: bool,
        shared: usize,
        entity: String,
    }
    let mut cands: Vec<Cand> = Vec::new();
    for t in ont.types.values().filter(|t| t.source.is_some()) {
        let ec = entity_columns(t);
        let eset: HashSet<&str> = ec.iter().map(|(_, c)| c.as_str()).collect();
        let shared = fset.intersection(&eset).count();
        let same = fset == eset;
        if same || qualifies(shared, fset.len(), eset.len()) {
            cands.push(Cand { same, shared, entity: t.name.clone() });
        }
    }
    cands.sort_by(|a, b| b.same.cmp(&a.same).then(b.shared.cmp(&a.shared)).then(a.entity.cmp(&b.entity)));
    let also = |c: &Cand, all: &[Cand]| -> Vec<String> { all.iter().skip(1).filter(|o| o.same == c.same && o.shared == c.shared).map(|o| o.entity.clone()).collect() };

    // the learned proposals waiting for a person (newest first)
    let mut best_pending: Option<(&Proposal, bool, usize)> = None;
    for p in pending {
        let Some(l) = &p.learned else { continue };
        let lset: HashSet<String> = l.headers.iter().map(|h| normalize_header(h)).collect();
        let lref: HashSet<&str> = lset.iter().map(|s| s.as_str()).collect();
        let shared = fset.intersection(&lref).count();
        let exact = lref == fset;
        if exact || qualifies(shared, fset.len(), lref.len()) {
            let better = match &best_pending {
                None => true,
                Some((_, e, s)) => (exact && !*e) || (exact == *e && shared > *s),
            };
            if better {
                best_pending = Some((p, exact, shared));
            }
        }
    }

    if let Some(c) = cands.first().filter(|c| c.same) {
        return Match::Same { entity: c.entity.clone(), also: also(c, &cands) };
    }
    if let Some((p, true, _)) = best_pending {
        return Match::Waiting { proposal: Box::new(p.clone()), exact: true };
    }
    if let Some(c) = cands.first() {
        let t = ont.type_(&c.entity).expect("a candidate is a type");
        let ec = entity_columns(t);
        let eset: HashSet<&str> = ec.iter().map(|(_, c)| c.as_str()).collect();
        let added: Vec<usize> = file.iter().enumerate().filter(|(_, h)| !eset.contains(h.as_str())).map(|(i, _)| i).collect();
        let missing: Vec<String> = ec.iter().filter(|(_, c)| !fset.contains(c.as_str())).map(|(f, _)| f.clone()).collect();
        // one added and one missing column of the same type: possibly a rename
        let added_kind = |i: usize| -> &'static str {
            let vals: Vec<&str> = input.columns[i].values.iter().map(|s| s.as_str()).filter(|s| !is_empty_value(s)).collect();
            guess_type(&input.columns[i].name, &vals).0
        };
        let field_kind = |f: &str| -> String {
            let k = t.properties.get(f).map(|p| p.normalize().kind).unwrap_or_default();
            canonical_kind(&k).unwrap_or("string").to_string()
        };
        let mut renamed = Vec::new();
        for kind in ["string", "integer", "number", "boolean", "date", "datetime"] {
            let a: Vec<usize> = added.iter().copied().filter(|&i| { let k = added_kind(i); k == kind || (kind == "number" && k == "integer") }).collect();
            let m: Vec<&String> = missing.iter().filter(|f| field_kind(f) == kind).collect();
            if a.len() == 1 && m.len() == 1 && !renamed.iter().any(|(_, i)| *i == a[0]) {
                renamed.push((m[0].clone(), a[0]));
            }
        }
        let cols = t.source.as_ref().map(|s| s.columns.clone()).unwrap_or_default();
        let at_risk: Vec<(String, Vec<String>)> = bound_metrics(ont, t)
            .iter()
            .filter_map(|m| {
                let gone: Vec<String> = missing.iter().filter(|f| reads_column(m, f) || cols.get(*f).map(|c| reads_column(m, c)).unwrap_or(false)).cloned().collect();
                (!gone.is_empty()).then(|| (m.name.clone(), gone))
            })
            .collect();
        return Match::Changed(Changed { entity: c.entity.clone(), shared: c.shared, added, missing, renamed, at_risk, also: also(c, &cands) });
    }
    if let Some((p, false, _)) = best_pending {
        return Match::Waiting { proposal: Box::new(p.clone()), exact: false };
    }
    Match::New
}

// ---------------------------------------------------------------------------
// How a known file is read
// ---------------------------------------------------------------------------

/// "accepted by Vinay on 27 Nov", "UNVERIFIED: …", or None when nothing is graded.
fn grade_phrase(ont: &Ontology, k: TKind, name: &str) -> Option<String> {
    if !ont.graded.has_file {
        return None;
    }
    let st = ont.grade_of(k, name);
    let r = st.record.as_ref()?;
    if st.changed {
        return st.say(k);
    }
    Some(match r.grade {
        crate::schema::Grade::Proposed => "UNVERIFIED: an agent proposed this definition and no person has reviewed it; say so before the number".to_string(),
        crate::schema::Grade::Accepted => format!("accepted by {} on {}", trust::who_said(&r.by), trust::short_date(&r.at)),
        crate::schema::Grade::Checked => format!("checked against {} by {} on {}", r.against.as_deref().unwrap_or("a source"), trust::who_said(&r.by), trust::short_date(&r.at)),
    })
}

/// Every metric that reads this entity's binding, with its recipe and who stands behind it.
pub fn read_it(ont: &Ontology, t: &TypeDef) -> Vec<Json> {
    bound_metrics(ont, t)
        .iter()
        .map(|m| {
            let s = m.source.clone().unwrap_or_default();
            let mut j = json!({ "metric": m.name, "measure": s.measure });
            if !s.filter.is_empty() {
                j["filter"] = json!(s.filter);
            }
            if !m.caveats.is_empty() {
                j["caveats"] = json!(m.caveats);
            }
            if let Some(b) = m.basis {
                j["basis"] = json!(b.word());
            }
            if ont.graded.has_file {
                j["trust"] = ont.grade_of(TKind::Metric, &m.name).to_json(TKind::Metric);
            }
            j
        })
        .collect()
}

fn recipe_words(m: &Json) -> String {
    let measure = m["measure"].as_str().unwrap_or("");
    match m["filter"].as_str() {
        Some(f) if !f.is_empty() => format!("{} over rows where {}", measure, f),
        _ => measure.to_string(),
    }
}

fn first_sentence(s: &str) -> String {
    let s = s.trim();
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len());
    s[..end].trim_end_matches('.').to_string()
}

/// Where a known entity's rows come from, in words.
fn source_words(ont: &Ontology, t: &TypeDef) -> String {
    let Some(b) = &t.source else { return "no binding".into() };
    let kind = ont.sources.get(&b.connection).map(|s| s.kind.clone()).unwrap_or_default();
    if is_from_user(&kind) {
        format!("{} (the file is usually called {}…)", b.connection, b.table)
    } else if !b.tool.is_empty() {
        format!("{} ({}), tool {}", b.connection, kind, b.tool)
    } else {
        format!("{} ({}), table {}", b.connection, kind, b.table)
    }
}

fn say_known(ont: &Ontology, t: &TypeDef, read: &[Json]) -> String {
    let kind = t.source.as_ref().and_then(|b| ont.sources.get(&b.connection)).map(|s| s.kind.clone()).unwrap_or_default();
    let what = if t.description.trim().is_empty() { t.name.clone() } else { format!("{} ({})", t.name, first_sentence(&t.description)) };
    let opening = if is_from_user(&kind) { format!("This is {}.", what) } else { format!("This file has the columns of {}, whose rows live in {}.", what, source_words(ont, t)) };
    let result = if read.is_empty() {
        "Nothing saved says which number counts for it yet: ask the user, then call learn_file with `answers` to save it.".to_string()
    } else {
        let parts: Vec<String> = read
            .iter()
            .map(|m| {
                let name = m["metric"].as_str().unwrap_or("");
                let grade = grade_phrase(ont, TKind::Metric, name).map(|g| format!(" ({})", g)).unwrap_or_default();
                let caveat = m["caveats"].as_str().map(|c| format!(" {}", c)).unwrap_or_default();
                format!("{}: {}{}.{}", name, recipe_words(m), grade, caveat)
            })
            .collect();
        format!("{} {}", if parts.len() == 1 { "Its result is" } else { "Its results are" }, parts.join(" "))
    };
    let close = if is_from_user(&kind) { "Compute it from the file with your own tools; Lorefile does not have the rows." } else { "Compute it from the file with your own tools." };
    format!("{} {} {}", opening, result, close)
}

fn field_json(f: &Field) -> Json {
    let mut j = json!({ "name": f.name, "type": f.kind });
    if f.header != f.name {
        j["column"] = json!(f.header);
    }
    if f.key {
        j["key"] = json!(true);
    }
    if f.money {
        j["money"] = json!(true);
    }
    if let Some(u) = &f.unit {
        j["unit"] = json!(u);
    }
    if f.description.is_none() {
        j["described"] = json!(false);
    }
    j
}

pub fn from_user_source(ont: &Ontology) -> Option<String> {
    if ont.sources.get(SOURCE).map(|s| is_from_user(&s.kind)).unwrap_or(false) {
        return Some(SOURCE.to_string());
    }
    ont.sources.values().find(|s| is_from_user(&s.kind)).map(|s| s.connection.clone())
}

const NEXT: &str = "Ask the user these questions in your own words, then call learn_file with the same columns, one short description per column (what it is and its unit; never quote values from the file), and `answers`. Lorefile saves the file's shape and meaning, never its rows.";

/// The answer `match_file` gives (and `learn_file` without `answers`).
pub fn match_reply(ont: &Ontology, input: &FileInput, m: &Match, waiting: Option<&Ontology>) -> Result<Json, String> {
    let n = input.columns.len();
    Ok(match m {
        Match::Same { entity, also } => {
            let t = ont.type_(entity).ok_or("the matched entity is gone")?;
            let read = read_it(ont, t);
            let mut j = json!({ "match": "same", "entity": t.name, "columns": { "in_file": n, "known": n }, "source": source_words(ont, t) });
            if ont.graded.has_file {
                j["trust"] = ont.grade_of(TKind::Binding, &t.name).to_json(TKind::Binding);
            }
            if let Some(b) = &t.source {
                if !b.columns.is_empty() {
                    j["column_names"] = json!(b.columns);
                }
            }
            j["read_it"] = json!(read);
            let guards: Vec<Json> = ont.constraints_for(&t.name).iter().map(|c| json!({ "id": c.id, "name": c.name, "severity": c.severity.to_string() })).collect();
            if !guards.is_empty() {
                j["guardrails"] = json!(guards);
            }
            if !also.is_empty() {
                j["also_matches"] = json!(also);
            }
            let mut say = say_known(ont, t, &read);
            if !also.is_empty() {
                say = format!("{} It also has the columns of {}: ask the user which one this file is.", say, also.join(", "));
            }
            j["say"] = json!(say);
            j
        }
        Match::Changed(c) => {
            let t = ont.type_(&c.entity).ok_or("the matched entity is gone")?;
            let d = draft(ont, input)?;
            let read = read_it(ont, t);
            let renamed_cols: Vec<usize> = c.renamed.iter().map(|(_, i)| *i).collect();
            let added: Vec<Json> = c.added.iter().map(|&i| { let mut f = d.fields[i].clone(); f.key = false; field_json(&f) }).collect();
            let renamed: Vec<Json> = c.renamed.iter().map(|(f, i)| json!({ "from": f, "to": input.columns[*i].name })).collect();
            let risk: Vec<Json> = c.at_risk.iter().map(|(m, cols)| json!({ "metric": m, "reads": cols })).collect();
            let mut j = json!({ "match": "changed", "entity": t.name, "shared": c.shared, "added": added, "missing": c.missing, "possibly_renamed": renamed, "metrics_at_risk": risk, "read_it": read });
            if !c.also.is_empty() {
                j["also_matches"] = json!(c.also);
            }
            let mut say = Vec::new();
            for (m, cols) in &c.at_risk {
                let recipe = read.iter().find(|r| r["metric"] == json!(m)).map(recipe_words).unwrap_or_default();
                say.push(format!(
                    "The saved result {} is {}, and this file has no {} column. Ask the user how that shows now before giving a number.",
                    m,
                    recipe,
                    cols.join(" or ")
                ));
            }
            let new_names: Vec<String> = c.added.iter().filter(|i| !renamed_cols.contains(i)).map(|&i| d.fields[i].name.clone()).collect();
            let plural = |n: usize, one: &str, many: &str| if n == 1 { one.to_string() } else { format!("{} {}", n, many) };
            let mut parts = Vec::new();
            if !c.added.is_empty() {
                parts.push(format!("{} ({})", plural(c.added.len(), "one new column", "new columns"), c.added.iter().map(|&i| d.fields[i].name.clone()).collect::<Vec<_>>().join(", ")));
            }
            if !c.missing.is_empty() {
                parts.push(format!("{} ({})", plural(c.missing.len(), "one gone", "gone"), c.missing.join(", ")));
            }
            say.push(format!("This is {} with {}.", t.name, parts.join(" and ")));
            if c.at_risk.is_empty() && !c.missing.is_empty() {
                say.push(format!("Nothing saved reads {}.", c.missing.join(" or ")));
            }
            for (f, i) in &c.renamed {
                say.push(format!("{} may be {} under a new name: ask the user, and if so pass answers.renamed {{\"{}\": \"{}\"}} to learn_file.", input.columns[*i].name, f, f, input.columns[*i].name));
            }
            if !new_names.is_empty() {
                say.push(format!("To save {}, call learn_file{}.", new_names.join(", "), if c.missing.is_empty() { String::new() } else { format!("; {} stay{} in Lorefile unless a person removes {}", c.missing.join(", "), if c.missing.len() == 1 { "s" } else { "" }, if c.missing.len() == 1 { "it" } else { "them" }) }));
            }
            if c.at_risk.is_empty() && !read.is_empty() {
                say.push(format!("{} Compute it from the file with your own tools.", say_known(ont, t, &read).split(" Compute it").next().unwrap_or("").split(". ").skip(1).collect::<Vec<_>>().join(". ")));
            }
            if !c.also.is_empty() {
                say.push(format!("It also shares as many columns with {}: ask the user which one this file is.", c.also.join(", ")));
            }
            j["say"] = json!(say.join(" ").replace("  ", " "));
            if !d.notes.is_empty() {
                j["notes"] = json!(d.notes);
            }
            j
        }
        Match::Waiting { proposal, exact } => {
            let l = proposal.learned.clone().unwrap_or_default();
            let day = trust::short_date(proposal.created_at.get(..10).unwrap_or(""));
            let waiting_sentence = format!("saved for review on {} and not accepted yet: accept it in Lorefile (Agents)", day);
            let read: Vec<Json> = waiting
                .and_then(|w| w.type_(&l.entity).map(|t| (w, t)))
                .map(|(w, t)| {
                    read_it(w, t)
                        .into_iter()
                        .map(|mut r| {
                            r["trust"] = json!({ "grade": "waiting", "proposal": proposal.id, "say": waiting_sentence });
                            r
                        })
                        .collect()
                })
                .unwrap_or_default();
            let recipe = if read.is_empty() {
                "It says nothing yet about which number counts.".to_string()
            } else {
                format!("It reads the result as {}.", read.iter().map(|r| format!("{} = {}", r["metric"].as_str().unwrap_or(""), recipe_words(r))).collect::<Vec<_>>().join("; "))
            };
            let mut j = json!({
                "match": "waiting",
                "entity": l.entity,
                "proposal": proposal.id,
                "saved_on": proposal.created_at,
                "same_columns": exact,
                "read_it": read,
                "say": format!("This is {} as it was described on {}: {}. {} Say it has not been accepted yet, then compute it from the file with your own tools; Lorefile does not have the rows.", l.entity, day, waiting_sentence, recipe),
            });
            if !exact {
                j["note"] = json!("the columns differ a little from the waiting description; learn_file with this file replaces it");
            }
            j
        }
        Match::New => {
            let d = draft(ont, input)?;
            let source = match from_user_source(ont) {
                Some(s) => format!("{} (already in this space), file usually called {}", s, d.table),
                None => format!("{} (made on first use), file usually called {}", SOURCE, d.table),
            };
            let mut draft_j = json!({ "entity": d.entity, "grain": d.grain(), "fields": d.fields.iter().map(field_json).collect::<Vec<_>>(), "source": source });
            let mut notes = d.notes.clone();
            match &d.entity {
                None => notes.push("no entity name could be made from the file name: pass `entity` to learn_file".into()),
                Some(e) if ont.type_(e).is_some() => notes.push(format!("{} exists with other columns: pass another `entity` to learn_file", e)),
                _ => {}
            }
            if d.entity.is_some() {
                draft_j["table"] = json!(d.table);
            }
            let mut j = json!({ "match": "new", "draft": draft_j, "questions": questions(&d), "next": NEXT });
            if !notes.is_empty() {
                j["notes"] = json!(notes);
            }
            if !d.left_out.is_empty() {
                j["left_out"] = json!(d.left_out);
            }
            j
        }
    })
}

// ---------------------------------------------------------------------------
// The plan: ops for patch::plan_many, one proposal
// ---------------------------------------------------------------------------

/// What `learn_file` saves.
pub enum Plan {
    /// Nothing to save, and why, in words for the agent.
    Nothing(String),
    Ops(Planned),
}

pub struct Planned {
    pub ops: Vec<Json>,
    /// "learned promo_sales.csv: PromoSale (14 fields, key line_id) from user_files; metric …"
    pub line: String,
    pub learned: Learned,
    pub entity: String,
    pub notes: Vec<String>,
    pub left_out: Vec<String>,
    pub fields: usize,
    pub described: usize,
    /// A saved metric already reads this entity's rows, and this change adds none.
    pub recipe_kept: bool,
}

/// A column the metric may read: the field's name, its type and the file's header.
struct Known {
    name: String,
    kind: String,
    header: String,
}

fn find<'k>(known: &'k [Known], col: &str) -> Option<&'k Known> {
    known.iter().find(|k| k.name == col || k.header == col).or_else(|| known.iter().find(|k| normalize_header(&k.header) == normalize_header(col) || normalize_header(&k.name) == normalize_header(col)))
}

fn sql_text(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// One `exclude` item → (the filter keeping the rows that count, words for the description).
fn exclusion(k: &Known, equals: &Json) -> Result<(String, String), String> {
    match k.kind.as_str() {
        "boolean" => {
            let v = match equals {
                Json::Bool(b) => Some(*b),
                Json::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                    "true" | "yes" | "1" => Some(true),
                    "false" | "no" | "0" => Some(false),
                    _ => None,
                },
                Json::Number(n) => n.as_i64().and_then(|i| match i { 1 => Some(true), 0 => Some(false), _ => None }),
                _ => None,
            };
            let v = v.ok_or_else(|| format!("answers.exclude: `equals` for {} must be true or false", k.name))?;
            Ok((format!("{} = {}", k.name, !v), format!("{} is {}", k.name, v)))
        }
        "integer" | "number" => {
            let n = match equals {
                Json::Number(n) => Some(n.to_string()),
                Json::String(s) => decimal(s.trim()).map(|_| s.trim().to_string()),
                _ => None,
            };
            let n = n.ok_or_else(|| format!("answers.exclude: `equals` for {} must be a number", k.name))?;
            Ok((format!("{} <> {}", k.name, n), format!("{} is {}", k.name, n)))
        }
        _ => {
            let s = match equals {
                Json::String(s) if !s.trim().is_empty() => s.trim().to_string(),
                Json::Number(n) => n.to_string(),
                Json::Bool(b) => b.to_string(),
                _ => return Err(format!("answers.exclude: `equals` for {} must be the value that does not count", k.name)),
            };
            Ok((format!("{} <> {}", k.name, sql_text(&s)), format!("{} is {}", k.name, sql_text(&s))))
        }
    }
}

/// Where a metric's rows come from: the entity and its binding.
struct Bound<'b> {
    entity: &'b str,
    connection: &'b str,
    table: &'b str,
}

/// What a plan says back besides its ops.
struct Said<'s> {
    sensitive: &'s [String],
    notes: Vec<String>,
    left_out: Vec<String>,
}

/// The metric from the answers: its op and the words for the one-line summary.
fn metric_op(ont: &Ontology, at: &Bound, known: &[Known], a: &Answers, said: &mut Said) -> Result<Option<(Json, String)>, String> {
    let (entity, connection, table, sensitive) = (at.entity, at.connection, at.table, said.sensitive);
    let Some(s) = &a.success else {
        if !a.exclude.is_empty() {
            said.notes.push("`exclude` was not used: it says which rows do not count toward a number, and no number (`success`) was given".into());
        }
        return Ok(None);
    };
    let f = find(known, &s.column).ok_or_else(|| format!("answers.success.column `{}` is not a column of this file", s.column))?;
    let how = s.how.trim().to_ascii_lowercase();
    let (measure, head) = match how.as_str() {
        "" | "sum" => {
            if f.kind != "number" && f.kind != "integer" {
                return Err(format!("answers.success: a sum needs a number column, and {} is {}; use how: count_distinct or count", f.name, f.kind));
            }
            (format!("sum({})", f.name), format!("Sum of {} in {}", f.name, entity))
        }
        "count_distinct" => (format!("count(distinct {})", f.name), format!("Number of distinct {} in {}", f.name, entity)),
        "count" => ("count(*)".to_string(), format!("Number of rows in {}", entity)),
        _ => return Err("answers.success.how is sum, count_distinct or count".into()),
    };
    let mut conds = Vec::new();
    let mut phrases = Vec::new();
    for x in &a.exclude {
        let k = find(known, &x.column).ok_or_else(|| format!("answers.exclude: `{}` is not a column of this file", x.column))?;
        let (c, w) = exclusion(k, &x.equals)?;
        conds.push(c);
        phrases.push(w);
    }
    let filter = conds.join(" and ");
    let default_name = match how.as_str() {
        "count" => format!("{}_count", snake(entity)),
        "count_distinct" => format!("{}_{}_count", snake(entity), snake(&f.name)),
        _ => format!("{}_{}", snake(entity), snake(&f.name)),
    };
    let name = nonempty(&a.metric).unwrap_or(default_name);
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || !name.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
        return Err("answers.metric is letters, digits and underscores, like promo_sale_revenue".into());
    }
    if ont.metrics.contains_key(&name) {
        return Err(if name == "success" {
            "this space already has a `success` metric; pass another answers.metric".to_string()
        } else {
            format!("a metric named {} already exists; pass another answers.metric", name)
        });
    }
    let description = format!("{}{}.", head, if phrases.is_empty() { String::new() } else { format!(", leaving out rows where {}", phrases.join(" or ")) });
    let mut op = json!({ "op": "add_metric", "metric": name, "description": description, "connection": connection, "table": table, "measure": measure });
    if !filter.is_empty() {
        op["filter"] = json!(filter);
    }
    if let Some(w) = nonempty(&a.exclude_words) {
        if quotes_a_value(&w, sensitive) {
            said.left_out.push("the caveat (`exclude_words`): it quotes a value from the file".into());
        } else {
            op["caveats"] = json!(w);
        }
    }
    if let Some(b) = nonempty(&a.basis) {
        if !matches!(b.as_str(), "transactional" | "attributed" | "modelled" | "experimental") {
            return Err("answers.basis is transactional, attributed, modelled or experimental".into());
        }
        op["basis"] = json!(b);
    }
    let words = format!("metric {} = {}{}", name, measure, if filter.is_empty() { String::new() } else { format!(" where {}", filter) });
    Ok(Some((op, words)))
}

fn plain_word(s: &str) -> bool {
    s.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ')) && s.len() <= 80
}

fn term_ops(ont: &Ontology, entity: &str, a: &Answers, notes: &mut Vec<String>) -> Vec<Json> {
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for w in &a.also_called {
        let w = w.trim();
        if w.is_empty() || seen.iter().any(|s| s.eq_ignore_ascii_case(w)) {
            continue;
        }
        seen.push(w.to_string());
        if ont.glossary.iter().any(|g| g.term.eq_ignore_ascii_case(w)) {
            notes.push(format!("the word \"{}\" is already in the glossary; left as it is", w));
            continue;
        }
        if !plain_word(w) {
            notes.push(format!("the word \"{}\" was not saved: a glossary word uses letters, digits, spaces, `_`, `-` and `.`", w));
            continue;
        }
        out.push(json!({ "op": "add_term", "term": w, "means": entity }));
    }
    out
}

fn line_for(input: &FileInput, entity: &str, what: &str, metric: Option<&str>) -> String {
    let file = if input.file_name.is_empty() { "a file".to_string() } else { input.file_name.clone() };
    format!("learned {}: {}{}", file, if what.is_empty() { entity.to_string() } else { format!("{} {}", entity, what) }, metric.map(|m| format!("; {}", m)).unwrap_or_default())
}

/// The ops for one `learn_file` with answers, in Part 1's order: the source (first time only),
/// the entity with every column, the binding, the metric, the words. One proposal.
pub fn plan(ont: &Ontology, input: &FileInput, m: &Match) -> Result<Plan, String> {
    // a waiting proposal about an entity that already exists (a changed file, saved but not
    // accepted): plan against that entity again, so the new proposal can replace the waiting one
    let rematched;
    let m = match m {
        Match::Waiting { proposal, .. } if proposal.learned.as_ref().map(|l| ont.type_(&l.entity).is_some()).unwrap_or(false) => {
            rematched = match_shape(ont, &[], input);
            &rematched
        }
        other => other,
    };
    let empty = Answers::default();
    let a = input.answers.as_ref().unwrap_or(&empty);
    let mut d = draft(ont, input)?;
    let headers: Vec<String> = file_headers(input);
    let names: Vec<String> = d.fields.iter().map(|f| f.name.clone()).collect();
    let allow: Vec<String> = a.exclude.iter().map(|x| example_text(&x.equals)).filter(|s| !s.is_empty()).collect();
    let sensitive = sensitive_values(input, &names, &allow);
    let mut notes = std::mem::take(&mut d.notes);
    let mut left_out = std::mem::take(&mut d.left_out);
    let mut ops: Vec<Json> = Vec::new();

    match m {
        Match::Same { entity, .. } => {
            let t = ont.type_(entity).ok_or("the matched entity is gone")?;
            let b = t.source.clone().unwrap_or_default();
            if a.success.is_none() || !bound_metrics(ont, t).is_empty() {
                return Ok(Plan::Nothing(format!(
                    "Already known as {}: nothing to save. {} To change how it is read, use propose_patch.",
                    t.name,
                    if bound_metrics(ont, t).is_empty() { "Nothing says which number counts yet: pass answers.success to save one." } else { "match_file says how to read it." }
                )));
            }
            let known: Vec<Known> = t.properties.iter().map(|(p, s)| Known { name: p.clone(), kind: s.normalize().kind, header: b.columns.get(p).cloned().unwrap_or_else(|| p.clone()) }).collect();
            let mut said = Said { sensitive: &sensitive, notes: std::mem::take(&mut notes), left_out: std::mem::take(&mut left_out) };
            let metric = metric_op(ont, &Bound { entity: &t.name, connection: &b.connection, table: &b.table }, &known, a, &mut said)?;
            notes = said.notes;
            left_out = said.left_out;
            let words = metric.as_ref().map(|(_, w)| w.clone());
            if let Some((op, _)) = metric {
                ops.push(op);
            }
            ops.extend(term_ops(ont, &t.name, a, &mut notes));
            let line = line_for(input, &t.name, "(known already)", words.as_deref());
            Ok(Plan::Ops(Planned { ops, line, learned: Learned { entity: t.name.clone(), headers, file_name: input.file_name.clone() }, entity: t.name.clone(), notes, left_out, fields: 0, described: 0, recipe_kept: false }))
        }
        Match::Changed(c) => {
            let t = ont.type_(&c.entity).ok_or("the matched entity is gone")?;
            let b = t.source.clone().unwrap_or_default();
            // renames the user confirmed: an old field now arrives under a new header
            let mut renamed_cols: Vec<usize> = Vec::new();
            let mut columns = b.columns.clone();
            for (old, new) in &a.renamed {
                if !t.properties.contains_key(old) {
                    return Err(format!("answers.renamed: {} has no field `{}`", t.name, old));
                }
                let i = input
                    .columns
                    .iter()
                    .position(|col| col.name == *new || normalize_header(&col.name) == normalize_header(new))
                    .filter(|i| c.added.contains(i))
                    .ok_or_else(|| format!("answers.renamed: `{}` is not a new column of this file", new))?;
                renamed_cols.push(i);
                columns.insert(old.clone(), input.columns[i].name.clone());
            }
            if !a.renamed.is_empty() {
                let mut op = json!({ "op": "set_source", "type": t.name, "connection": b.connection, "table": b.table, "columns": columns });
                if !b.key.is_empty() {
                    op["key"] = json!(b.key);
                }
                if !b.tool.is_empty() {
                    op["tool"] = json!(b.tool);
                }
                if !b.params.is_empty() {
                    op["params"] = serde_json::to_value(&b.params).unwrap_or(Json::Null);
                }
                ops.push(op);
            }
            let mut known: Vec<Known> = t.properties.iter().map(|(p, s)| Known { name: p.clone(), kind: s.normalize().kind, header: columns.get(p).cloned().unwrap_or_else(|| p.clone()) }).collect();
            let mut added_names = Vec::new();
            let mut described = 0;
            for &i in c.added.iter().filter(|i| !renamed_cols.contains(i)) {
                let f = &d.fields[i];
                let mut name = f.name.clone();
                let mut n = 2;
                while t.properties.keys().any(|k| k.eq_ignore_ascii_case(&name)) || known.iter().any(|k| k.name.eq_ignore_ascii_case(&name)) {
                    name = format!("{}_{}", f.name, n);
                    n += 1;
                }
                let mut op = json!({ "op": "add_property", "type": t.name, "property": name, "datatype": f.kind });
                if let Some(u) = &f.unit {
                    op["unit"] = json!(u);
                }
                if let Some(desc) = &f.description {
                    op["description"] = json!(desc);
                    described += 1;
                }
                if f.header != name {
                    op["column"] = json!(f.header);
                }
                ops.push(op);
                known.push(Known { name: name.clone(), kind: f.kind.clone(), header: f.header.clone() });
                added_names.push(name);
            }
            let mut said = Said { sensitive: &sensitive, notes: std::mem::take(&mut notes), left_out: std::mem::take(&mut left_out) };
            let metric = metric_op(ont, &Bound { entity: &t.name, connection: &b.connection, table: &b.table }, &known, a, &mut said)?;
            notes = said.notes;
            left_out = said.left_out;
            let words = metric.as_ref().map(|(_, w)| w.clone());
            if let Some((op, _)) = metric {
                ops.push(op);
            }
            ops.extend(term_ops(ont, &t.name, a, &mut notes));
            if !c.missing.is_empty() {
                notes.push(format!("{} {} not in this file and stay{} in Lorefile: removing meaning is a person's call on the page", c.missing.join(", "), if c.missing.len() == 1 { "is" } else { "are" }, if c.missing.len() == 1 { "s" } else { "" }));
            }
            if ops.is_empty() {
                return Ok(Plan::Nothing(format!("Nothing to save: every column of this file is already known as part of {}. {}", t.name, notes.join(" "))));
            }
            let mut what = Vec::new();
            if !added_names.is_empty() {
                what.push(format!("+{}", added_names.join(", +")));
            }
            for (old, new) in &a.renamed {
                what.push(format!("{} now read from {}", old, new));
            }
            let line = line_for(input, &t.name, &format!("({})", if what.is_empty() { "known already".to_string() } else { what.join("; ") }), words.as_deref());
            let n = added_names.len();
            let recipe_kept = words.is_none() && !bound_metrics(ont, t).is_empty();
            Ok(Plan::Ops(Planned { ops, line, learned: Learned { entity: t.name.clone(), headers, file_name: input.file_name.clone() }, entity: t.name.clone(), notes, left_out, fields: n, described, recipe_kept }))
        }
        Match::Waiting { .. } | Match::New => {
            let entity = d.entity.clone().ok_or("pass `entity`: no name could be made from the file name")?;
            if ont.type_(&entity).is_some() {
                return Err(format!("{} exists with other columns; pass another `entity`", entity));
            }
            let connection = match from_user_source(ont) {
                Some(s) => s,
                None => {
                    if ont.sources.contains_key(SOURCE) {
                        return Err(format!("a source named {} exists and is not a `kind: {}` source; rename it first", SOURCE, KIND));
                    }
                    ops.push(json!({ "op": "add_source", "connection": SOURCE, "kind": KIND, "description": SOURCE_DESCRIPTION }));
                    SOURCE.to_string()
                }
            };
            let mut props = Map::new();
            for f in &d.fields {
                let mut p = Map::new();
                p.insert("type".into(), json!(f.kind));
                if f.key {
                    p.insert("key".into(), json!(true));
                }
                if let Some(u) = &f.unit {
                    p.insert("unit".into(), json!(u));
                }
                if let Some(desc) = &f.description {
                    p.insert("description".into(), json!(desc));
                }
                props.insert(f.name.clone(), Json::Object(p));
            }
            let mut add_type = json!({ "op": "add_type", "name": entity, "description": d.description.clone().unwrap_or_default(), "properties": props });
            if let Some(dom) = &d.domain {
                add_type["domain"] = json!(dom);
            }
            ops.push(add_type);
            let columns: Map<String, Json> = d.fields.iter().filter(|f| f.header != f.name).map(|f| (f.name.clone(), json!(f.header))).collect();
            let mut bind = json!({ "op": "set_source", "type": entity, "connection": connection, "table": d.table });
            if let Some(k) = d.key_name() {
                bind["key"] = json!(k);
            }
            if !columns.is_empty() {
                bind["columns"] = Json::Object(columns);
            }
            ops.push(bind);
            let known: Vec<Known> = d.fields.iter().map(|f| Known { name: f.name.clone(), kind: f.kind.clone(), header: f.header.clone() }).collect();
            let mut said = Said { sensitive: &sensitive, notes: std::mem::take(&mut notes), left_out: std::mem::take(&mut left_out) };
            let metric = metric_op(ont, &Bound { entity: &entity, connection: &connection, table: &d.table }, &known, a, &mut said)?;
            notes = said.notes;
            left_out = said.left_out;
            let words = metric.as_ref().map(|(_, w)| w.clone());
            if let Some((op, _)) = metric {
                ops.push(op);
            }
            ops.extend(term_ops(ont, &entity, a, &mut notes));
            if d.key.is_none() {
                notes.push("no column identifies one row, so the entity has no key; ask the user which column does and pass `key`".into());
            }
            let unpriced: Vec<&str> = d.fields.iter().filter(|f| f.money && f.unit.is_none()).map(|f| f.name.as_str()).collect();
            if !unpriced.is_empty() {
                notes.push(format!("{} {} no currency: pass answers.currency or a column's `unit`", unpriced.join(", "), if unpriced.len() == 1 { "has" } else { "have" }));
            }
            let what = format!("({} fields{}) from {}", d.fields.len(), d.key_name().map(|k| format!(", key {}", k)).unwrap_or_default(), connection);
            let line = line_for(input, &entity, &what, words.as_deref());
            let described = d.fields.iter().filter(|f| f.description.is_some()).count();
            Ok(Plan::Ops(Planned { ops, line, learned: Learned { entity: entity.clone(), headers, file_name: input.file_name.clone() }, entity, notes, left_out, fields: d.fields.len(), described, recipe_kept: false }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_ontology;
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/learn").join(name)
    }

    /// What an agent sends for a CSV: the headers, the first five values per column, and (optionally) counts.
    fn args_from_csv(path: &Path, file_name: &str, counts: bool) -> Json {
        let mut rdr = csv::Reader::from_path(path).unwrap();
        let headers: Vec<String> = rdr.headers().unwrap().iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = rdr.records().map(|r| r.unwrap().iter().map(|s| s.to_string()).collect()).collect();
        let columns: Vec<Json> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let vals: Vec<&String> = rows.iter().map(|r| &r[i]).collect();
                let mut c = json!({ "name": h, "examples": vals.iter().take(5).collect::<Vec<_>>() });
                if counts {
                    c["distinct"] = json!(vals.iter().collect::<HashSet<_>>().len());
                    c["empty"] = json!(vals.iter().filter(|v| v.is_empty()).count());
                }
                c
            })
            .collect();
        let mut a = json!({ "file_name": file_name, "columns": columns });
        if counts {
            a["row_count"] = json!(rows.len());
        }
        a
    }

    fn describe_all(a: &mut Json) {
        for c in a["columns"].as_array_mut().unwrap() {
            let d = match c["name"].as_str().unwrap() {
                "line_id" => "Identifier of one order line.",
                "order_id" => "The order this line belongs to; an order can have several lines.",
                "revenue" => "What the customer paid for the line: units × unit_price − discount.",
                "refunded" => "True when the line was refunded.",
                "unit_price" => "Price of one unit before the discount.",
                _ => "A column of the promo-sales export.",
            };
            c["description"] = json!(d);
        }
    }

    fn input(a: &Json) -> FileInput {
        parse(a).unwrap()
    }

    fn blank_space(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lorefile-learn-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("types")).unwrap();
        std::fs::write(dir.join("ontology.yaml"), "name: branda\ndomains:\n  - name: Core\n    types: []\n").unwrap();
        std::fs::write(dir.join("glossary.yaml"), "terms: []\n").unwrap();
        dir
    }

    fn the_answers() -> Json {
        json!({ "success": { "column": "revenue", "how": "sum" }, "exclude": [{ "column": "refunded", "equals": true }], "exclude_words": "Refunded lines never count.", "currency": "USD", "also_called": ["promo report"] })
    }

    /// November, learned and written into `dir` (what Accept does), returning the ontology after.
    fn learn_november(dir: &Path) -> Ontology {
        let ont = load_ontology(dir).unwrap().0;
        let mut a = args_from_csv(&fixture("promo_sales_2026-11.csv"), "promo_sales.csv", true);
        describe_all(&mut a);
        a["answers"] = the_answers();
        let i = input(&a);
        let m = match_shape(&ont, &[], &i);
        let Plan::Ops(p) = plan(&ont, &i, &m).unwrap() else { panic!("a plan") };
        let batch = crate::patch::plan_many(&ont, &p.ops).unwrap();
        for (f, t) in &batch.files {
            let path = dir.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        }
        load_ontology(dir).unwrap().0
    }

    #[test]
    fn entity_names_come_from_the_file_name_without_dates_and_copy_marks() {
        for f in ["promo_sales.csv", "promo_sales_2026-12.csv", "Promo Sales (1).csv", "promo_sales_final_v2.xlsx › Sheet1", "promo_sales-2026-11-30.csv", "promo_sales_nov.csv", "promo_sales_q4 copy.csv"] {
            assert_eq!(names_from_file(f), Some(("PromoSale".to_string(), "promo_sales".to_string())), "{}", f);
        }
        assert_eq!(names_from_file("orders.csv").unwrap().0, "Order");
        assert_eq!(names_from_file("addresses.csv").unwrap().0, "Address");
        assert_eq!(names_from_file("status.csv").unwrap().0, "Status");
        assert_eq!(names_from_file("categories.csv").unwrap().0, "Category");
        assert_eq!(names_from_file("report.xlsx › Sales"), Some(("Sale".to_string(), "report_sales".to_string())), "a named sheet says what it holds");
        assert_eq!(names_from_file("2026-11.csv"), None, "nothing left to name it by");
    }

    #[test]
    fn types_follow_fixed_rules_and_never_become_enums() {
        let a = args_from_csv(&fixture("promo_sales_2026-11.csv"), "promo_sales.csv", false);
        let ont = load_ontology(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ecommerce")).unwrap().0;
        let d = draft(&ont, &input(&a)).unwrap();
        let f = |n: &str| d.fields.iter().find(|f| f.name == n).unwrap().clone();
        assert_eq!(f("line_id").kind, "string");
        assert_eq!(f("order_date").kind, "date");
        assert_eq!(f("units").kind, "integer");
        for m in ["unit_price", "discount", "revenue"] {
            assert_eq!((f(m).kind.as_str(), f(m).money), ("number", true), "{}", m);
        }
        assert_eq!(f("refunded").kind, "boolean");
        assert_eq!(f("campaign").kind, "string", "never enum");
        assert!(!f("units").money, "units is a count, not money");

        assert_eq!(guess_type("zip", &["00123", "10001"]).0, "string", "a leading zero stays text");
        assert_eq!(guess_type("units", &["0", "1", "1"]).0, "integer");
        assert_eq!(guess_type("is_test", &["0", "1", "1"]).0, "boolean");
        let (k, note) = guess_type("order_date", &["11/20/2026", "11/21/2026"]);
        assert_eq!(k, "string");
        assert!(note.unwrap().contains("looks like a date, saved as text"));
        assert_eq!(guess_type("created_at", &["2026-11-20T10:00:00Z", "2026-11-20 11:30:00"]).0, "datetime");
        assert_eq!(guess_type("amount", &["1,234.50", "12.00"]).0, "number");
    }

    #[test]
    fn currency_comes_from_a_clear_symbol_and_a_dollar_only_suggests() {
        let ont = load_ontology(&blank_space("money")).unwrap().0;
        let a = json!({ "file_name": "x.csv", "columns": [
            { "name": "id", "examples": ["a1", "a2"] },
            { "name": "price_eur", "examples": ["€12,50", "€3,00"] },
            { "name": "fee", "examples": ["£3.00", "£1.20"] },
            { "name": "total", "examples": ["$4.00", "$1.00"] }
        ] });
        let d = draft(&ont, &input(&a)).unwrap();
        let unit = |n: &str| d.fields.iter().find(|f| f.name == n).unwrap().unit.clone();
        assert_eq!(unit("price_eur").as_deref(), Some("EUR"));
        assert_eq!(unit("fee").as_deref(), Some("GBP"));
        assert_eq!(unit("total"), None, "$ is USD, CAD or AUD");
        let q = questions(&d);
        let c = q.iter().find(|q| q["id"] == "currency").expect("the currency question stays");
        assert_eq!(c["suggested"], "USD");
        assert!(c["ask"].as_str().unwrap().contains("total"), "{}", c);
    }

    #[test]
    fn the_key_is_the_unique_id_column_with_or_without_counts() {
        let ont = load_ontology(&blank_space("key")).unwrap().0;
        for counts in [true, false] {
            let a = args_from_csv(&fixture("promo_sales_2026-11.csv"), "promo_sales.csv", counts);
            if counts {
                assert_eq!(a["columns"][1]["distinct"], 32, "order_id repeats: 32 orders over 40 lines");
            }
            let d = draft(&ont, &input(&a)).unwrap();
            assert_eq!(d.key_name(), Some("line_id"), "counts: {}", counts);
            assert_eq!(d.grain(), "one row per line (key line_id)");
        }
        let a = json!({ "file_name": "notes.csv", "columns": [{ "name": "note", "examples": ["x", "y"] }, { "name": "score", "examples": ["1", "1"] }] });
        let d = draft(&ont, &input(&a)).unwrap();
        assert_eq!(d.key, None);
        assert!(questions(&d).iter().any(|q| q["id"] == "key"), "question (d)");
        let a = json!({ "file_name": "notes.csv", "key": "nope", "columns": [{ "name": "note" }] });
        assert!(draft(&ont, &input(&a)).unwrap_err().contains("must be one of the file's columns"));
    }

    #[test]
    fn a_header_that_is_not_a_plain_name_is_mapped_and_clashes_get_a_suffix() {
        let mut notes = Vec::new();
        let names = field_names(&["Unit Price ($)".to_string(), "unit_price".to_string(), "line_id".to_string(), "LineID".to_string()], &mut notes);
        assert_eq!(names, ["unit_price", "unit_price_2", "line_id", "LineID"]);
        assert!(notes[0].contains("unit_price_2"), "{:?}", notes);
        let dir = blank_space("headers");
        let ont = load_ontology(&dir).unwrap().0;
        let a = json!({ "file_name": "prices.csv", "columns": [{ "name": "sku_code", "examples": ["A", "B"] }, { "name": "Unit Price ($)", "examples": ["1.00", "2.00"] }], "answers": {} });
        let i = input(&a);
        let Plan::Ops(p) = plan(&ont, &i, &match_shape(&ont, &[], &i)).unwrap() else { panic!() };
        let bind = p.ops.iter().find(|o| o["op"] == "set_source").unwrap();
        assert_eq!(bind["columns"], json!({ "unit_price": "Unit Price ($)" }));
        let batch = crate::patch::plan_many(&ont, &p.ops).unwrap();
        let ty = &batch.files.iter().find(|(f, _)| f.starts_with("types/")).unwrap().1;
        assert!(ty.contains("unit_price: Unit Price ($)"), "{}", ty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_description_that_quotes_a_value_is_left_out_and_column_names_are_not_values() {
        let ont = load_ontology(&blank_space("quotes")).unwrap().0;
        let mut a = args_from_csv(&fixture("promo_sales_2026-11.csv"), "promo_sales.csv", false);
        describe_all(&mut a);
        for c in a["columns"].as_array_mut().unwrap() {
            match c["name"].as_str().unwrap() {
                "promo_code" => c["description"] = json!("The promo code, like WELCOME10."),
                "product" => c["description"] = json!("The product's name, e.g. Market tote."),
                _ => {}
            }
        }
        let d = draft(&ont, &input(&a)).unwrap();
        let desc = |n: &str| d.fields.iter().find(|f| f.name == n).unwrap().description.clone();
        assert_eq!(desc("promo_code"), None);
        assert_eq!(desc("product"), None);
        assert!(desc("revenue").unwrap().contains("units × unit_price"), "column names are not values");
        assert_eq!(d.left_out.len(), 2, "{:?}", d.left_out);
        let said = d.left_out.join(" ");
        assert!(said.contains("`promo_code`") && said.contains("`product`"));
        assert!(!said.contains("WELCOME10") && !said.contains("Market tote"), "the reply never repeats the value: {}", said);
    }

    #[test]
    fn next_month_is_recognised_by_its_columns() {
        let dir = blank_space("match");
        let ont = learn_november(&dir);
        let dec = input(&args_from_csv(&fixture("promo_sales_2026-12.csv"), "whatever-name.csv", false));
        assert!(matches!(match_shape(&ont, &[], &dec), Match::Same { ref entity, .. } if entity == "PromoSale"), "the file name plays no part");
        let reply = match_reply(&ont, &dec, &match_shape(&ont, &[], &dec), None).unwrap();
        assert_eq!(reply["read_it"][0]["measure"], "sum(revenue)");
        assert_eq!(reply["read_it"][0]["filter"], "refunded = false");

        let gift = input(&args_from_csv(&fixture("promo_sales_2026-12_gift_wrap.csv"), "promo_sales_2026-12.csv", false));
        let Match::Changed(c) = match_shape(&ont, &[], &gift) else { panic!("changed") };
        assert_eq!(c.added.iter().map(|&i| gift.columns[i].name.clone()).collect::<Vec<_>>(), ["gift_wrap"]);
        assert_eq!(c.missing, ["region"]);
        assert!(c.at_risk.is_empty());

        let gone = input(&args_from_csv(&fixture("promo_sales_2026-12_no_refunded.csv"), "promo_sales_2026-12.csv", false));
        let m = match_shape(&ont, &[], &gone);
        let Match::Changed(c) = &m else { panic!("changed") };
        assert_eq!(c.at_risk, vec![("promo_sale_revenue".to_string(), vec!["refunded".to_string()])]);
        let say = match_reply(&ont, &gone, &m, None).unwrap()["say"].as_str().unwrap().to_string();
        assert!(say.starts_with("The saved result promo_sale_revenue"), "the risk leads: {}", say);

        // a rename: one added and one missing column of the same type
        let mut renamed = args_from_csv(&fixture("promo_sales_2026-12.csv"), "promo_sales_2026-12.csv", false);
        renamed["columns"][3]["name"] = json!("promo");
        let r = input(&renamed);
        let Match::Changed(c) = match_shape(&ont, &[], &r) else { panic!("changed") };
        assert_eq!(c.renamed, vec![("promo_code".to_string(), 3)]);

        let few = input(&json!({ "file_name": "x.csv", "columns": [{ "name": "line_id" }, { "name": "revenue" }, { "name": "colour" }, { "name": "size" }, { "name": "weight" }] }));
        assert!(matches!(match_shape(&ont, &[], &few), Match::New), "2 shared columns is below the threshold");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_orders_export_matches_the_ecommerce_order() {
        let ont = load_ontology(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/ecommerce")).unwrap().0;
        let t = ont.type_("Order").unwrap();
        let cols: Vec<Json> = entity_columns(t).iter().map(|(f, _)| json!({ "name": t.source.as_ref().unwrap().columns.get(f).cloned().unwrap_or_else(|| f.clone()) })).collect();
        let i = input(&json!({ "file_name": "shopify_orders_export.csv", "columns": cols }));
        assert!(matches!(match_shape(&ont, &[], &i), Match::Same { ref entity, .. } if entity == "Order"));
        let say = match_reply(&ont, &i, &match_shape(&ont, &[], &i), None).unwrap()["say"].as_str().unwrap().to_string();
        assert!(say.contains("whose rows live in shop_files (csv), table orders"), "{}", say);
        assert!(!say.contains("Lorefile does not have the rows"), "a csv source may be readable here: {}", say);
    }

    #[test]
    fn november_on_a_blank_space_is_four_files_that_compile_clean_together() {
        let dir = blank_space("draft");
        let ont = load_ontology(&dir).unwrap().0;
        let mut a = args_from_csv(&fixture("promo_sales_2026-11.csv"), "promo_sales.csv", true);
        describe_all(&mut a);
        a["answers"] = the_answers();
        let i = input(&a);
        let Plan::Ops(p) = plan(&ont, &i, &match_shape(&ont, &[], &i)).unwrap() else { panic!() };
        let ops: Vec<&str> = p.ops.iter().map(|o| o["op"].as_str().unwrap()).collect();
        assert_eq!(ops, ["add_source", "add_type", "set_source", "add_metric", "add_term"]);
        assert_eq!(p.line, "learned promo_sales.csv: PromoSale (14 fields, key line_id) from user_files; metric promo_sale_revenue = sum(revenue) where refunded = false");
        let batch = crate::patch::plan_many(&ont, &p.ops).unwrap();
        let files: Vec<&str> = batch.files.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(files, ["sources/user_files.yaml", "types/promo_sale.yaml", "logic/metrics.yaml", "glossary.yaml"]);
        let ov: crate::loader::Overrides = batch.files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect();
        let (o, report) = crate::edit::check_many(&dir, &ov).unwrap();
        assert!(report.ok(), "{:?}", report.diags);
        let t = o.type_("PromoSale").unwrap();
        let headers = ["line_id", "order_id", "order_date", "promo_code", "campaign", "channel", "region", "sku", "product", "units", "unit_price", "discount", "revenue", "refunded"];
        assert_eq!(t.properties.keys().map(|k| k.as_str()).collect::<Vec<_>>(), headers, "14 fields named exactly as the headers");
        assert_eq!(t.key_property().as_deref(), Some("line_id"));
        assert_eq!(t.domain.as_deref(), Some("Core"));
        assert_eq!(o.sources["user_files"].kind, "from_user");
        let m = &o.metrics["promo_sale_revenue"];
        let s = m.source.as_ref().unwrap();
        assert_eq!((s.filter.as_str(), s.measure.as_str(), m.caveats.as_str()), ("refunded = false", "sum(revenue)", "Refunded lines never count."));
        assert_eq!(m.description, "Sum of revenue in PromoSale, leaving out rows where refunded is true.");
        assert!(m.basis.is_none(), "basis only when the agent passes it");
        assert_eq!(t.properties["revenue"].normalize().unit.as_deref(), Some("USD"));
        assert!(o.glossary.iter().any(|g| g.term == "promo report" && g.means == "PromoSale"));
        // no example value anywhere in what would be written
        for (_, text) in &batch.files {
            for v in ["L-1009", "BR-5008", "WELCOME10", "Market tote", "43.20"] {
                assert!(!text.contains(v), "{} leaked into\n{}", v, text);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn answers_left_empty_save_the_shape_only_and_a_second_file_reuses_the_source() {
        let dir = learn_space_with_shape();
        let ont = load_ontology(&dir).unwrap().0;
        assert!(ont.metrics.is_empty(), "answers: {{}} saves no metric");
        let a = json!({ "file_name": "orders.csv", "columns": [{ "name": "order_id", "examples": ["o1", "o2"] }, { "name": "total", "examples": ["1.00", "2.50"] }, { "name": "placed_on", "examples": ["2026-11-01"] }], "answers": {} });
        let i = input(&a);
        let Plan::Ops(p) = plan(&ont, &i, &match_shape(&ont, &[], &i)).unwrap() else { panic!() };
        assert!(!p.ops.iter().any(|o| o["op"] == "add_source"), "user_files is reused");
        assert_eq!(p.ops.iter().find(|o| o["op"] == "set_source").unwrap()["connection"], "user_files");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn learn_space_with_shape() -> PathBuf {
        let dir = blank_space("shape");
        let ont = load_ontology(&dir).unwrap().0;
        let mut a = args_from_csv(&fixture("promo_sales_2026-11.csv"), "promo_sales.csv", true);
        a["answers"] = json!({});
        let i = input(&a);
        let Plan::Ops(p) = plan(&ont, &i, &match_shape(&ont, &[], &i)).unwrap() else { panic!() };
        assert!(!p.ops.iter().any(|o| o["op"] == "add_metric"));
        for (f, t) in crate::patch::plan_many(&ont, &p.ops).unwrap().files {
            let path = dir.join(&f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        }
        dir
    }

    #[test]
    fn a_known_file_saves_nothing_unless_it_brings_the_missing_number() {
        let dir = learn_space_with_shape();
        let ont = load_ontology(&dir).unwrap().0;
        let mut a = args_from_csv(&fixture("promo_sales_2026-12.csv"), "promo_sales_2026-12.csv", false);
        a["answers"] = json!({});
        let i = input(&a);
        assert!(matches!(plan(&ont, &i, &match_shape(&ont, &[], &i)).unwrap(), Plan::Nothing(ref why) if why.starts_with("Already known as PromoSale")));
        a["answers"] = the_answers();
        let i = input(&a);
        let Plan::Ops(p) = plan(&ont, &i, &match_shape(&ont, &[], &i)).unwrap() else { panic!("the number was missing, so it is saved") };
        assert_eq!(p.ops.iter().map(|o| o["op"].as_str().unwrap()).collect::<Vec<_>>(), ["add_metric", "add_term"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_file_adds_fields_and_maps_a_confirmed_rename() {
        let dir = blank_space("changed");
        let ont = learn_november(&dir);
        let mut a = args_from_csv(&fixture("promo_sales_2026-12_gift_wrap.csv"), "promo_sales_2026-12.csv", false);
        a["columns"][3]["name"] = json!("promo");
        a["answers"] = json!({ "renamed": { "promo_code": "promo" } });
        let i = input(&a);
        let m = match_shape(&ont, &[], &i);
        let Plan::Ops(p) = plan(&ont, &i, &m).unwrap() else { panic!() };
        assert_eq!(p.ops.iter().map(|o| o["op"].as_str().unwrap()).collect::<Vec<_>>(), ["set_source", "add_property"]);
        let batch = crate::patch::plan_many(&ont, &p.ops).unwrap();
        let ty = &batch.files[0].1;
        assert!(ty.contains("gift_wrap:") && ty.contains("promo_code: promo"), "{}", ty);
        assert!(ty.contains("region:"), "a missing column is never removed");
        assert!(p.notes.iter().any(|n| n.contains("region")), "{:?}", p.notes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_never_repeat_a_value() {
        let e = parse(&json!({ "columns": [{ "name": "a", "distinct": "ZZ-SENTINEL-41" }] })).unwrap_err();
        assert!(!e.contains("ZZ-SENTINEL-41"), "{}", e);
        let ont = load_ontology(&blank_space("errors")).unwrap().0;
        let a = json!({ "file_name": "x.csv", "columns": [{ "name": "id", "examples": ["ZZ-SENTINEL-41", "b"] }, { "name": "flag", "examples": ["true"] }], "answers": { "exclude": [{ "column": "flag", "equals": "ZZ-SENTINEL-41" }], "success": { "column": "id", "how": "count_distinct" } } });
        let i = input(&a);
        let e = plan(&ont, &i, &match_shape(&ont, &[], &i)).err().unwrap();
        assert!(e.contains("must be true or false") && !e.contains("ZZ-SENTINEL-41"), "{}", e);
    }

    /// The saved recipe is real SQL over the real columns: run it through `query` against the
    /// fixtures. (It proves the recipe, not that an agent applies it; the live test does that.)
    #[cfg(feature = "duckdb")]
    #[test]
    fn the_saved_recipe_gives_the_known_totals() {
        let dir = blank_space("recipe");
        let ont = learn_november(&dir);
        let m = ont.metrics["promo_sale_revenue"].clone();
        let s = m.source.clone().unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        for f in ["promo_sales_2026-11.csv", "promo_sales_2026-12.csv"] {
            std::fs::copy(fixture(f), dir.join("data").join(f)).unwrap();
        }
        std::fs::write(dir.join("sources/files.yaml"), "connection: files\nkind: csv\npath: data\ndescription: The fixtures, for this test only.\n").unwrap();
        let total = |table: &str, sql: &str| -> f64 {
            let now = load_ontology(&dir).unwrap().0;
            let p = crate::patch::plan(&now, &json!({ "op": "set_source", "type": "PromoSale", "connection": "files", "table": table, "key": "line_id" })).unwrap();
            std::fs::write(dir.join(&p.file), &p.text).unwrap();
            let o = load_ontology(&dir).unwrap().0;
            let data = crate::data::DataLayer::new(&o);
            let r = data.query(sql, 10).unwrap();
            let v = r.rows[0].values().next().unwrap().clone();
            (v.as_f64().unwrap() * 100.0).round() / 100.0
        };
        let recipe = format!("select {} as v from PromoSale where {}", s.measure, s.filter);
        assert_eq!(recipe, "select sum(revenue) as v from PromoSale where refunded = false");
        assert_eq!(total("promo_sales_2026-11", &recipe), 1275.20);
        assert_eq!(total("promo_sales_2026-12", &recipe), 1209.60);
        assert_eq!(total("promo_sales_2026-11", &format!("select {} as v from PromoSale", s.measure)), 1435.40, "without the filter: refunds counted");
        // the grain trap: dropping whole orders that had any refunded line is a different, wrong number
        assert_eq!(total("promo_sales_2026-11", "select sum(revenue) as v from PromoSale where order_id not in (select order_id from PromoSale where refunded)"), 1142.40);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_file_learned_again_while_it_waits_is_planned_against_its_entity() {
        let dir = blank_space("rewait");
        let ont = learn_november(&dir);
        let mut a = args_from_csv(&fixture("promo_sales_2026-12_gift_wrap.csv"), "promo_sales_2026-12.csv", false);
        a["answers"] = json!({});
        let i = input(&a);
        let waiting = Proposal { id: "p-1".into(), status: "pending".into(), learned: Some(Learned { entity: "PromoSale".into(), headers: file_headers(&i), file_name: String::new() }), ..Default::default() };
        let m = match_shape(&ont, &[waiting], &i);
        assert!(matches!(m, Match::Waiting { exact: true, .. }), "{:?}", m);
        let Plan::Ops(p) = plan(&ont, &i, &m).unwrap() else { panic!("a plan, not a refusal") };
        assert_eq!(p.ops.iter().map(|o| o["op"].as_str().unwrap()).collect::<Vec<_>>(), ["add_property"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
