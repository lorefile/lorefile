//! The YAML file format. Every struct here maps 1:1 to something a human or an
//! agent writes in the `ontology/` folder. Keep it boring and well-described:
//! the `description` fields are what make the ontology readable to an LLM.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Value as Yaml;

/// `ontology.yaml` — the manifest at the root of the folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Optional grouping of types for the UI and the summary.
    #[serde(default)]
    pub domains: Vec<Domain>,
    /// The base every derived IRI starts with: `https://acme.example/ontology#`.
    /// Ends in `#` or `/`. Default `https://lorefile.dev/ns/<space>#`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Short names for IRIs, so `iri: schema:Order` can be written:
    /// `{schema: https://schema.org/}`. Common ones (schema, owl, skos …) are built in.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub prefixes: IndexMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Domain {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub types: Vec<String>,
}

/// A property can be written long-form (`{type: string, key: true}`) or as a
/// bare type name shorthand (`name: string`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropertySpec {
    Short(String),
    Full(Property),
}

impl PropertySpec {
    pub fn normalize(&self) -> Property {
        match self {
            PropertySpec::Short(kind) => Property {
                kind: kind.clone(),
                ..Property::default()
            },
            PropertySpec::Full(p) => p.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Property {
    /// string | integer | number | boolean | date | datetime | enum | json
    #[serde(rename = "type", default = "default_string")]
    pub kind: String,
    #[serde(default)]
    pub key: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: String,
    /// Allowed values when `type: enum`.
    #[serde(default)]
    pub values: Vec<Yaml>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// A standard identity for this field (absolute, or `prefix:local`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
}

fn default_string() -> String {
    "string".to_string()
}

/// A relationship declared inline inside a type: `supplies: {to: Part, …}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipInline {
    pub to: String,
    #[serde(default)]
    pub cardinality: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub properties: IndexMap<String, PropertySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EdgeSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
}

/// A relationship declared in its own file under `relationships/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipStandalone {
    #[serde(rename = "relationship")]
    pub name: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub cardinality: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub properties: IndexMap<String, PropertySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EdgeSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
    /// Set by the loader: which file this came from.
    #[serde(skip_deserializing)]
    pub file: String,
}

/// Where the rows for a type live. Phase 1 reads `csv` connections; every
/// other kind is declared so agents can find the data with their own tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeSource {
    pub connection: String,
    /// Table, view, or file stem for data kinds. Not used by `kind: mcp` sources.
    #[serde(default)]
    pub table: String,
    #[serde(default)]
    pub key: String,
    /// property name → column name. Missing entries default to the same name.
    #[serde(default)]
    pub columns: IndexMap<String, String>,
    /// For `kind: mcp` sources: the tool to call on that server, and the parameters to pass it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub params: IndexMap<String, Yaml>,
}

/// Where the rows for a relationship (an edge table) live.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeSource {
    pub connection: String,
    pub table: String,
    pub from_key: String,
    pub to_key: String,
    #[serde(default)]
    pub columns: IndexMap<String, String>,
}

/// `types/*.yaml`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeDef {
    #[serde(rename = "type")]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default)]
    pub properties: IndexMap<String, PropertySpec>,
    #[serde(default)]
    pub relationships: IndexMap<String, RelationshipInline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TypeSource>,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Small inline reference data (regions, tiers, a handful of named things).
    #[serde(default)]
    pub instances: Vec<IndexMap<String, Yaml>>,
    /// A standard identity (`schema:Order`, `https://…`); derived from the
    /// namespace and the name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
    /// Set by the loader: which file this came from.
    #[serde(skip_deserializing)]
    pub file: String,
}

impl TypeDef {
    pub fn key_property(&self) -> Option<String> {
        self.properties
            .iter()
            .find(|(_, p)| p.normalize().key)
            .map(|(n, _)| n.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    #[default]
    Warn,
    Block,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "info"),
            Severity::Warn => write!(f, "warn"),
            Severity::Block => write!(f, "block"),
        }
    }
}

/// `constraints/**/*.yaml` — each file holds a `constraints:` list.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// A type name, a relationship name, or an action name.
    pub applies_to: String,
    /// CEL expression. `A implies B` is accepted as sugar for `!(A) || (B)`.
    pub rule: String,
    /// Optional SQL predicate for data-level checks (Phase 2, DuckDB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(default)]
    pub severity: Severity,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
    #[serde(skip_deserializing)]
    pub file: String,
}

/// Which edge an action creates or removes, in terms of its inputs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeRef {
    pub relationship: String,
    /// Name of the action input that is the `from` side.
    pub from: String,
    /// Name of the action input that is the `to` side.
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<EdgeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<EdgeRef>,
}

/// `actions/*.yaml` — the things an agent is allowed to do.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDef {
    #[serde(rename = "action")]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// input name → type name
    #[serde(default)]
    pub inputs: IndexMap<String, String>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// Constraint IDs to evaluate. Constraints with `applies_to: <action>` are added automatically.
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub requires_approval_from: String,
    #[serde(default)]
    pub side_effects: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
    #[serde(skip_deserializing)]
    pub file: String,
}

/// `sources/*.yaml`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceDef {
    pub connection: String,
    /// csv | parquet | postgres | mysql | snowflake | bigquery | duckdb | s3 | mcp | …
    pub kind: String,
    #[serde(default)]
    pub description: String,
    /// For `kind: mcp`: the MCP server name as the agent has it configured (e.g. `meta-ads`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// For file kinds: folder (relative to the ontology root) holding the tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Everything else (account, database, host…). Secrets stay in env vars.
    #[serde(flatten)]
    pub config: IndexMap<String, Yaml>,
    #[serde(skip_deserializing)]
    pub file: String,
}

/// `glossary.yaml`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glossary {
    #[serde(default)]
    pub terms: Vec<GlossaryTerm>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlossaryTerm {
    pub term: String,
    /// The canonical type, relationship, or action this word refers to.
    pub means: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iri: Option<String>,
}

// ---- list wrappers so a file can hold several objects ----

/// A named number: what it means, where it comes from, who owns it. Lives in
/// `logic/*.yaml` under `metrics:`. `success` is the conventional name for the
/// one number a business is run on, so packs and prompts can ask for it anywhere.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDef {
    #[serde(rename = "metric")]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Where the number comes from when it has one definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MetricSource>,
    /// Several definitions chosen by a condition: one per product line, market, channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<MetricVariant>,
    /// day | week | month | campaign …: the level the number is meaningful at.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub grain: String,
    /// A full SQL definition when a source recipe is not enough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caveats: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// What kind of number this is: counted, attributed, modelled or read from a test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<Basis>,
    #[serde(skip_deserializing)]
    pub file: String,
}

/// What kind of number a metric is. The grade in `trust.yaml` says who stands
/// behind the definition; `basis` says what the number is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Basis {
    /// Counted from the business's own records: what happened.
    Transactional,
    /// Credit a platform or an attribution rule assigns to a channel.
    Attributed,
    /// An estimate from a statistical model.
    Modelled,
    /// Read from a controlled test, for that test's cells and dates only.
    Experimental,
}

impl Basis {
    pub fn word(&self) -> &'static str {
        match self {
            Basis::Transactional => "transactional",
            Basis::Attributed => "attributed",
            Basis::Modelled => "modelled",
            Basis::Experimental => "experimental",
        }
    }
}

/// A recipe for fetching a metric: a connection plus a table or an MCP tool,
/// an optional filter, and what to measure in the rows that come back.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSource {
    pub connection: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub table: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub params: IndexMap<String, Yaml>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filter: String,
    /// `count distinct order_id`, `sum(revenue)` …
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub measure: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricVariant {
    /// A condition in plain words or CEL: `product_line == "saas"`.
    pub when: String,
    #[serde(default)]
    pub description: String,
    pub source: MetricSource,
    /// When variants differ (store orders are transactional, Meta's count is attributed); wins over the metric's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<Basis>,
}

/// An operation agents can run: a contract (typed inputs and outputs) and a
/// pointer to where it runs. The ontology never runs it. The agent reads the
/// contract, gathers the inputs, and calls the tool or endpoint named here,
/// which authenticates the call itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionDef {
    #[serde(rename = "function")]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// input name → an entity name, or string | integer | number | boolean | date | datetime | json.
    #[serde(default)]
    pub inputs: IndexMap<String, String>,
    #[serde(default)]
    pub outputs: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<Runs>,
    /// Where the result lands, if the agent is expected to store it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writes_back: Option<WritesBack>,
    /// Agent roles that may call it (matched against the agent's declared name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub may_be_called_by: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub caveats: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_deserializing)]
    pub file: String,
}

/// Where a function runs. `mcp`: a tool on an MCP server declared under
/// sources/ with `kind: mcp`. `http`: an endpoint your engineers own.
/// `manual`: a person does it; the ontology just records the contract.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runs {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    /// Arguments to pass; `{{input}}` takes the value of that input.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub args: IndexMap<String, Yaml>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub method: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WritesBack {
    pub entity: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub connection: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub table: String,
}

// ---- trust.yaml: who stands behind what ----

/// How well a definition is backed, lowest first. No entry at all is "ungraded".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    /// An agent wrote it and no person has looked.
    Proposed,
    /// A person stands behind the words.
    Accepted,
    /// A person ran it against real rows and it was right.
    Checked,
}

impl Grade {
    pub fn word(&self) -> &'static str {
        match self {
            Grade::Proposed => "proposed",
            Grade::Accepted => "accepted",
            Grade::Checked => "checked",
        }
    }
}

/// One line of `trust.yaml`: the grade, who gave it, when, and a fingerprint
/// of what was graded, so a change behind the grade's back stops it counting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustRecord {
    pub grade: Grade,
    #[serde(default)]
    pub by: String,
    /// The UTC date, `2026-11-28`.
    #[serde(default)]
    pub at: String,
    /// A short stable hash of the thing as compiled when it was graded.
    #[serde(default)]
    pub of: String,
    /// For `checked`: the source under `sources/` whose rows were used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub against: Option<String>,
    /// For `checked`: what was compared with what.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The four kinds that carry a grade in the first version, one mapping each, keyed by name.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustBook {
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub metrics: IndexMap<String, TrustRecord>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub bindings: IndexMap<String, TrustRecord>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub terms: IndexMap<String, TrustRecord>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub guardrails: IndexMap<String, TrustRecord>,
}

/// `trust.yaml` at the root of a space. Written by Lorefile only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustFile {
    #[serde(default)]
    pub trust: TrustBook,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionsFile {
    pub functions: Vec<FunctionDef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsFile {
    pub metrics: Vec<MetricDef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypesFile {
    pub types: Vec<TypeDef>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationshipsFile {
    pub relationships: Vec<RelationshipStandalone>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintsFile {
    pub constraints: Vec<Constraint>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionsFile {
    pub actions: Vec<ActionDef>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcesFile {
    pub sources: Vec<SourceDef>,
}
