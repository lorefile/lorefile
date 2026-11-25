//! Local web UI and JSON API. One embedded HTML page, a handful of endpoints,
//! all backed by the same functions the MCP server uses. With `--allow-edit`
//! the page can also write YAML files, through `edit.rs`.
//!
//! Every space is served under `/s/<id>/` (page and API alike); a bare `/api/...`
//! goes to the default space so single-folder set-ups keep working.

use crate::edit::{self, Change, EditConfig, Outcome};
use crate::mcp::Server as McpServer;
use crate::patch;
use crate::proposals;
use crate::spaces::{Loaded, Registry, Space};
use anyhow::Result;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const INDEX_HTML: &str = include_str!("ui/index.html");

type State = Loaded;

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

fn overview(st: &State, space: &Space, root: &str, single: bool, edit: EditConfig) -> Json {
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
    let types: Vec<Json> = ont
        .types
        .values()
        .map(|t| {
            let fields = t.properties.len();
            let described = t.properties.values().filter(|p| !p.normalize().description.trim().is_empty()).count();
            json!({
                "name": t.name, "domain": ont.domain_of(&t.name), "description": t.description, "file": t.file, "key": t.key_property(),
                "fields": fields, "described": described, "has_source": t.source.is_some() || !t.instances.is_empty(),
                "source_kind": t.source.as_ref().and_then(|s| ont.sources.get(&s.connection)).map(|s| s.kind.clone()),
                "source_connection": t.source.as_ref().map(|s| s.connection.clone()),
                "source_table": t.source.as_ref().map(|s| s.table.clone()),
                "owner": t.owner, "tags": t.tags,
            })
        })
        .collect();
    let pending = proposals::list(&st.dir).into_iter().filter(|p| p.status == "pending").count();
    json!({
        "manifest": ont.manifest,
        "stats": ont.stats(),
        "types": types,
        "proposals_pending": pending,
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
        "root": root,
        "single": single,
        "space": { "id": space.id, "name": space.name, "builtin": space.builtin, "group": space.group },
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
fn mutate(st: &mut State, builtin: bool, edit: EditConfig, method: &Method, path: &str, params: &HashMap<String, String>, body: &Json, as_tool: bool) -> Response<std::io::Cursor<Vec<u8>>> {
    if builtin {
        return json_response_private(403, &json!({ "error": "this is a shipped example and stays read-only; copy it into a space of your own to change it", "editable": false, "builtin": true }));
    }
    // an agent's proposal is validated and queued even when edit mode is off
    if as_tool {
        let args = body.get("arguments").cloned().unwrap_or(json!({}));
        let (out, queued) = McpServer::propose(&st.dir, edit, &st.report, str_field(&args, "file"), str_field(&args, "content"), str_field(&args, "reason"), "agent via API");
        if out.written {
            st.reload();
        }
        return json_response_private(200, &McpServer::propose_change_result(&out, edit, queued.as_ref()));
    }
    if path == "/api/proposals/dismiss" {
        return match proposals::set_status(&st.dir, str_field(body, "id"), "dismissed") {
            Ok(p) => {
                proposals::log(&st.dir, "person via UI", &format!("set aside the proposed change to {}", p.file), &p.file);
                json_response_private(200, &json!({ "ok": true, "proposal": p }))
            }
            Err(e) => json_response_private(404, &json!({ "error": e })),
        };
    }
    if !edit.allow {
        return json_response_private(403, &json!({ "error": "edit mode is off; start the server with --allow-edit", "editable": false }));
    }
    let mut extra = json!({});
    let outcome: Outcome = if *method == Method::Delete {
        let file = params.get("file").cloned().unwrap_or_else(|| str_field(body, "file").to_string());
        edit::apply(&st.dir, edit, &st.report, &file, Change::Delete, "UI", None, "")
    } else if path == "/api/create" {
        let (kind, name, content) = (str_field(body, "kind"), str_field(body, "name"), str_field(body, "content"));
        let file = body.get("file").and_then(|f| f.as_str()).filter(|f| !f.trim().is_empty());
        match edit::plan_create(&st.dir, kind, name, content, file) {
            Ok((rel, text)) => edit::apply(&st.dir, edit, &st.report, &rel, Change::Write(text), "UI", Some((kind, name)), ""),
            Err(e) => Outcome { file: file.unwrap_or("").to_string(), refused: Some(e), ..Default::default() },
        }
    } else if path == "/api/patch" {
        match patch::plan(&st.ont, body) {
            Ok(p) => {
                extra["summary"] = json!(p.summary);
                edit::apply(&st.dir, edit, &st.report, &p.file, Change::Write(p.text), "UI", Some((&p.kind, &p.name)), &p.summary)
            }
            Err(e) => Outcome { file: str_field(body, "type").to_string(), refused: Some(e), ..Default::default() },
        }
    } else if path == "/api/proposals/accept" {
        match proposals::get(&st.dir, str_field(body, "id")) {
            Some(p) => {
                let out = edit::apply(&st.dir, edit, &st.report, &p.file, Change::Write(p.content.clone()), "UI (accepting an agent's proposal)", Some((&p.kind, &p.name)), &p.reason);
                if out.written {
                    let _ = proposals::set_status(&st.dir, &p.id, "accepted");
                    proposals::log(&st.dir, "person via UI", &format!("accepted the change to {} proposed by an agent", p.file), &p.file);
                }
                extra["proposal_id"] = json!(p.id);
                out
            }
            None => Outcome { refused: Some("no such proposal".into()), ..Default::default() },
        }
    } else {
        edit::apply(&st.dir, edit, &st.report, str_field(body, "file"), Change::Write(str_field(body, "content").to_string()), "UI", None, str_field(body, "reason"))
    };
    if outcome.written {
        if path == "/api/patch" {
            proposals::log(&st.dir, "person via UI", extra["summary"].as_str().unwrap_or("edited"), &outcome.file);
        } else if path == "/api/edit" || path == "/api/create" {
            proposals::log(&st.dir, "person via UI", &format!("edited {}", outcome.file), &outcome.file);
        } else if *method == Method::Delete {
            proposals::log(&st.dir, "person via UI", &format!("deleted {}", outcome.file), &outcome.file);
        }
        st.reload();
    }
    let mut j = outcome.to_json();
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            j[k] = v.clone();
        }
    }
    if outcome.written {
        j["git"] = edit::git_status(&st.dir).unwrap_or(Json::Null);
        j["stats"] = json!(st.ont.stats());
    }
    json_response_private(outcome.status(), &j)
}

/// `/s/<id>/rest` → (Some(id), "/rest"); anything else is left alone.
fn split_space(path: &str) -> (Option<String>, String) {
    if let Some(rest) = path.strip_prefix("/s/") {
        let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if !id.is_empty() {
            return (Some(percent_decode(id)), format!("/{}", tail));
        }
    }
    (None, path.to_string())
}

fn spaces_json(reg: &mut Registry, edit: EditConfig) -> Json {
    json!({
        "spaces": reg.summaries(),
        "default": reg.default_id(),
        "root": reg.root.display().to_string(),
        "single": reg.single,
        "editable": edit.allow,
        "can_create": edit.allow && !reg.single,
    })
}

pub fn run(mut reg: Registry, host: &str, port: u16, edit: EditConfig) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let server = Server::http(&addr).map_err(|e| anyhow::anyhow!("cannot bind {}: {}", addr, e))?;
    let mine = reg.user_spaces().len();
    let shipped = reg.spaces.len() - mine;
    eprintln!("UI      → http://{}", addr);
    eprintln!("API     → http://{}/api/spaces", addr);
    if reg.single {
        eprintln!("Space   → {} (one ontology folder){}", reg.root.display(), if shipped > 0 { format!(", plus {} shipped examples to look at", shipped) } else { String::new() });
    } else {
        eprintln!("Spaces  → {} of yours in {}{}", mine, reg.root.display(), if shipped > 0 { format!(", plus {} shipped examples", shipped) } else { String::new() });
    }
    eprintln!("MCP     → stella-ontology mcp --dir {}", reg.root.display());
    if edit.allow {
        eprintln!("Edit    → on: the page writes YAML into the spaces{}", if edit.commit { " and commits each change" } else { " (changes stay uncommitted)" });
    } else {
        eprintln!("Edit    → off (start with --allow-edit to edit from the page)");
    }
    let root_s = reg.root.display().to_string();
    let single = reg.single;

    for mut req in server.incoming_requests() {
        let (full_path, params) = query_params(req.url());
        let (prefix_space, path) = split_space(&full_path);
        let method = req.method().clone();
        let has_body = method == Method::Post || method == Method::Delete || method == Method::Put;
        let mut body_text = String::new();
        if has_body {
            let _ = req.as_reader().read_to_string(&mut body_text);
        }
        let body: Json = if has_body { serde_json::from_str(&body_text).unwrap_or(json!({})) } else { json!({}) };
        let json_body = header_value(&req, "Content-Type").map(|c| c.to_ascii_lowercase().contains("application/json")).unwrap_or(false);

        // the page itself, for any space
        if method == Method::Get && (path == "/" || path == "/index.html" || path.is_empty()) {
            let _ = req.respond(text_response(200, "text/html; charset=utf-8", INDEX_HTML.to_string()));
            continue;
        }

        // the list of spaces, and creating one
        if path == "/api/spaces" {
            let resp = match method {
                Method::Get => json_response(200, &spaces_json(&mut reg, edit)),
                Method::Post if !same_origin(&req, &addr) => json_response_private(403, &json!({ "error": "cross-origin writes are refused" })),
                Method::Post if !json_body => json_response_private(415, &json!({ "error": "send the body as application/json" })),
                Method::Post if !edit.allow => json_response_private(403, &json!({ "error": "edit mode is off; start the server with --allow-edit to create spaces", "editable": false })),
                Method::Post => match reg.create(str_field(&body, "name"), str_field(&body, "from")) {
                    Ok(space) => {
                        proposals::log(&space.dir, "person via UI", &format!("created the space {}{}", space.id, if str_field(&body, "from").is_empty() { String::new() } else { format!(" from {}", str_field(&body, "from")) }), "ontology.yaml");
                        json_response_private(200, &json!({ "ok": true, "written": true, "space": space, "summary": format!("Created the space {}", space.id), "spaces": spaces_json(&mut reg, edit) }))
                    }
                    Err(e) => json_response_private(400, &json!({ "error": e })),
                },
                _ => json_response(405, &json!({ "error": "method not allowed" })),
            };
            let _ = req.respond(resp);
            continue;
        }

        // which space does this request mean?
        let want = prefix_space.clone().or_else(|| params.get("space").cloned()).or_else(|| reg.default_id());
        let Some(space_id) = want else {
            let _ = req.respond(json_response(404, &json!({ "error": "no space yet: create one in the page (or start the server on an ontology folder)", "no_spaces": true })));
            continue;
        };
        let Some(space) = reg.info(&space_id).cloned() else {
            let _ = req.respond(json_response(404, &json!({ "error": format!("no space named '{}'", space_id), "spaces": reg.spaces.keys().collect::<Vec<_>>() })));
            continue;
        };
        // shipped examples never take writes, whatever the flags say
        let space_edit = if space.builtin { EditConfig::default() } else { edit };

        if method == Method::Post && path == "/api/reload" {
            reg.rescan();
            reg.touched(&space_id);
            let resp = match reg.get(&space_id) {
                Ok(st) => {
                    st.reload();
                    json_response(200, &json!({ "ok": st.report.ok(), "stats": st.ont.stats(), "diagnostics": st.report.diags }))
                }
                Err(e) => json_response(500, &json!({ "error": e.to_string() })),
            };
            let _ = req.respond(resp);
            continue;
        }

        let st = match reg.get(&space_id) {
            Ok(st) => st,
            Err(e) => {
                let _ = req.respond(json_response(500, &json!({ "error": format!("{} could not be loaded: {:#}", space_id, e), "space": space_id })));
                continue;
            }
        };

        // writes: edit, create, delete, and propose_change through /api/tool
        let as_tool = method == Method::Post && path == "/api/tool" && str_field(&body, "name") == "propose_change";
        let is_write = as_tool
            || (method == Method::Post && (path == "/api/edit" || path == "/api/create" || path == "/api/patch" || path == "/api/proposals/accept" || path == "/api/proposals/dismiss"))
            || (method == Method::Delete && path == "/api/file");
        if is_write {
            let resp = if !same_origin(&req, &addr) {
                json_response_private(403, &json!({ "error": "cross-origin writes are refused" }))
            } else if method == Method::Post && !json_body {
                json_response_private(415, &json!({ "error": "send the body as application/json" }))
            } else {
                mutate(st, space.builtin, space_edit, &method, &path, &params, &body, as_tool)
            };
            reg.touched(&space_id);
            let _ = req.respond(resp);
            continue;
        }

        let ont = &st.ont;
        let mcp = McpServer::with_data(ont, st.data());
        let resp = match (method.clone(), path.as_str()) {
            (Method::Get, "/api/summary") => json_response(200, &overview(st, &space, &root_s, single, space_edit)),
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
            (Method::Get, "/api/proposals") => json_response(200, &json!({ "proposals": proposals::list(&st.dir), "activity": proposals::activity(&st.dir, 50), "editable": space_edit.allow })),
            (Method::Get, "/api/activity") => json_response(200, &json!({ "activity": proposals::activity(&st.dir, 100) })),
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

#[cfg(test)]
mod tests {
    use super::split_space;

    #[test]
    fn the_space_prefix_is_split_off_the_path() {
        assert_eq!(split_space("/s/acme/api/summary"), (Some("acme".into()), "/api/summary".into()));
        assert_eq!(split_space("/s/%40shopify/"), (Some("@shopify".into()), "/".into()));
        assert_eq!(split_space("/s/acme"), (Some("acme".into()), "/".into()));
        assert_eq!(split_space("/api/summary"), (None, "/api/summary".into()));
        assert_eq!(split_space("/"), (None, "/".into()));
    }
}
