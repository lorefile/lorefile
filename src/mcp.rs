//! MCP server over stdio. Hand-rolled JSON-RPC so the protocol surface is
//! visible in one file and there is no SDK churn. Speaks the 2025-06-18
//! protocol and is backwards compatible with 2024-11-05 clients.

use crate::data::DataLayer;
use crate::edit::{self, Change, EditConfig, Outcome};
use crate::ontology::Ontology;
use crate::proposals::{self, Proposal};
use crate::rules;
use crate::spaces::Registry;
use crate::validate;
use crate::validate::Report;
use anyhow::Result;
use indexmap::IndexMap;
use serde_json::{json, Value as Json};
use std::cell::Cell;
use std::io::{self, BufRead, Write};
use std::path::Path;

const PROTOCOL: &str = "2025-06-18";

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
            "inputSchema": { "type": "object", "properties": {} }
        }));
    }
    tools
}

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
            "description": "Propose a change to the ontology: send the full new text of one YAML file plus a reason. It is compiled and checked first. When the server allows edits and the change is clean it is written; otherwise it is queued for a person to review in the web page, and you get the proposal id. Use it when you learn something the ontology is missing, such as a type, a field, a link or a guardrail.",
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
    /// Share a data layer (and its open DuckDB) instead of opening a fresh one.
    pub fn with_data(ont: &'a Ontology, data: DataLayer<'a>) -> Self {
        Server { ont, data, edit: EditConfig::default(), reload_needed: Cell::new(false) }
    }

    pub fn with_edit(mut self, edit: EditConfig) -> Self {
        self.edit = edit;
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
    pub fn propose_change_result(out: &Outcome, edit: EditConfig, queued: Option<&Proposal>) -> Json {
        if let Some(why) = &out.refused {
            return Self::text(format!("{}: {}", out.file, why), true);
        }
        let note = if let Some(p) = queued {
            if out.ok {
                format!("queued for review as {}: nothing was written; a person can accept it in the web page (Agents › Waiting for your review)", p.id)
            } else {
                format!("queued as {} but it has {} error(s), so it cannot be accepted until they are fixed; nothing was written", p.id, out.errors())
            }
        } else if out.written {
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

    /// Apply a proposed change. Written changes are logged; changes that were
    /// validated but not written (edit mode off, or errors) are queued for a
    /// person to review. Returns the outcome and the queued proposal, if any.
    pub fn propose(root: &Path, edit: EditConfig, before: &Report, file: &str, content: &str, reason: &str, via: &str) -> (Outcome, Option<Proposal>) {
        let out = edit::apply(root, edit, before, file, Change::Write(content.to_string()), via, None, reason);
        let why = if reason.is_empty() { String::new() } else { format!(": {}", reason) };
        if out.written {
            proposals::log(root, via, &format!("changed {}{}", file, why), file);
            return (out, None);
        }
        if out.refused.is_some() {
            return (out, None);
        }
        match proposals::store(root, file, content, reason, via, &out) {
            Ok(p) => {
                proposals::log(root, via, &format!("proposed a change to {} (waiting for review){}", file, why), file);
                (out, Some(p))
            }
            Err(_) => (out, None),
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
                let (out, queued) = Self::propose(&ont.root, self.edit, &before, &file, &content, &reason, "agent via MCP");
                if out.written {
                    self.reload_needed.set(true);
                }
                Self::propose_change_result(&out, self.edit, queued.as_ref())
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
}

/// One stdio server, many spaces. Each request names its space (or gets the
/// implied one) and is answered by a `Server` built on that space's state.
pub struct Hub {
    pub reg: Registry,
    pub edit: EditConfig,
    /// `--space`: serve one space only, and hide the `space` argument.
    pub only: Option<String>,
}

impl Hub {
    fn edit_for(&self, id: &str) -> EditConfig {
        match self.reg.info(id) {
            Some(s) if s.builtin => EditConfig::default(),
            _ => self.edit,
        }
    }

    fn pick(&self, want: Option<&str>) -> Result<String, String> {
        match &self.only {
            Some(o) => Ok(o.clone()),
            None => self.reg.resolve(want),
        }
    }

    fn spaces_text(&mut self) -> String {
        let list = self.reg.summaries();
        let mut j = json!({ "root": self.reg.root.display().to_string(), "spaces": list });
        if let Some(d) = self.reg.default_id() {
            j["implied"] = json!(d);
        }
        Server::pretty(&j)
    }

    fn instructions(&self) -> String {
        let mine = self.reg.user_spaces();
        let shipped = self.reg.spaces.len() - mine.len();
        let scope = match (&self.only, mine.len()) {
            (Some(o), _) => format!("This server holds one ontology space, '{}'.", o),
            (None, 0) => format!("This server holds no space of the user's own yet, only {} shipped examples (ids start with @). Name one with `space`, e.g. \"@supply-chain\".", shipped),
            (None, 1) => format!("This server holds the ontology space '{}'{}. Tools default to it; pass `space` to look at a shipped example (ids start with @).", mine[0].id, if shipped > 0 { format!(" plus {} shipped examples", shipped) } else { String::new() }),
            (None, n) => format!("This server holds {} ontology spaces: {}. Every tool takes `space`; ask the user which business or brand they mean when it is not obvious, or call `spaces`.", n, mine.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", ")),
        };
        format!(
            "{} Call `summary` first to learn the vocabulary. Before saying an action is possible, call `validate`. Use `where_is` to find where a type's rows live; if it is readable here, `instances` and `query` (read-only SQL over the ontology tables and data views) answer from the real rows, otherwise query it with your own connector. To change an ontology, send the full new text of one YAML file to `propose_change`; it is compiled and checked first{}. Shipped examples cannot be changed.",
            scope,
            if self.edit.allow { ", then written" } else { ", and queued for a person to accept in the web page" }
        )
    }

    /// Resource URIs carry the space: `ontology://<space>/summary`. The flat
    /// `ontology://summary` form still works and means the implied space.
    fn split_uri(&self, uri: &str) -> Option<(String, String)> {
        let rest = uri.strip_prefix("ontology://")?;
        let (first, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if self.reg.spaces.contains_key(first) {
            return Some((first.to_string(), format!("ontology://{}", tail)));
        }
        let id = self.pick(None).ok()?;
        Some((id, uri.to_string()))
    }

    fn with_space<T>(&mut self, id: &str, f: impl FnOnce(&Server<'_>) -> T) -> Result<T, String> {
        let edit = self.edit_for(id);
        let l = self.reg.get(id).map_err(|e| format!("{:#}", e))?;
        let srv = Server::with_data(&l.ont, l.data()).with_edit(edit);
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
                    "serverInfo": { "name": "stella-ontology", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": self.instructions()
                }))
            }
            "ping" => ok(json!({})),
            "tools/list" => ok(json!({ "tools": tool_definitions_for(&self.reg, self.only.as_deref()) })),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let mut args = params.get("arguments").cloned().unwrap_or(json!({}));
                if name == "spaces" {
                    return ok(Server::text(self.spaces_text(), false));
                }
                let want = args.get("space").and_then(|v| v.as_str()).map(|s| s.to_string());
                if let Some(o) = args.as_object_mut() {
                    o.remove("space");
                }
                let space = match self.pick(want.as_deref()) {
                    Ok(s) => s,
                    Err(e) => return ok(Server::text(e, true)),
                };
                if name == "propose_change" && self.reg.info(&space).map(|s| s.builtin).unwrap_or(false) {
                    return ok(Server::text(format!("{} is a shipped example and stays read-only. Create a space of your own (or copy the example) and propose the change there.", space), true));
                }
                match self.with_space(&space, |srv| srv.call_tool(name, &args)) {
                    Ok(r) => ok(r),
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

pub fn run_stdio(reg: Registry, edit: EditConfig, only: Option<String>) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let only = match only {
        Some(o) => Some(reg.resolve(Some(&o)).map_err(|e| anyhow::anyhow!(e))?),
        None => None,
    };
    let mine = reg.user_spaces().len();
    eprintln!(
        "stella-ontology MCP server ready on stdio ({} space{} of yours, {} shipped{}{})",
        mine,
        if mine == 1 { "" } else { "s" },
        reg.spaces.len() - mine,
        only.as_ref().map(|o| format!(", serving only {}", o)).unwrap_or_default(),
        if edit.allow { ", edit mode on" } else { "" }
    );
    let mut hub = Hub { reg, edit, only };
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
