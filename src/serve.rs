//! Local web UI and JSON API. One embedded HTML page, a handful of endpoints,
//! all backed by the same functions the MCP server uses. With `--allow-edit`
//! the page can also write YAML files, through `edit.rs`.

use crate::data::DataLayer;
#[cfg(feature = "duckdb")]
use crate::duck::Db;
use crate::edit::{self, Change, EditConfig, Outcome};
use crate::load_ontology;
use crate::mcp::Server as McpServer;
use crate::ontology::Ontology;
use crate::validate::Report;
use anyhow::Result;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(feature = "duckdb")]
use std::sync::Arc;
use std::sync::RwLock;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const INDEX_HTML: &str = include_str!("ui/index.html");

struct State {
    dir: PathBuf,
    ont: Ontology,
    report: Report,
    /// One DuckDB for the whole server, rebuilt on reload. None when it could not open.
    #[cfg(feature = "duckdb")]
    db: Option<Arc<Db>>,
}

impl State {
    fn new(dir: PathBuf, ont: Ontology, report: Report) -> State {
        #[cfg(feature = "duckdb")]
        let db = match Db::open(&ont) {
            Ok(db) => Some(Arc::new(db)),
            Err(e) => {
                eprintln!("warn  DuckDB could not open: {:#}", e);
                None
            }
        };
        State {
            dir,
            ont,
            report,
            #[cfg(feature = "duckdb")]
            db,
        }
    }

    fn data(&self) -> DataLayer<'_> {
        #[cfg(feature = "duckdb")]
        if let Some(db) = &self.db {
            return DataLayer::with_db(&self.ont, db.clone());
        }
        DataLayer::new(&self.ont)
    }

    /// Re-read the folder after a write. Keeps the old state if the reload fails.
    fn reload(&mut self) {
        match load_ontology(&self.dir) {
            Ok((o, r)) => *self = State::new(self.dir.clone(), o, r),
            Err(e) => eprintln!("warn  reload failed: {:#}", e),
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = &s[i + 1..i + 3];
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn query_params(url: &str) -> (String, HashMap<String, String>) {
    let (path, qs) = url.split_once('?').unwrap_or((url, ""));
    let mut map = HashMap::new();
    for pair in qs.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(k), percent_decode(v));
    }
    (path.to_string(), map)
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn json_response(status: u16, body: &Json) -> Response<std::io::Cursor<Vec<u8>>> {
    let text = serde_json::to_string(body).unwrap_or_default();
    Response::from_string(text)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Access-Control-Allow-Origin", "*"))
        .with_header(header("Cache-Control", "no-store"))
}

/// Like `json_response` but without the open CORS header: replies to writes
/// are for the page itself, never for another site.
fn json_response_private(status: u16, body: &Json) -> Response<std::io::Cursor<Vec<u8>>> {
    let text = serde_json::to_string(body).unwrap_or_default();
    Response::from_string(text)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"))
}

fn text_response(status: u16, mime: &str, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", mime))
        .with_header(header("Cache-Control", "no-store"))
}

fn header_value(req: &Request, name: &'static str) -> Option<String> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str().to_string())
}

fn str_field<'a>(body: &'a Json, key: &str) -> &'a str {
    body.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn overview(st: &State, edit: EditConfig) -> Json {
    let ont = &st.ont;
    let mut domains: Vec<Json> = Vec::new();
    let mut placed: Vec<String> = Vec::new();
    for d in &ont.manifest.domains {
        let types: Vec<Json> = ont
            .types
            .values()
            .filter(|t| ont.domain_of(&t.name).as_deref() == Some(d.name.as_str()))
            .map(|t| json!({ "name": t.name, "description": t.description }))
            .collect();
        for t in &types {
            placed.push(t["name"].as_str().unwrap_or("").to_string());
        }
        domains.push(json!({ "name": d.name, "description": d.description, "types": types }));
    }
    let rest: Vec<Json> = ont
        .types
        .values()
        .filter(|t| !placed.contains(&t.name))
        .map(|t| json!({ "name": t.name, "description": t.description }))
        .collect();
    if !rest.is_empty() {
        domains.push(json!({ "name": if domains.is_empty() { "Types" } else { "Other" }, "description": "", "types": rest }));
    }
    let mut constraint_files: Vec<&str> = ont.constraints.values().map(|c| c.file.as_str()).collect();
    constraint_files.sort();
    constraint_files.dedup();
    json!({
        "manifest": ont.manifest,
        "stats": ont.stats(),
        "domains": domains,
        "relationships": ont.relationships,
        "constraints": ont.constraints.values().collect::<Vec<_>>(),
        "constraint_files": constraint_files,
        "actions": ont.actions.values().collect::<Vec<_>>(),
        "sources": ont.sources.values().collect::<Vec<_>>(),
        "glossary": ont.glossary,
        "diagnostics": st.report.diags,
        "ok": st.report.ok(),
        "dir": st.dir.display().to_string(),
        "editable": edit.allow,
        "commit_edits": edit.commit,
        "git": edit::git_status(&st.dir),
    })
}

/// Is this write request coming from our own page? A browser sends `Origin`
/// on every cross-site POST, so a request from another site is refused even
/// though the server only listens on 127.0.0.1.
fn same_origin(req: &Request, addr: &str) -> bool {
    let Some(origin) = header_value(req, "Origin") else { return true };
    let host = origin.trim_start_matches("http://").trim_start_matches("https://");
    let port = addr.rsplit(':').next().unwrap_or("");
    host == addr || host == format!("localhost:{}", port) || host == format!("[::1]:{}", port)
}

/// The write endpoints. Takes the write lock, applies the change, reloads.
fn mutate(state: &RwLock<State>, edit: EditConfig, method: &Method, path: &str, params: &HashMap<String, String>, body: &Json, as_tool: bool) -> Response<std::io::Cursor<Vec<u8>>> {
    if !edit.allow {
        let msg = "edit mode is off; start the server with --allow-edit";
        return if as_tool {
            json_response_private(200, &McpServer::text_result(msg, true))
        } else {
            json_response_private(403, &json!({ "error": msg, "editable": false }))
        };
    }
    let mut st = state.write().unwrap();
    let outcome: Outcome = if as_tool {
        let args = body.get("arguments").cloned().unwrap_or(json!({}));
        edit::apply(&st.dir, edit, &st.report, str_field(&args, "file"), Change::Write(str_field(&args, "content").to_string()), "API", None, str_field(&args, "reason"))
    } else if *method == Method::Delete {
        let file = params.get("file").cloned().unwrap_or_else(|| str_field(body, "file").to_string());
        edit::apply(&st.dir, edit, &st.report, &file, Change::Delete, "UI", None, "")
    } else if path == "/api/create" {
        let (kind, name, content) = (str_field(body, "kind"), str_field(body, "name"), str_field(body, "content"));
        let file = body.get("file").and_then(|f| f.as_str()).filter(|f| !f.trim().is_empty());
        match edit::plan_create(&st.dir, kind, name, content, file) {
            Ok((rel, text)) => edit::apply(&st.dir, edit, &st.report, &rel, Change::Write(text), "UI", Some((kind, name)), ""),
            Err(e) => Outcome { file: file.unwrap_or("").to_string(), refused: Some(e), ..Default::default() },
        }
    } else {
        edit::apply(&st.dir, edit, &st.report, str_field(body, "file"), Change::Write(str_field(body, "content").to_string()), "UI", None, str_field(body, "reason"))
    };
    if outcome.written {
        st.reload();
    }
    if as_tool {
        return json_response_private(200, &McpServer::propose_change_result(&outcome, edit));
    }
    let mut j = outcome.to_json();
    if outcome.written {
        j["git"] = edit::git_status(&st.dir).unwrap_or(Json::Null);
        j["stats"] = json!(st.ont.stats());
    }
    json_response_private(outcome.status(), &j)
}

pub fn run(dir: PathBuf, port: u16, edit: EditConfig) -> Result<()> {
    let (ont, report) = load_ontology(&dir)?;
    let state = RwLock::new(State::new(dir.clone(), ont, report));
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(&addr).map_err(|e| anyhow::anyhow!("cannot bind {}: {}", addr, e))?;
    eprintln!("UI      → http://{}", addr);
    eprintln!("API     → http://{}/api/summary", addr);
    eprintln!("MCP     → stella-ontology mcp --dir {}", dir.display());
    if edit.allow {
        eprintln!("Edit    → on: the page writes YAML into {}{}", dir.display(), if edit.commit { " and commits each change" } else { " (changes stay uncommitted)" });
    } else {
        eprintln!("Edit    → off (start with --allow-edit to edit from the page)");
    }

    for mut req in server.incoming_requests() {
        let (path, params) = query_params(req.url());
        let method = req.method().clone();
        let has_body = method == Method::Post || method == Method::Delete || method == Method::Put;
        let mut body_text = String::new();
        if has_body {
            let _ = req.as_reader().read_to_string(&mut body_text);
        }
        let body: Json = if has_body { serde_json::from_str(&body_text).unwrap_or(json!({})) } else { json!({}) };

        if method == Method::Post && path == "/api/reload" {
            let mut st = state.write().unwrap();
            match load_ontology(&st.dir) {
                Ok((o, r)) => {
                    *st = State::new(st.dir.clone(), o, r);
                    let _ = req.respond(json_response(200, &json!({ "ok": st.report.ok(), "stats": st.ont.stats(), "diagnostics": st.report.diags })));
                }
                Err(e) => {
                    let _ = req.respond(json_response(500, &json!({ "error": e.to_string() })));
                }
            }
            continue;
        }

        // writes: edit, create, delete, and propose_change through /api/tool
        let as_tool = method == Method::Post && path == "/api/tool" && str_field(&body, "name") == "propose_change";
        let is_write = as_tool || (method == Method::Post && (path == "/api/edit" || path == "/api/create")) || (method == Method::Delete && path == "/api/file");
        if is_write {
            let json_body = header_value(&req, "Content-Type").map(|c| c.to_ascii_lowercase().contains("application/json")).unwrap_or(false);
            let resp = if !same_origin(&req, &addr) {
                json_response_private(403, &json!({ "error": "cross-origin writes are refused" }))
            } else if method == Method::Post && !json_body {
                json_response_private(415, &json!({ "error": "send the body as application/json" }))
            } else {
                mutate(&state, edit, &method, &path, &params, &body, as_tool)
            };
            let _ = req.respond(resp);
            continue;
        }

        let st = state.read().unwrap();
        let ont = &st.ont;
        let mcp = McpServer::with_data(ont, st.data());
        let resp = match (method.clone(), path.as_str()) {
            (Method::Get, "/") | (Method::Get, "/index.html") => text_response(200, "text/html; charset=utf-8", INDEX_HTML.to_string()),
            (Method::Get, "/api/summary") => json_response(200, &overview(&st, edit)),
            (Method::Get, "/api/summary.md") => text_response(200, "text/markdown; charset=utf-8", ont.summary_markdown()),
            (Method::Get, "/api/describe") => {
                let name = params.get("name").cloned().unwrap_or_default();
                match ont.describe(&name) {
                    Some(j) => json_response(200, &j),
                    None => json_response(404, &json!({ "error": format!("nothing named {}", name) })),
                }
            }
            (Method::Get, "/api/search") => {
                let q = params.get("q").cloned().unwrap_or_default();
                let limit = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(20);
                json_response(200, &json!(ont.search(&q, limit)))
            }
            (Method::Get, "/api/neighbors") => {
                let name = params.get("name").cloned().unwrap_or_default();
                let depth = params.get("depth").and_then(|d| d.parse().ok()).unwrap_or(1);
                match ont.neighbors(&name, depth) {
                    Some(sg) => json_response(200, &json!(sg)),
                    None => json_response(404, &json!({ "error": "unknown type" })),
                }
            }
            (Method::Get, "/api/rules") => {
                let name = params.get("name").cloned().unwrap_or_default();
                json_response(200, &json!(ont.constraints_for(&name)))
            }
            (Method::Get, "/api/where") => {
                let name = params.get("name").cloned().unwrap_or_default();
                let data = st.data();
                match data.where_is(&name) {
                    Ok(j) => json_response(200, &j),
                    Err(e) => json_response(404, &json!({ "error": e.to_string() })),
                }
            }
            (Method::Get, "/api/instances") => {
                let name = params.get("name").cloned().unwrap_or_default();
                let limit: usize = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50);
                let data = st.data();
                match data.instances(&name) {
                    Ok(rows) => {
                        let total = rows.len();
                        let shown: Vec<_> = rows.into_iter().take(limit).collect();
                        json_response(200, &json!({ "type": name, "total": total, "rows": shown }))
                    }
                    Err(e) => json_response(404, &json!({ "error": e.to_string() })),
                }
            }
            (Method::Get, "/api/yaml") => {
                let file = params.get("file").cloned().unwrap_or_default();
                let safe = !file.contains("..") && (file.ends_with(".yaml") || file.ends_with(".yml")) && ont.files.contains(&file);
                if safe {
                    match std::fs::read_to_string(ont.root.join(&file)) {
                        Ok(s) => text_response(200, "text/yaml; charset=utf-8", s),
                        Err(e) => json_response(404, &json!({ "error": e.to_string() })),
                    }
                } else {
                    json_response(400, &json!({ "error": "unknown file" }))
                }
            }
            (Method::Get, "/api/templates") => json_response(200, &edit::templates()),
            (Method::Get, "/api/tools") => json_response(200, &crate::mcp::tool_definitions()),
            (Method::Get, "/api/tables") => match st.data().sql_catalog() {
                Ok(j) => json_response(200, &j),
                Err(e) => json_response(500, &json!({ "error": e.to_string() })),
            },
            (Method::Get, "/api/query") | (Method::Post, "/api/query") => {
                let (sql, limit) = if method == Method::Post {
                    (
                        body.get("sql").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                        body.get("limit").and_then(|l| l.as_u64()).unwrap_or(crate::data::MAX_QUERY_ROWS as u64) as usize,
                    )
                } else {
                    (params.get("sql").cloned().unwrap_or_default(), params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(crate::data::MAX_QUERY_ROWS))
                };
                match st.data().query(&sql, limit) {
                    Ok(res) => json_response(200, &json!(res)),
                    Err(e) => json_response(400, &json!({ "error": e.to_string() })),
                }
            }
            (Method::Post, "/api/check") => json_response(200, &mcp.call_tool("validate", &body)),
            (Method::Post, "/api/tool") => {
                let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = body.get("arguments").cloned().unwrap_or(json!({}));
                json_response(200, &mcp.call_tool(name, &args))
            }
            _ => json_response(404, &json!({ "error": "not found" })),
        };
        let _ = req.respond(resp);
    }
    Ok(())
}
