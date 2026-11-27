//! The W3C bridge: OWL, RDFS, SKOS and SHACL.
//!
//! `export` writes a space as one RDF graph: the space is an `owl:Ontology`,
//! each entity an `owl:Class` (its glossary words as `skos:altLabel`), each
//! field an `owl:DatatypeProperty` with an XSD range, each link an
//! `owl:ObjectProperty` with its sentence as `rdfs:comment`. SHACL says how each
//! entity uses its fields and links (one property shape each: name, order,
//! datatype, required, allowed values, at most one) and carries every guardrail
//! as a `sh:NodeShape` with its severity and message, with native constraints
//! when the CEL rule is simple and the rule itself always as `lorefile:cel`.
//! What the standards have no home for (actions, functions, metrics, sources,
//! owners, data bindings) uses a small `lorefile:` vocabulary, so the graph is
//! valid RDF that other tools can store, and Lorefile reads back losslessly.
//!
//! `import` reads Turtle, N-Triples, RDF/XML or JSON-LD. A graph Lorefile wrote
//! comes back exactly (its nodes carry `lorefile:file`). Anything else is
//! mapped the way the standards mean it: classes become entities, datatype
//! properties fields, object properties links, `skos:altLabel` glossary words,
//! SHACL property shapes guardrails, and the report lists what was guessed,
//! dropped, or kept for a person. Nothing is written unless the result compiles.
//!
//! No reasoner: statements that exist only for inference are counted and dropped.

use crate::iri::{self, Key};
use crate::loader::{self, Overrides};
use crate::ontology::{snake, Ontology, Relationship};
use crate::schema::*;
use crate::validate;
use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use oxrdf::{BlankNode, GraphNameRef, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use serde_json::{json, Map, Value as Json};
use serde_yaml::Value as Yaml;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};

/// Bumped if the shape of the `lorefile:` vocabulary ever changes.
pub const FORMAT_VERSION: i64 = 1;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const SKOS: &str = "http://www.w3.org/2004/02/skos/core#";
const SH: &str = "http://www.w3.org/ns/shacl#";
const DCTERMS: &str = "http://purl.org/dc/terms/";
const VANN: &str = "http://purl.org/vocab/vann/";
const SCHEMA: &str = "https://schema.org/";
const PROV: &str = "http://www.w3.org/ns/prov#";
const LF: &str = iri::VOCAB;

fn v(ns: &str, local: &str) -> String {
    format!("{}{}", ns, local)
}

/// The output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Turtle,
    JsonLd,
    NTriples,
    RdfXml,
}

impl Format {
    pub fn parse(s: &str) -> Result<Format> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "turtle" | "ttl" => Format::Turtle,
            "jsonld" | "json-ld" | "json" => Format::JsonLd,
            "ntriples" | "n-triples" | "nt" => Format::NTriples,
            "rdfxml" | "rdf/xml" | "xml" | "rdf" | "owl" => Format::RdfXml,
            other => bail!("unknown RDF format `{}`: turtle, jsonld, ntriples or rdfxml", other),
        })
    }
    /// From a file name's extension, if it says.
    pub fn from_path(p: &Path) -> Option<Format> {
        match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "ttl" | "turtle" => Some(Format::Turtle),
            "jsonld" | "json" => Some(Format::JsonLd),
            "nt" => Some(Format::NTriples),
            "rdf" | "owl" | "xml" => Some(Format::RdfXml),
            _ => None,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Format::Turtle => "ttl",
            Format::JsonLd => "jsonld",
            Format::NTriples => "nt",
            Format::RdfXml => "rdf",
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            Format::Turtle => "text/turtle; charset=utf-8",
            Format::JsonLd => "application/ld+json",
            Format::NTriples => "application/n-triples",
            Format::RdfXml => "application/rdf+xml",
        }
    }
}

// ===========================================================================
// The graph export builds: subject blocks with ordered properties, nested
// blank nodes and lists, so the Turtle reads like something a person wrote.
// ===========================================================================

#[derive(Debug, Clone)]
pub enum Obj {
    Iri(String),
    /// lexical form, datatype (None = xsd:string), language
    Lit(String, Option<String>, Option<String>),
    Node(Vec<(String, Obj)>),
    List(Vec<Obj>),
}

fn lit(s: &str) -> Obj {
    Obj::Lit(s.to_string(), None, None)
}
fn int(n: i64) -> Obj {
    Obj::Lit(n.to_string(), Some(v(XSD, "integer")), None)
}
fn boolean(b: bool) -> Obj {
    Obj::Lit(b.to_string(), Some(v(XSD, "boolean")), None)
}
fn json_lit(j: &Json) -> Obj {
    Obj::Lit(serde_json::to_string(j).unwrap_or_default(), Some(v(RDF, "JSON")), None)
}
fn any_uri(s: &str) -> Obj {
    Obj::Lit(s.to_string(), Some(v(XSD, "anyURI")), None)
}
fn iri_o(s: &str) -> Obj {
    Obj::Iri(s.to_string())
}

/// A YAML value as an RDF literal (`sh:in` members), if it is a plain scalar.
fn yaml_lit(y: &Yaml) -> Option<Obj> {
    match y {
        Yaml::String(s) => Some(lit(s)),
        Yaml::Bool(b) => Some(boolean(*b)),
        Yaml::Number(n) if n.is_i64() || n.is_u64() => Some(Obj::Lit(n.to_string(), Some(v(XSD, "integer")), None)),
        Yaml::Number(n) => {
            let f = n.as_f64()?;
            if !f.is_finite() {
                return None;
            }
            let mut s = format!("{:?}", f);
            if !s.contains('.') && !s.contains('e') {
                s.push_str(".0");
            }
            let dt = v(XSD, if s.contains('e') { "double" } else { "decimal" });
            Some(Obj::Lit(s, Some(dt), None))
        }
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    /// None: a blank node (`[] …` in Turtle).
    pub subject: Option<String>,
    pub props: Vec<(String, Obj)>,
}

impl Block {
    fn new(subject: Option<String>) -> Block {
        Block { subject, props: Vec::new() }
    }
    fn add(&mut self, p: String, o: Obj) {
        self.props.push((p, o));
    }
    fn text(&mut self, p: String, s: &str) {
        if !s.trim().is_empty() {
            self.props.push((p, lit(s)));
        }
    }
}

fn node_text(props: &mut Vec<(String, Obj)>, p: String, s: &str) {
    if !s.trim().is_empty() {
        props.push((p, lit(s)));
    }
}

/// Blank nodes waiting to have their own triples written.
type Pending = VecDeque<(NamedOrBlankNode, Vec<(String, Obj)>)>;

/// A space as RDF, ready to write in any of the four formats.
pub struct Graph {
    pub blocks: Vec<Block>,
    /// The space's namespace (the empty prefix in Turtle).
    pub namespace: String,
    /// Every prefix the writer may use, built-in ones first.
    pub prefixes: IndexMap<String, String>,
    pub name: String,
    pub notes: Vec<String>,
}

// ---- datatypes ---------------------------------------------------------------

fn xsd_of(kind: &str) -> Option<String> {
    Some(match kind {
        "string" | "enum" => v(XSD, "string"),
        "integer" => v(XSD, "integer"),
        "number" => v(XSD, "decimal"),
        "boolean" => v(XSD, "boolean"),
        "date" => v(XSD, "date"),
        "datetime" => v(XSD, "dateTime"),
        "json" => v(RDF, "JSON"),
        _ => return None,
    })
}

/// The Lorefile kind for a datatype IRI (XSD, schema.org's, or RDF's).
fn kind_of_datatype(dt: &str, has_values: bool) -> Option<&'static str> {
    let local = dt.rsplit(['#', '/']).next().unwrap_or("");
    let ns_ok = dt.starts_with(XSD) || dt.starts_with(SCHEMA) || dt.starts_with("http://schema.org/") || dt.starts_with(RDF) || dt.starts_with(RDFS);
    if !ns_ok {
        return None;
    }
    Some(match local {
        "string" | "normalizedString" | "token" | "anyURI" | "langString" | "Literal" | "Text" | "URL" | "Time" | "time" | "CssSelectorType" | "XPathType" | "PronounceableText" | "language" | "Name" | "NCName" | "HTML" => {
            if has_values {
                "enum"
            } else {
                "string"
            }
        }
        "integer" | "int" | "long" | "short" | "byte" | "nonNegativeInteger" | "positiveInteger" | "negativeInteger" | "nonPositiveInteger" | "unsignedInt" | "unsignedLong" | "unsignedShort" | "unsignedByte" | "Integer" => "integer",
        "decimal" | "double" | "float" | "Number" | "Float" => "number",
        "boolean" | "Boolean" => "boolean",
        "date" | "Date" => "date",
        "dateTime" | "dateTimeStamp" | "DateTime" => "datetime",
        "JSON" => "json",
        _ => return None,
    })
}

fn severity_iri(s: Severity) -> String {
    v(SH, match s {
        Severity::Block => "Violation",
        Severity::Warn => "Warning",
        Severity::Info => "Info",
    })
}

fn severity_of(iri: &str) -> Severity {
    match iri.strip_prefix(SH) {
        Some("Warning") => Severity::Warn,
        Some("Info") => Severity::Info,
        _ => Severity::Block,
    }
}

fn norm_card(c: &str) -> String {
    c.trim().to_ascii_lowercase().replace('_', "-")
}

/// A link declared in its own file under `relationships/`, not inside its entity.
fn is_standalone(ont: &Ontology, r: &Relationship) -> bool {
    !ont.types.get(&r.from).map(|t| t.relationships.contains_key(&r.name) && t.file == r.file).unwrap_or(false)
}

// ---- CEL → SHACL, for the simple rules ---------------------------------------

/// Splits on `sep` outside quotes, parentheses and brackets.
fn split_top<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start, mut i) = (0i32, None::<u8>, 0usize, 0usize);
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == b'\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ if depth == 0 && s[i..].starts_with(sep) => {
                    out.push(&s[start..i]);
                    i += sep.len();
                    start = i;
                    continue;
                }
                _ => {}
            },
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// `(x)` → `x` when the parentheses wrap the whole thing.
fn unwrap_parens(s: &str) -> &str {
    let mut s = s.trim();
    while s.starts_with('(') && s.ends_with(')') && split_top(&s[1..s.len() - 1], ")").len() == 1 && balanced_parens(&s[1..s.len() - 1]) {
        s = s[1..s.len() - 1].trim();
    }
    s
}

fn balanced_parens(s: &str) -> bool {
    let mut d = 0i32;
    let mut quote = None;
    let mut esc = false;
    for c in s.chars() {
        if let Some(q) = quote {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' => d += 1,
            ')' => {
                d -= 1;
                if d < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    d == 0
}

/// A CEL literal (string, number, boolean) as an RDF literal.
fn cel_literal(s: &str) -> Option<Obj> {
    let s = s.trim();
    if s == "true" || s == "false" {
        return Some(boolean(s == "true"));
    }
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2) || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2) {
        let inner = &s[1..s.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next()? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => out.push(other),
                }
            } else if c == s.chars().next().unwrap() {
                return None;
            } else {
                out.push(c);
            }
        }
        return Some(lit(&out));
    }
    if s.parse::<i64>().is_ok() {
        return Some(Obj::Lit(s.to_string(), Some(v(XSD, "integer")), None));
    }
    if s.parse::<f64>().is_ok() && s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '+') {
        return Some(Obj::Lit(s.to_string(), Some(v(XSD, "decimal")), None));
    }
    None
}

/// The field a CEL operand names, if it is one of the entity's fields.
fn cel_field(t: &TypeDef, s: &str) -> Option<String> {
    let s = s.trim();
    let name = s.strip_prefix("self.").or_else(|| s.strip_prefix(&format!("{}.", snake(&t.name)))).unwrap_or(s);
    t.properties.contains_key(name).then(|| name.to_string())
}

/// The SHACL form of a type guardrail whose rule is a conjunction of
/// `field in [...]`, `field <op> literal` and `has(self.field)`; None when any
/// part has no SHACL form.
fn shacl_form(t: &TypeDef, rule: &str) -> Option<IndexMap<String, Vec<(String, Obj)>>> {
    let mut out: IndexMap<String, Vec<(String, Obj)>> = IndexMap::new();
    for atom in split_top(rule.trim(), "&&") {
        let a = unwrap_parens(atom);
        if a.contains(" implies ") || a.contains("||") {
            return None;
        }
        if let Some(inner) = a.strip_prefix("has(").and_then(|x| x.strip_suffix(')')) {
            let f = cel_field(t, inner)?;
            if !inner.contains('.') {
                return None;
            }
            out.entry(f).or_default().push((v(SH, "minCount"), int(1)));
            continue;
        }
        let in_parts = split_top(a, " in ");
        if in_parts.len() == 2 {
            let f = cel_field(t, in_parts[0])?;
            let list = in_parts[1].trim();
            let inner = list.strip_prefix('[')?.strip_suffix(']')?;
            let items: Option<Vec<Obj>> = if inner.trim().is_empty() { Some(vec![]) } else { split_top(inner, ",").into_iter().map(cel_literal).collect() };
            out.entry(f).or_default().push((v(SH, "in"), Obj::List(items?)));
            continue;
        }
        let mut matched = false;
        for (op, pred) in [(">=", "minInclusive"), ("<=", "maxInclusive"), ("==", "hasValue"), (">", "minExclusive"), ("<", "maxExclusive")] {
            let parts = split_top(a, op);
            if parts.len() == 2 && !parts[1].starts_with('=') {
                let f = cel_field(t, parts[0])?;
                let l = cel_literal(parts[1])?;
                if pred != "hasValue" && matches!(&l, Obj::Lit(_, None, _)) {
                    // comparing text: ISO dates order as strings, but SHACL compares typed values
                    return None;
                }
                out.entry(f).or_default().push((v(SH, pred), l));
                matched = true;
                break;
            }
        }
        if !matched {
            return None;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

// ===========================================================================
// Export
// ===========================================================================

fn to_json<T: serde::Serialize>(v: &T) -> Json {
    serde_json::to_value(v).unwrap_or(Json::Null)
}

/// The object as JSON without the fields the graph carries elsewhere.
fn definition<T: serde::Serialize>(v: &T) -> Json {
    let mut j = to_json(v);
    if let Some(m) = j.as_object_mut() {
        m.shift_remove("file");
        m.shift_remove("iri");
    }
    j
}

pub fn export(ont: &Ontology) -> Graph {
    let ns = ont.iris.namespace.clone();
    let mut notes = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    let rdf_type = v(RDF, "type");

    // ---- the space
    let mut b = Block::new(Some(ont.iris.ontology_iri()));
    b.add(rdf_type.clone(), iri_o(&v(OWL, "Ontology")));
    b.add(v(RDFS, "label"), lit(&ont.manifest.name));
    b.text(v(RDFS, "comment"), &ont.manifest.description);
    b.text(v(OWL, "versionInfo"), &ont.manifest.version);
    b.add(v(VANN, "preferredNamespaceUri"), any_uri(&ns));
    for (p, base) in &ont.manifest.prefixes {
        b.add(v(SH, "declare"), Obj::Node(vec![(v(SH, "prefix"), lit(p)), (v(SH, "namespace"), any_uri(base))]));
    }
    b.add(v(LF, "formatVersion"), int(FORMAT_VERSION));
    blocks.push(b);

    // ---- domains: one SKOS concept scheme each
    let domain_names: HashSet<&str> = ont.manifest.domains.iter().map(|d| d.name.as_str()).collect();
    for (i, d) in ont.manifest.domains.iter().enumerate() {
        let mut b = Block::new(Some(ont.iri_domain(&d.name)));
        b.add(rdf_type.clone(), iri_o(&v(SKOS, "ConceptScheme")));
        b.add(v(RDFS, "label"), lit(&d.name));
        b.text(v(RDFS, "comment"), &d.description);
        if !d.types.is_empty() {
            b.add(v(LF, "types"), Obj::List(d.types.iter().map(|t| iri_o(&ont.iri_type(t))).collect()));
        }
        b.add(v(LF, "order"), int(i as i64));
        blocks.push(b);
    }

    // who uses each field and link IRI (shared ones get one declaration)
    let mut field_users: IndexMap<String, Vec<(String, String)>> = IndexMap::new();
    // a kind's inherited fields are its parent's: declared (and given a domain) there only
    for t in ont.types.values() {
        for pn in t.properties.keys().filter(|n| !t.inherited.contains_key(*n)) {
            field_users.entry(ont.iri_field(&t.name, pn)).or_default().push((t.name.clone(), pn.clone()));
        }
    }
    // a parent's shape is named, so its kinds' shapes can point at it with sh:node
    let shape_iri = |name: &str| format!("{}Shape", ont.iri_type(name));
    let has_kinds = |name: &str| ont.types.values().any(|x| x.parent.as_deref() == Some(name));
    let mut link_users: IndexMap<String, Vec<usize>> = IndexMap::new();
    for (i, r) in ont.relationships.iter().enumerate() {
        link_users.entry(ont.iri_link(r)).or_default().push(i);
    }
    let mut declared: HashSet<String> = HashSet::new();
    let class_or_union = |classes: Vec<String>| -> Obj {
        let mut uniq: Vec<String> = Vec::new();
        for c in classes {
            if !uniq.contains(&c) {
                uniq.push(c);
            }
        }
        if uniq.len() == 1 {
            iri_o(&uniq[0])
        } else {
            Obj::Node(vec![(v(RDF, "type"), iri_o(&v(OWL, "Class"))), (v(OWL, "unionOf"), Obj::List(uniq.iter().map(|c| iri_o(c)).collect()))])
        }
    };

    // links grouped by the entity they leave from
    let mut links_from: IndexMap<String, Vec<usize>> = IndexMap::new();
    for t in ont.types.keys() {
        links_from.insert(t.clone(), Vec::new());
    }
    for (i, r) in ont.relationships.iter().enumerate() {
        links_from.entry(r.from.clone()).or_default().push(i);
    }

    for (from, link_idx) in &links_from {
        let t = ont.types.get(from);
        let class = ont.iri_type(from);
        let mut shape_props: Vec<Obj> = Vec::new();

        if let Some(t) = t {
            let ti = ont.types.get_index_of(from).unwrap_or(0);
            let mut b = Block::new(Some(class.clone()));
            b.add(rdf_type.clone(), iri_o(&v(OWL, "Class")));
            b.add(v(RDFS, "label"), lit(&t.name));
            b.add(v(SKOS, "prefLabel"), lit(&t.name));
            b.text(v(RDFS, "comment"), &t.description);
            if let Some(d) = ont.domain_of(&t.name).filter(|d| domain_names.contains(d.as_str())) {
                b.add(v(SKOS, "inScheme"), iri_o(&ont.iri_domain(&d)));
            }
            if let Some(d) = &t.domain {
                b.add(v(LF, "domain"), lit(d));
            }
            b.text(v(LF, "owner"), &t.owner);
            for tag in &t.tags {
                b.add(v(DCTERMS, "subject"), lit(tag));
            }
            if let Some(src) = &t.source {
                b.add(v(LF, "source"), json_lit(&to_json(src)));
            }
            if !t.instances.is_empty() {
                b.add(v(LF, "instances"), json_lit(&to_json(&t.instances)));
            }
            if let Some(p) = &t.parent {
                b.add(v(RDFS, "subClassOf"), iri_o(&ont.iri_type(p)));
                b.add(v(LF, "extends"), lit(p));
            }
            if let Some(w) = &t.where_ {
                b.add(v(LF, "where"), json_lit(&to_json(w)));
            }
            b.add(v(LF, "file"), lit(&t.file));
            b.add(v(LF, "order"), int(ti as i64));
            blocks.push(b);

            // fields: its own (a kind's inherited ones arrive through sh:node)
            for (fi, (pn, ps)) in t.properties.iter().filter(|(n, _)| !t.inherited.contains_key(*n)).enumerate() {
                let p = ps.normalize();
                let fiter = ont.iri_field(&t.name, pn);
                if declared.insert(fiter.clone()) {
                    let users = &field_users[&fiter];
                    let mut b = Block::new(Some(fiter.clone()));
                    b.add(rdf_type.clone(), iri_o(&v(OWL, "DatatypeProperty")));
                    b.add(v(RDFS, "label"), lit(pn));
                    b.text(v(RDFS, "comment"), &p.description);
                    b.add(v(RDFS, "domain"), class_or_union(users.iter().map(|(tn, _)| ont.iri_type(tn)).collect()));
                    b.add(v(RDFS, "range"), iri_o(&xsd_of(&p.kind).unwrap_or_else(|| v(RDFS, "Literal"))));
                    blocks.push(b);
                }
                let mut ps_props: Vec<(String, Obj)> = vec![(v(SH, "path"), iri_o(&fiter)), (v(SH, "name"), lit(pn))];
                node_text(&mut ps_props, v(SH, "description"), &p.description);
                ps_props.push((v(SH, "order"), int(fi as i64)));
                if let Some(dt) = xsd_of(&p.kind) {
                    ps_props.push((v(SH, "datatype"), iri_o(&dt)));
                }
                if p.required {
                    ps_props.push((v(SH, "minCount"), int(1)));
                }
                let scalars: Option<Vec<Obj>> = p.values.iter().map(yaml_lit).collect();
                match scalars {
                    Some(list) if !list.is_empty() => ps_props.push((v(SH, "in"), Obj::List(list))),
                    Some(_) => {}
                    None => ps_props.push((v(LF, "values"), json_lit(&to_json(&p.values)))),
                }
                let recovered = xsd_of(&p.kind).and_then(|dt| kind_of_datatype(&dt, !p.values.is_empty())).unwrap_or("string");
                if recovered != p.kind {
                    ps_props.push((v(LF, "type"), lit(&p.kind)));
                }
                if p.key {
                    ps_props.push((v(LF, "key"), boolean(true)));
                }
                if let Some(u) = &p.unit {
                    ps_props.push((v(LF, "unit"), lit(u)));
                }
                shape_props.push(Obj::Node(ps_props));
            }
        }

        // links
        let nfields = t.map(|t| t.properties.len()).unwrap_or(0);
        for (li, &gi) in link_idx.iter().enumerate() {
            let r = &ont.relationships[gi];
            let liri = ont.iri_link(r);
            let card = norm_card(&r.cardinality);
            if declared.insert(liri.clone()) {
                let users: Vec<&Relationship> = link_users[&liri].iter().map(|i| &ont.relationships[*i]).collect();
                let mut b = Block::new(Some(liri.clone()));
                b.add(rdf_type.clone(), iri_o(&v(OWL, "ObjectProperty")));
                if card == "many-to-one" || card == "one-to-one" {
                    b.add(rdf_type.clone(), iri_o(&v(OWL, "FunctionalProperty")));
                }
                if card == "one-to-one" || card == "one-to-many" {
                    b.add(rdf_type.clone(), iri_o(&v(OWL, "InverseFunctionalProperty")));
                }
                b.add(v(RDFS, "label"), lit(&r.name));
                b.text(v(RDFS, "comment"), &r.description);
                b.add(v(RDFS, "domain"), class_or_union(users.iter().map(|u| ont.iri_type(&u.from)).collect()));
                b.add(v(RDFS, "range"), class_or_union(users.iter().map(|u| ont.iri_type(&u.to)).collect()));
                blocks.push(b);
            }
            let mut ps_props: Vec<(String, Obj)> = vec![(v(SH, "path"), iri_o(&liri)), (v(SH, "name"), lit(&r.name))];
            node_text(&mut ps_props, v(SH, "description"), &r.description);
            ps_props.push((v(SH, "order"), int((nfields + li) as i64)));
            ps_props.push((v(SH, "class"), iri_o(&ont.iri_type(&r.to))));
            if card == "many-to-one" || card == "one-to-one" {
                ps_props.push((v(SH, "maxCount"), int(1)));
            }
            if !r.cardinality.is_empty() {
                ps_props.push((v(LF, "cardinality"), lit(&r.cardinality)));
            }
            if !r.properties.is_empty() {
                ps_props.push((v(LF, "properties"), json_lit(&to_json(&r.properties))));
            }
            if let Some(src) = &r.source {
                ps_props.push((v(LF, "edgeSource"), json_lit(&to_json(src))));
            }
            if is_standalone(ont, r) {
                ps_props.push((v(LF, "file"), lit(&r.file)));
                ps_props.push((v(LF, "order"), int(gi as i64)));
            }
            shape_props.push(Obj::Node(ps_props));
        }

        // a kind: its parent's constraints apply through sh:node, and `where` as a fixed value
        let kind_parent = t.and_then(|t| t.parent.clone());
        if let Some(t) = t {
            if let Some(w) = &t.where_ {
                for (f, vals) in w {
                    let path = ont.iri_field(&t.name, f);
                    let lits: Vec<Obj> = vals.all().iter().filter_map(yaml_lit).collect();
                    let mut node: Vec<(String, Obj)> = vec![(v(SH, "path"), iri_o(&path)), (v(LF, "whereOf"), boolean(true))];
                    if lits.len() == 1 {
                        node.push((v(SH, "hasValue"), lits[0].clone()));
                    } else if !lits.is_empty() {
                        node.push((v(SH, "in"), Obj::List(lits)));
                    }
                    shape_props.push(Obj::Node(node));
                }
            }
        }
        if !shape_props.is_empty() || kind_parent.is_some() || has_kinds(from) {
            let mut b = Block::new(if has_kinds(from) { Some(shape_iri(from)) } else { None });
            b.add(rdf_type.clone(), iri_o(&v(SH, "NodeShape")));
            b.add(rdf_type.clone(), iri_o(&v(LF, "EntityShape")));
            b.add(v(SH, "targetClass"), iri_o(&class));
            if let Some(p) = &kind_parent {
                b.add(v(SH, "node"), iri_o(&shape_iri(p)));
            }
            for p in shape_props {
                b.add(v(SH, "property"), p);
            }
            blocks.push(b);
        }
    }

    // ---- glossary: each word a SKOS concept, and an alternative label on what it means
    let mut alt: HashMap<String, Vec<String>> = HashMap::new();
    for (i, g) in ont.glossary.iter().enumerate() {
        let target = ont.iri_of_name(&g.means);
        let mut b = Block::new(Some(ont.iri_term(i)));
        b.add(rdf_type.clone(), iri_o(&v(SKOS, "Concept")));
        b.add(v(SKOS, "prefLabel"), lit(&g.term));
        b.text(v(SKOS, "definition"), &g.description);
        if let Some(t) = &target {
            b.add(v(LF, "means"), iri_o(t));
            alt.entry(t.clone()).or_default().push(g.term.clone());
        }
        b.add(v(LF, "meansName"), lit(&g.means));
        b.add(v(LF, "order"), int(i as i64));
        blocks.push(b);
    }

    // ---- guardrails: SHACL shapes
    let mut celonly = 0;
    for (ci, c) in ont.constraints.values().enumerate() {
        let actions: Vec<&ActionDef> = ont
            .actions
            .values()
            .filter(|a| a.name.eq_ignore_ascii_case(&c.applies_to) || a.checks.iter().any(|id| id.eq_ignore_ascii_case(&c.id)))
            .collect();
        let mut b = Block::new(Some(ont.iri_guardrail(&c.id)));
        b.add(rdf_type.clone(), iri_o(&v(SH, "NodeShape")));
        b.add(v(LF, "id"), lit(&c.id));
        b.add(v(SH, "message"), lit(&c.name));
        b.text(v(RDFS, "comment"), &c.description);
        b.add(v(SH, "severity"), iri_o(&severity_iri(c.severity)));
        let target_type = ont.types.get(&c.applies_to).or_else(|| ont.type_(&c.applies_to).filter(|_| ont.kind_of(&c.applies_to) == Some(crate::ontology::Kind::Type)));
        if let Some(t) = target_type {
            b.add(v(SH, "targetClass"), iri_o(&ont.iri_type(&t.name)));
        } else {
            for r in ont.relationship(&c.applies_to) {
                b.add(v(SH, "targetSubjectsOf"), iri_o(&ont.iri_link(r)));
            }
        }
        if !actions.is_empty() {
            b.add(v(SH, "deactivated"), boolean(true));
            for a in &actions {
                b.add(v(LF, "action"), iri_o(&ont.iri_action(&a.name)));
            }
        }
        match target_type.and_then(|t| shacl_form(t, &c.rule).map(|f| (t, f))) {
            Some((t, form)) => {
                for (field, cons) in form {
                    let mut ps = vec![(v(SH, "path"), iri_o(&ont.iri_field(&t.name, &field)))];
                    ps.extend(cons);
                    b.add(v(SH, "property"), Obj::Node(ps));
                }
            }
            None => {
                celonly += 1;
                b.add(v(LF, "celOnly"), boolean(true));
            }
        }
        b.add(v(LF, "cel"), lit(&c.rule));
        b.add(v(LF, "appliesTo"), lit(&c.applies_to));
        b.text(v(LF, "rationale"), &c.rationale);
        b.text(v(LF, "owner"), &c.owner);
        if let Some(sql) = &c.sql {
            b.add(v(LF, "sql"), lit(sql));
        }
        for tag in &c.tags {
            b.add(v(DCTERMS, "subject"), lit(tag));
        }
        b.add(v(LF, "file"), lit(&c.file));
        b.add(v(LF, "order"), int(ci as i64));
        blocks.push(b);
    }
    if !ont.constraints.is_empty() {
        notes.push(format!(
            "{} of {} guardrails have a native SHACL form; the other {} are carried as lorefile:cel only (marked lorefile:celOnly), which SHACL tools store but do not run",
            ont.constraints.len() - celonly,
            ont.constraints.len(),
            celonly
        ));
    }

    // ---- what the standards have no words for
    for (i, a) in ont.actions.values().enumerate() {
        let mut b = Block::new(Some(ont.iri_action(&a.name)));
        b.add(rdf_type.clone(), iri_o(&v(LF, "Action")));
        b.add(v(RDFS, "label"), lit(&a.name));
        b.text(v(RDFS, "comment"), &a.description);
        b.add(v(LF, "definition"), json_lit(&definition(a)));
        b.add(v(LF, "file"), lit(&a.file));
        b.add(v(LF, "order"), int(i as i64));
        blocks.push(b);
    }
    for (i, m) in ont.metrics.values().enumerate() {
        let mut b = Block::new(Some(ont.iri_metric(&m.name)));
        b.add(rdf_type.clone(), iri_o(&v(LF, "Metric")));
        b.add(v(RDFS, "label"), lit(&m.name));
        b.text(v(RDFS, "comment"), &m.description);
        b.add(v(LF, "definition"), json_lit(&definition(m)));
        b.add(v(LF, "file"), lit(&m.file));
        b.add(v(LF, "order"), int(i as i64));
        blocks.push(b);
    }
    for (i, f) in ont.functions.values().enumerate() {
        let mut b = Block::new(Some(ont.iri_function(&f.name)));
        b.add(rdf_type.clone(), iri_o(&v(LF, "Function")));
        b.add(v(RDFS, "label"), lit(&f.name));
        b.text(v(RDFS, "comment"), &f.description);
        b.add(v(LF, "definition"), json_lit(&definition(f)));
        b.add(v(LF, "file"), lit(&f.file));
        b.add(v(LF, "order"), int(i as i64));
        blocks.push(b);
    }
    for (i, s) in ont.sources.values().enumerate() {
        let mut b = Block::new(Some(ont.iri_source(&s.connection)));
        b.add(rdf_type.clone(), iri_o(&v(LF, "Source")));
        b.add(v(RDFS, "label"), lit(&s.connection));
        b.text(v(RDFS, "comment"), &s.description);
        b.add(v(LF, "definition"), json_lit(&definition(s)));
        b.add(v(LF, "file"), lit(&s.file));
        b.add(v(LF, "order"), int(i as i64));
        blocks.push(b);
    }

    // ---- grades: `lorefile:trust` on the thing, and a PROV-O record of who, when and against what
    if let Some(book) = &ont.trust {
        use crate::trust::Kind as TK;
        let mut agents: IndexMap<String, String> = IndexMap::new();
        let mut records: Vec<Block> = Vec::new();
        let mut orphans = 0;
        for k in TK::ALL {
            for (name, rec) in crate::trust::entries(book, k) {
                let thing = match k {
                    TK::Metric if ont.metrics.contains_key(name) => Some(ont.iri_metric(name)),
                    TK::Binding if ont.types.get(name).map(|t| t.source.is_some()).unwrap_or(false) => Some(ont.iri_type(name)),
                    TK::Term => ont.glossary.iter().position(|g| &g.term == name).map(|i| ont.iri_term(i)),
                    TK::Guardrail if ont.constraints.contains_key(name) => Some(ont.iri_guardrail(name)),
                    _ => None,
                };
                let Some(thing) = thing else {
                    orphans += 1;
                    continue;
                };
                let grade_iri = v(LF, match rec.grade { Grade::Proposed => "Proposed", Grade::Accepted => "Accepted", Grade::Checked => "Checked" });
                let by = if rec.by.trim().is_empty() { "person via page".to_string() } else { rec.by.clone() };
                // one node per distinct name: two names that slug alike ("Priya K", "Priya-K", or
                // any two written in another script) must not share a node, or PROV names the wrong person
                let agent = match agents.iter().find(|(_, b)| **b == by) {
                    Some((a, _)) => a.clone(),
                    None => {
                        let base = format!("{}agent.{}", ns, iri::slug(&by));
                        let mut a = base.clone();
                        let mut n = 2;
                        while agents.contains_key(&a) {
                            a = format!("{}-{}", base, n);
                            n += 1;
                        }
                        agents.insert(a.clone(), by.clone());
                        a
                    }
                };
                let st = ont.grade_of(k, name);
                // the grade sits on the thing only while it still counts (not changed since)
                if let Some(b) = blocks.iter_mut().find(|b| b.subject.as_deref() == Some(thing.as_str())) {
                    if st.grade.is_some() {
                        b.add(v(LF, if k == TK::Binding { "bindingTrust" } else { "trust" }), iri_o(&grade_iri));
                        if rec.grade == Grade::Proposed && k != TK::Binding {
                            b.add(v(PROV, "wasAttributedTo"), iri_o(&agent));
                        }
                    }
                }
                let mut r = Block::new(Some(format!("{}trust.{}.{}", ns, k.key(), iri::local(name))));
                r.add(rdf_type.clone(), iri_o(&v(LF, "Grading")));
                r.add(rdf_type.clone(), iri_o(&v(PROV, "Activity")));
                r.add(v(LF, "about"), iri_o(&thing));
                if k == TK::Binding {
                    r.add(v(LF, "aspect"), lit("binding"));
                }
                r.add(v(LF, "grade"), iri_o(&grade_iri));
                r.add(v(PROV, "wasAssociatedWith"), iri_o(&agent));
                if chrono::NaiveDate::parse_from_str(rec.at.trim(), "%Y-%m-%d").is_ok() {
                    r.add(v(PROV, "endedAtTime"), Obj::Lit(format!("{}T00:00:00Z", rec.at.trim()), Some(v(XSD, "dateTime")), None));
                }
                r.add(v(PROV, "used"), iri_o(&thing));
                if let Some(a) = rec.against.as_deref().filter(|a| ont.sources.contains_key(*a)) {
                    r.add(v(PROV, "used"), iri_o(&ont.iri_source(a)));
                }
                if let Some(n) = &rec.note {
                    r.text(v(RDFS, "comment"), n);
                }
                r.add(v(LF, "of"), lit(&rec.of));
                if st.changed {
                    r.add(v(LF, "changedSince"), boolean(true));
                }
                r.add(v(LF, "definition"), json_lit(&json!({ "kind": k.key(), "name": name, "record": rec })));
                r.add(v(LF, "file"), lit(crate::trust::FILE));
                records.push(r);
            }
        }
        for (agent, by) in agents {
            let mut b = Block::new(Some(agent));
            b.add(rdf_type.clone(), iri_o(&v(PROV, if crate::trust::Who::of(&by) == crate::trust::Who::Agent { "SoftwareAgent" } else { "Person" })));
            b.add(v(RDFS, "label"), lit(&by));
            blocks.push(b);
        }
        if !records.is_empty() {
            notes.push(format!("{} grades from trust.yaml: lorefile:trust on each graded thing (lorefile:bindingTrust on an entity for its data binding), and a lorefile:Grading record that is also a prov:Activity saying who, when and against which source", records.len()));
        }
        if orphans > 0 {
            notes.push(format!("{} grades in trust.yaml name something that no longer exists; left out", orphans));
        }
        blocks.extend(records);
    }

    // glossary words as alternative labels on what they mean
    for b in blocks.iter_mut() {
        if let Some(words) = b.subject.as_ref().and_then(|s| alt.remove(s)) {
            let at = b.props.iter().position(|(p, _)| p == &v(SKOS, "prefLabel") || p == &v(RDFS, "label")).map(|i| i + 1).unwrap_or(b.props.len());
            let at = if b.props.get(at).map(|(p, _)| p == &v(SKOS, "prefLabel")).unwrap_or(false) { at + 1 } else { at };
            for (k, w) in words.into_iter().enumerate() {
                b.props.insert(at + k, (v(SKOS, "altLabel"), lit(&w)));
            }
        }
    }

    let mut prefixes: IndexMap<String, String> = IndexMap::new();
    for (p, base) in iri::BUILT_IN {
        prefixes.insert(p.to_string(), base.to_string());
    }
    prefixes.insert("vann".into(), VANN.into());
    for (p, base) in &ont.manifest.prefixes {
        prefixes.insert(p.clone(), base.clone());
    }
    Graph { blocks, namespace: ns, prefixes, name: ont.manifest.name.clone(), notes }
}

// ---- writers -------------------------------------------------------------------

/// A Turtle string literal, in the quoting that needs the fewest escapes.
fn turtle_string(s: &str) -> String {
    let esc = |s: &str, quote: char| -> String {
        let mut o = String::new();
        for c in s.chars() {
            match c {
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                '\t' => o.push_str("\\t"),
                c if c == quote => {
                    o.push('\\');
                    o.push(c);
                }
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04X}", c as u32)),
                c => o.push(c),
            }
        }
        o
    };
    if s.contains('\n') {
        let mut o = String::new();
        for c in s.chars() {
            match c {
                '\\' => o.push_str("\\\\"),
                '"' => o.push_str("\\\""),
                '\r' => o.push_str("\\r"),
                c if (c as u32) < 0x20 && c != '\n' && c != '\t' => o.push_str(&format!("\\u{:04X}", c as u32)),
                c => o.push(c),
            }
        }
        return format!("\"\"\"{}\"\"\"", o);
    }
    if s.contains('"') && !s.contains('\'') {
        format!("'{}'", esc(s, '\''))
    } else {
        format!("\"{}\"", esc(s, '"'))
    }
}

struct TurtleWriter<'g> {
    g: &'g Graph,
    used: BTreeSet<String>,
}

impl TurtleWriter<'_> {
    fn name(&mut self, iri: &str) -> String {
        if iri == v(RDF, "type") {
            return "a".into();
        }
        let c = iri::compact_with(&self.g.prefixes, &self.g.namespace, iri);
        if c == iri {
            return format!("<{}>", iri);
        }
        let p = c.split(':').next().unwrap_or("").to_string();
        self.used.insert(p);
        c
    }

    fn obj(&mut self, o: &Obj, indent: usize) -> String {
        match o {
            Obj::Iri(i) => {
                let n = self.name(i);
                if n == "a" {
                    let c = iri::compact_with(&self.g.prefixes, &self.g.namespace, i);
                    self.used.insert("rdf".into());
                    c
                } else {
                    n
                }
            }
            Obj::Lit(s, dt, lang) => {
                if let Some(l) = lang {
                    return format!("{}@{}", turtle_string(s), l);
                }
                match dt.as_deref() {
                    None => turtle_string(s),
                    Some(d) if d == v(XSD, "string") => turtle_string(s),
                    Some(d) if d == v(XSD, "integer") && s.parse::<i64>().is_ok() && !s.starts_with('+') => s.clone(),
                    Some(d) if d == v(XSD, "boolean") && (s == "true" || s == "false") => s.clone(),
                    Some(d) if d == v(XSD, "decimal") && s.contains('.') && s.parse::<f64>().is_ok() && !s.ends_with('.') && !s.starts_with('.') && !s.starts_with("-.") => s.clone(),
                    Some(d) => format!("{}^^{}", turtle_string(s), self.name(d)),
                }
            }
            Obj::List(items) => {
                if items.is_empty() {
                    self.used.insert("rdf".into());
                    return "rdf:nil".into();
                }
                let parts: Vec<String> = items.iter().map(|x| self.obj(x, indent)).collect();
                format!("( {} )", parts.join(" "))
            }
            Obj::Node(props) => {
                if props.is_empty() {
                    return "[]".into();
                }
                let pad = " ".repeat(indent + 4);
                let body = self.props(props, indent + 4);
                if props.len() <= 2 && !body.contains('\n') {
                    format!("[ {} ]", body.replace(&format!(" ;\n{}", pad), " ; "))
                } else {
                    format!("[\n{}{}\n{}]", pad, body, " ".repeat(indent))
                }
            }
        }
    }

    /// `p o , o ;\n    p o` with the same predicate's objects together.
    fn props(&mut self, props: &[(String, Obj)], indent: usize) -> String {
        let mut grouped: IndexMap<&str, Vec<&Obj>> = IndexMap::new();
        for (p, o) in props {
            grouped.entry(p.as_str()).or_default().push(o);
        }
        let pad = " ".repeat(indent);
        let mut lines = Vec::new();
        for (p, objs) in grouped {
            let pn = self.name(p);
            let os: Vec<String> = objs.iter().map(|o| self.obj(o, indent)).collect();
            lines.push(format!("{} {}", pn, os.join(" , ")));
        }
        lines.join(&format!(" ;\n{}", pad))
    }
}

impl Graph {
    pub fn turtle(&self) -> String {
        let mut w = TurtleWriter { g: self, used: BTreeSet::new() };
        let mut body = String::new();
        for b in &self.blocks {
            let s = match &b.subject {
                Some(i) => w.name(i),
                None => "[]".into(),
            };
            let props = w.props(&b.props, 4);
            body.push_str(&format!("{} {} .\n\n", s, props));
        }
        let mut head = format!(
            "# {}: OWL, SKOS and SHACL, written by `lorefile export --rdf`.\n\
             # Entities are OWL classes, fields and links OWL properties, glossary words SKOS labels.\n\
             # SHACL shapes say how each entity uses its fields and links, and carry the guardrails.\n\
             # The lorefile: words hold what the standards have no home for (actions, metrics,\n\
             # functions, sources, owners, each guardrail's CEL rule); `lorefile import --from-rdf`\n\
             # reads all of it back.\n\n",
            self.name
        );
        head.push_str(&format!("@prefix : <{}> .\n", self.namespace));
        for (p, base) in &self.prefixes {
            if w.used.contains(p) && !p.is_empty() {
                head.push_str(&format!("@prefix {}: <{}> .\n", p, base));
            }
        }
        head.push('\n');
        head + body.trim_end() + "\n"
    }

    /// Every triple, blank nodes numbered in order. A node's own triples come
    /// before its nested nodes', so each subject's triples are together.
    pub fn triples(&self) -> Result<Vec<Triple>> {
        let mut out = Vec::new();
        let mut n = 0usize;
        let mut fresh = || {
            n += 1;
            BlankNode::new_unchecked(format!("b{}", n))
        };
        let named = |i: &str| NamedNode::new(i).map_err(|e| anyhow!("`{}` is not a valid IRI: {}", i, e));
        for b in &self.blocks {
            let subject: NamedOrBlankNode = match &b.subject {
                Some(i) => named(i)?.into(),
                None => fresh().into(),
            };
            let mut queue: Pending = VecDeque::from([(subject, b.props.clone())]);
            while let Some((s, props)) = queue.pop_front() {
                for (p, o) in props {
                    let pred = named(&p)?;
                    let term = Self::term(&o, &mut fresh, &mut queue, &mut out)?;
                    out.push(Triple::new(s.clone(), pred, term));
                }
            }
        }
        Ok(out)
    }

    fn term(o: &Obj, fresh: &mut dyn FnMut() -> BlankNode, queue: &mut Pending, out: &mut Vec<Triple>) -> Result<Term> {
        Ok(match o {
            Obj::Iri(i) => NamedNode::new(i.as_str()).map_err(|e| anyhow!("`{}` is not a valid IRI: {}", i, e))?.into(),
            Obj::Lit(s, dt, lang) => match (dt, lang) {
                (_, Some(l)) => Literal::new_language_tagged_literal(s.as_str(), l.as_str()).map_err(|e| anyhow!("{}", e))?.into(),
                (Some(d), None) => Literal::new_typed_literal(s.as_str(), NamedNode::new(d.as_str()).map_err(|e| anyhow!("{}", e))?).into(),
                (None, None) => Literal::new_simple_literal(s.as_str()).into(),
            },
            Obj::Node(props) => {
                let b = fresh();
                queue.push_back((b.clone().into(), props.clone()));
                b.into()
            }
            Obj::List(items) => {
                if items.is_empty() {
                    return Ok(NamedNode::new_unchecked(v(RDF, "nil")).into());
                }
                let heads: Vec<BlankNode> = items.iter().map(|_| fresh()).collect();
                for (k, item) in items.iter().enumerate() {
                    let t = Self::term(item, fresh, queue, out)?;
                    out.push(Triple::new(heads[k].clone(), NamedNode::new_unchecked(v(RDF, "first")), t));
                    let rest: Term = if k + 1 < heads.len() { heads[k + 1].clone().into() } else { NamedNode::new_unchecked(v(RDF, "nil")).into() };
                    out.push(Triple::new(heads[k].clone(), NamedNode::new_unchecked(v(RDF, "rest")), rest));
                }
                heads[0].clone().into()
            }
        })
    }

    /// Prefixes for the formats that need named ones: the space's namespace is `space`.
    fn named_prefixes(&self) -> Vec<(String, String)> {
        let mut out = vec![("space".to_string(), self.namespace.clone())];
        for (p, b) in &self.prefixes {
            if !p.is_empty() && p != "space" {
                out.push((p.clone(), b.clone()));
            }
        }
        out
    }

    pub fn write(&self, format: Format) -> Result<String> {
        Ok(match format {
            Format::Turtle => self.turtle(),
            Format::NTriples => {
                let mut ser = oxttl::NTriplesSerializer::new().for_writer(Vec::new());
                for t in self.triples()? {
                    ser.serialize_triple(&t)?;
                }
                String::from_utf8(ser.finish())?
            }
            Format::JsonLd => {
                let mut ser = oxjsonld::JsonLdSerializer::new();
                for (p, b) in self.named_prefixes() {
                    ser = ser.with_prefix(p, b).map_err(|e| anyhow!("{}", e))?;
                }
                let mut w = ser.for_writer(Vec::new());
                for t in self.triples()? {
                    w.serialize_quad(t.as_ref().in_graph(GraphNameRef::DefaultGraph))?;
                }
                let bytes = w.finish()?;
                let mut j: Json = serde_json::from_slice(&bytes).context("reading back the JSON-LD")?;
                let prefixes: IndexMap<String, String> = self.named_prefixes().into_iter().collect();
                compact_jsonld(&mut j, &prefixes);
                serde_json::to_string_pretty(&j)? + "\n"
            }
            Format::RdfXml => {
                let mut ser = oxrdfxml::RdfXmlSerializer::new();
                for (p, b) in self.named_prefixes() {
                    ser = ser.with_prefix(p, b).map_err(|e| anyhow!("{}", e))?;
                }
                let mut w = ser.for_writer(Vec::new());
                for t in self.triples()? {
                    w.serialize_triple(&t)?;
                }
                String::from_utf8(w.finish()?)?
            }
        })
    }
}

/// Writes IRIs as `prefix:local` where the `@context` defines the prefix:
/// property keys, `@id` and `@type` values. JSON-LD expands them back, so the
/// graph is the same and the file is a third of the size.
fn compact_jsonld(j: &mut Json, prefixes: &IndexMap<String, String>) {
    let short = |s: &str| -> String {
        let c = iri::compact_with(prefixes, "\u{0}", s);
        if c.starts_with(':') { s.to_string() } else { c }
    };
    match j {
        Json::Object(m) => {
            let entries: Vec<(String, Json)> = std::mem::take(m).into_iter().collect();
            for (k, mut val) in entries {
                if k == "@context" {
                    m.insert(k, val);
                    continue;
                }
                if k == "@id" || k == "@type" {
                    if let Json::String(s) = &val {
                        val = Json::String(short(s));
                    }
                    m.insert(k, val);
                    continue;
                }
                compact_jsonld(&mut val, prefixes);
                let key = if k.starts_with('@') { k } else { short(&k) };
                m.insert(key, val);
            }
        }
        Json::Array(a) => {
            for x in a {
                compact_jsonld(x, prefixes);
            }
        }
        _ => {}
    }
}

// ===========================================================================
// Reading RDF
// ===========================================================================

/// Turtle (and N-Triples, which Turtle reads), RDF/XML or JSON-LD: from the
/// extension, else from the first characters.
/// The statements of a file, and the prefixes it declared.
pub type Parsed = (Vec<Triple>, Vec<(String, String)>);

pub fn parse(path: &Path) -> Result<Parsed> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let base = format!("file://{}", path.canonicalize().unwrap_or(path.to_path_buf()).display()).replace(' ', "%20");
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).trim_start_matches('\u{feff}').trim_start().to_string();
    let format = match Format::from_path(path) {
        Some(Format::RdfXml) if !head.starts_with('<') => Format::Turtle,
        Some(f) => f,
        None if head.starts_with('<') && (head.starts_with("<?xml") || head.starts_with("<rdf:RDF") || head.contains("xmlns")) => Format::RdfXml,
        None if head.starts_with('{') || head.starts_with('[') => Format::JsonLd,
        None => Format::Turtle,
    };
    let where_ = |e: &dyn std::fmt::Display| anyhow!("{} is not valid {}: {}", path.display(), match format { Format::RdfXml => "RDF/XML", Format::JsonLd => "JSON-LD", _ => "Turtle or N-Triples" }, e);
    let mut triples = Vec::new();
    let prefixes: Vec<(String, String)> = match format {
        Format::Turtle | Format::NTriples => {
            let mut p = oxttl::TurtleParser::new().with_base_iri(base.as_str()).map_err(|e| anyhow!("{}", e))?.for_slice(&bytes);
            for t in p.by_ref() {
                triples.push(t.map_err(|e| where_(&e))?);
            }
            p.prefixes().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        }
        Format::RdfXml => {
            let mut p = oxrdfxml::RdfXmlParser::new().with_base_iri(base.as_str()).map_err(|e| anyhow!("{}", e))?.for_slice(&bytes);
            for t in p.by_ref() {
                triples.push(t.map_err(|e| where_(&e))?);
            }
            p.prefixes().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        }
        Format::JsonLd => {
            let mut p = oxjsonld::JsonLdParser::new().with_base_iri(base.as_str()).map_err(|e| anyhow!("{}", e))?.for_slice(&bytes);
            for q in p.by_ref() {
                let q = q.map_err(|e| where_(&e))?;
                triples.push(Triple::new(q.subject, q.predicate, q.object));
            }
            p.prefixes().map(|(a, b)| (a.to_string(), b.to_string())).collect()
        }
    };
    Ok((triples, prefixes))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Id {
    N(String),
    B(String),
}

fn term_id(t: &Term) -> Option<Id> {
    match t {
        Term::NamedNode(n) => Some(Id::N(n.as_str().to_string())),
        Term::BlankNode(b) => Some(Id::B(b.as_str().to_string())),
        _ => None,
    }
}

fn subject_id(s: &NamedOrBlankNode) -> Id {
    match s {
        NamedOrBlankNode::NamedNode(n) => Id::N(n.as_str().to_string()),
        NamedOrBlankNode::BlankNode(b) => Id::B(b.as_str().to_string()),
    }
}

/// The graph, indexed by subject.
struct G {
    props: HashMap<Id, Vec<(String, Term)>>,
    order: Vec<Id>,
}

impl G {
    fn new(triples: &[Triple]) -> G {
        let mut props: HashMap<Id, Vec<(String, Term)>> = HashMap::new();
        let mut order = Vec::new();
        for t in triples {
            let s = subject_id(&t.subject);
            let e = props.entry(s.clone()).or_insert_with(|| {
                order.push(s.clone());
                Vec::new()
            });
            e.push((t.predicate.as_str().to_string(), t.object.clone()));
        }
        G { props, order }
    }
    fn all<'a>(&'a self, s: &Id, p: &str) -> Vec<&'a Term> {
        self.props.get(s).map(|v| v.iter().filter(|(q, _)| q == p).map(|(_, o)| o).collect()).unwrap_or_default()
    }
    fn iris(&self, s: &Id, p: &str) -> Vec<String> {
        self.all(s, p).into_iter().filter_map(|t| if let Term::NamedNode(n) = t { Some(n.as_str().to_string()) } else { None }).collect()
    }
    fn ids(&self, s: &Id, p: &str) -> Vec<Id> {
        self.all(s, p).into_iter().filter_map(term_id).collect()
    }
    fn literals(&self, s: &Id, p: &str) -> Vec<&Literal> {
        self.all(s, p).into_iter().filter_map(|t| if let Term::Literal(l) = t { Some(l) } else { None }).collect()
    }
    /// The best literal: no language, then English, then any.
    fn text(&self, s: &Id, p: &str) -> Option<String> {
        let ls = self.literals(s, p);
        ls.iter()
            .find(|l| l.language().is_none())
            .or_else(|| ls.iter().find(|l| l.language().map(|x| x.to_ascii_lowercase().starts_with("en")).unwrap_or(false)))
            .or_else(|| ls.first())
            .map(|l| l.value().to_string())
    }
    /// All literals without a language or in English.
    fn texts(&self, s: &Id, p: &str) -> Vec<String> {
        self.literals(s, p)
            .into_iter()
            .filter(|l| l.language().map(|x| x.to_ascii_lowercase().starts_with("en")).unwrap_or(true))
            .map(|l| l.value().to_string())
            .collect()
    }
    fn int(&self, s: &Id, p: &str) -> Option<i64> {
        self.literals(s, p).first().and_then(|l| l.value().trim().parse::<f64>().ok()).map(|f| f as i64)
    }
    fn flag(&self, s: &Id, p: &str) -> bool {
        self.literals(s, p).iter().any(|l| l.value() == "true" || l.value() == "1")
    }
    fn json(&self, s: &Id, p: &str) -> Option<Json> {
        self.literals(s, p).first().and_then(|l| serde_json::from_str(l.value()).ok())
    }
    fn is_a(&self, s: &Id, t: &str) -> bool {
        self.all(s, &v(RDF, "type")).into_iter().any(|x| matches!(x, Term::NamedNode(n) if n.as_str() == t))
    }
    fn of_type(&self, t: &str) -> Vec<Id> {
        self.order.iter().filter(|s| self.is_a(s, t)).cloned().collect()
    }
    /// The members of an RDF list.
    fn list(&self, head: &Term) -> Vec<Term> {
        let mut out = Vec::new();
        let mut cur = term_id(head);
        let mut seen = HashSet::new();
        while let Some(id) = cur {
            if id == Id::N(v(RDF, "nil")) || !seen.insert(id.clone()) {
                break;
            }
            match self.all(&id, &v(RDF, "first")).first() {
                Some(t) => out.push((*t).clone()),
                None => break,
            }
            cur = self.all(&id, &v(RDF, "rest")).first().and_then(|t| term_id(t));
        }
        out
    }
    /// Named classes from `p` (an IRI, or a blank `owl:unionOf` list).
    fn class_refs(&self, s: &Id, p: &str) -> Vec<String> {
        let mut out = Vec::new();
        for t in self.all(s, p) {
            match t {
                Term::NamedNode(n) => out.push(n.as_str().to_string()),
                Term::BlankNode(b) => {
                    let bid = Id::B(b.as_str().to_string());
                    for head in self.all(&bid, &v(OWL, "unionOf")) {
                        for m in self.list(head) {
                            if let Term::NamedNode(n) = m {
                                out.push(n.as_str().to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}

fn local_name(iri: &str) -> String {
    let cut = iri.rfind(['#', '/']).or_else(|| iri.rfind(':')).map(|i| i + 1).unwrap_or(0);
    let l = &iri[cut..];
    percent_decode(l)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(x) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next().map(|x| x.is_ascii_alphabetic() || x == '_').unwrap_or(false) && s.chars().all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// `purchase order` → `PurchaseOrder`.
fn pascal(s: &str) -> String {
    let mut out = String::new();
    for w in s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let mut cs = w.chars();
        if let Some(f) = cs.next() {
            out.push(f.to_ascii_uppercase());
            out.push_str(cs.as_str());
        }
    }
    if out.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        out.insert(0, 'C');
    }
    out
}

/// `orderDate` → `order_date`; `order date` → `order_date`.
fn snake_name(s: &str) -> String {
    let mut out = snake(s.trim()).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect::<String>();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() || out.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("f_{}", out)
    } else {
        out
    }
}

/// schema.org writes `[[Order]]` for links and some HTML; keep the words.
fn clean_text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("[[") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("]]") {
            Some(j) => {
                out.push_str(&rest[i + 2..i + 2 + j]);
                rest = &rest[i + 4 + j..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    // drop HTML tags
    let mut plain = String::new();
    let mut in_tag = false;
    for c in out.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => plain.push(c),
            _ => {}
        }
    }
    plain.replace("\\n", " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn yaml_of(l: &Literal) -> Yaml {
    let dt = l.datatype().as_str().to_string();
    let s = l.value();
    let num = |t: &str| serde_yaml::from_str::<Yaml>(t).ok().filter(|y| y.is_number());
    if dt == v(XSD, "boolean") {
        return Yaml::Bool(s == "true" || s == "1");
    }
    if let Some(k) = kind_of_datatype(&dt, false) {
        if k == "integer" || k == "number" {
            if let Some(n) = num(s) {
                return n;
            }
        }
    }
    Yaml::String(s.to_string())
}

fn yaml_to_json(y: &Yaml) -> Json {
    serde_json::to_value(y).unwrap_or(Json::Null)
}

/// A literal as a CEL literal.
fn cel_of(l: &Literal) -> String {
    match yaml_of(l) {
        Yaml::Bool(b) => b.to_string(),
        Yaml::Number(n) => n.to_string(),
        _ => serde_json::to_string(l.value()).unwrap_or_default(),
    }
}

// ===========================================================================
// Import
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FileKind {
    Manifest,
    Types,
    Relationships,
    Constraints,
    Actions,
    Sources,
    Metrics,
    Functions,
    Glossary,
}

/// Objects grouped into files, the way the space had them when the graph says
/// so (`lorefile:file`), else a plain default layout.
#[derive(Default)]
struct Layout {
    files: IndexMap<String, (FileKind, Vec<Json>)>,
}

fn safe_rel(p: &str) -> Option<String> {
    let path = Path::new(p);
    if p.is_empty() || p.contains('\\') || path.is_absolute() {
        return None;
    }
    // the grades file is never a place an imported object may ask to be written:
    // a file from outside could otherwise carry a ready-made trust.yaml in
    if crate::edit::is_trust_file(p) {
        return None;
    }
    let ext = path.extension()?.to_str()?;
    if ext != "yaml" && ext != "yml" {
        return None;
    }
    for c in path.components() {
        match c {
            Component::Normal(x) => {
                let x = x.to_string_lossy();
                if x.starts_with('.') || x == "data" || x == "target" {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(p.to_string())
}

impl Layout {
    fn add(&mut self, kind: FileKind, wanted: Option<&str>, default: &str, obj: Json) {
        let fits = |l: &Layout, p: &str| l.files.get(p).map(|(k, _)| *k == kind).unwrap_or(true);
        let mut path = wanted.and_then(safe_rel).filter(|p| fits(self, p)).unwrap_or_else(|| default.to_string());
        let mut n = 2;
        while !fits(self, &path) {
            path = format!("{}-{}.yaml", default.trim_end_matches(".yaml"), n);
            n += 1;
        }
        self.files.entry(path).or_insert((kind, Vec::new())).1.push(obj);
    }
}

fn prune(v: &mut Json, keys: &[&str]) {
    if let Some(m) = v.as_object_mut() {
        for k in keys {
            let empty = match m.get(*k) {
                Some(Json::Null) | Some(Json::Bool(false)) => true,
                Some(Json::String(x)) => x.is_empty(),
                Some(Json::Array(x)) => x.is_empty(),
                Some(Json::Object(x)) => x.is_empty(),
                _ => false,
            };
            if empty {
                m.shift_remove(*k);
            }
        }
    }
}

fn prune_properties(props: &mut Json) {
    if let Some(m) = props.as_object_mut() {
        for (_, p) in m.iter_mut() {
            if p.is_object() {
                prune(p, &["key", "required", "description", "values", "unit", "iri"]);
                if p.as_object().map(|o| o.len() == 1 && o.contains_key("type")).unwrap_or(false) {
                    *p = p["type"].clone();
                }
            }
        }
    }
}

fn tidy(kind: FileKind, mut v: Json) -> Json {
    match kind {
        FileKind::Types => {
            prune_properties(&mut v["properties"]);
            if let Some(rels) = v.get_mut("relationships").and_then(|r| r.as_object_mut()) {
                for (_, r) in rels.iter_mut() {
                    prune_properties(&mut r["properties"]);
                    prune(r, &["cardinality", "description", "properties", "source", "iri"]);
                }
            }
            prune(&mut v, &["iri", "description", "domain", "properties", "relationships", "source", "owner", "tags", "instances"]);
        }
        FileKind::Relationships => {
            prune_properties(&mut v["properties"]);
            prune(&mut v, &["iri", "cardinality", "description", "properties", "source"]);
        }
        FileKind::Constraints => prune(&mut v, &["iri", "description", "sql", "rationale", "owner", "tags"]),
        FileKind::Actions => prune(&mut v, &["iri", "description", "inputs", "effects", "checks", "requires_approval_from", "side_effects", "tags"]),
        FileKind::Sources => prune(&mut v, &["description", "server", "path"]),
        FileKind::Metrics => prune(&mut v, &[]),
        FileKind::Functions => prune(&mut v, &["description", "inputs", "outputs"]),
        FileKind::Glossary => prune(&mut v, &["iri", "description"]),
        FileKind::Manifest => prune(&mut v, &["version", "description", "domains", "namespace", "prefixes"]),
    }
    v
}

fn render(kind: FileKind, objs: Vec<Json>, header: &str) -> String {
    let objs: Vec<Json> = objs.into_iter().map(|o| tidy(kind, o)).collect();
    let single = objs.len() == 1;
    let doc = match kind {
        FileKind::Manifest => objs.into_iter().next().unwrap_or(json!({})),
        FileKind::Types if single => objs.into_iter().next().unwrap(),
        FileKind::Types => json!({ "types": objs }),
        FileKind::Relationships if single => objs.into_iter().next().unwrap(),
        FileKind::Relationships => json!({ "relationships": objs }),
        FileKind::Actions if single => objs.into_iter().next().unwrap(),
        FileKind::Actions => json!({ "actions": objs }),
        FileKind::Sources if single => objs.into_iter().next().unwrap(),
        FileKind::Sources => json!({ "sources": objs }),
        FileKind::Constraints => json!({ "constraints": objs }),
        FileKind::Metrics => json!({ "metrics": objs }),
        FileKind::Functions => json!({ "functions": objs }),
        FileKind::Glossary => json!({ "terms": objs }),
    };
    format!("{}{}", header, crate::ossie::to_yaml(&doc))
}

/// What an import is told about the space it goes into.
pub struct Target {
    /// The space's compiled ontology, when it exists already.
    pub existing: Option<Ontology>,
    /// The folder name, for a new space with no name of its own.
    pub folder_name: String,
}

#[derive(Default)]
pub struct ImportOptions {
    /// Classes to import (names, `prefix:Name`, IRIs, or a namespace such as `schema:`).
    pub only: Vec<String>,
    /// How many links away from those classes to reach.
    pub depth: usize,
    /// Restore the grades a Lorefile export carries (`lorefile:Grading`) where they still match.
    pub keep_grades: bool,
    /// Keep `rdfs:subClassOf` as `subclass-of:` tags, as before kinds, instead of `extends`
    /// with the parents brought along.
    pub no_parents: bool,
}

/// The files an import produced, and what it guessed, dropped or kept.
pub struct Imported {
    pub files: Vec<(String, String)>,
    /// The grades the file claims, as Lorefile wrote them; imports start ungraded unless `--keep-grades`.
    pub trust: Option<TrustBook>,
    pub notes: Vec<String>,
    pub counts: IndexMap<&'static str, usize>,
}

struct Ent {
    json: Json,
    file: Option<String>,
    order: i64,
    /// Written by Lorefile (exact restore) or read from another tool's graph.
    ours: bool,
}

struct Notes {
    guessed: Vec<String>,
    dropped: Vec<String>,
    kept: Vec<String>,
    todo: Vec<String>,
}

const RESERVED_NS: &[&str] = &[RDF, RDFS, OWL, XSD, SH, SKOS, LF, VANN, DCTERMS, PROV];

pub fn import(path: &Path, target: &Target, opts: &ImportOptions) -> Result<Imported> {
    let (triples, file_prefixes) = parse(path)?;
    if triples.is_empty() {
        bail!("{} holds no RDF statements", path.display());
    }
    let g = G::new(&triples);
    let mut notes = Notes { guessed: vec![], dropped: vec![], kept: vec![], todo: vec![] };
    let rdfs_label = v(RDFS, "label");
    let rdfs_comment = v(RDFS, "comment");

    // ---- the space: name, namespace, prefixes
    let onto = g.of_type(&v(OWL, "Ontology")).into_iter().next();
    let ours_graph = onto.as_ref().map(|o| g.int(o, &v(LF, "formatVersion")).is_some()).unwrap_or(false);
    if let Some(fv) = onto.as_ref().and_then(|o| g.int(o, &v(LF, "formatVersion"))) {
        if fv > FORMAT_VERSION {
            notes.guessed.push(format!("the graph says lorefile:formatVersion {}; this build reads {}", fv, FORMAT_VERSION));
        }
    }
    let src_name = onto.as_ref().and_then(|o| g.text(o, &rdfs_label).or_else(|| g.text(o, &v(DCTERMS, "title"))));
    let mut manifest_prefixes: IndexMap<String, String> = IndexMap::new();
    if let Some(o) = &onto {
        for d in g.ids(o, &v(SH, "declare")) {
            if let (Some(p), Some(n)) = (g.text(&d, &v(SH, "prefix")), g.text(&d, &v(SH, "namespace"))) {
                manifest_prefixes.insert(p, n);
            }
        }
    }
    let (space_name, ns, prefixes, declare_ns) = match &target.existing {
        Some(ont) => (ont.manifest.name.clone(), ont.iris.namespace.clone(), ont.iris.prefixes.clone(), None),
        None => {
            let name = src_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| target.folder_name.clone());
            let default_ns = iri::default_namespace(&name);
            let src_ns = onto.as_ref().and_then(|o| g.text(o, &v(VANN, "preferredNamespaceUri"))).filter(|n| iri::is_absolute(n));
            let declare = if ours_graph { src_ns.filter(|n| n != &default_ns) } else { None };
            let ns = declare.clone().unwrap_or(default_ns);
            let mut prefixes: IndexMap<String, String> = IndexMap::new();
            for (p, b) in iri::BUILT_IN {
                prefixes.insert(p.to_string(), b.to_string());
            }
            for (p, b) in &manifest_prefixes {
                prefixes.insert(p.clone(), b.clone());
            }
            (name, ns, prefixes, declare)
        }
    };
    // the file's own prefixes may name what we write, for a new space only
    let mut extra_prefixes: IndexMap<String, String> = IndexMap::new();
    if target.existing.is_none() {
        for (p, b) in &file_prefixes {
            if !p.is_empty() && iri::is_prefix_name(p) && !prefixes.contains_key(p) && !prefixes.values().any(|x| x == b) && iri::is_absolute(b) && b != &ns {
                extra_prefixes.insert(p.clone(), b.clone());
            }
        }
    }
    let existing_types: HashMap<String, String> = target.existing.as_ref().map(|o| o.types.keys().map(|k| (k.clone(), o.iri_type(k))).collect()).unwrap_or_default();

    // ---- datatypes (never entities)
    let mut datatypes: HashSet<String> = HashSet::new();
    for s in &g.order {
        if let Id::N(i) = s {
            if g.is_a(s, &v(RDFS, "Datatype")) || g.is_a(s, &v(SCHEMA, "DataType")) || g.is_a(s, "http://schema.org/DataType") || i.starts_with(XSD) {
                datatypes.insert(i.clone());
                // the class of datatypes (schema:DataType) is not an entity either
                for t in g.iris(s, &v(RDF, "type")) {
                    if !RESERVED_NS.iter().any(|n| t.starts_with(n)) {
                        datatypes.insert(t);
                    }
                }
            }
            // a class of classes is not an entity
            if g.iris(s, &v(RDFS, "subClassOf")).iter().any(|p| p == &v(RDFS, "Class") || p == &v(OWL, "Class") || p == &v(RDFS, "Datatype")) {
                datatypes.insert(i.clone());
            }
        }
    }
    loop {
        let before = datatypes.len();
        for s in &g.order {
            if let Id::N(i) = s {
                if !datatypes.contains(i) && g.iris(s, &v(RDFS, "subClassOf")).iter().any(|p| datatypes.contains(p)) {
                    datatypes.insert(i.clone());
                }
            }
        }
        if datatypes.len() == before {
            break;
        }
    }
    let is_datatype = |i: &str| datatypes.contains(i) || kind_of_datatype(i, false).is_some();
    let reserved = |i: &str| RESERVED_NS.iter().any(|n| i.starts_with(n));

    // ---- property nodes (another tool's)
    let prop_types = [v(OWL, "DatatypeProperty"), v(OWL, "ObjectProperty"), v(RDF, "Property"), v(OWL, "FunctionalProperty")];
    let domain_preds = [v(RDFS, "domain"), v(SCHEMA, "domainIncludes"), "http://schema.org/domainIncludes".to_string()];
    let range_preds = [v(RDFS, "range"), v(SCHEMA, "rangeIncludes"), "http://schema.org/rangeIncludes".to_string()];
    let props_ids: Vec<Id> = g
        .order
        .iter()
        .filter(|s| matches!(s, Id::N(i) if !reserved(i)))
        .filter(|s| prop_types.iter().any(|t| g.is_a(s, t)) || domain_preds.iter().any(|p| !g.all(s, p).is_empty()))
        .filter(|s| !g.is_a(s, &v(OWL, "AnnotationProperty")) && !g.is_a(s, &v(LF, "Action")))
        .cloned()
        .collect();
    let domains_of = |s: &Id| -> Vec<String> { domain_preds.iter().flat_map(|p| g.class_refs(s, p)).collect() };
    let ranges_of = |s: &Id| -> Vec<String> { range_preds.iter().flat_map(|p| g.class_refs(s, p)).collect() };

    // ---- shapes
    let entity_shapes: HashMap<String, Id> = g
        .of_type(&v(LF, "EntityShape"))
        .into_iter()
        .filter_map(|s| g.iris(&s, &v(SH, "targetClass")).first().cloned().map(|c| (c, s)))
        .collect();
    let rule_shapes: Vec<Id> = g.order.iter().filter(|s| !g.literals(s, &v(LF, "cel")).is_empty()).cloned().collect();
    let foreign_shapes: Vec<Id> = g
        .order
        .iter()
        .filter(|s| g.is_a(s, &v(SH, "NodeShape")) || (!g.all(s, &v(SH, "property")).is_empty() && !g.all(s, &v(SH, "targetClass")).is_empty()))
        .filter(|s| !g.is_a(s, &v(LF, "EntityShape")) && g.literals(s, &v(LF, "cel")).is_empty())
        .cloned()
        .collect();

    // ---- classes
    let class_types = [v(OWL, "Class"), v(RDFS, "Class")];
    let mut candidates: Vec<String> = Vec::new();
    let push_class = |c: &str, cands: &mut Vec<String>| {
        if !reserved(c) && !is_datatype(c) && !cands.iter().any(|x| x == c) && c != v(OWL, "Thing") {
            cands.push(c.to_string());
        }
    };
    for s in &g.order {
        if let Id::N(i) = s {
            if class_types.iter().any(|t| g.is_a(s, t)) && !g.is_a(s, &v(SH, "NodeShape")) {
                push_class(i, &mut candidates);
            }
        }
    }
    for s in &foreign_shapes {
        for c in g.iris(s, &v(SH, "targetClass")) {
            push_class(&c, &mut candidates);
        }
    }
    // a property's domain is an entity when the file says something about that class
    let mut undescribed: BTreeSet<String> = BTreeSet::new();
    for p in &props_ids {
        for d in domains_of(p) {
            if g.props.contains_key(&Id::N(d.clone())) {
                push_class(&d, &mut candidates);
            } else if !reserved(&d) && !is_datatype(&d) {
                undescribed.insert(local_name(&d));
            }
        }
    }

    if !undescribed.is_empty() && opts.only.is_empty() {
        let shown: Vec<String> = undescribed.iter().take(12).cloned().collect();
        notes.dropped.push(format!(
            "properties also apply to {} classes the file does not describe, which were not made entities: {}{}",
            undescribed.len(),
            shown.join(", "),
            if undescribed.len() > shown.len() { ", …" } else { "" }
        ));
    }

    // --only and --depth
    let selected: Vec<String> = if opts.only.is_empty() {
        candidates.clone()
    } else {
        let mut all_prefixes: IndexMap<String, String> = prefixes.clone();
        for (p, b) in &file_prefixes {
            all_prefixes.entry(p.clone()).or_insert(b.clone());
        }
        let name_of = |c: &str| -> Vec<String> {
            let id = Id::N(c.to_string());
            let mut n = vec![local_name(c).to_lowercase()];
            if let Some(l) = g.text(&id, &rdfs_label) {
                n.push(l.to_lowercase());
            }
            n
        };
        let mut start: Vec<String> = Vec::new();
        for tok in opts.only.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            let before = start.len();
            let as_ns = if let Some(p) = tok.strip_suffix(':') { all_prefixes.get(p).cloned() } else if tok.contains("://") && (tok.ends_with('/') || tok.ends_with('#')) { Some(tok.to_string()) } else { None };
            if let Some(base) = as_ns {
                start.extend(candidates.iter().filter(|c| c.starts_with(&base)).cloned());
            } else if tok.contains(':') {
                let full = iri::expand(tok, &ns, &all_prefixes).unwrap_or_else(|_| tok.to_string());
                start.extend(candidates.iter().filter(|c| **c == full).cloned());
            } else {
                let low = tok.to_lowercase();
                start.extend(candidates.iter().filter(|c| name_of(c).contains(&low)).cloned());
            }
            if start.len() == before {
                bail!("--only `{}` matches no class in {}", tok, path.display());
            }
        }
        // neighbours: through properties, and up to parent classes
        let mut adj: HashMap<String, BTreeSet<String>> = HashMap::new();
        for p in &props_ids {
            for d in domains_of(p) {
                for r in ranges_of(p) {
                    if candidates.contains(&r) && candidates.contains(&d) {
                        adj.entry(d.clone()).or_default().insert(r.clone());
                        adj.entry(r.clone()).or_default().insert(d.clone());
                    }
                }
            }
        }
        for c in &candidates {
            for parent in g.iris(&Id::N(c.clone()), &v(RDFS, "subClassOf")) {
                if candidates.contains(&parent) {
                    adj.entry(c.clone()).or_default().insert(parent);
                }
            }
        }
        let mut seen: HashSet<String> = start.iter().cloned().collect();
        let mut frontier = start.clone();
        for _ in 0..opts.depth {
            let mut next = Vec::new();
            for c in &frontier {
                for n in adj.get(c).into_iter().flatten() {
                    if seen.insert(n.clone()) {
                        next.push(n.clone());
                    }
                }
            }
            frontier = next;
        }
        // parents come along (up to six, never owl:Thing), so fields on a parent reach its kinds
        if !opts.no_parents {
            let mut todo: Vec<(String, usize)> = seen.iter().map(|c| (c.clone(), 0)).collect();
            while let Some((c, depth)) = todo.pop() {
                if depth >= 6 {
                    continue;
                }
                for p in g.iris(&Id::N(c.clone()), &v(RDFS, "subClassOf")) {
                    if candidates.contains(&p) && p != v(OWL, "Thing") && seen.insert(p.clone()) {
                        todo.push((p, depth + 1));
                    }
                }
            }
        }
        let picked: Vec<String> = candidates.iter().filter(|c| seen.contains(*c)).cloned().collect();
        notes.guessed.push(format!(
            "--only kept {} of {} classes{}",
            picked.len(),
            candidates.len(),
            if opts.depth > 0 { format!(" (the ones named and those up to {} link{} away)", opts.depth, if opts.depth == 1 { "" } else { "s" }) } else { " (add --depth 1 to bring the classes they link to)".into() }
        ));
        picked
    };

    // ---- entities
    let mut entities: IndexMap<String, Ent> = IndexMap::new();
    let mut class_name: HashMap<String, String> = HashMap::new();
    let mut taken: HashSet<String> = existing_types.keys().map(|k| k.to_lowercase()).collect();
    let mut file_order = 0i64;
    for c in &selected {
        let id = Id::N(c.clone());
        let ours = !g.literals(&id, &v(LF, "file")).is_empty();
        let mut name = if ours {
            g.text(&id, &rdfs_label).unwrap_or_else(|| local_name(c))
        } else {
            let l = local_name(c);
            if is_ident(&l) {
                l
            } else {
                pascal(&g.text(&id, &rdfs_label).unwrap_or(l))
            }
        };
        if let Some(existing_iri) = existing_types.iter().find(|(k, _)| k.eq_ignore_ascii_case(&name)).map(|(_, v)| v.clone()) {
            if existing_iri == *c {
                notes.kept.push(format!("{} is already in the space; left as it is", name));
                class_name.insert(c.clone(), name.clone());
                continue;
            }
        }
        if taken.contains(&name.to_lowercase()) {
            let base = name.clone();
            let mut n = 2;
            while taken.contains(&name.to_lowercase()) {
                name = format!("{}{}", base, n);
                n += 1;
            }
            notes.guessed.push(format!("<{}> is called {} here: {} was taken", c, name, base));
        }
        taken.insert(name.to_lowercase());
        class_name.insert(c.clone(), name.clone());
        let desc = if ours {
            g.text(&id, &rdfs_comment).unwrap_or_default()
        } else {
            clean_text(&g.text(&id, &rdfs_comment).or_else(|| g.text(&id, &v(SKOS, "definition"))).unwrap_or_default())
        };
        let mut j = json!({ "type": name, "iri": null, "description": desc });
        if ours {
            if let Some(d) = g.text(&id, &v(LF, "domain")) {
                j["domain"] = json!(d);
            }
        }
        j["properties"] = json!({});
        j["relationships"] = json!({});
        if ours {
            if let Some(src) = g.json(&id, &v(LF, "source")) {
                j["source"] = src;
            }
            j["owner"] = json!(g.text(&id, &v(LF, "owner")).unwrap_or_default());
            j["tags"] = json!(g.texts(&id, &v(DCTERMS, "subject")));
            if let Some(inst) = g.json(&id, &v(LF, "instances")) {
                j["instances"] = inst;
            }
            if let Some(e) = g.text(&id, &v(LF, "extends")) {
                j["extends"] = json!(e);
            }
            if let Some(w) = g.json(&id, &v(LF, "where")) {
                j["where"] = w;
            }
        } else {
            let mut parents: Vec<String> = g.iris(&id, &v(RDFS, "subClassOf")).into_iter().filter(|p| !reserved(p) && p != &v(OWL, "Thing")).collect();
            parents.sort();
            let mut tags = Vec::new();
            // the first imported parent (by IRI) becomes `extends`; the rest stay tags. Equivalent
            // classes (each a subclass of the other) get no `extends`, which would go in a circle.
            let circular = |p: &str| g.iris(&Id::N(p.to_string()), &v(RDFS, "subClassOf")).iter().any(|x| x == c);
            let chosen = if opts.no_parents { None } else { parents.iter().find(|p| selected.contains(*p) && !circular(p)).cloned() };
            for p in &parents {
                let pn = class_name.get(p).cloned().unwrap_or_else(|| local_name(p));
                if Some(p) == chosen.as_ref() {
                    j["extends"] = json!(pn);
                    notes.guessed.push(format!("{} is a kind of {} (rdfs:subClassOf): saved as extends: {}", name, pn, pn));
                    continue;
                }
                tags.push(format!("subclass-of:{}", pn));
                if circular(p) {
                    notes.guessed.push(format!("{} and {} are each a subclass of the other (equivalent classes): no extends for either, kept as the tag subclass-of:{}", name, pn, pn));
                } else if chosen.is_some() {
                    notes.guessed.push(format!("{} is also a kind of {}: an entity extends one other, so it is kept as the tag subclass-of:{}", name, pn, pn));
                } else {
                    notes.guessed.push(format!("{} is a kind of {} (rdfs:subClassOf): kept as the tag subclass-of:{}{}", name, pn, pn, if opts.no_parents { " (--no-parents)" } else { ", since that class is not imported" }));
                }
            }
            if g.all(&id, &v(RDFS, "subClassOf")).iter().any(|t| matches!(t, Term::BlankNode(_))) {
                notes.dropped.push(format!("{}: OWL restrictions on the class (used only for reasoning)", name));
            }
            j["owner"] = json!("");
            j["tags"] = json!(tags);
        }
        let full = c.clone();
        if full != iri::derive(&ns, &Key::Entity(name.clone()), &[]) {
            j["iri"] = json!(full);
        }
        let order = if ours { g.int(&id, &v(LF, "order")).unwrap_or(1_000_000 + file_order) } else { 1_000_000 + file_order };
        file_order += 1;
        entities.insert(name, Ent { json: j, file: g.text(&id, &v(LF, "file")).filter(|_| ours), order, ours });
    }
    let name_of_class = |c: &str| -> String {
        class_name.get(c).cloned().unwrap_or_else(|| match c.strip_prefix(ns.as_str()) {
            Some(rest) => percent_decode(rest),
            None => existing_types.iter().find(|(_, v)| v.as_str() == c).map(|(k, _)| k.clone()).unwrap_or_else(|| local_name(c)),
        })
    };

    // ---- fields and links from Lorefile's entity shapes
    struct Standalone {
        json: Json,
        file: String,
        order: i64,
    }
    let mut standalone: Vec<Standalone> = Vec::new();
    let mut covered: HashSet<(String, String)> = HashSet::new(); // (class iri, property iri)
    let mut link_iris: HashMap<String, Vec<(String, String)>> = HashMap::new(); // property iri → (entity, link)
    for (class, shape) in &entity_shapes {
        let ename = name_of_class(class);
        let owned = entities.get(&ename).map(|e| e.ours).unwrap_or(false);
        if !owned && entities.contains_key(&ename) {
            continue;
        }
        if !owned && !opts.only.is_empty() {
            continue;
        }
        let mut fields: Vec<(i64, String, Json)> = Vec::new();
        let mut links: Vec<(i64, String, Json)> = Vec::new();
        for ps in g.ids(shape, &v(SH, "property")) {
            let Some(path) = g.iris(&ps, &v(SH, "path")).into_iter().next() else { continue };
            covered.insert((class.clone(), path.clone()));
            let Some(fname) = g.text(&ps, &v(SH, "name")) else { continue };
            let order = g.int(&ps, &v(SH, "order")).unwrap_or(0);
            let desc = g.text(&ps, &v(SH, "description")).unwrap_or_default();
            if let Some(to) = g.iris(&ps, &v(SH, "class")).into_iter().next() {
                let mut l = json!({ "to": name_of_class(&to), "iri": null, "cardinality": g.text(&ps, &v(LF, "cardinality")).unwrap_or_default(), "description": desc, "properties": g.json(&ps, &v(LF, "properties")).unwrap_or(json!({})) });
                if let Some(src) = g.json(&ps, &v(LF, "edgeSource")) {
                    l["source"] = src;
                }
                if path != iri::derive(&ns, &Key::Link(ename.clone(), fname.clone()), &[]) {
                    l["iri"] = json!(path);
                }
                link_iris.entry(path.clone()).or_default().push((ename.clone(), fname.clone()));
                let file = g.text(&ps, &v(LF, "file"));
                let ent_file = entities.get(&ename).and_then(|e| e.file.clone());
                match file {
                    Some(f) if Some(&f) != ent_file.as_ref() || !owned => {
                        let mut s = json!({ "relationship": fname, "iri": l["iri"].clone(), "from": ename, "to": l["to"].clone(), "cardinality": l["cardinality"].clone(), "description": l["description"].clone(), "properties": l["properties"].clone() });
                        if let Some(src) = l.get("source") {
                            s["source"] = src.clone();
                        }
                        standalone.push(Standalone { json: s, file: f, order: g.int(&ps, &v(LF, "order")).unwrap_or(0) });
                    }
                    _ => links.push((order, fname, l)),
                }
            } else {
                let values: Vec<Json> = g.all(&ps, &v(SH, "in")).first().map(|h| g.list(h)).unwrap_or_default().iter().filter_map(|t| if let Term::Literal(l) = t { Some(yaml_to_json(&yaml_of(l))) } else { None }).collect();
                let values = g.json(&ps, &v(LF, "values")).and_then(|x| x.as_array().cloned()).unwrap_or(values);
                let dt = g.iris(&ps, &v(SH, "datatype")).into_iter().next();
                let kind = g.text(&ps, &v(LF, "type")).unwrap_or_else(|| dt.as_deref().and_then(|d| kind_of_datatype(d, !values.is_empty())).unwrap_or("string").to_string());
                let mut f = json!({ "type": kind, "key": g.flag(&ps, &v(LF, "key")), "required": g.int(&ps, &v(SH, "minCount")).unwrap_or(0) >= 1, "description": desc, "values": values });
                if let Some(u) = g.text(&ps, &v(LF, "unit")) {
                    f["unit"] = json!(u);
                }
                if path != iri::derive(&ns, &Key::Field(ename.clone(), fname.clone()), &[]) {
                    f["iri"] = json!(path);
                }
                fields.push((order, fname, f));
            }
        }
        if let Some(e) = entities.get_mut(&ename) {
            fields.sort_by_key(|x| x.0);
            links.sort_by_key(|x| x.0);
            for (_, n, f) in fields {
                e.json["properties"][&n] = f;
            }
            for (_, n, l) in links {
                e.json["relationships"][&n] = l;
            }
        }
    }

    // ---- fields and links from another tool's properties
    let mut became_fields: Vec<String> = Vec::new();
    let mut no_target: Vec<String> = Vec::new();
    let mut extra_ranges: Vec<String> = Vec::new();
    let mut prop_count = 0usize;
    for p in &props_ids {
        let Id::N(piri) = p else { continue };
        let doms: Vec<String> = domains_of(p).into_iter().filter(|d| class_name.contains_key(d)).collect();
        if doms.is_empty() {
            continue;
        }
        let ranges = ranges_of(p);
        let dt_ranges: Vec<&String> = ranges.iter().filter(|r| is_datatype(r)).collect();
        let class_ranges: Vec<&String> = ranges.iter().filter(|r| !is_datatype(r) && !reserved(r)).collect();
        let picked_class = class_ranges.iter().find(|r| class_name.contains_key(**r)).map(|r| (*r).clone());
        let is_object = g.is_a(p, &v(OWL, "ObjectProperty"));
        let is_data = g.is_a(p, &v(OWL, "DatatypeProperty"));
        let label = local_name(piri);
        let fname = if is_ident(&label) { snake_name(&label) } else { snake_name(&g.text(p, &rdfs_label).unwrap_or(label.clone())) };
        let desc = clean_text(&g.text(p, &rdfs_comment).or_else(|| g.text(p, &v(SKOS, "definition"))).unwrap_or_default());
        for d in &doms {
            if covered.contains(&(d.clone(), piri.clone())) {
                continue;
            }
            let ename = class_name[d].clone();
            let Some(e) = entities.get_mut(&ename) else { continue };
            let as_link = !is_data && picked_class.is_some();
            if as_link {
                let to = name_of_class(picked_class.as_ref().unwrap());
                let functional = g.is_a(p, &v(OWL, "FunctionalProperty"));
                let inv = g.is_a(p, &v(OWL, "InverseFunctionalProperty"));
                let card = match (functional, inv) {
                    (true, true) => "one-to-one",
                    (true, false) => "many-to-one",
                    (false, true) => "one-to-many",
                    _ => "",
                };
                let mut name = fname.clone();
                while e.json["relationships"].get(&name).is_some() || e.json["properties"].get(&name).is_some() {
                    name.push('_');
                }
                e.json["relationships"][&name] = json!({ "to": to, "iri": piri, "cardinality": card, "description": desc, "properties": {} });
                link_iris.entry(piri.clone()).or_default().push((ename.clone(), name.clone()));
                prop_count += 1;
                let others: Vec<String> = class_ranges.iter().filter(|r| ***r != *picked_class.as_ref().unwrap()).map(|r| name_of_class(r)).collect();
                if !others.is_empty() {
                    extra_ranges.push(format!("{}.{} → {} (also {})", ename, name, to, others.join(", ")));
                }
            } else if is_object && dt_ranges.is_empty() {
                no_target.push(format!("{}.{} → {}", ename, fname, if class_ranges.is_empty() { "?".to_string() } else { class_ranges.iter().map(|r| name_of_class(r)).collect::<Vec<_>>().join(" | ") }));
            } else if dt_ranges.is_empty() && !class_ranges.is_empty() && !is_data {
                no_target.push(format!("{}.{} → {}", ename, fname, class_ranges.iter().map(|r| name_of_class(r)).collect::<Vec<_>>().join(" | ")));
            } else {
                let kind = dt_ranges.iter().find_map(|r| kind_of_datatype(r, false)).unwrap_or("string");
                if dt_ranges.is_empty() {
                    notes.guessed.push(format!("{}.{} says no range; read as text", ename, fname));
                } else if dt_ranges.iter().all(|r| kind_of_datatype(r, false).is_none()) {
                    notes.guessed.push(format!("{}.{}: datatype {} read as text", ename, fname, dt_ranges[0]));
                }
                if !class_ranges.is_empty() {
                    became_fields.push(format!("{}.{}", ename, fname));
                }
                let mut name = fname.clone();
                while e.json["properties"].get(&name).is_some() || e.json["relationships"].get(&name).is_some() {
                    name.push('_');
                }
                e.json["properties"][&name] = json!({ "type": kind, "key": false, "required": false, "description": desc, "values": [], "iri": piri });
                prop_count += 1;
            }
        }
    }
    if !became_fields.is_empty() {
        notes.guessed.push(format!("{} properties that can also point at a class not imported became plain fields: {}", became_fields.len(), became_fields.join(", ")));
    }
    if !no_target.is_empty() {
        notes.dropped.push(format!("{} links whose target class was not imported (name it in --only, or raise --depth): {}", no_target.len(), no_target.join(", ")));
    }
    if !extra_ranges.is_empty() {
        notes.guessed.push(format!("{} links range over several classes; a Lorefile link has one target, the first imported one: {}", extra_ranges.len(), extra_ranges.join("; ")));
    }
    // owl:hasKey marks the key field
    for (c, ename) in &class_name {
        let Some(e) = entities.get_mut(ename) else { continue };
        if e.ours {
            continue;
        }
        for head in g.all(&Id::N(c.clone()), &v(OWL, "hasKey")) {
            for k in g.list(head) {
                if let Term::NamedNode(kn) = k {
                    let field = e.json["properties"].as_object().and_then(|m| m.iter().find(|(_, f)| f["iri"] == json!(kn.as_str())).map(|(n, _)| n.clone()));
                    if let Some(f) = field {
                        e.json["properties"][&f]["key"] = json!(true);
                    }
                }
            }
        }
    }

    // ---- guardrails Lorefile wrote
    struct Rule {
        json: Json,
        file: Option<String>,
        order: i64,
    }
    let mut rules: Vec<Rule> = Vec::new();
    let mut guardrail_iris: HashMap<String, String> = HashMap::new();
    if opts.only.is_empty() {
        for s in &rule_shapes {
            let Some(rule) = g.text(s, &v(LF, "cel")) else { continue };
            let sid = match s {
                Id::N(i) => i.clone(),
                Id::B(_) => String::new(),
            };
            let id = g.text(s, &v(LF, "id")).unwrap_or_else(|| local_name(&sid));
            let applies = g.text(s, &v(LF, "appliesTo")).or_else(|| g.iris(s, &v(SH, "targetClass")).first().map(|c| name_of_class(c))).unwrap_or_default();
            let mut j = json!({
                "id": id, "iri": null, "name": g.text(s, &v(SH, "message")).unwrap_or_default(),
                "description": g.text(s, &rdfs_comment).unwrap_or_default(),
                "applies_to": applies, "rule": rule,
            });
            if let Some(sql) = g.text(s, &v(LF, "sql")) {
                j["sql"] = json!(sql);
            }
            j["severity"] = json!(severity_of(&g.iris(s, &v(SH, "severity")).first().cloned().unwrap_or_default()).to_string());
            j["rationale"] = json!(g.text(s, &v(LF, "rationale")).unwrap_or_default());
            j["owner"] = json!(g.text(s, &v(LF, "owner")).unwrap_or_default());
            j["tags"] = json!(g.texts(s, &v(DCTERMS, "subject")));
            if !sid.is_empty() && sid != iri::derive(&ns, &Key::Guardrail(id.clone()), &[]) {
                j["iri"] = json!(sid);
            }
            guardrail_iris.insert(sid.clone(), id.clone());
            rules.push(Rule { json: j, file: g.text(s, &v(LF, "file")), order: g.int(s, &v(LF, "order")).unwrap_or(1_000_000) });
        }
    } else if !rule_shapes.is_empty() {
        notes.dropped.push(format!("--only imports entities: {} Lorefile guardrails left out", rule_shapes.len()));
    }

    // ---- another tool's SHACL shapes: fields they describe, and guardrails
    let mut shacl_rules = 0usize;
    let mut shacl_kept: Vec<String> = Vec::new();
    let mut seen_ids: HashSet<String> = rules.iter().map(|r| r.json["id"].as_str().unwrap_or("").to_string()).collect();
    let mut rule_order = 2_000_000i64;
    for s in &foreign_shapes {
        let mut targets = g.iris(s, &v(SH, "targetClass"));
        if let Id::N(i) = s {
            if class_types.iter().any(|t| g.is_a(s, t)) {
                targets.push(i.clone());
            }
        }
        let shape_label = match s {
            Id::N(i) => local_name(i),
            Id::B(_) => String::new(),
        };
        if targets.is_empty() {
            if !g.all(s, &v(SH, "property")).is_empty() {
                shacl_kept.push(format!("{}: no sh:targetClass (other kinds of target are not read)", if shape_label.is_empty() { "a blank shape".into() } else { shape_label.clone() }));
            }
            continue;
        }
        if g.flag(s, &v(SH, "deactivated")) {
            notes.guessed.push(format!("shape {} is switched off (sh:deactivated); imported as guardrails anyway, for a person to look at", shape_label));
        }
        let shape_sev = g.iris(s, &v(SH, "severity")).first().cloned();
        let shape_msg = g.text(s, &v(SH, "message"));
        let shape_comment = g.text(s, &rdfs_comment).map(|c| clean_text(&c));
        for node_level in ["in", "closed", "sparql", "or", "and", "not", "xone", "node", "class", "hasValue", "pattern"] {
            if !g.all(s, &v(SH, node_level)).is_empty() {
                shacl_kept.push(format!("{}: sh:{} on the shape itself", shape_label, node_level));
            }
        }
        for c in &targets {
            let Some(ename) = class_name.get(c).cloned() else { continue };
            let Some(e) = entities.get_mut(&ename) else { continue };
            let pss = g.ids(s, &v(SH, "property"));
            let one = pss.len() == 1;
            for ps in pss {
                let Some(path) = g.iris(&ps, &v(SH, "path")).into_iter().next() else {
                    shacl_kept.push(format!("{} on {}: a property path that is not one property", shape_label, ename));
                    continue;
                };
                // the field (or link) this path is, made if the graph had no property for it
                let existing_field = e.json["properties"].as_object().and_then(|m| m.iter().find(|(n, f)| f["iri"] == json!(path) || (f["iri"].is_null() && iri::derive(&ns, &Key::Field(ename.clone(), (*n).clone()), &[]) == path)).map(|(n, _)| n.clone()));
                let existing_link = e.json["relationships"].as_object().and_then(|m| m.iter().find(|(n, l)| l["iri"] == json!(path) || (l["iri"].is_null() && iri::derive(&ns, &Key::Link(ename.clone(), (*n).clone()), &[]) == path)).map(|(n, _)| n.clone()));
                let sh_class = g.iris(&ps, &v(SH, "class")).into_iter().next();
                let fname = match (&existing_field, &existing_link) {
                    (Some(f), _) => f.clone(),
                    (None, Some(l)) => {
                        if g.all(&ps, &v(SH, "minCount")).iter().chain(g.all(&ps, &v(SH, "in")).iter()).count() > 0 {
                            shacl_kept.push(format!("{} on {}.{}: constraints on a link", shape_label, ename, l));
                        }
                        continue;
                    }
                    (None, None) => {
                        let base = g.text(&ps, &v(SH, "name")).map(|n| snake_name(&n)).unwrap_or_else(|| snake_name(&local_name(&path)));
                        if let Some(to) = sh_class.as_ref().filter(|t| class_name.contains_key(*t)) {
                            let mut name = base.clone();
                            while e.json["relationships"].get(&name).is_some() || e.json["properties"].get(&name).is_some() {
                                name.push('_');
                            }
                            e.json["relationships"][&name] = json!({ "to": name_of_class(to), "iri": path, "cardinality": if g.int(&ps, &v(SH, "maxCount")) == Some(1) { "many-to-one" } else { "" }, "description": g.text(&ps, &v(SH, "description")).unwrap_or_default(), "properties": {} });
                            continue;
                        }
                        let values: Vec<Json> = g.all(&ps, &v(SH, "in")).first().map(|h| g.list(h)).unwrap_or_default().iter().filter_map(|t| if let Term::Literal(l) = t { Some(yaml_to_json(&yaml_of(l))) } else { None }).collect();
                        let kind = g.iris(&ps, &v(SH, "datatype")).first().and_then(|d| kind_of_datatype(d, !values.is_empty())).unwrap_or(if values.is_empty() { "string" } else { "enum" });
                        let mut name = base.clone();
                        while e.json["properties"].get(&name).is_some() || e.json["relationships"].get(&name).is_some() {
                            name.push('_');
                        }
                        e.json["properties"][&name] = json!({ "type": kind, "key": false, "required": false, "description": g.text(&ps, &v(SH, "description")).unwrap_or_default(), "values": values, "iri": path });
                        name
                    }
                };
                // constraints → CEL
                let mut atoms: Vec<String> = Vec::new();
                let mut says: Vec<String> = Vec::new();
                let field_ref = format!("self.{}", fname);
                if g.int(&ps, &v(SH, "minCount")).unwrap_or(0) >= 1 {
                    atoms.push(format!("has({})", field_ref));
                    says.push("must be set".into());
                }
                if let Some(h) = g.all(&ps, &v(SH, "in")).first() {
                    let items: Vec<String> = g.list(h).iter().filter_map(|t| if let Term::Literal(l) = t { Some(cel_of(l)) } else { None }).collect();
                    let shown: Vec<String> = g.list(h).iter().filter_map(|t| if let Term::Literal(l) = t { Some(l.value().to_string()) } else { None }).collect();
                    atoms.push(format!("{} in [{}]", fname, items.join(", ")));
                    says.push(format!("is one of {}", shown.join(", ")));
                }
                for (pred, op, word) in [("minInclusive", ">=", "at least"), ("maxInclusive", "<=", "at most"), ("minExclusive", ">", "above"), ("maxExclusive", "<", "below")] {
                    if let Some(l) = g.literals(&ps, &v(SH, pred)).first() {
                        atoms.push(format!("{} {} {}", fname, op, cel_of(l)));
                        says.push(format!("is {} {}", word, l.value()));
                    }
                }
                for (pred, op, word) in [("minLength", ">=", "at least"), ("maxLength", "<=", "at most")] {
                    if let Some(n) = g.int(&ps, &v(SH, pred)) {
                        atoms.push(format!("size({}) {} {}", fname, op, n));
                        says.push(format!("has {} {} characters", word, n));
                    }
                }
                if let Some(pat) = g.text(&ps, &v(SH, "pattern")) {
                    atoms.push(format!("{}.matches({})", fname, serde_json::to_string(&pat).unwrap_or_default()));
                    says.push(format!("matches {}", pat));
                }
                if let Some(l) = g.literals(&ps, &v(SH, "hasValue")).first() {
                    atoms.push(format!("{} == {}", fname, cel_of(l)));
                    says.push(format!("is {}", l.value()));
                }
                for unsupported in ["class", "node", "or", "and", "not", "xone", "qualifiedValueShape", "equals", "disjoint", "lessThan", "lessThanOrEquals", "sparql", "languageIn", "uniqueLang"] {
                    if !g.all(&ps, &v(SH, unsupported)).is_empty() {
                        shacl_kept.push(format!("{} on {}.{}: sh:{}", shape_label, ename, fname, unsupported));
                    }
                }
                if atoms.is_empty() {
                    continue;
                }
                let base_id = format!("{}-{}", if shape_label.is_empty() { ename.clone() } else { shape_label.clone() }, fname).chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect::<String>();
                let mut id = base_id.clone();
                let mut n = 2;
                while !seen_ids.insert(id.clone()) {
                    id = format!("{}-{}", base_id, n);
                    n += 1;
                }
                let sev = g.iris(&ps, &v(SH, "severity")).first().cloned().or(shape_sev.clone()).unwrap_or_default();
                let name = g.text(&ps, &v(SH, "message")).or(shape_msg.clone()).unwrap_or_else(|| format!("{} {} {}", ename, fname.replace('_', " "), says.join(" and ")));
                let mut j = json!({ "id": id, "iri": null, "name": name, "description": g.text(&ps, &v(SH, "description")).or(shape_comment.clone()).unwrap_or_default(), "applies_to": ename, "rule": atoms.join(" && ") });
                j["severity"] = json!(severity_of(&sev).to_string());
                j["rationale"] = json!("");
                j["owner"] = json!("");
                j["tags"] = json!(["from-shacl"]);
                if let Id::N(si) = s {
                    if one && targets.len() == 1 {
                        j["iri"] = json!(si);
                    }
                }
                rules.push(Rule { json: j, file: None, order: rule_order });
                rule_order += 1;
                shacl_rules += 1;
            }
        }
    }
    if shacl_rules > 0 {
        notes.guessed.push(format!("{} SHACL property constraints became guardrails (tagged from-shacl); each needs a rationale and an owner", shacl_rules));
    }
    if !shacl_kept.is_empty() {
        notes.kept.push(format!("{} SHACL constraints have no CEL form, so they are listed here for a person: {}", shacl_kept.len(), shacl_kept.join("; ")));
    }

    // ---- glossary
    struct TermOut {
        json: Json,
        order: i64,
    }
    let mut terms: Vec<TermOut> = Vec::new();
    let mut have_terms: HashSet<(String, String)> = HashSet::new();
    let mut lone_concepts = 0;
    for s in g.of_type(&v(SKOS, "Concept")) {
        if g.is_a(&s, &v(OWL, "Class")) || g.is_a(&s, &v(RDFS, "Class")) {
            continue;
        }
        let means = g.text(&s, &v(LF, "meansName")).or_else(|| {
            let t = g.iris(&s, &v(LF, "means")).into_iter().next()?;
            class_name.get(&t).cloned().or_else(|| link_iris.get(&t).and_then(|l| l.first()).map(|(_, n)| n.clone()))
        });
        let Some(means) = means else {
            lone_concepts += 1;
            continue;
        };
        if !opts.only.is_empty() && !entities.contains_key(&means) {
            continue;
        }
        let word = g.text(&s, &v(SKOS, "prefLabel")).unwrap_or_else(|| match &s {
            Id::N(i) => local_name(i),
            Id::B(_) => String::new(),
        });
        if word.is_empty() {
            continue;
        }
        let mut j = json!({ "term": word, "iri": null, "means": means, "description": g.text(&s, &v(SKOS, "definition")).unwrap_or_default() });
        if let Id::N(i) = &s {
            if *i != iri::derive(&ns, &Key::Term(0), std::slice::from_ref(&word)) {
                j["iri"] = json!(i);
            }
        }
        have_terms.insert((word.to_lowercase(), means.to_lowercase()));
        terms.push(TermOut { json: j, order: g.int(&s, &v(LF, "order")).unwrap_or(1_000_000) });
    }
    if lone_concepts > 0 {
        notes.dropped.push(format!("{} SKOS concepts mean no entity, link or action here, so they are not glossary words", lone_concepts));
    }
    let mut alt_order = 3_000_000i64;
    for (c, ename) in &class_name {
        if !entities.contains_key(ename) {
            continue;
        }
        let id = Id::N(c.clone());
        for w in g.texts(&id, &v(SKOS, "altLabel")) {
            if have_terms.insert((w.to_lowercase(), ename.to_lowercase())) {
                terms.push(TermOut { json: json!({ "term": w, "iri": null, "means": ename, "description": "" }), order: alt_order });
                alt_order += 1;
            }
        }
    }
    for (piri, uses) in &link_iris {
        for w in g.texts(&Id::N(piri.clone()), &v(SKOS, "altLabel")) {
            if let Some((_, lname)) = uses.first() {
                if have_terms.insert((w.to_lowercase(), lname.to_lowercase())) {
                    terms.push(TermOut { json: json!({ "term": w, "iri": null, "means": lname, "description": "" }), order: alt_order });
                    alt_order += 1;
                }
            }
        }
    }

    // ---- actions, metrics, functions, sources (lorefile: vocabulary)
    let mut vocab: Vec<(FileKind, Json, Option<String>, i64)> = Vec::new();
    if opts.only.is_empty() {
        for (t, kind, name_key) in [("Action", FileKind::Actions, "action"), ("Metric", FileKind::Metrics, "metric"), ("Function", FileKind::Functions, "function"), ("Source", FileKind::Sources, "connection")] {
            for s in g.of_type(&v(LF, t)) {
                let Some(mut def) = g.json(&s, &v(LF, "definition")).filter(|d| d.is_object()) else {
                    notes.dropped.push(format!("a lorefile:{} without a usable definition", t));
                    continue;
                };
                if let Some(c) = g.text(&s, &rdfs_comment) {
                    if def.get("description").map(|d| d.as_str() != Some(c.as_str())).unwrap_or(true) {
                        def["description"] = json!(c);
                    }
                }
                let name = def[name_key].as_str().unwrap_or("").to_string();
                if let (Id::N(i), FileKind::Actions) = (&s, kind) {
                    if *i != iri::derive(&ns, &Key::Action(name.clone()), &[]) {
                        // keep `action` first, then `iri`
                        let mut m = Map::new();
                        for (k, val) in def.as_object().cloned().unwrap_or_default() {
                            m.insert(k.clone(), val);
                            if k == "action" {
                                m.insert("iri".into(), json!(i));
                            }
                        }
                        def = Json::Object(m);
                    }
                }
                vocab.push((kind, def, g.text(&s, &v(LF, "file")), g.int(&s, &v(LF, "order")).unwrap_or(1_000_000)));
            }
        }
    } else {
        let n: usize = ["Action", "Metric", "Function", "Source"].iter().map(|t| g.of_type(&v(LF, t)).len()).sum();
        if n > 0 {
            notes.dropped.push(format!("--only imports entities: {} actions, metrics, functions and sources left out", n));
        }
    }

    // ---- grades: only Lorefile's own records are claims; another tool's PROV is provenance, not a grade
    let mut claimed = TrustBook::default();
    let mut unread = 0;
    for s in g.of_type(&v(LF, "Grading")) {
        let Some(d) = g.json(&s, &v(LF, "definition")) else {
            unread += 1;
            continue;
        };
        let kind = crate::trust::Kind::ALL.into_iter().find(|k| k.key() == d["kind"].as_str().unwrap_or(""));
        let rec: Option<TrustRecord> = serde_json::from_value(d["record"].clone()).ok();
        match (kind, rec, d["name"].as_str()) {
            (Some(k), Some(rec), Some(name)) => {
                crate::trust::entries_mut(&mut claimed, k).insert(name.to_string(), rec);
            }
            _ => unread += 1,
        }
    }
    if unread > 0 {
        notes.dropped.push(format!("{} grade claim{} in the file could not be read and {} ignored", unread, if unread == 1 { "" } else { "s" }, if unread == 1 { "was" } else { "were" }));
    }
    let foreign = g.of_type(&v(PROV, "Activity")).into_iter().filter(|s| !g.is_a(s, &v(LF, "Grading"))).count();
    if foreign > 0 {
        notes.dropped.push(format!("{} PROV activities from another tool: provenance, never read as grades", foreign));
    }

    // ---- what was read only for reasoning
    let reasoning = ["equivalentClass", "disjointWith", "inverseOf", "equivalentProperty", "propertyChainAxiom", "someValuesFrom", "allValuesFrom", "onProperty", "minCardinality", "maxCardinality", "cardinality", "qualifiedCardinality", "intersectionOf", "complementOf", "TransitiveProperty", "SymmetricProperty", "sameAs"];
    let reasoning_iris: HashSet<String> = reasoning.iter().map(|l| v(OWL, l)).chain([v(RDFS, "subPropertyOf")]).collect();
    let n_reasoning = triples
        .iter()
        .filter(|t| reasoning_iris.contains(t.predicate.as_str()) || matches!(&t.object, Term::NamedNode(n) if t.predicate.as_str() == v(RDF, "type") && reasoning_iris.contains(n.as_str())))
        .count();
    if n_reasoning > 0 {
        notes.dropped.push(format!("{} OWL statements used only for reasoning (equivalent and disjoint classes, restrictions, inverses, sub-properties)", n_reasoning));
    }

    // ---- to do
    let foreign: Vec<&Ent> = entities.values().filter(|e| !e.ours).collect();
    if !foreign.is_empty() {
        let no_desc = foreign.iter().filter(|e| e.json["description"].as_str().unwrap_or("").is_empty()).count();
        let no_links_sentence = foreign.iter().map(|e| e.json["relationships"].as_object().map(|m| m.values().filter(|l| l["description"].as_str().unwrap_or("").is_empty()).count()).unwrap_or(0)).sum::<usize>();
        notes.todo.push(format!(
            "{} entities have no owner{}{}",
            foreign.len(),
            if no_desc > 0 { format!(", {} no description", no_desc) } else { String::new() },
            if no_links_sentence > 0 { format!(", {} links no sentence", no_links_sentence) } else { String::new() }
        ));
    }

    // ---- IRIs written compactly, with the prefixes they need
    let mut used_extra: BTreeSet<String> = BTreeSet::new();
    let mut compact = |full: &str| -> String {
        let mut all = prefixes.clone();
        for (p, b) in &extra_prefixes {
            all.insert(p.clone(), b.clone());
        }
        let c = iri::compact_with(&all, &ns, full);
        if c != full {
            let p = c.split(':').next().unwrap_or("").to_string();
            if extra_prefixes.contains_key(&p) {
                used_extra.insert(p);
            }
        }
        c
    };
    let mut fix = |j: &mut Json| {
        if let Some(s) = j.get("iri").and_then(|x| x.as_str()).map(String::from) {
            j["iri"] = json!(compact(&s));
        }
    };
    for e in entities.values_mut() {
        fix(&mut e.json);
        if let Some(m) = e.json["properties"].as_object_mut() {
            for f in m.values_mut() {
                fix(f);
            }
        }
        if let Some(m) = e.json["relationships"].as_object_mut() {
            for l in m.values_mut() {
                fix(l);
            }
        }
    }
    for s in standalone.iter_mut() {
        fix(&mut s.json);
    }
    for r in rules.iter_mut() {
        fix(&mut r.json);
    }
    for t in terms.iter_mut() {
        fix(&mut t.json);
    }
    for (_, def, _, _) in vocab.iter_mut() {
        fix(def);
    }
    for p in &used_extra {
        manifest_prefixes.insert(p.clone(), extra_prefixes[p].clone());
    }

    // ---- files
    let mut layout = Layout::default();
    let mut counts: IndexMap<&'static str, usize> = IndexMap::new();
    if target.existing.is_none() {
        let mut m = json!({ "name": space_name });
        if let Some(o) = &onto {
            m["version"] = json!(g.text(o, &v(OWL, "versionInfo")).unwrap_or_default());
            m["description"] = json!(g.text(o, &rdfs_comment).or_else(|| g.text(o, &v(DCTERMS, "description"))).map(|d| if ours_graph { d } else { clean_text(&d) }).unwrap_or_default());
        }
        let mut domains: Vec<(i64, Json)> = Vec::new();
        if ours_graph {
            for d in g.of_type(&v(SKOS, "ConceptScheme")) {
                let Some(name) = g.text(&d, &rdfs_label) else { continue };
                let types: Vec<String> = g.all(&d, &v(LF, "types")).first().map(|h| g.list(h)).unwrap_or_default().iter().filter_map(|t| if let Term::NamedNode(n) = t { Some(name_of_class(n.as_str())) } else { None }).collect();
                domains.push((g.int(&d, &v(LF, "order")).unwrap_or(0), json!({ "name": name, "description": g.text(&d, &rdfs_comment).unwrap_or_default(), "types": types })));
            }
        }
        domains.sort_by_key(|d| d.0);
        m["domains"] = json!(domains.into_iter().map(|d| d.1).collect::<Vec<_>>());
        if let Some(n) = &declare_ns {
            m["namespace"] = json!(n);
        }
        m["prefixes"] = json!(manifest_prefixes);
        // kinds are format 2: an older Lorefile refuses the space plainly instead of misreading it
        if entities.values().any(|e| e.json.get("extends").is_some()) {
            m["format"] = json!(2);
        }
        layout.add(FileKind::Manifest, None, "ontology.yaml", m);
    } else if !used_extra.is_empty() {
        notes.todo.push(format!("add these prefixes to ontology.yaml to shorten the IRIs: {}", used_extra.iter().map(|p| format!("{}: {}", p, extra_prefixes[p])).collect::<Vec<_>>().join(", ")));
    }

    let mut ents: Vec<(String, Ent)> = entities.into_iter().collect();
    ents.sort_by_key(|(_, e)| e.order);
    let mut n_fields = 0;
    let mut n_links = standalone.len();
    for (name, e) in ents {
        n_fields += e.json["properties"].as_object().map(|m| m.len()).unwrap_or(0);
        n_links += e.json["relationships"].as_object().map(|m| m.len()).unwrap_or(0);
        let default = format!("types/{}.yaml", snake_name(&name));
        layout.add(FileKind::Types, e.file.as_deref(), &default, e.json);
    }
    counts.insert("entities", layout.files.values().filter(|(k, _)| *k == FileKind::Types).map(|(_, o)| o.len()).sum());
    counts.insert("fields", n_fields);
    counts.insert("links", n_links);
    standalone.sort_by_key(|s| s.order);
    for s in standalone {
        layout.add(FileKind::Relationships, Some(&s.file), "relationships/links.yaml", s.json);
    }
    rules.sort_by_key(|r| r.order);
    counts.insert("guardrails", rules.len());
    for r in rules {
        let default = format!("constraints/{}.yaml", snake_name(r.json["applies_to"].as_str().unwrap_or("imported")));
        layout.add(FileKind::Constraints, r.file.as_deref(), &default, r.json);
    }
    terms.sort_by_key(|t| t.order);
    counts.insert("glossary_terms", terms.len());
    for t in terms {
        layout.add(FileKind::Glossary, None, "glossary.yaml", t.json);
    }
    vocab.sort_by_key(|x| (x.0 as u8, x.3));
    for (kind, def, file, _) in vocab {
        let (default, key) = match kind {
            FileKind::Actions => (format!("actions/{}.yaml", def["action"].as_str().unwrap_or("action")), "actions"),
            FileKind::Sources => (format!("sources/{}.yaml", def["connection"].as_str().unwrap_or("source")), "sources"),
            FileKind::Metrics => ("logic/metrics.yaml".to_string(), "metrics"),
            _ => ("logic/functions.yaml".to_string(), "functions"),
        };
        *counts.entry(key).or_insert(0) += 1;
        layout.add(kind, file.as_deref(), &default, def);
    }
    let _ = prop_count;

    let mut out_notes = Vec::new();
    for (label, list) in [("guessed", &notes.guessed), ("dropped", &notes.dropped), ("kept", &notes.kept), ("to do", &notes.todo)] {
        for n in list {
            out_notes.push(format!("{}: {}", label, n));
        }
    }
    let header = format!(
        "# Imported from {} by `lorefile import --from-rdf`.\n",
        path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
    );
    let files = layout.files.into_iter().map(|(p, (k, objs))| (p, render(k, objs, &header))).collect();
    let trust = if crate::trust::is_empty(&claimed) { None } else { Some(claimed) };
    Ok(Imported { files, trust, notes: out_notes, counts })
}

/// The line of `text` an error is most likely about.
fn line_of(text: &str, message: &str) -> Option<usize> {
    let quoted: Vec<&str> = message.split('`').skip(1).step_by(2).collect();
    for q in quoted.iter().rev() {
        if q.is_empty() {
            continue;
        }
        if let Some(i) = text.lines().position(|l| l.contains(q)) {
            return Some(i + 1);
        }
    }
    None
}

/// What `lorefile import --from-rdf` did.
pub struct Written {
    pub dir: PathBuf,
    pub files: Vec<String>,
    pub imported: Imported,
    pub warnings: usize,
}

/// Builds the files, compiles the space with them laid over it, and writes
/// them only when that compiles with no new errors. Never overwrites a file.
pub fn import_into(file: &Path, into: &Path, opts: &ImportOptions) -> Result<Written> {
    let exists = into.exists();
    if exists && !into.is_dir() {
        bail!("{} is a file; --into takes a folder", into.display());
    }
    let has_manifest = exists && (into.join("ontology.yaml").exists() || into.join("ontology.yml").exists());
    if exists && !has_manifest {
        let holds_spaces = std::fs::read_dir(into)?.filter_map(|e| e.ok()).any(|e| e.path().is_dir() && e.path().join("ontology.yaml").exists());
        if holds_spaces {
            bail!("{} is a folder of spaces; import into a new folder inside it, e.g. --into {}", into.display(), into.join("imported").display());
        }
    }
    let existing = if has_manifest { Some(Ontology::from_raw(loader::load(into)?)) } else { None };
    let folder_name = into.file_name().map(|f| f.to_string_lossy().to_string()).filter(|f| !f.is_empty() && f != ".").unwrap_or_else(|| "imported".into());
    let mut imported = import(file, &Target { existing, folder_name }, opts)?;

    let mut files: Vec<(String, String)> = Vec::new();
    for (rel, text) in std::mem::take(&mut imported.files) {
        let mut path = rel.clone();
        let mut n = 1;
        while into.join(&path).exists() || files.iter().any(|(p, _)| p == &path) {
            let stem = rel.trim_end_matches(".yaml").trim_end_matches(".yml");
            path = if n == 1 { format!("{}-rdf.yaml", stem) } else { format!("{}-rdf-{}.yaml", stem, n) };
            n += 1;
        }
        files.push((path, text));
    }
    if has_manifest {
        imported.notes.push(format!("kept: the existing ontology.yaml in {}", into.display()));
    }

    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let scratch = std::env::temp_dir().join(format!("lorefile-rdf-import-{}-{}", std::process::id(), nanos));
    let root = if exists {
        into.to_path_buf()
    } else {
        std::fs::create_dir_all(&scratch)?;
        scratch.clone()
    };
    let compile = |ov: &Overrides| -> Result<(Ontology, validate::Report)> {
        let ont = Ontology::from_raw(loader::load_with(&root, ov)?);
        let rep = validate::validate(&ont);
        Ok((ont, rep))
    };
    let before: HashSet<(String, String)> = if exists { compile(&Overrides::new())?.1.errors().iter().map(|d| (d.code.to_string(), d.message.clone())).collect() } else { HashSet::new() };
    let ov: Overrides = files.iter().map(|(p, t)| (p.clone(), Some(t.clone()))).collect();
    let result = compile(&ov);
    if !exists {
        let _ = std::fs::remove_dir_all(&scratch);
    }
    let (ont, rep) = result?;
    let new_errors: Vec<_> = rep.errors().into_iter().filter(|d| !before.contains(&(d.code.to_string(), d.message.clone()))).collect();
    if !new_errors.is_empty() {
        let lines: Vec<String> = new_errors
            .iter()
            .map(|d| {
                let file = if d.file.is_empty() { d.message.split(':').next().unwrap_or("").to_string() } else { d.file.clone() };
                match files.iter().find(|(p, _)| p == &file).and_then(|(_, t)| line_of(t, &d.message)) {
                    Some(n) => format!("error {} {}:{}: {}", d.code, file, n, d.message),
                    None if !d.file.is_empty() => format!("error {} {}: {}", d.code, d.file, d.message),
                    None => format!("error {}: {}", d.code, d.message),
                }
            })
            .collect();
        bail!("the imported space does not compile, so nothing was written:\n{}", lines.join("\n"));
    }
    if let Some(t) = crate::trust::after_import(&ont, imported.trust.as_ref(), opts.keep_grades, into, &mut imported.notes) {
        files.push(t);
    }
    for (rel, text) in &files {
        let p = into.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&p, text).with_context(|| format!("writing {}", p.display()))?;
    }
    Ok(Written { dir: into.to_path_buf(), files: files.into_iter().map(|(p, _)| p).collect(), imported, warnings: rep.warnings().len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> TypeDef {
        let mut t = TypeDef { name: "Order".into(), ..Default::default() };
        for f in ["status", "total_usd", "order_id"] {
            t.properties.insert(f.into(), PropertySpec::Short("string".into()));
        }
        t
    }

    #[test]
    fn simple_rules_get_a_shacl_form() {
        let t = order();
        let f = shacl_form(&t, r#"status in ["placed", "paid", "packing"]"#).unwrap();
        assert!(matches!(&f["status"][0], (p, Obj::List(items)) if p.ends_with("#in") && items.len() == 3));
        let f = shacl_form(&t, "total_usd >= 0.0 && self.total_usd < 5000").unwrap();
        assert_eq!(f["total_usd"].len(), 2);
        assert!(shacl_form(&t, "total_usd > 2000.0 implies status == 'paid'").is_none());
        assert!(shacl_form(&t, "size(status) > 2").is_none());
        assert!(shacl_form(&t, "unknown_field == 1").is_none());
        assert!(shacl_form(&t, "has(self.order_id)").is_some());
    }

    #[test]
    fn turtle_strings_need_few_escapes() {
        assert_eq!(turtle_string("plain"), "\"plain\"");
        assert_eq!(turtle_string(r#"{"a":1}"#), r#"'{"a":1}'"#);
        assert_eq!(turtle_string("it's \"x\""), r#""it's \"x\"""#);
        assert_eq!(turtle_string("two\nlines"), "\"\"\"two\nlines\"\"\"");
    }

    #[test]
    fn names_from_other_vocabularies() {
        assert_eq!(snake_name("orderDate"), "order_date");
        assert_eq!(snake_name("order date"), "order_date");
        assert_eq!(pascal("purchase order"), "PurchaseOrder");
        assert_eq!(clean_text("A [[Person]] or <a href=\"x\">an org</a>.\\n More."), "A Person or an org. More.");
        assert_eq!(local_name("https://schema.org/Order"), "Order");
        assert_eq!(local_name("https://lorefile.dev/ns/x#term.first_order"), "term.first_order");
    }
}
