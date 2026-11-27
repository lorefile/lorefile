//! MCP server over stdio, and the `Hub` that `serve` also answers `POST /mcp`
//! with (Streamable HTTP, plain JSON replies). Hand-rolled JSON-RPC so the
//! protocol surface is visible in one file and there is no SDK churn. Speaks
//! the 2025-06-18 protocol and is backwards compatible with 2024-11-05 clients.

use crate::data::DataLayer;
use crate::edit::{self, Change, EditConfig, Note, Outcome};
use crate::learn;
use crate::ontology::Ontology;
use crate::patch;
use crate::proposals::{self, Proposal, ProposedFile};
use crate::rules;
use crate::spaces::Registry;
use crate::validate;
use crate::validate::Report;
use anyhow::Result;
use indexmap::IndexMap;
use serde_json::{json, Value as Json};
use std::cell::Cell;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL: &str = "2025-06-18";

/// The recipe for a file from the user, in the text every agent gets at `initialize`.
const FILE_RECIPE: &str = "When the user gives you a file (a CSV, a spreadsheet, an export), call `match_file` with its name, its column names and up to five example values per column: it tells you whether Lorefile already knows this kind of file and how to read it (which number is the result, which rows do not count). If they ask you to save the file, they mean its shape and meaning, not its rows: ask the questions `match_file` returns, then call `learn_file` with the answers. Lorefile never stores rows or example values.";
const FILE_RECIPE_READ: &str = "When the user gives you a file (a CSV, a spreadsheet, an export), call `match_file` with its name, its column names and up to five example values per column: it tells you whether Lorefile already knows this kind of file and how to read it (which number is the result, which rows do not count). Lorefile never stores rows or example values.";

/// Who an agent's change is by over MCP: stdio, Claude Code's header. A connection that signed
/// in with OAuth is named instead: `agent via MCP (connection "ChatGPT", approved 27 Nov)`.
pub const VIA: &str = "agent via MCP";

/// The tools that change an ontology: left out for a connection that may only read, and refused on shipped examples.
const PROPOSE_TOOLS: [&str; 3] = ["propose_patch", "propose_change", "learn_file"];

/// The tool list a client sees. With several spaces every tool takes a `space`
/// argument naming the ontology it is about, and a `spaces` tool lists them.
pub fn tool_definitions_for(reg: &Registry, only: Option<&str>) -> Json {
    let mut tools = tool_definitions();
    if only.is_some() || reg.spaces.len() <= 1 {
        return tools;
    }
    let ids: Vec<String> = reg.spaces.keys().cloned().collect();
    let mine: Vec<&str> = reg.user_spaces().iter().map(|s| s.id.as_str()).collect();
    let hint = if mine.len() == 1 {
        format!("Which space (ontology) this is about. Defaults to {} when left out. Shipped examples start with @.", mine[0])
    } else {
        format!("Which space (ontology) this is about: one of {}. Shipped examples start with @. Call `spaces` to see what each one holds.", ids.join(", "))
    };
    if let Some(arr) = tools.as_array_mut() {
        for t in arr.iter_mut() {
            t["inputSchema"]["properties"]["space"] = json!({ "type": "string", "description": hint, "enum": ids });
        }
        arr.insert(0, json!({
            "name": "spaces",
            "description": "List the spaces this server holds: one ontology per brand or business unit, plus the shipped examples (ids starting with @). Call it when the user has not said which space they mean.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": annotations_for("spaces")
        }));
    }
    tools
}

/// The input of `match_file` (names, examples, counts) and `learn_file` (plus the meaning and the
/// answers). Complete, with `additionalProperties: false`, so a strict client keeps every field.
fn file_schema(learn: bool) -> Json {
    let mut column = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "description": "The column's name exactly as the file's header writes it" },
            "examples": { "type": "array", "maxItems": 5, "items": { "type": ["string", "number", "boolean", "null"] }, "description": "Up to five values from this column, used only to guess its type; never stored" },
            "distinct": { "type": "integer", "minimum": 0, "description": "How many different values the column has, if you can count them" },
            "empty": { "type": "integer", "minimum": 0, "description": "How many rows leave it empty, if you can count them" }
        }
    });
    let mut props = json!({
        "file_name": { "type": "string", "maxLength": 200, "description": "The file's name, e.g. promo_sales_2026-12.csv; a workbook sheet as `report.xlsx › Sales`. Used for naming, never for matching" },
        "row_count": { "type": "integer", "minimum": 0, "description": "How many rows the file has, if you know" },
        "columns": { "type": "array", "minItems": 1, "maxItems": 200, "description": "Every column, in the file's order", "items": column.clone() }
    });
    if learn {
        column["properties"]["description"] = json!({ "type": "string", "description": "One short sentence: what the column is and its unit. Never quote values from the file" });
        column["properties"]["type"] = json!({ "type": "string", "enum": ["string", "integer", "number", "boolean", "date", "datetime"], "description": "Only if you know better than the examples show; it is checked against them" });
        column["properties"]["unit"] = json!({ "type": "string", "description": "For money, the currency code (USD, EUR); for a quantity, its unit" });
        props["columns"]["items"] = column;
        props["entity"] = json!({ "type": "string", "description": "The entity's name, e.g. PromoSale. Left out, it comes from the file name" });
        props["description"] = json!({ "type": "string", "description": "What one row is, e.g. \"One order line sold with a promo code, from the monthly promo-sales export.\"" });
        props["key"] = json!({ "type": "string", "description": "The column that identifies one row, when you know it" });
        props["reason"] = json!({ "type": "string", "description": "One sentence for the person who reviews it, e.g. what the user said" });
        props["answers"] = json!({
            "type": "object",
            "additionalProperties": false,
            "description": "What the user said. Left out: nothing is saved and you get the questions. {} saves the shape only",
            "properties": {
                "success": { "type": "object", "additionalProperties": false, "required": ["column"], "description": "Which number is the result", "properties": {
                    "column": { "type": "string" },
                    "how": { "type": "string", "enum": ["sum", "count_distinct", "count"], "default": "sum" }
                } },
                "exclude": { "type": "array", "description": "Rows that do not count, e.g. [{\"column\": \"refunded\", \"equals\": true}]; several are joined with and", "items": { "type": "object", "additionalProperties": false, "required": ["column", "equals"], "properties": {
                    "column": { "type": "string" },
                    "equals": { "type": ["string", "number", "boolean"] }
                } } },
                "exclude_words": { "type": "string", "description": "What does not count, in the user's words, e.g. \"Refunded lines never count.\"" },
                "currency": { "type": "string", "description": "The currency of the money columns, e.g. USD" },
                "metric": { "type": "string", "description": "The number's name. Left out: <entity>_<column>, e.g. promo_sale_revenue. Call it success only when the user says this is the one number the business runs on" },
                "basis": { "type": "string", "enum": ["transactional", "attributed", "modelled", "experimental"] },
                "also_called": { "type": "array", "items": { "type": "string" }, "description": "Words the user actually uses for this file, e.g. \"promo report\"" },
                "renamed": { "type": "object", "additionalProperties": { "type": "string" }, "description": "For a changed file: a known field → its new column name, once the user confirms, e.g. {\"promo_code\": \"promo\"}" }
            }
        });
    }
    json!({ "type": "object", "additionalProperties": false, "required": ["columns"], "properties": props })
}

/// What a client may assume about a tool (MCP `annotations`). ChatGPT asks the user to
/// confirm every call to a tool that does not say it only reads, so every read tool says so.
/// The propose tools only add or describe and a person accepts; `propose_change` keeps the
/// default `destructiveHint`, since with edit mode on it replaces a whole file.
pub fn annotations_for(name: &str) -> Json {
    match name {
        "propose_patch" | "learn_file" => json!({ "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false }),
        n if READ_TOOLS.contains(&n) => json!({ "readOnlyHint": true, "openWorldHint": false }),
        // propose_change, and any tool added later that nobody has marked: not read-only
        _ => json!({ "readOnlyHint": false, "openWorldHint": false }),
    }
}

/// The tools that only read. A new tool is a write until it is added here.
const READ_TOOLS: [&str; 14] = ["spaces", "search", "describe", "neighbors", "path", "constraints_for", "validate", "where_is", "instances", "summary", "query", "tables", "match_file", "check"];

pub fn tool_definitions() -> Json {
    let mut tools = tool_list();
    if let Some(arr) = tools.as_array_mut() {
        for t in arr.iter_mut() {
            let name = t["name"].as_str().unwrap_or("").to_string();
            t["annotations"] = annotations_for(&name);
        }
    }
    tools
}

fn tool_list() -> Json {
    json!([
        {
            "name": "search",
            "description": "Find entities (types), relationships, rules, actions, metrics, functions, glossary terms and data sources by plain-text query. Start here when you don't know the exact name.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string", "description": "Words to look for, e.g. 'lead time' or 'vendor'" },
                "limit": { "type": "integer", "default": 15 }
            }, "required": ["query"] }
        },
        {
            "name": "describe",
            "description": "Full detail of one thing: an entity type (properties, relationships, rules, data source, actions), a relationship, an action (inputs, effects, checks, approval), a rule, a metric (what counts and the recipe for where the number comes from), a function (what an agent can run: inputs, outputs, which tool), or a data source. Accepts glossary terms like 'vendor'.",
            "inputSchema": { "type": "object", "properties": {
                "name": { "type": "string", "description": "Exact name, e.g. 'Supplier', 'supplies', 'switch_supplier', 'SC-041'" }
            }, "required": ["name"] }
        },
        {
            "name": "neighbors",
            "description": "The subgraph around a type: which types it connects to within N hops and through which relationships.",
            "inputSchema": { "type": "object", "properties": {
                "name": { "type": "string" },
                "depth": { "type": "integer", "default": 1, "minimum": 1, "maximum": 6 }
            }, "required": ["name"] }
        },
        {
            "name": "path",
            "description": "Shortest chain of relationships connecting two types, e.g. how a Customer relates to a Supplier.",
            "inputSchema": { "type": "object", "properties": {
                "from": { "type": "string" }, "to": { "type": "string" }
            }, "required": ["from", "to"] }
        },
        {
            "name": "constraints_for",
            "description": "Every guardrail that applies to an entity type, relationship, or action, sorted by severity (block > warn > info). Call this before telling a user an action is possible; `validate` then checks them against real rows.",
            "inputSchema": { "type": "object", "properties": {
                "name": { "type": "string" }
            }, "required": ["name"] }
        },
        {
            "name": "validate",
            "description": "Check whether an action may be taken with these inputs. Evaluates every applicable rule against real records, returns pass/warn/fail with reasons, who must approve, and alternative inputs that would pass. ALWAYS call this before telling a user an action is possible.",
            "inputSchema": { "type": "object", "properties": {
                "action": { "type": "string", "description": "Action name, e.g. 'switch_supplier'" },
                "inputs": { "type": "object", "description": "input name → key value (e.g. \"V-10422\") or an inline record (e.g. {\"tier\": 3}) when the data is not readable here", "additionalProperties": true }
            }, "required": ["action", "inputs"] }
        },
        {
            "name": "where_is",
            "description": "Where a type's rows live: connection, kind (csv, parquet, duckdb, postgres, s3, snowflake, mcp…), table, key column, column mapping, and whether it is readable here. Readable kinds can be read with `instances` or joined in SQL with `query`. For `kind: mcp` the answer names the server, tool and params to call on another MCP server; for the rest, use your own connector.",
            "inputSchema": { "type": "object", "properties": {
                "type": { "type": "string" }
            }, "required": ["type"] }
        },
        {
            "name": "instances",
            "description": "Rows of a type (or edge rows of a relationship) when the data is readable here (inline instances, csv, parquet, duckdb, postgres, s3). Optional key lookup. For other warehouse-backed types use where_is and your own connector.",
            "inputSchema": { "type": "object", "properties": {
                "type": { "type": "string" },
                "key": { "type": "string", "description": "Return just the row with this key" },
                "limit": { "type": "integer", "default": 25 }
            }, "required": ["type"] }
        },
        {
            "name": "summary",
            "description": "Compact Markdown overview of the whole ontology: entities by domain, blocking rules, actions, metrics (what counts and where it comes from), functions (what can be run and where), data sources, glossary. Under ~4k tokens. Load this first.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "query",
            "description": "Run ONE read-only SQL statement (SELECT or WITH; DuckDB dialect) and get rows back as JSON objects, at most 500. The ontology is exposed as tables: nodes (types), properties, edges (relationships), constraints (rules), actions, sources. Every type or relationship whose data is readable here is a view named after it (e.g. Supplier, Part, supplies), so one statement joins rules to rows: select s.name, c.id from Supplier s join constraints c on c.applies_to = 'Supplier' where s.tier = 1. Discover columns with select * from information_schema.columns where table_name = 'Supplier'. Use `tables` to list what is queryable.",
            "inputSchema": { "type": "object", "properties": {
                "sql": { "type": "string", "description": "A single SELECT or WITH statement" },
                "limit": { "type": "integer", "default": 500, "minimum": 1, "maximum": 500 }
            }, "required": ["sql"] }
        },
        {
            "name": "tables",
            "description": "What `query` can see: the ontology tables, one row per data view (type or relationship → view name and where it reads from), and any binding that could not be attached and why.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "match_file",
            "description": "Read-only. Tell Lorefile about a file the user gave you (CSV, spreadsheet sheet, export) without sending its rows: its name, its column names in order, up to five example values per column (used only to guess types, never stored), and if you can, the row count and each column's distinct and empty counts. Answers whether this kind of file is already known (same), known with columns added or missing (changed), saved and waiting for a person (waiting), or new; for a known file, how to read it: the number that counts and the rows that do not, with who stands behind each definition; for a new or changed file, what Lorefile would save and the questions to ask the user first. Call it whenever the user gives you a file, before answering questions about it.",
            "inputSchema": file_schema(false)
        },
        {
            "name": "learn_file",
            "description": "Save what a file means, so the next file of the same shape is read the same way by any agent: one entity with every column as a field (the file's own names, types, the key), where its rows come from (a file the user gives the agent), and from the user's answers the number that counts and the rows that do not. Saved as one change a person accepts. Send the same columns as to match_file, one short description per column (what it is and its unit; never quote values from the file), and `answers`. Without `answers` nothing is saved and you get the questions to ask; with `answers: {}` only the shape is saved. Lorefile never stores rows or example values.",
            "inputSchema": file_schema(true)
        },
        {
            "name": "propose_patch",
            "description": "Save what you settled to the ontology without writing YAML: one op, or `ops: [..]` for several that depend on each other (a source, then the entity, then its binding, then the metric), planned in order and saved as one change a person accepts as one. Use it the moment the user settles a definition during your work (which event counts as success and where it comes from, which table or tool holds an entity's rows, what a word means, a rule to respect), so the next question runs the same way for everyone. The change is compiled and checked; when the server allows edits and it is clean it is written, otherwise it is queued for a person to accept in the web page. Ops and their fields: add_type {name, domain, description, properties: {field: type | {type, key, unit, description}}, key} (with properties, no <name>_id is made) · add_property {type, property, datatype, description, unit, key, column (the file's header when it differs)} · set_type_description {type, text} · set_property_description {type, property, text} · add_relationship {type, name, to, cardinality, description} · set_relationship_description {type, relationship, text} · add_constraint {applies_to, name, rule (CEL), severity block|warn|info, rationale, owner} · set_source {type, connection, table | tool + params, key, columns: {field: column}} · add_term {term, means (an entity), description} · add_metric {metric, description, connection, table | tool + params, filter, measure, grain, owner, caveats, basis transactional|attributed|modelled|experimental, variants: [{when, description, connection, table | tool + params, filter, measure, basis}], sql} · add_function {function, description, inputs: {name: entity|primitive}, outputs: {…}, kind mcp|http|manual, server, tool, args, url, owner, caveats, may_be_called_by} · add_source {connection, kind (bigquery|snowflake|postgres|csv|parquet|s3|mcp…), server (for mcp), description, project, dataset, path} · record_check {kind metric|binding|guardrail, name, against (the source whose rows were used), note (what was compared with what)}: report that a definition a person accepted gave the right number against real rows; it is always queued, on its own (not in `ops`), and only a person's click marks it checked. Always add `reason`. Order matters when teaching from scratch: add_source and add_type first, then set_source, add_metric, add_term, since each is checked against what exists.",
            "inputSchema": { "type": "object", "properties": {
                "op": { "type": "string", "enum": ["add_type", "add_property", "set_type_description", "set_property_description", "add_relationship", "set_relationship_description", "add_constraint", "set_source", "add_source", "add_term", "add_metric", "add_function", "record_check"] },
                "ops": { "type": "array", "description": "Several ops as one change, in the order they depend on each other; each item has the same fields as a single call", "items": { "type": "object", "additionalProperties": true } },
                "reason": { "type": "string", "description": "One sentence on why, for the person who reviews it" }
            }, "additionalProperties": true }
        },
        {
            "name": "propose_change",
            "description": "Propose a change to the ontology: send the full new text of one YAML file plus a reason. trust.yaml (the grades) is written by Lorefile only and is refused here. It is compiled and checked first. When the server allows edits and the change is clean it is written; otherwise it is queued for a person to review in the web page, and you get the proposal id. Use it when you learn something the ontology is missing, such as a type, a field, a link or a guardrail.",
            "inputSchema": { "type": "object", "properties": {
                "file": { "type": "string", "description": "Path relative to the ontology folder, ending in .yaml, e.g. types/supplier.yaml" },
                "content": { "type": "string", "description": "The full new text of the file" },
                "reason": { "type": "string", "description": "One or two sentences on why; becomes the commit message body when edits are committed" }
            }, "required": ["file", "content"] }
        }
    ])
}

pub struct Server<'a> {
    pub ont: &'a Ontology,
    pub data: DataLayer<'a>,
    /// Whether `propose_change` may write, and whether it commits.
    pub edit: EditConfig,
    /// Set after a write so the owner reloads the ontology before the next request.
    pub reload_needed: Cell<bool>,
    /// The folder of spaces, when `summary` should say how the space's history stands
    /// (left out for shipped examples, which have none).
    pub history_root: Option<PathBuf>,
    /// The author of proposals and activity lines: `agent via MCP`, or a named connection.
    pub via: String,
    /// An OAuth connection: answers leave out where things are on this machine.
    pub guest: bool,
    /// The page's Agents screen for this space, when the server knows its address (HTTP only).
    pub link: Option<String>,
}

impl<'a> Server<'a> {
    /// Share a data layer (and its open DuckDB) instead of opening a fresh one.
    pub fn with_data(ont: &'a Ontology, data: DataLayer<'a>) -> Self {
        Server { ont, data, edit: EditConfig::default(), reload_needed: Cell::new(false), history_root: None, via: VIA.to_string(), guest: false, link: None }
    }

    pub fn with_link(mut self, link: Option<String>) -> Self {
        self.link = link;
        self
    }

    /// Add the one History line to `summary`, reading the space's git history.
    pub fn with_history(mut self, spaces_root: Option<PathBuf>) -> Self {
        self.history_root = spaces_root;
        self
    }

    pub fn with_edit(mut self, edit: EditConfig) -> Self {
        self.edit = edit;
        self
    }

    pub fn with_via(mut self, via: &str) -> Self {
        self.via = via.to_string();
        self
    }

    pub fn with_guest(mut self, guest: bool) -> Self {
        self.guest = guest;
        self
    }

    pub fn text(v: impl ToString, is_error: bool) -> Json {
        let mut j = json!({ "content": [{ "type": "text", "text": v.to_string() }] });
        if is_error {
            j["isError"] = json!(true);
        }
        j
    }


    /// The `tools/call` result for `propose_change`, shared with `/api/tool`.
    /// Diagnostics as JSON plus a `note` saying what happened to the file.
    pub fn propose_change_result(out: &Outcome, edit: EditConfig, queued: Option<&Proposal>, summary: &str) -> Json {
        if let Some(why) = &out.refused {
            return Self::text(format!("{}: {}", out.file, why), true);
        }
        let note = Self::outcome_note(out, edit, queued);
        let mut j = out.to_json();
        if !summary.is_empty() {
            j["summary"] = json!(summary);
        }
        j["note"] = json!(note);
        Self::text(Self::pretty(&j), false)
    }

    /// What happened to a proposed change, in a sentence: written, queued (and why), or refused.
    pub fn outcome_note(out: &Outcome, edit: EditConfig, queued: Option<&Proposal>) -> String {
        if let (Some(p), Some(held)) = (queued, &out.held) {
            format!("{}: queued as {}. Nothing was written; a person can accept it in the web page (Agents › Waiting for your review), and on Accept its grade becomes accepted", held, p.id)
        } else if let Some(p) = queued {
            if out.ok {
                format!("queued for review as {}: nothing was written; a person can accept it in the web page (Agents › Waiting for your review)", p.id)
            } else {
                format!("queued as {} but it has {} error(s), so it cannot be accepted until they are fixed; nothing was written", p.id, out.errors())
            }
        } else if out.written {
            let said = match &out.commit {
                Some(h) => format!("written to {} and saved to history as {}", out.file, h),
                None if out.git_error.is_some() => format!("written to {}; commit failed: {}", out.file, out.git_error.clone().unwrap_or_default()),
                None if edit.commit => format!("written to {}; it was already in history as it is", out.file),
                None => format!("written to {}; the change is uncommitted, review it with git diff", out.file),
            };
            if out.stamped.iter().any(|g| g.ends_with(": proposed")) {
                format!("{}. Grade: proposed. Tell the user a person still has to accept it (Agents › Written by agents, not reviewed)", said)
            } else {
                said
            }
        } else if !out.ok {
            format!("refused: the change leaves {} error(s); nothing was written", out.errors())
        } else if !edit.allow {
            "edit mode is off; the change was validated but not written".to_string()
        } else {
            "nothing was written".to_string()
        }
    }

    /// Apply a proposed change. Written changes are logged; changes that were
    /// validated but not written (edit mode off, or errors) are queued for a
    /// person to review. Returns the outcome and the queued proposal, if any.
    #[allow(clippy::too_many_arguments)] // one call site per transport; a struct would only rename the arguments
    pub fn propose(root: &Path, edit: EditConfig, before: &Report, file: &str, content: &str, reason: &str, via: &str, label: Option<(&str, &str)>, summary: &str) -> (Outcome, Option<Proposal>) {
        // a raw file edit still gets a sentence, from what the file declares
        let derived = if summary.is_empty() {
            let (kind, name) = match label {
                Some((k, n)) => (k.to_string(), n.to_string()),
                None => edit::describe_content(file, content),
            };
            if kind.is_empty() || kind == "file" { String::new() } else { format!("changed the {} {}", kind, name) }
        } else {
            String::new()
        };
        let summary: &str = if summary.is_empty() { derived.as_str() } else { summary };
        let line = edit::agent_line(via, summary);
        // an agent never overwrites what a person accepted or checked: queued instead, even with edits on
        let held = if edit.allow { edit::graded_overwrites(root, &[(file.to_string(), content.to_string())]) } else { vec![] };
        let cfg = if held.is_empty() { edit } else { EditConfig { allow: false, commit: false, ..edit } };
        let mut out = edit::apply(root, cfg, before, file, Change::Write(content.to_string()), &Note { via, summary: &line, reason, ..Default::default() }, label);
        if !held.is_empty() && out.refused.is_none() {
            out.held = Some(format!("{}, so this change waits for a person", held.join("; ")));
        }
        let why = if reason.is_empty() { String::new() } else { format!(": {}", reason) };
        if out.written {
            proposals::log(root, via, &format!("{}{}", if summary.is_empty() { format!("changed {}", file) } else { summary.to_string() }, why), file);
            return (out, None);
        }
        if out.refused.is_some() {
            return (out, None);
        }
        match proposals::store(root, file, content, reason, via, &out, summary) {
            Ok(p) => {
                proposals::log(root, via, &format!("proposed {} (waiting for review){}", if summary.is_empty() { format!("a change to {}", file) } else { summary.to_string() }, why), file);
                (out, Some(p))
            }
            Err(_) => (out, None),
        }
    }

    /// A batch of files as one change: written when allowed and clean, else queued as one proposal.
    pub fn propose_many(root: &Path, edit: EditConfig, before: &Report, plan: &patch::BatchPlan, reason: &str, via: &str) -> (Outcome, Option<Proposal>) {
        Self::propose_many_titled(root, edit, before, plan, reason, via, None)
    }

    /// `propose_many` with one sentence for the whole change (`learn_file`'s "learned …" line)
    /// instead of the ops' sentences joined.
    pub fn propose_many_titled(root: &Path, edit: EditConfig, before: &Report, plan: &patch::BatchPlan, reason: &str, via: &str, title: Option<&str>) -> (Outcome, Option<Proposal>) {
        let summary = match title {
            Some(t) => t.to_string(),
            None if plan.summaries.len() == 1 => plan.summaries[0].clone(),
            None => format!("{} changes: {}", plan.summaries.len(), plan.summaries.join("; ")),
        };
        let label = if plan.kinds.len() == 1 { Some((plan.kinds[0].0.as_str(), plan.kinds[0].1.as_str())) } else { None };
        let line = edit::agent_line(via, &summary);
        let held = if edit.allow { edit::graded_overwrites(root, &plan.files) } else { vec![] };
        let cfg = if held.is_empty() { edit } else { EditConfig { allow: false, commit: false, ..edit } };
        let mut out = edit::apply_many(root, cfg, before, &plan.files, &Note { via, summary: &line, reason, ..Default::default() }, label);
        if !held.is_empty() && out.refused.is_none() {
            out.held = Some(format!("{}, so this change waits for a person", held.join("; ")));
        }
        let why = if reason.is_empty() { String::new() } else { format!(": {}", reason) };
        if out.written {
            proposals::log(root, via, &format!("{}{}", summary, why), &out.file);
            return (out, None);
        }
        if out.refused.is_some() {
            return (out, None);
        }
        let files: Vec<ProposedFile> = plan
            .files
            .iter()
            .enumerate()
            .map(|(i, (f, t))| ProposedFile { file: f.clone(), content: t.clone(), before: std::fs::read_to_string(root.join(f)).unwrap_or_default(), summary: plan.summaries.get(i).cloned().unwrap_or_default() })
            .collect();
        match proposals::store_many(root, &files, reason, via, &out, &summary) {
            Ok(p) => {
                proposals::log(root, via, &format!("proposed {} (waiting for review){}", summary, why), &out.file);
                (out, Some(p))
            }
            Err(_) => (out, None),
        }
    }

    /// `propose_patch` with `op: record_check`: an agent reports a check. Always
    /// queued, whatever the edit mode; the grade becomes checked only when a
    /// person clicks "Yes, mark as checked".
    pub fn record_check(ont: &Ontology, args: &Json, via: &str) -> Json {
        let reason = Self::arg_str(args, &["reason"]).unwrap_or_default();
        let claim = match patch::plan_check(ont, args) {
            Ok(c) => c,
            Err(e) => return Self::text(format!("{} (nothing was queued)", e), true),
        };
        match proposals::store_check(&ont.root, &claim, &reason, via) {
            Ok(p) => {
                proposals::log(&ont.root, via, &format!("reported a check of {} {} against {} (waiting for a person)", claim.kind, claim.name, claim.against), crate::trust::FILE);
                Self::text(Self::pretty(&json!({
                    "ok": true, "written": false, "queued": p.id, "kind": claim.kind, "name": claim.name, "against": claim.against,
                    "note": format!("queued as {}: nothing is graded yet. A person confirms the check in the web page (Agents › Waiting for your review); only then is {} marked checked, in their name.", p.id, claim.name),
                })), false)
            }
            Err(e) => Self::text(e, true),
        }
    }

    pub fn pretty(v: &Json) -> String {
        serde_json::to_string_pretty(v).unwrap_or_default()
    }

    fn arg_str(args: &Json, keys: &[&str]) -> Option<String> {
        for k in keys {
            if let Some(s) = args.get(*k).and_then(|v| v.as_str()) {
                return Some(s.to_string());
            }
        }
        None
    }

    /// Run one tool. Returns an MCP `tools/call` result object.
    pub fn call_tool(&self, name: &str, args: &Json) -> Json {
        let ont = self.ont;
        match name {
            "search" => {
                let q = Self::arg_str(args, &["query", "q"]).unwrap_or_default();
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(15) as usize;
                let hits = ont.search(&q, limit);
                if hits.is_empty() {
                    return Self::text(format!("No matches for '{}'. Try `summary` to see what exists.", q), false);
                }
                Self::text(Self::pretty(&json!(hits)), false)
            }
            "describe" => {
                let n = Self::arg_str(args, &["name", "type"]).unwrap_or_default();
                match ont.describe(&n) {
                    Some(j) => Self::text(Self::pretty(&j), false),
                    None => {
                        let hints: Vec<String> = ont.search(&n, 5).into_iter().map(|h| h.name).collect();
                        Self::text(format!("Nothing named '{}'. Closest: {}", n, hints.join(", ")), true)
                    }
                }
            }
            "neighbors" => {
                let n = Self::arg_str(args, &["name", "type"]).unwrap_or_default();
                let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                match ont.neighbors(&n, depth) {
                    Some(sg) => Self::text(Self::pretty(&json!(sg)), false),
                    None => Self::text(format!("Unknown type '{}'", n), true),
                }
            }
            "path" => {
                let f = Self::arg_str(args, &["from"]).unwrap_or_default();
                let t = Self::arg_str(args, &["to"]).unwrap_or_default();
                match ont.path(&f, &t) {
                    Some(p) => Self::text(Self::pretty(&json!({ "from": f, "to": t, "hops": p.len(), "path": p })), false),
                    None => Self::text(format!("No path between '{}' and '{}' (or one is not a type)", f, t), true),
                }
            }
            "constraints_for" => {
                let n = Self::arg_str(args, &["name", "type", "action"]).unwrap_or_default();
                if ont.kind_of(&n).is_none() {
                    return Self::text(format!("Nothing named '{}'", n), true);
                }
                let cs = ont.constraints_for(&n);
                Self::text(Self::pretty(&json!({ "name": n, "count": cs.len(), "constraints": cs })), false)
            }
            "validate" | "check" => {
                let action = Self::arg_str(args, &["action"]).unwrap_or_default();
                let inputs: IndexMap<String, Json> = match args.get("inputs") {
                    Some(Json::Object(o)) => o.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                    _ => IndexMap::new(),
                };
                match rules::check_action(ont, &self.data, &action, &inputs) {
                    Ok(rep) => Self::text(Self::pretty(&json!(rep)), false),
                    Err(e) => Self::text(e.to_string(), true),
                }
            }
            "where_is" => {
                let n = Self::arg_str(args, &["type", "name"]).unwrap_or_default();
                match self.data.where_is(&n) {
                    Ok(j) => Self::text(Self::pretty(&j), false),
                    Err(e) => Self::text(e.to_string(), true),
                }
            }
            "instances" => {
                let n = Self::arg_str(args, &["type", "name"]).unwrap_or_default();
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(25) as usize;
                // rows that arrive as a person's file are never here: say where they are instead of "0 rows"
                if let Some(h) = self.data.user_file_hint(&n) {
                    return Self::text(Self::pretty(&json!({ "name": n, "readable_here": false, "hint": h })), false);
                }
                if let Some(key) = Self::arg_str(args, &["key"]) {
                    return match self.data.find(&n, &key) {
                        Ok(Some(r)) => Self::text(Self::pretty(&json!(r)), false),
                        Ok(None) => Self::text(format!("No {} with key '{}'", n, key), true),
                        Err(e) => Self::text(e.to_string(), true),
                    };
                }
                let rows = if ont.type_(&n).is_none() && !ont.relationship(&n).is_empty() { self.data.edges(&n) } else { self.data.instances(&n) };
                match rows {
                    Ok(rows) => {
                        let total = rows.len();
                        let shown: Vec<_> = rows.into_iter().take(limit).collect();
                        Self::text(Self::pretty(&json!({ "name": n, "total": total, "shown": shown.len(), "rows": shown })), false)
                    }
                    Err(e) => Self::text(e.to_string(), true),
                }
            }
            "summary" => {
                let mut md = ont.summary_markdown();
                if let Some(sr) = &self.history_root {
                    let mut line = edit::history_line(&ont.root, sr);
                    if self.guest && line.starts_with("History: kept by the repository at ") {
                        // an OAuth connection is not told where things are on this machine
                        line = "History: kept by a repository outside the ontology; accepted changes stay uncommitted unless the server runs with --commit-edits.".to_string();
                    }
                    md.push_str(&format!("\n\n{}\n", line));
                }
                Self::text(md, false)
            }
            "query" | "sql" => {
                let sql = Self::arg_str(args, &["sql", "query"]).unwrap_or_default();
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(crate::data::MAX_QUERY_ROWS as u64) as usize;
                match self.data.query(&sql, limit) {
                    Ok(res) => Self::text(Self::pretty(&json!(res)), false),
                    Err(e) => Self::text(e.to_string(), true),
                }
            }
            "tables" => match self.data.sql_catalog() {
                Ok(j) => Self::text(Self::pretty(&j), false),
                Err(e) => Self::text(e.to_string(), true),
            },
            "match_file" => self.match_file(args),
            "learn_file" => self.learn_file(args),
            "propose_change" => {
                let file = Self::arg_str(args, &["file", "path"]).unwrap_or_default();
                let content = Self::arg_str(args, &["content", "text"]).unwrap_or_default();
                let reason = Self::arg_str(args, &["reason"]).unwrap_or_default();
                let before = validate::validate(ont);
                let (out, queued) = Self::propose(&ont.root, self.edit, &before, &file, &content, &reason, &self.via, None, "");
                if out.written {
                    self.reload_needed.set(true);
                }
                Self::propose_change_result(&out, self.edit, queued.as_ref(), "")
            }
            "propose_patch" if Self::arg_str(args, &["op"]).as_deref() == Some("record_check") => Self::record_check(ont, args, &self.via),
            "propose_patch" => {
                let reason = Self::arg_str(args, &["reason"]).unwrap_or_default();
                if let Some(ops) = args.get("ops").and_then(|o| o.as_array()) {
                    return match patch::plan_many(ont, ops) {
                        Ok(plan) => {
                            let before = validate::validate(ont);
                            let (out, queued) = Self::propose_many(&ont.root, self.edit, &before, &plan, &reason, &self.via);
                            if out.written {
                                self.reload_needed.set(true);
                            }
                            let summary = if plan.summaries.len() == 1 { plan.summaries[0].clone() } else { format!("{} changes: {}", plan.summaries.len(), plan.summaries.join("; ")) };
                            Self::propose_change_result(&out, self.edit, queued.as_ref(), &summary)
                        }
                        Err(e) => Self::text(format!("{} (nothing was written)", e), true),
                    };
                }
                match patch::plan(ont, args) {
                    Ok(p) => {
                        let before = validate::validate(ont);
                        let (out, queued) = Self::propose(&ont.root, self.edit, &before, &p.file, &p.text, &reason, &self.via, Some((&p.kind, &p.name)), &p.summary);
                        if out.written {
                            self.reload_needed.set(true);
                        }
                        Self::propose_change_result(&out, self.edit, queued.as_ref(), &p.summary)
                    }
                    Err(e) => Self::text(format!("{} (nothing was written)", e), true),
                }
            }
            other => Self::text(format!("Unknown tool '{}'", other), true),
        }
    }

    /// The `learn_file` proposals still waiting for a person, newest first.
    fn pending_learned(&self) -> Vec<Proposal> {
        proposals::list(&self.ont.root).into_iter().filter(|p| p.status == "pending" && p.learned.is_some()).collect()
    }

    /// The space as it would be with a waiting proposal accepted, to say how it reads the file.
    fn with_proposal(&self, p: &Proposal) -> Option<Ontology> {
        let files: Vec<(String, String)> = if p.files.is_empty() { vec![(p.file.clone(), p.content.clone())] } else { p.files.iter().map(|f| (f.file.clone(), f.content.clone())).collect() };
        let ov: crate::loader::Overrides = files.into_iter().map(|(f, t)| (f, Some(t))).collect();
        edit::check_many(&self.ont.root, &ov).ok().map(|(o, _)| o)
    }

    fn file_reply(&self, input: &learn::FileInput, m: &learn::Match) -> Result<Json, String> {
        let waiting = match m {
            learn::Match::Waiting { proposal, .. } => self.with_proposal(proposal),
            _ => None,
        };
        learn::match_reply(self.ont, input, m, waiting.as_ref())
    }

    /// `match_file`: does Lorefile know this kind of file, and how is it read? Reads only.
    fn match_file(&self, args: &Json) -> Json {
        let input = match learn::parse(args) {
            Ok(i) => i,
            Err(e) => return Self::text(e, true),
        };
        let m = learn::match_shape(self.ont, &self.pending_learned(), &input);
        match self.file_reply(&input, &m) {
            Ok(j) => Self::text(Self::pretty(&j), false),
            Err(e) => Self::text(e, true),
        }
    }

    /// `learn_file`: save what a file means as one change. Without `answers`, a dry run that
    /// returns the questions. A newer learn of a file that is still waiting replaces it.
    fn learn_file(&self, args: &Json) -> Json {
        let input = match learn::parse(args) {
            Ok(i) => i,
            Err(e) => return Self::text(format!("{} (nothing was saved)", e), true),
        };
        let pending = self.pending_learned();
        let m = learn::match_shape(self.ont, &pending, &input);
        if input.answers.is_none() {
            return match self.file_reply(&input, &m) {
                Ok(mut j) => {
                    j["saved"] = json!(false);
                    if matches!(m, learn::Match::New | learn::Match::Changed(_)) {
                        j["next"] = json!("Nothing was saved. Ask the user these questions in your own words, then call learn_file again with the same columns and `answers` (`answers: {}` saves the shape only).");
                    }
                    Self::text(Self::pretty(&j), false)
                }
                Err(e) => Self::text(e, true),
            };
        }
        let planned = match learn::plan(self.ont, &input, &m) {
            Ok(learn::Plan::Ops(p)) => p,
            Ok(learn::Plan::Nothing(why)) => {
                let mut j = self.file_reply(&input, &m).unwrap_or_else(|_| json!({}));
                j["saved"] = json!(false);
                j["say"] = json!(why);
                return Self::text(Self::pretty(&j), false);
            }
            Err(e) => return Self::text(format!("{} (nothing was saved)", e), true),
        };
        let plan = match patch::plan_many(self.ont, &planned.ops) {
            Ok(p) => p,
            Err(e) => return Self::text(format!("{} (nothing was saved)", e), true),
        };
        // the same file still waiting: an identical draft is not queued twice; a different one replaces it
        let earlier = pending.iter().find(|p| p.learned.as_ref().map(|l| learn::same_headers(&l.headers, &planned.learned.headers)).unwrap_or(false));
        if let Some(old) = earlier {
            let old_files: Vec<(String, String)> = if old.files.is_empty() { vec![(old.file.clone(), old.content.clone())] } else { old.files.iter().map(|f| (f.file.clone(), f.content.clone())).collect() };
            if old_files == plan.files {
                return Self::text(Self::pretty(&json!({
                    "saved": false,
                    "queued": old.id,
                    "learned": planned.line,
                    "say": format!("Already waiting as {}, exactly as described: nothing new to save. A person accepts it in Lorefile (Agents › Waiting for your review){}.", old.id, self.link.as_ref().map(|l| format!(": {}", l)).unwrap_or_default()),
                })), false);
            }
        }
        let reason = input.reason.clone().unwrap_or_else(|| format!("learned {}", if input.file_name.is_empty() { "a file" } else { input.file_name.as_str() }));
        let before = validate::validate(self.ont);
        let (out, queued) = Self::propose_many_titled(&self.ont.root, self.edit, &before, &plan, &reason, &self.via, Some(&planned.line));
        if let Some(why) = &out.refused {
            return Self::text(format!("{} (nothing was saved)", why), true);
        }
        if out.written {
            self.reload_needed.set(true);
        }
        if let Some(p) = &queued {
            let _ = proposals::set_learned(&self.ont.root, &p.id, &planned.learned);
        }
        let mut replaced = None;
        if let Some(old) = earlier {
            if out.written || queued.is_some() {
                let by = queued.as_ref().map(|p| p.id.clone()).or_else(|| out.commit.clone()).unwrap_or_else(|| "a written change".into());
                if proposals::dismiss_replaced(&self.ont.root, &old.id, &by).is_ok() {
                    replaced = Some(old.id.clone());
                }
            }
        }
        let note = Self::outcome_note(&out, self.edit, queued.as_ref());
        let mut j = out.to_json();
        j["saved"] = json!(out.written || queued.is_some());
        j["queued"] = json!(queued.as_ref().map(|p| p.id.clone()));
        j["learned"] = json!(planned.line);
        j["entity"] = json!(planned.entity);
        j["note"] = json!(note);
        if let (Some(_), Some(l)) = (&queued, &self.link) {
            j["accept_at"] = json!(l);
        }
        if let Some(r) = &replaced {
            j["replaced"] = json!(r);
        }
        if !planned.left_out.is_empty() {
            j["left_out"] = json!(planned.left_out);
        }
        if !planned.notes.is_empty() {
            j["notes"] = json!(planned.notes);
        }
        let shape = if planned.fields > 0 { format!(" ({} column{}, {} described)", planned.fields, if planned.fields == 1 { "" } else { "s" }, planned.described) } else { String::new() };
        let what = format!("{}{}", planned.entity, shape);
        let say = if !out.ok {
            format!("Not saved cleanly: the change has {} error(s); see diagnostics.", out.errors())
        } else if let Some(p) = &queued {
            format!(
                "Saved for review as {}: {}. {} A person accepts it in Lorefile (Agents › Waiting for your review){}; until then, say it has not been accepted yet.",
                p.id,
                what,
                match planned.line.split_once("; ") {
                    Some((_, m)) => format!("The result: {}.", m),
                    None if planned.recipe_kept => "How the result is read stays as it was saved.".to_string(),
                    None => "No number was saved.".to_string(),
                },
                self.link.as_ref().map(|l| format!(": {}", l)).unwrap_or_default()
            )
        } else {
            format!("Saved: {}. Grade: proposed. Tell the user a person still has to accept it (Agents › Written by agents, not reviewed).", what)
        };
        j["say"] = json!(say);
        Self::text(Self::pretty(&j), false)
    }

    pub fn resources(&self) -> Json {
        let mut list = vec![
            json!({ "uri": "ontology://summary", "name": "Ontology summary", "description": "Compact Markdown of the whole ontology for a system prompt", "mimeType": "text/markdown" }),
            json!({ "uri": "ontology://glossary", "name": "Glossary", "description": "Business words → canonical names", "mimeType": "text/markdown" }),
        ];
        for t in self.ont.types.values().take(200) {
            list.push(json!({ "uri": format!("ontology://types/{}", t.name), "name": t.name, "description": t.description, "mimeType": "text/markdown" }));
        }
        for a in self.ont.actions.values().take(100) {
            list.push(json!({ "uri": format!("ontology://actions/{}", a.name), "name": a.name, "description": a.description, "mimeType": "application/json" }));
        }
        json!(list)
    }

    pub fn read_resource(&self, uri: &str) -> Option<(String, String)> {
        if uri == "ontology://summary" {
            return Some(("text/markdown".into(), self.ont.summary_markdown()));
        }
        if uri == "ontology://glossary" {
            let mut md = String::from("# Glossary\n\n");
            for g in &self.ont.glossary {
                md.push_str(&format!("- **{}** → {}{}\n", g.term, g.means, if g.description.is_empty() { String::new() } else { format!(": {}", g.description) }));
            }
            return Some(("text/markdown".into(), md));
        }
        if let Some(name) = uri.strip_prefix("ontology://types/") {
            return self.ont.describe_markdown(name).map(|m| ("text/markdown".to_string(), m));
        }
        if let Some(name) = uri.strip_prefix("ontology://actions/") {
            return self.ont.describe(name).map(|j| ("application/json".to_string(), Self::pretty(&j)));
        }
        None
    }
}

/// One MCP server, many spaces. Each request names its space (or gets the
/// implied one) and is answered by a `Server` built on that space's state.
/// Stdio owns its registry; `serve` lends the one behind its lock, so both
/// transports answer through the same code.
pub struct Hub<'r> {
    pub reg: &'r mut Registry,
    pub edit: EditConfig,
    /// `--space` (or `/s/<id>/mcp` over HTTP): serve one space only, and hide the `space` argument.
    pub only: Option<String>,
    /// `false` for an OAuth connection without `lorefile:propose`: the propose tools are left
    /// out of `tools/list` and refused in `tools/call`.
    pub can_propose: bool,
    /// The author of what this caller proposes: `VIA`, or a named connection.
    pub via: String,
    /// An OAuth connection (a guest): `spaces` leaves out the folders, and errors name no other space.
    pub guest: bool,
    /// The address the page answers on (HTTP only), for links to a space's Agents screen.
    pub link_base: Option<String>,
}

impl Hub<'_> {
    /// One HTTP body: a single message, or a batch (an array). Notifications get
    /// no reply, so a body of notifications only answers `None` (HTTP 202).
    pub fn handle_body(&mut self, msg: &Json) -> Option<Json> {
        match msg {
            Json::Array(a) if a.is_empty() => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": "invalid request: an empty batch" } })),
            Json::Array(a) => {
                let out: Vec<Json> = a.iter().filter_map(|m| self.handle(m)).collect();
                if out.is_empty() { None } else { Some(Json::Array(out)) }
            }
            m => self.handle(m),
        }
    }

    /// The flags as they apply to one space: shipped ones never take writes,
    /// and commits follow the space's history (see `EditConfig::for_space`).
    fn edit_for(&self, id: &str) -> EditConfig {
        match self.reg.info(id) {
            Some(s) if s.builtin => EditConfig::default(),
            Some(s) if self.edit.allow => self.edit.for_space(&s.dir, &self.reg.root),
            _ => self.edit,
        }
    }

    fn pick(&self, want: Option<&str>) -> Result<String, String> {
        match &self.only {
            Some(o) => Ok(o.clone()),
            None => self.reg.resolve(want),
        }
    }

    /// `spaces`: every space, or only the one this server serves (`--space`, `/s/<id>/mcp`).
    /// A guest is not told the folders they live in.
    fn spaces_text(&mut self) -> String {
        let mut list = self.reg.summaries();
        if let Some(o) = &self.only {
            list.retain(|s| s["id"].as_str() == Some(o.as_str()));
        }
        if self.guest {
            for s in list.iter_mut() {
                if let Some(m) = s.as_object_mut() {
                    m.remove("dir");
                }
            }
        }
        let mut j = if self.guest { json!({ "spaces": list }) } else { json!({ "root": self.reg.root.display().to_string(), "spaces": list }) };
        match &self.only {
            Some(o) => j["implied"] = json!(o),
            None => {
                if let Some(d) = self.reg.default_id() {
                    j["implied"] = json!(d);
                }
            }
        }
        Server::pretty(&j)
    }

    fn instructions(&self) -> String {
        let mine = self.reg.user_spaces();
        let shipped = self.reg.spaces.len() - mine.len();
        let scope = match (&self.only, mine.len()) {
            (Some(o), _) => format!("This Lorefile server holds one ontology space, '{}'.", o),
            (None, 0) => format!("This Lorefile server holds no space of the user's own yet, only {} shipped examples (ids start with @). Name one with `space`, e.g. \"@supply-chain\".", shipped),
            (None, 1) => format!("This Lorefile server holds the ontology space '{}'{}. Tools default to it; pass `space` to look at a shipped example (ids start with @).", mine[0].id, if shipped > 0 { format!(" plus {} shipped examples", shipped) } else { String::new() }),
            (None, n) => format!("This Lorefile server holds {} ontology spaces: {}. Every tool takes `space`; ask the user which business or brand they mean when it is not obvious, or call `spaces`.", n, mine.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", ")),
        };
        if !self.can_propose {
            return format!(
                "{} Call `summary` first to learn the vocabulary. Before saying an action is possible, call `validate`. Use `where_is` to find where a type's rows live; if it is readable here, `instances` and `query` (read-only SQL over the ontology tables and data views) answer from the real rows, otherwise query it with your own connector. {} This connection may only read: it cannot propose changes, so when the user settles a definition, tell them it can be saved from a connection allowed to suggest changes. Definitions carry a trust grade. When you rely on one marked proposed or UNVERIFIED, say so before the number or the action: an agent wrote it and no person has reviewed it. Checked means a person compared it with real rows; say who and when if the user asks how sure this is. Numbers with a different `basis` (transactional, attributed, modelled, experimental) are shown side by side, never added.",
                scope, FILE_RECIPE_READ
            );
        }
        format!(
            "{} Call `summary` first to learn the vocabulary. Before saying an action is possible, call `validate`. Use `where_is` to find where a type's rows live; if it is readable here, `instances` and `query` (read-only SQL over the ontology tables and data views) answer from the real rows, otherwise query it with your own connector. To change an ontology, use `propose_patch` for one small fact (a description, a link, a guardrail, a glossary term, a metric, a data binding) or `propose_change` with the full new text of one YAML file; both are compiled and checked first{}. {} When the user settles a definition during your work (which event counts as success and where it comes from, which table or tool holds an entity's rows, what a word means, a rule to respect), offer to save it with `propose_patch`, so the next question runs the same way for everyone. Shipped examples cannot be changed. Definitions carry a trust grade. When you rely on one marked proposed or UNVERIFIED, say so before the number or the action: an agent wrote it and no person has reviewed it. Checked means a person compared it with real rows; say who and when if the user asks how sure this is. You cannot set a grade: a person accepts or checks in the web page{}. Numbers with a different `basis` (transactional, attributed, modelled, experimental) are shown side by side, never added.",
            scope,
            if self.edit.allow { ", then written" } else { ", and queued for a person to accept in the web page" },
            FILE_RECIPE,
            if self.edit.allow { ", and a change to something a person accepted or checked is queued for them instead of written" } else { "" }
        )
    }

    /// Resource URIs carry the space: `ontology://<space>/summary`. The flat
    /// `ontology://summary` form still works and means the implied space.
    fn split_uri(&self, uri: &str) -> Option<(String, String)> {
        let rest = uri.strip_prefix("ontology://")?;
        let (first, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if self.reg.spaces.contains_key(first) {
            // a server for one space reads that space only
            if self.only.as_deref().map(|o| o != first).unwrap_or(false) {
                return None;
            }
            return Some((first.to_string(), format!("ontology://{}", tail)));
        }
        let id = self.pick(None).ok()?;
        Some((id, uri.to_string()))
    }

    fn with_space<T>(&mut self, id: &str, f: impl FnOnce(&Server<'_>) -> T) -> Result<T, String> {
        let edit = self.edit_for(id);
        let history = match self.reg.info(id) {
            Some(s) if !s.builtin => Some(self.reg.root.clone()),
            _ => None,
        };
        let l = self.reg.get(id).map_err(|e| format!("{:#}", e))?;
        let link = self.link_base.as_ref().map(|b| format!("{}/s/{}/#agents", b.trim_end_matches('/'), id));
        let srv = Server::with_data(&l.ont, l.data()).with_edit(edit).with_history(history).with_via(&self.via).with_guest(self.guest).with_link(link);
        let out = f(&srv);
        let reload = srv.reload_needed.get();
        drop(srv);
        if reload {
            l.reload();
            self.reg.touched(id);
        }
        Ok(out)
    }

    pub fn handle(&mut self, msg: &Json) -> Option<Json> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        // notifications never get a reply
        if id.is_none() || method.starts_with("notifications/") {
            return None;
        }
        let ok = |result: Json| Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        let err = |code: i64, message: String| Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }));

        match method {
            "initialize" => {
                let requested = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(PROTOCOL);
                let version = if requested == "2024-11-05" || requested == "2025-03-26" { requested } else { PROTOCOL };
                ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false }, "resources": { "subscribe": false, "listChanged": false } },
                    "serverInfo": { "name": "lorefile", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": self.instructions()
                }))
            }
            "ping" => ok(json!({})),
            "tools/list" => {
                self.reg.rescan();
                let mut tools = tool_definitions_for(self.reg, self.only.as_deref());
                if !self.can_propose {
                    if let Some(a) = tools.as_array_mut() {
                        a.retain(|t| !PROPOSE_TOOLS.contains(&t["name"].as_str().unwrap_or("")));
                    }
                }
                ok(json!({ "tools": tools }))
            }
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let mut args = params.get("arguments").cloned().unwrap_or(json!({}));
                if name == "spaces" {
                    return ok(Server::text(self.spaces_text(), false));
                }
                if !self.can_propose && PROPOSE_TOOLS.contains(&name) {
                    return ok(Server::text(crate::oauth::read_only_refusal(), true));
                }
                let want = args.get("space").and_then(|v| v.as_str()).map(|s| s.to_string());
                if let Some(o) = args.as_object_mut() {
                    o.remove("space");
                }
                let space = match self.pick(want.as_deref()) {
                    Ok(s) => s,
                    Err(_) => {
                        // a space created since this server started: look again before giving up
                        self.reg.rescan();
                        match self.pick(want.as_deref()) {
                            Ok(s) => s,
                            Err(e) => return ok(Server::text(e, true)),
                        }
                    }
                };
                if PROPOSE_TOOLS.contains(&name) && self.reg.info(&space).map(|s| s.builtin).unwrap_or(false) {
                    return ok(Server::text(format!("{} is a shipped example and stays read-only. Create a space of your own (or copy the example) and propose the change there.", space), true));
                }
                match self.with_space(&space, |srv| srv.call_tool(name, &args)) {
                    Ok(mut r) => {
                        // a miss in the implied space: say where we looked and where else to look
                        if want.is_none() && self.reg.spaces.len() > 1 && r["isError"].as_bool().unwrap_or(false) {
                            let others: Vec<&str> = self.reg.spaces.keys().filter(|k| **k != space).map(|k| k.as_str()).collect();
                            if let Some(t) = r["content"][0]["text"].as_str() {
                                let hint = format!("{}\n(looked in the space `{}`, the default here; other spaces: {}. Pass `space` to look elsewhere.)", t, space, others.join(", "));
                                r["content"][0]["text"] = json!(hint);
                            }
                        }
                        ok(r)
                    }
                    Err(e) => ok(Server::text(e, true)),
                }
            }
            "resources/list" => {
                // the user's spaces, or the shipped ones when there are none of theirs
                let mut ids: Vec<String> = self.reg.user_spaces().iter().map(|s| s.id.clone()).collect();
                if let Some(o) = &self.only {
                    ids = vec![o.clone()];
                } else if ids.is_empty() {
                    ids = self.reg.spaces.keys().cloned().collect();
                }
                let prefix = ids.len() > 1;
                let mut all = Vec::new();
                for sid in ids {
                    if let Ok(list) = self.with_space(&sid, |srv| srv.resources()) {
                        for mut r in list.as_array().cloned().unwrap_or_default() {
                            if prefix {
                                let uri = r["uri"].as_str().unwrap_or("").replace("ontology://", &format!("ontology://{}/", sid));
                                r["uri"] = json!(uri);
                                r["name"] = json!(format!("{} · {}", sid, r["name"].as_str().unwrap_or("")));
                            }
                            all.push(r);
                        }
                    }
                }
                ok(json!({ "resources": all }))
            }
            "resources/templates/list" => ok(json!({ "resourceTemplates": [
                { "uriTemplate": "ontology://{space}/types/{name}", "name": "Type", "mimeType": "text/markdown" },
                { "uriTemplate": "ontology://{space}/actions/{name}", "name": "Action", "mimeType": "application/json" },
                { "uriTemplate": "ontology://{space}/summary", "name": "Summary", "mimeType": "text/markdown" }
            ] })),
            "resources/read" => {
                let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                let Some((space, inner)) = self.split_uri(uri) else { return err(-32002, format!("resource not found: {}", uri)) };
                match self.with_space(&space, |srv| srv.read_resource(&inner)) {
                    Ok(Some((mime, text))) => ok(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] })),
                    Ok(None) => err(-32002, format!("resource not found: {}", uri)),
                    Err(e) => err(-32002, e),
                }
            }
            "prompts/list" => ok(json!({ "prompts": [] })),
            "completion/complete" => ok(json!({ "completion": { "values": [] } })),
            _ => err(-32601, format!("method not found: {}", method)),
        }
    }
}

pub fn run_stdio(mut reg: Registry, edit: EditConfig, only: Option<String>) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let only = match only {
        Some(o) => Some(reg.resolve(Some(&o)).map_err(|e| anyhow::anyhow!(e))?),
        None => None,
    };
    let mine = reg.user_spaces().len();
    eprintln!(
        "lorefile MCP server ready on stdio ({} space{} of yours, {} shipped{}{})",
        mine,
        if mine == 1 { "" } else { "s" },
        reg.spaces.len() - mine,
        only.as_ref().map(|o| format!(", serving only {}", o)).unwrap_or_default(),
        if edit.allow { ", edit mode on" } else { "" }
    );
    let mut hub = Hub { reg: &mut reg, edit, only, can_propose: true, via: VIA.to_string(), guest: false, link_base: None };
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Json = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let resp = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {}", e) } });
                writeln!(out, "{}", resp)?;
                out.flush()?;
                continue;
            }
        };
        // batches are arrays
        let msgs: Vec<Json> = match msg {
            Json::Array(a) => a,
            m => vec![m],
        };
        for m in msgs {
            if let Some(resp) = hub.handle(&m) {
                writeln!(out, "{}", resp)?;
                out.flush()?;
            }
        }
    }
    Ok(())
}
