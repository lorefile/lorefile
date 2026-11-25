//! MCP server over stdio. Hand-rolled JSON-RPC so the protocol surface is
//! visible in one file and there is no SDK churn. Speaks the 2025-06-18
//! protocol and is backwards compatible with 2024-11-05 clients.

use crate::data::DataLayer;
use crate::edit::{self, Change, EditConfig, Outcome};
use crate::ontology::Ontology;
use crate::rules;
use crate::validate;
use anyhow::Result;
use indexmap::IndexMap;
use serde_json::{json, Value as Json};
use std::cell::Cell;
use std::io::{self, BufRead, Write};

const PROTOCOL: &str = "2025-06-18";

pub fn tool_definitions() -> Json {
    json!([
        {
            "name": "search",
            "description": "Find types, relationships, rules, actions, glossary terms and data sources by plain-text query. Start here when you don't know the exact name.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string", "description": "Words to look for, e.g. 'lead time' or 'vendor'" },
                "limit": { "type": "integer", "default": 15 }
            }, "required": ["query"] }
        },
        {
            "name": "describe",
            "description": "Full detail of one thing: a type (properties, relationships, rules, data source, actions), a relationship, an action (inputs, effects, checks, approval), a rule, or a data source. Accepts glossary terms like 'vendor'.",
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
            "description": "Every rule that applies to a type, relationship, or action, sorted by severity (block > warn > info). Call this before proposing any change.",
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
            "description": "Where a type's rows live: connection, kind (csv, parquet, duckdb, postgres, s3, snowflake…), table, key column, column mapping, and whether it is readable here. Readable kinds can be read with `instances` or joined in SQL with `query`; for the rest, use your own connector.",
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
            "description": "Compact Markdown overview of the whole ontology: types by domain, blocking rules, actions, data sources, glossary. Under ~4k tokens. Load this first.",
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
            "name": "propose_change",
            "description": "Propose a change to ONE ontology YAML file and have the compiler check it: the whole ontology is validated with the change applied and the diagnostics come back (errors block, warnings do not). When the server runs with --allow-edit the file is written (and committed with --commit-edits); otherwise nothing is written and the result says so, which makes this a safe dry run. Send the complete new content of the file, not a patch. To add something, use a new path such as types/widget.yaml or actions/ship_order.yaml; rules live in a `constraints:` list file. Read the current text first with describe (its `file` field) and keep the author's formatting.",
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
}

impl<'a> Server<'a> {
    pub fn new(ont: &'a Ontology) -> Self {
        Server { ont, data: DataLayer::new(ont), edit: EditConfig::default(), reload_needed: Cell::new(false) }
    }

    /// Share a data layer (and its open DuckDB) instead of opening a fresh one.
    pub fn with_data(ont: &'a Ontology, data: DataLayer<'a>) -> Self {
        Server { ont, data, edit: EditConfig::default(), reload_needed: Cell::new(false) }
    }

    pub fn with_edit(mut self, edit: EditConfig) -> Self {
        self.edit = edit;
        self
    }

    fn text(v: impl ToString, is_error: bool) -> Json {
        let mut j = json!({ "content": [{ "type": "text", "text": v.to_string() }] });
        if is_error {
            j["isError"] = json!(true);
        }
        j
    }

    /// A `tools/call` result holding one text block.
    pub fn text_result(v: impl ToString, is_error: bool) -> Json {
        Self::text(v, is_error)
    }

    /// The `tools/call` result for `propose_change`, shared with `/api/tool`.
    /// Diagnostics as JSON plus a `note` saying what happened to the file.
    pub fn propose_change_result(out: &Outcome, edit: EditConfig) -> Json {
        if let Some(why) = &out.refused {
            return Self::text(format!("{}: {}", out.file, why), true);
        }
        let note = if out.written {
            match &out.commit {
                Some(h) => format!("written to {} and committed as {}", out.file, h),
                None if edit.commit => format!("written to {}; commit failed: {}", out.file, out.git_error.clone().unwrap_or_default()),
                None => format!("written to {}; the change is uncommitted, review it with git diff", out.file),
            }
        } else if !out.ok {
            format!("refused: the change leaves {} error(s); nothing was written", out.errors())
        } else if !edit.allow {
            "edit mode is off; the change was validated but not written".to_string()
        } else {
            "nothing was written".to_string()
        };
        let mut j = out.to_json();
        j["note"] = json!(note);
        Self::text(Self::pretty(&j), false)
    }

    fn pretty(v: &Json) -> String {
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
            "summary" => Self::text(ont.summary_markdown(), false),
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
            "propose_change" => {
                let file = Self::arg_str(args, &["file", "path"]).unwrap_or_default();
                let content = Self::arg_str(args, &["content", "text"]).unwrap_or_default();
                let reason = Self::arg_str(args, &["reason"]).unwrap_or_default();
                let before = validate::validate(ont);
                let out = edit::apply(&ont.root, self.edit, &before, &file, Change::Write(content), "MCP", None, &reason);
                if out.written {
                    self.reload_needed.set(true);
                }
                Self::propose_change_result(&out, self.edit)
            }
            other => Self::text(format!("Unknown tool '{}'", other), true),
        }
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

    pub fn handle(&self, msg: &Json) -> Option<Json> {
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
                    "serverInfo": { "name": "stella-ontology", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": format!(
                        "This server holds the business ontology '{}': {} types, {} rules, {} actions. Call `summary` first to learn the vocabulary. Before saying an action is possible, call `validate`. Use `where_is` to find where a type's rows live; if it is readable here, `instances` and `query` (read-only SQL over the ontology tables and data views) answer from the real rows, otherwise query it with your own connector. To change the ontology itself, send the full new text of one YAML file to `propose_change`; it is compiled and checked first{}.",
                        self.ont.manifest.name, self.ont.types.len(), self.ont.constraints.len(), self.ont.actions.len(),
                        if self.edit.allow { ", then written" } else { " (edit mode is off here, so it is a dry run)" }
                    )
                }))
            }
            "ping" => ok(json!({})),
            "tools/list" => ok(json!({ "tools": tool_definitions() })),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                ok(self.call_tool(name, &args))
            }
            "resources/list" => ok(json!({ "resources": self.resources() })),
            "resources/templates/list" => ok(json!({ "resourceTemplates": [
                { "uriTemplate": "ontology://types/{name}", "name": "Type", "mimeType": "text/markdown" },
                { "uriTemplate": "ontology://actions/{name}", "name": "Action", "mimeType": "application/json" }
            ] })),
            "resources/read" => {
                let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                match self.read_resource(uri) {
                    Some((mime, text)) => ok(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] })),
                    None => err(-32002, format!("resource not found: {}", uri)),
                }
            }
            "prompts/list" => ok(json!({ "prompts": [] })),
            "completion/complete" => ok(json!({ "completion": { "values": [] } })),
            _ => err(-32601, format!("method not found: {}", method)),
        }
    }
}

pub fn run_stdio(ont: Ontology, edit: EditConfig) -> Result<()> {
    let mut ont = ont;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    eprintln!(
        "stella-ontology MCP server ready on stdio ({} types, {} rules{})",
        ont.types.len(),
        ont.constraints.len(),
        if edit.allow { ", edit mode on" } else { "" }
    );
    let mut server = Server::new(&ont).with_edit(edit);
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
            if let Some(resp) = server.handle(&m) {
                writeln!(out, "{}", resp)?;
                out.flush()?;
            }
        }
        // a write happened: re-read the folder so the next call sees it
        if server.reload_needed.get() {
            drop(server);
            let root = ont.root.clone();
            match crate::load_ontology(&root) {
                Ok((o, _)) => ont = o,
                Err(e) => eprintln!("warn  reload after edit failed: {:#}", e),
            }
            server = Server::new(&ont).with_edit(edit);
        }
    }
    Ok(())
}
