//! IRIs: a standard identity for every entity, field, link, glossary term,
//! guardrail and action, so the same thing can be named in Lorefile, in Apache
//! Ossie and in OWL, SKOS and SHACL.
//!
//! A space's `ontology.yaml` may set `namespace:` (default
//! `https://lorefile.dev/ns/<space>#`) and `prefixes:`. Each object may set
//! `iri:`, absolute or `prefix:local` (`:local` is the space's own namespace).
//! Without one, the IRI is derived from the namespace and the name:
//!
//! | thing     | derived IRI                     |
//! |-----------|---------------------------------|
//! | entity    | `<ns>Order`                     |
//! | field     | `<ns>Order.total_usd`           |
//! | link      | `<ns>Order.placed_by`           |
//! | guardrail | `<ns>EC-001`                    |
//! | action    | `<ns>cancel_order`              |
//! | term      | `<ns>term.basket`               |
//! | metric, function | `<ns>success`            |
//! | source    | `<ns>source.shop_files`         |
//! | domain    | `<ns>domain.orders`             |
//!
//! The compiler resolves them all (`Ontology::iris`), and the validator turns
//! the problems found here into E016 (one IRI, two things) and E017 (an IRI
//! that does not resolve).

use crate::schema::Manifest;
use indexmap::IndexMap;
use std::collections::HashMap;

/// Prefixes every space knows without declaring them (`prefixes:` may override).
pub const BUILT_IN: &[(&str, &str)] = &[
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
    ("owl", "http://www.w3.org/2002/07/owl#"),
    ("xsd", "http://www.w3.org/2001/XMLSchema#"),
    ("skos", "http://www.w3.org/2004/02/skos/core#"),
    ("sh", "http://www.w3.org/ns/shacl#"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("schema", "https://schema.org/"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("prov", "http://www.w3.org/ns/prov#"),
    ("lorefile", VOCAB),
];

/// Lorefile's own words for what the standards have no home for.
pub const VOCAB: &str = "https://lorefile.dev/vocab#";

/// What a resolved IRI names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Space,
    Entity(String),
    Field(String, String),
    Link(String, String),
    /// Index into the glossary.
    Term(usize),
    Guardrail(String),
    Action(String),
    Metric(String),
    Function(String),
    Source(String),
    Domain(String),
}

impl Key {
    /// `entity Order`, `field Order.status` … for messages.
    pub fn describe(&self, terms: &[String]) -> String {
        match self {
            Key::Space => "the space itself".into(),
            Key::Entity(n) => format!("entity `{}`", n),
            Key::Field(t, f) => format!("field `{}.{}`", t, f),
            Key::Link(t, l) => format!("link `{}.{}`", t, l),
            Key::Term(i) => format!("glossary term `{}`", terms.get(*i).cloned().unwrap_or_default()),
            Key::Guardrail(id) => format!("guardrail `{}`", id),
            Key::Action(n) => format!("action `{}`", n),
            Key::Metric(n) => format!("metric `{}`", n),
            Key::Function(n) => format!("function `{}`", n),
            Key::Source(n) => format!("source `{}`", n),
            Key::Domain(n) => format!("domain `{}`", n),
        }
    }
}

/// Every IRI in a space, resolved.
#[derive(Debug, Default, Clone)]
pub struct Iris {
    /// The namespace in force (declared, or the default).
    pub namespace: String,
    /// Whether `ontology.yaml` declared it.
    pub declared_namespace: bool,
    /// Built-in prefixes, then the manifest's (which win).
    pub prefixes: IndexMap<String, String>,
    pub map: HashMap<Key, String>,
    /// Which keys were written with `iri:` (not derived).
    pub declared: HashMap<Key, String>,
    /// (file, message) for values that do not resolve: E017.
    pub problems: Vec<(String, String)>,
}

impl Iris {
    /// The IRI of `key`, derived on the spot when the compiler has none (a
    /// link to an entity that does not exist, say).
    pub fn get(&self, key: &Key) -> String {
        self.map.get(key).cloned().unwrap_or_else(|| derive(&self.namespace, key, &[]))
    }

    /// `https://schema.org/Order` → `schema:Order` when a prefix fits and the
    /// rest is a plain local name; else the full IRI.
    pub fn compact(&self, iri: &str) -> String {
        compact_with(&self.prefixes, &self.namespace, iri)
    }

    /// The IRI a name would get without `iri:`.
    pub fn derived(&self, key: &Key, terms: &[String]) -> String {
        derive(&self.namespace, key, terms)
    }

    /// The `owl:Ontology` IRI: the namespace without its trailing `#` or `/`.
    pub fn ontology_iri(&self) -> String {
        self.get(&Key::Space)
    }
}

/// `lumen-ecommerce` → `lumen-ecommerce`; `My Shop!` → `my-shop`. The same slug
/// Ossie export uses for file names, so a space keeps one identity everywhere.
pub fn slug(name: &str) -> String {
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

/// The namespace a space gets when `ontology.yaml` names none.
pub fn default_namespace(space_name: &str) -> String {
    format!("https://lorefile.dev/ns/{}#", slug(space_name))
}

/// A name made safe for the end of an IRI: spaces become `_`, anything an IRI
/// cannot hold is percent-encoded.
pub fn local(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_whitespace() {
            out.push('_');
        } else if c.is_alphanumeric() || "-._~".contains(c) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

pub fn derive(ns: &str, key: &Key, terms: &[String]) -> String {
    match key {
        Key::Space => ns.trim_end_matches(['#', '/']).to_string(),
        Key::Entity(n) | Key::Guardrail(n) | Key::Action(n) | Key::Metric(n) | Key::Function(n) => format!("{}{}", ns, local(n)),
        Key::Field(t, f) | Key::Link(t, f) => format!("{}{}.{}", ns, local(t), local(f)),
        Key::Term(i) => format!("{}term.{}", ns, local(terms.get(*i).map(String::as_str).unwrap_or("term"))),
        Key::Source(n) => format!("{}source.{}", ns, local(n)),
        Key::Domain(n) => format!("{}domain.{}", ns, slug(n)),
    }
}

/// A scheme, then nothing an IRI may not contain. Strict enough that every
/// value it accepts is one an RDF library accepts too.
pub fn is_absolute(iri: &str) -> bool {
    let Some((scheme, rest)) = iri.split_once(':') else { return false };
    let mut chars = scheme.chars();
    let scheme_ok = chars.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    scheme_ok
        && !rest.is_empty()
        && !iri.chars().any(|c| c.is_whitespace() || c.is_control() || "<>\"{}|\\^`".contains(c))
        && iri.matches('#').count() <= 1
        && percent_ok(iri)
}

fn percent_ok(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() || !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit() {
                return false;
            }
            i += 2;
        }
        i += 1;
    }
    true
}

/// A prefix name Turtle accepts: a letter, then letters, digits, `_`, `-`, `.`, not ending in `.`.
pub fn is_prefix_name(p: &str) -> bool {
    let mut chars = p.chars();
    chars.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) && p.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) && !p.ends_with('.')
}

/// A local name a Turtle prefixed name can carry without escapes.
pub fn is_plain_local(l: &str) -> bool {
    if l.is_empty() {
        return true;
    }
    let first = l.chars().next().unwrap();
    (first.is_ascii_alphanumeric() || first == '_')
        && l.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        && !l.ends_with('.')
}

pub fn compact_with(prefixes: &IndexMap<String, String>, ns: &str, iri: &str) -> String {
    if let Some(rest) = iri.strip_prefix(ns) {
        if is_plain_local(rest) && !rest.is_empty() {
            return format!(":{}", rest);
        }
    }
    let mut best: Option<(&str, &str)> = None;
    for (p, base) in prefixes {
        if let Some(rest) = iri.strip_prefix(base.as_str()) {
            if is_plain_local(rest) && !rest.is_empty() && best.map(|(_, b)| base.len() > b.len()).unwrap_or(true) {
                best = Some((p, base));
            }
        }
    }
    match best {
        Some((p, base)) => format!("{}:{}", p, &iri[base.len()..]),
        None => iri.to_string(),
    }
}

/// `schema:Order` → `https://schema.org/Order`; `:Order` → the namespace plus
/// `Order`; an absolute IRI stays as it is.
pub fn expand(value: &str, ns: &str, prefixes: &IndexMap<String, String>) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err("`iri:` is empty".into());
    }
    if let Some((p, rest)) = v.split_once(':') {
        if !rest.starts_with("//") {
            let base = if p.is_empty() { Some(ns) } else { prefixes.get(p).map(String::as_str) };
            if let Some(base) = base {
                let full = format!("{}{}", base, rest);
                return if is_absolute(&full) { Ok(full) } else { Err(format!("`{}` expands to `{}`, which is not a valid IRI", v, full)) };
            }
        }
        if is_absolute(v) {
            // `foo:bar` with an undeclared `foo` reads as an absolute IRI only
            // for schemes people use bare; anything else is a missing prefix
            let scheme = p.to_ascii_lowercase();
            if rest.starts_with("//") || matches!(scheme.as_str(), "urn" | "tag" | "mailto" | "did" | "doi" | "isbn" | "uuid" | "data") {
                return Ok(v.to_string());
            }
        }
        return Err(format!("`{}` uses the prefix `{}`, which is not declared; add it under `prefixes:` in ontology.yaml, or write the full IRI", v, p));
    }
    Err(format!("`{}` is not an IRI: write an absolute IRI (`https://…`), `prefix:local`, or `:local` for this space's namespace", v))
}

/// Resolves the namespace and prefixes of a manifest. Problems go to `problems`.
pub fn base(manifest: &Manifest) -> Iris {
    let mut iris = Iris::default();
    for (p, b) in BUILT_IN {
        iris.prefixes.insert(p.to_string(), b.to_string());
    }
    for (p, b) in &manifest.prefixes {
        if !is_prefix_name(p) {
            iris.problems.push(("ontology.yaml".into(), format!("prefix `{}` is not a valid prefix name (a letter, then letters, digits, `_`, `-` or `.`)", p)));
            continue;
        }
        if !is_absolute(b.trim()) {
            iris.problems.push(("ontology.yaml".into(), format!("prefix `{}` maps to `{}`, which is not an absolute IRI", p, b)));
            continue;
        }
        iris.prefixes.insert(p.clone(), b.trim().to_string());
    }
    iris.namespace = default_namespace(&manifest.name);
    if let Some(ns) = manifest.namespace.as_deref().map(str::trim) {
        if !is_absolute(ns) {
            iris.problems.push(("ontology.yaml".into(), format!("namespace `{}` is not an absolute IRI", ns)));
        } else if !(ns.ends_with('#') || ns.ends_with('/') || ns.ends_with(':')) {
            iris.problems.push(("ontology.yaml".into(), format!("namespace `{}` must end in `#` or `/`, so names can follow it (`{}#Order`)", ns, ns)));
        } else {
            iris.namespace = ns.to_string();
            iris.declared_namespace = true;
        }
    }
    iris
}

impl Iris {
    /// Records the IRI of `key`: the declared `iri:` when there is one and it
    /// resolves, else the derived one.
    pub fn put(&mut self, key: Key, declared: Option<&str>, file: &str, terms: &[String]) {
        let derived = derive(&self.namespace, &key, terms);
        let iri = match declared {
            None => derived,
            Some(v) => match expand(v, &self.namespace, &self.prefixes) {
                Ok(full) => {
                    self.declared.insert(key.clone(), v.trim().to_string());
                    full
                }
                Err(e) => {
                    self.problems.push((file.to_string(), format!("{}: {}", key.describe(terms), e)));
                    derived
                }
            },
        };
        self.map.insert(key, iri);
    }
}

/// One IRI used by two things: E016 when either was written with `iri:`, a
/// warning (W112) when both were derived. Fields of different entities may
/// share an IRI (that is how `schema:name` is reused), and so may links.
pub struct Clash {
    pub iri: String,
    pub keys: Vec<Key>,
    pub declared: bool,
}

pub fn clashes(iris: &Iris) -> Vec<Clash> {
    let mut by_iri: HashMap<&str, Vec<&Key>> = HashMap::new();
    for (k, v) in &iris.map {
        by_iri.entry(v.as_str()).or_default().push(k);
    }
    let mut out = Vec::new();
    for (iri, mut keys) in by_iri {
        if keys.len() < 2 {
            continue;
        }
        keys.sort();
        let all_fields = keys.iter().all(|k| matches!(k, Key::Field(..)));
        let all_links = keys.iter().all(|k| matches!(k, Key::Link(..)));
        if all_fields || all_links {
            // shared across entities is fine; twice on one entity is not
            let mut owners: Vec<&str> = keys.iter().map(|k| match k { Key::Field(t, _) | Key::Link(t, _) => t.as_str(), _ => "" }).collect();
            owners.sort();
            let n = owners.len();
            owners.dedup();
            if owners.len() == n {
                continue;
            }
        }
        let declared = keys.iter().any(|k| iris.declared.contains_key(*k));
        out.push(Clash { iri: iri.to_string(), keys: keys.into_iter().cloned().collect(), declared });
    }
    out.sort_by(|a, b| a.iri.cmp(&b.iri));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_expand_and_compact() {
        let mut m = Manifest { name: "Lumen Shop".into(), ..Default::default() };
        m.prefixes.insert("acme".into(), "https://acme.example/ont/".into());
        let iris = base(&m);
        assert!(iris.problems.is_empty(), "{:?}", iris.problems);
        assert_eq!(iris.namespace, "https://lorefile.dev/ns/lumen-shop#");
        assert_eq!(expand("schema:Order", &iris.namespace, &iris.prefixes).unwrap(), "https://schema.org/Order");
        assert_eq!(expand("acme:Part", &iris.namespace, &iris.prefixes).unwrap(), "https://acme.example/ont/Part");
        assert_eq!(expand(":Order", &iris.namespace, &iris.prefixes).unwrap(), "https://lorefile.dev/ns/lumen-shop#Order");
        assert_eq!(expand("https://example.com/x#Y", &iris.namespace, &iris.prefixes).unwrap(), "https://example.com/x#Y");
        assert_eq!(expand("urn:isbn:0451450523", &iris.namespace, &iris.prefixes).unwrap(), "urn:isbn:0451450523");
        assert!(expand("nope:Order", &iris.namespace, &iris.prefixes).unwrap_err().contains("not declared"));
        assert!(expand("Order", &iris.namespace, &iris.prefixes).is_err());
        assert!(expand("https://exa mple.com/", &iris.namespace, &iris.prefixes).is_err());
        assert_eq!(iris.compact("https://schema.org/Order"), "schema:Order");
        assert_eq!(iris.compact("https://lorefile.dev/ns/lumen-shop#Order.total_usd"), ":Order.total_usd");
        assert_eq!(derive(&iris.namespace, &Key::Term(0), &["first order".into()]), "https://lorefile.dev/ns/lumen-shop#term.first_order");
        assert_eq!(iris.ontology_iri(), "https://lorefile.dev/ns/lumen-shop");
    }

    #[test]
    fn a_namespace_must_end_in_a_separator() {
        let m = Manifest { name: "x".into(), namespace: Some("https://acme.example/ontology".into()), ..Default::default() };
        let iris = base(&m);
        assert_eq!(iris.problems.len(), 1);
        assert_eq!(iris.namespace, "https://lorefile.dev/ns/x#");
    }
}
