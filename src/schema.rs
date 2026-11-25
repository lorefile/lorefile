//! The YAML file format. Every struct here maps 1:1 to something a human or an
//! agent writes in the `ontology/` folder. Keep it boring and well-described:
//! the `description` fields are what make the ontology readable to an LLM.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Value as Yaml;

/// `ontology.yaml` — the manifest at the root of the folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Optional grouping of types for the UI and the summary.
    #[serde(default)]
    pub domains: Vec<Domain>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

fn default_string() -> String {
    "string".to_string()
}

/// A relationship declared inline inside a type: `supplies: {to: Part, …}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

/// A relationship declared in its own file under `relationships/`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

/// Where the rows for a type live. Phase 1 reads `csv` connections; every
/// other kind is declared so agents can find the data with their own tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Set by the loader: which file this came from.
    #[serde(skip)]
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
    #[serde(skip)]
    pub file: String,
}

/// Which edge an action creates or removes, in terms of its inputs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EdgeRef {
    pub relationship: String,
    /// Name of the action input that is the `from` side.
    pub from: String,
    /// Name of the action input that is the `to` side.
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Effect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<EdgeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<EdgeRef>,
}

/// `actions/*.yaml` — the things an agent is allowed to do.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(skip)]
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
    #[serde(skip)]
    pub file: String,
}

/// `glossary.yaml`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Glossary {
    #[serde(default)]
    pub terms: Vec<GlossaryTerm>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlossaryTerm {
    pub term: String,
    /// The canonical type, relationship, or action this word refers to.
    pub means: String,
    #[serde(default)]
    pub description: String,
}

// ---- list wrappers so a file can hold several objects ----

#[derive(Debug, Default, Deserialize)]
pub struct TypesFile {
    pub types: Vec<TypeDef>,
}
#[derive(Debug, Default, Deserialize)]
pub struct RelationshipsFile {
    pub relationships: Vec<RelationshipStandalone>,
}
#[derive(Debug, Default, Deserialize)]
pub struct ConstraintsFile {
    pub constraints: Vec<Constraint>,
}
#[derive(Debug, Default, Deserialize)]
pub struct ActionsFile {
    pub actions: Vec<ActionDef>,
}
#[derive(Debug, Default, Deserialize)]
pub struct SourcesFile {
    pub sources: Vec<SourceDef>,
}
