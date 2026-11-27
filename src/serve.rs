//! Local web UI and JSON API. One embedded HTML page, a handful of endpoints,
//! all backed by the same functions the MCP server uses. With `--allow-edit`
//! the page can also write YAML files, through `edit.rs`.
//!
//! Every space is served under `/s/<id>/` (page and API alike); a bare `/api/...`
//! goes to the default space so single-folder set-ups keep working.

//!
//! Every route but the login itself needs the server's token: the page keeps it
//! as a cookie, API and MCP clients send `Authorization: Bearer <token>`.
//! `POST /mcp` speaks MCP's Streamable HTTP transport through the same `Hub`
//! as `lorefile mcp` does over stdio.

use crate::edit::{self, Change, EditConfig, Note, Outcome, PAGE};
use crate::mcp::{Hub, Server as McpServer};
use crate::oauth;
use crate::patch;
use crate::proposals;
use crate::review;
use crate::spaces::{Loaded, Registry, Space};
use anyhow::Result;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const INDEX_HTML: &str = include_str!("ui/index.html");

type State = Loaded;

// ---------------------------------------------------------------------------
// Login: one token, the way Jupyter and code-server do it
// ---------------------------------------------------------------------------

pub const TOKEN_ENV: &str = "LOREFILE_TOKEN";

/// Where the server's token came from, for the start-up lines.
pub enum TokenFrom {
    Flag,
    Env,
    File(PathBuf),
    /// Made on this start and saved to the file.
    New(PathBuf),
    /// Made on this start, but the file could not be written: it lasts until the server stops.
    Unsaved(PathBuf, String),
}

/// The login the server runs with. `token: None` is `--no-auth`.
pub struct Login {
    pub token: Option<String>,
    pub from: Option<TokenFrom>,
}

/// `<root>/.lorefile/auth.toml`, where a generated token is kept between starts.
pub fn auth_file(root: &Path) -> PathBuf {
    root.join(".lorefile").join("auth.toml")
}

/// What a token may hold: the characters of base64, base64url, hex and UUIDs,
/// all safe in a cookie and a header.
fn token_ok(t: &str) -> Result<(), String> {
    if t.is_empty() {
        return Err("the token is empty".into());
    }
    if let Some(c) = t.chars().find(|c| !(c.is_ascii_alphanumeric() || "-._~+/=".contains(*c))) {
        return Err(format!("the token holds `{}`; use letters, digits and - . _ ~ + / = only", c));
    }
    Ok(())
}

fn read_auth_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("token")?.trim_start().strip_prefix('=')?;
        let v = rest.trim().trim_matches('"').trim_matches('\'').trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// Save the token where only this user can read it (mode 0600 on Unix).
fn write_auth_file(root: &Path, token: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let path = crate::spaces::lorefile_dir(root).join("auth.toml");
    let body = format!(
        "# Lorefile's login token for this folder. Anyone who has it can open the page and call\n# the API and MCP (and write, when the server runs with --allow-edit).\n# Delete this file and restart to get a new one; LOREFILE_TOKEN or --token take precedence.\ntoken = \"{}\"\n",
        token
    );
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path)?;
    f.write_all(body.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

/// Unpadded base64url, the alphabet that is safe in URLs, headers and cookies.
pub(crate) fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..=chunk.len() {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// 32 random bytes from the operating system, as 43 characters of base64url.
pub fn new_token() -> Result<String> {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).map_err(|e| anyhow::anyhow!("the operating system gave no random bytes for the login token: {}", e))?;
    Ok(base64url(&b))
}

/// The token, in this order: `--token`, `LOREFILE_TOKEN`, `<root>/.lorefile/auth.toml`,
/// else a new one saved there. `--no-auth` turns the login off.
pub fn resolve_login(root: &Path, flag: Option<String>, no_auth: bool) -> Result<Login> {
    if no_auth {
        return Ok(Login { token: None, from: None });
    }
    let checked = |t: &str, what: &str, from: TokenFrom| -> Result<Login> {
        token_ok(t).map_err(|e| anyhow::anyhow!("{}: {}", what, e))?;
        Ok(Login { token: Some(t.to_string()), from: Some(from) })
    };
    if let Some(t) = flag {
        return checked(t.trim(), "--token", TokenFrom::Flag);
    }
    if let Ok(t) = std::env::var(TOKEN_ENV) {
        if !t.trim().is_empty() {
            return checked(t.trim(), TOKEN_ENV, TokenFrom::Env);
        }
    }
    let path = auth_file(root);
    if let Some(t) = read_auth_file(&path) {
        let what = format!("{} (fix it, or delete it to get a new token)", path.display());
        return checked(&t, &what, TokenFrom::File(path));
    }
    let t = new_token()?;
    let from = match write_auth_file(root, &t) {
        Ok(p) => TokenFrom::New(p),
        Err(e) => TokenFrom::Unsaved(path, e.to_string()),
    };
    Ok(Login { token: Some(t), from: Some(from) })
}

/// 127.0.0.1, ::1 and localhost: addresses only this machine can reach.
pub fn is_loopback(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// Equal tokens, compared in time that does not depend on where they differ.
pub(crate) fn same_token(given: &str, want: &str) -> bool {
    let (a, b) = (given.as_bytes(), want.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

fn bearer(req: &Request) -> Option<String> {
    let v = header_value(req, "Authorization")?;
    let (scheme, rest) = v.trim().split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim().to_string())
}

fn cookie_values(req: &Request, name: &str) -> Vec<String> {
    req.headers()
        .iter()
        .filter(|h| h.field.equiv("Cookie"))
        .flat_map(|h| h.value.as_str().split(';').map(|p| p.trim().to_string()).collect::<Vec<_>>())
        .filter_map(|p| p.split_once('=').filter(|(k, _)| k.trim() == name).map(|(_, v)| v.trim().trim_matches('"').to_string()))
        .collect()
}

/// Whether the request carries the token: a bearer header, or (when `cookie_ok`) the page's cookie.
fn signed_in(req: &Request, ctx: &Ctx, cookie_ok: bool) -> bool {
    let Some(want) = &ctx.token else { return true };
    if bearer(req).map(|t| same_token(&t, want)).unwrap_or(false) {
        return true;
    }
    cookie_ok && cookie_values(req, &ctx.cookie).iter().any(|t| same_token(t, want))
}

/// The page's cookie, or (with `None`) the header that clears it. One name per
/// port, because browsers share cookies across the ports of one host.
fn cookie_header(req: &Request, ctx: &Ctx, value: Option<&str>) -> Header {
    // X-Forwarded-Proto, Forwarded or CF-Visitor: the request came in over HTTPS
    let secure = oauth::asked(&|name| header_value(req, name)).https;
    let (v, age) = match value {
        Some(t) => (t, 400 * 24 * 3600),
        None => ("", 0),
    };
    header("Set-Cookie", &format!("{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}", ctx.cookie, v, age, if secure { "; Secure" } else { "" }))
}

/// A path on this server to land on after signing in; anything else lands on `/`.
fn local_path(next: &str) -> String {
    let ok = next.starts_with('/') && !next.starts_with("//") && next.chars().all(|c| c.is_ascii_graphic() && c != '\\');
    if ok { next.to_string() } else { "/".to_string() }
}

fn redirect(to: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(String::new())
        .with_status_code(StatusCode(303))
        .with_header(header("Location", &local_path(to)))
        .with_header(header("Cache-Control", "no-store"))
}

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

const SIGN_IN_FIRST: &str = "sign in first: open the link the server printed (it ends in ?token=…), or send Authorization: Bearer <token>";
pub(crate) const WRONG_TOKEN: &str = "That is not this server's token. Check for a missing character, or for a newer token if the server was started again with another one.";

/// The login screen: one box, plain and light like the page.
fn login_page(next: &str, error: Option<&str>) -> Response<std::io::Cursor<Vec<u8>>> {
    let err = error.map(|e| format!("<div class=\"err\">{}</div>", html_escape(e))).unwrap_or_default();
    let html = LOGIN_HTML.replace("{{style}}", PAGE_STYLE).replace("{{error}}", &err).replace("{{next}}", &html_escape(&local_path(next)));
    text_response(401, "text/html; charset=utf-8", html)
}

/// The login screen's look, shared with the OAuth approval page: plain and light like the page.
pub(crate) const PAGE_STYLE: &str = r##"
:root{--bg:#f7f7f5;--panel:#fff;--ink:#1a1a1f;--muted:#6e6e78;--line:#e6e6e2;--line2:#d9d9d4;--accent:#2b5cff;--block:#b91c1c;--block-bg:#fde8e8;--mono:ui-monospace,SFMono-Regular,Menlo,monospace;--sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Inter,Roboto,sans-serif}
*{box-sizing:border-box}html,body{margin:0;min-height:100%;background:var(--bg);color:var(--ink);font:14px/1.5 var(--sans)}
body{display:flex;align-items:center;justify-content:center;min-height:100vh;padding:16px}
main{width:100%;max-width:440px;background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:20px 22px;display:flex;flex-direction:column;gap:12px}
.brand{display:flex;align-items:center;gap:10px;font-weight:700;font-size:15px}
.logo{width:28px;height:28px;border-radius:8px;background:var(--accent);display:flex;align-items:center;justify-content:center}
label{font-weight:600}
.row{display:flex;gap:8px;margin-top:6px}
input{flex:1;min-width:0;font:inherit;font-family:var(--mono);font-size:13px;padding:8px 10px;border:1px solid var(--line2);border-radius:9px;background:var(--panel);color:inherit}
button{font:inherit;padding:7px 16px;border:1px solid var(--accent);background:var(--accent);color:#fff;border-radius:9px;cursor:pointer;min-height:36px}
p{margin:0;font-size:13px;color:var(--muted)}code{font-family:var(--mono);font-size:12px}
.err{background:var(--block-bg);color:var(--block);border-radius:10px;padding:8px 12px;font-size:13px}
"##;

const LOGIN_HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light">
<title>Lorefile · sign in</title>
<style>{{style}}</style></head>
<body><main>
<div class="brand"><div class="logo"><svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><circle cx="5" cy="6" r="2"/><circle cx="19" cy="6" r="2"/><circle cx="12" cy="20" r="2"/><path d="M7 7l3 3M17 7l-3 3M12 15v3"/></svg></div>Lorefile</div>
{{error}}
<form method="post" action="/api/login">
<label for="token">Paste the token printed in the terminal</label>
<div class="row"><input id="token" name="token" type="password" autocomplete="current-password" spellcheck="false" autofocus required></div>
<label for="me" style="display:block;margin-top:10px">Your name <span style="font-weight:400;color:var(--muted)">optional · shown on what you accept</span></label>
<div class="row"><input id="me" type="text" autocomplete="name" spellcheck="false" maxlength="60" placeholder="e.g. Vinay" style="font-family:var(--sans)"><button type="submit">Go</button></div>
<input type="hidden" name="next" value="{{next}}">
</form>
<p>Running in Docker? It is in <code>LOREFILE_TOKEN</code>.</p>
<p>Started it yourself? It is also saved in <code>.lorefile/auth.toml</code> in the folder the server was started on.</p>
</main>
<script>
// The name stays in this browser (the page reads it from the same place) and never goes to the
// server with the token. The input has no name attribute, so the form does not send it.
(() => { try {
  const me = document.getElementById('me'), kept = localStorage.getItem('lorefile-name') || '';
  if (kept && !me.value) me.value = kept;
  const ok = (n) => n.length <= 60 && /^[\p{L}\p{N} .'_-]+$/u.test(n) && !/agent/i.test(n) && !n.toLowerCase().split(/\s+/).includes('via');
  document.querySelector('form').addEventListener('submit', () => { try { const v = me.value.trim(); if (v && ok(v)) localStorage.setItem('lorefile-name', v); } catch (e) {} });
} catch (e) {} })();
// A link from another site arrives without the cookie (it is SameSite=Strict). Ask once from
// this page, where the cookie is sent, and come back in if it is already there.
(async () => { try {
  const k = 'lorefile-auth-retry', last = +(sessionStorage.getItem(k) || 0);
  if (Date.now() - last < 10000) return;
  const r = await fetch('/api/auth', { credentials: 'same-origin' }); const j = await r.json();
  if (j.signed_in) { sessionStorage.setItem(k, String(Date.now())); location.replace(location.pathname + location.search + location.hash); }
} catch (e) {} })();
</script>
</body></html>
"##;

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = s.get(i + 1..i + 3).unwrap_or("");
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

pub(crate) fn query_params(url: &str) -> (String, HashMap<String, String>) {
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
        .with_header(header("Cache-Control", "no-store"))
}

/// The same shape, kept separate so a write's reply is never confused with a read's.
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
        domains.push(json!({ "name": d.name, "description": d.description, "types": types, "color": d.color, "listed": d.types }));
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
            // a field a kind gets from its parent is the parent's to describe
            let described = fields - t.own_properties().filter(|(_, p)| p.normalize().description.trim().is_empty()).count();
            json!({
                "name": t.name, "domain": ont.domain_of(&t.name), "description": t.description, "file": t.file, "key": t.key_property(),
                "fields": fields, "described": described, "has_source": t.source.is_some() || !t.instances.is_empty() || t.rows_of.as_ref().and_then(|b| ont.types.get(b)).map(|b| b.source.is_some() || !b.instances.is_empty()).unwrap_or(false),
                "source_kind": t.source.as_ref().and_then(|s| ont.sources.get(&s.connection)).map(|s| s.kind.clone()),
                "source_connection": t.source.as_ref().map(|s| s.connection.clone()),
                "source_table": t.source.as_ref().map(|s| s.table.clone()),
                "owner": t.owner, "tags": t.tags, "iri": ont.iri_type(&t.name),
                "extends": t.parent, "kinds": ont.kinds_of(&t.name), "where": t.is_where_kind().then(|| crate::ontology::Ontology::where_words(&t.where_all)),
            })
        })
        .collect();
    let pending = proposals::list(&st.dir).into_iter().filter(|p| p.status == "pending").count();
    let trust = trust_json(ont);
    json!({
        "trust": trust,
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
        "metrics": ont.metrics.values().map(|m| metric_json(m, ont)).collect::<Vec<_>>(),
        "functions": ont.functions.values().collect::<Vec<_>>(),
        "files": ont.files,
        "version": env!("CARGO_PKG_VERSION"),
        "namespace": ont.iris.namespace,
        "rdf": cfg!(feature = "rdf"),
        "diagnostics": st.report.diags,
        "advice": crate::validate::advise(ont),
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

/// A metric as the files say it, plus its label and its recipe in words (`sentence`, one per
/// variant in `sentences`). A recipe that can't be put into words gives no sentence: the page
/// says "counted by its own recipe" and shows the formula under Details.
pub(crate) fn metric_json(m: &crate::schema::MetricDef, ont: &crate::ontology::Ontology) -> Json {
    let mut j = serde_json::to_value(m).unwrap_or(json!({}));
    // in words only when all of it is: anything shown "as written" is a formula, for Details
    let say = |src: &crate::schema::MetricSource| match review::words::recipe_sentence(src, &review::words::rows_of(ont, src)) {
        (s, None) => Some(s),
        _ => None,
    };
    let sentences: Vec<Json> = if let Some(src) = &m.source {
        vec![json!(say(src).map(|s| edit::sentence(&s)))]
    } else {
        m.variants.iter().map(|v| json!(say(&v.source).map(|s| format!("When {}: {}", v.when, s)))).collect()
    };
    j["label"] = json!(review::words::label(&m.name));
    j["sentence"] = sentences.first().cloned().unwrap_or(Json::Null);
    j["sentences"] = json!(sentences);
    j
}

/// Every graded kind with its resolved grade: what the page's chips show and `GET /api/trust` answers.
fn trust_json(ont: &crate::ontology::Ontology) -> Json {
    let orphans: Vec<Json> = ont.graded.orphans.iter().map(|(k, n, r)| json!({ "kind": k, "name": n, "grade": r.grade.word() })).collect();
    json!({ "file": ont.graded.has_file, "counts": ont.graded.counts(), "items": ont.graded.list(), "orphans": orphans })
}

/// The People list for a space: owners, grades, the activity log and, when history is on, the
/// names of the last 500 commits' authors (never their email).
fn people_derived(st: &crate::spaces::Loaded, history_on: bool) -> crate::people::Derived {
    let mut seen: Vec<crate::people::Seen> = Vec::new();
    for a in proposals::activity(&st.dir, 2000) {
        if let Some(n) = a["who"].as_str().and_then(crate::people::person_from_by) {
            seen.push(crate::people::Seen { name: n, source: "activity", at: a["at"].as_str().map(|s| s.to_string()) });
        }
    }
    if history_on {
        for (author, at) in edit::git_authors(&st.dir, 500) {
            if let Some(n) = crate::people::person_from_by(&author) {
                seen.push(crate::people::Seen { name: n, source: "history", at: Some(at) });
            }
        }
    }
    crate::people::derive(&st.ont, &seen)
}

/// The `by` words for a write from the page: "Priya via page" when the page sent a name (Settings), else "person via page".
fn page_by(body: &Json) -> String {
    let name = body.get("by").and_then(|v| v.as_str());
    crate::trust::by_words(name, "page")
}

/// Writes must come from our own page or a deliberate client. The page sends
/// `X-Lorefile: write`: a custom header, so a cross-site browser request needs a
/// preflight that this server never grants. When the browser also sends
/// `Origin`, it has to match the `Host` the page was loaded from, whatever
/// address the server is bound to or proxied behind.
fn write_allowed(req: &Request) -> bool {
    let marked = header_value(req, "X-Lorefile").map(|v| v.trim() == "write").unwrap_or(false);
    marked && origin_matches(req)
}

/// No `Origin` (a client that is not a browser), or one that names the `Host`.
fn origin_matches(req: &Request) -> bool {
    match (header_value(req, "Origin"), header_value(req, "Host")) {
        (Some(origin), Some(host)) => {
            let o = origin.trim().trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/');
            o.eq_ignore_ascii_case(host.trim())
        }
        _ => true,
    }
}

const SHIPPED_READ_ONLY: &str = "this is a shipped example and stays read-only; copy it into a space of your own to change it";
const SHIPPED_HISTORY: &str = "off (shipped example, read-only)";

const WRITE_REFUSED: &str = "writes need the `X-Lorefile: write` header the page sends, and an Origin that matches the Host; cross-site requests are refused";

/// Bodies larger than this are refused before they are read.
const MAX_BODY: usize = 4 * 1024 * 1024;

/// The write endpoints. Takes the write lock, applies the change, reloads.
#[allow(clippy::too_many_arguments)] // one call site per transport; a struct would only rename the arguments
fn mutate(st: &mut State, builtin: bool, edit: EditConfig, method: &Method, path: &str, params: &HashMap<String, String>, body: &Json, as_tool: bool) -> Response<std::io::Cursor<Vec<u8>>> {
    if builtin {
        return json_response_private(403, &json!({ "error": SHIPPED_READ_ONLY, "editable": false, "builtin": true }));
    }
    // an agent's proposal is validated and queued even when edit mode is off: the same code as MCP
    if as_tool {
        let args = body.get("arguments").cloned().unwrap_or(json!({}));
        let (reply, reload) = {
            let srv = McpServer::with_data(&st.ont, st.data()).with_edit(edit).with_via("agent via API");
            let r = srv.call_tool(str_field(body, "name"), &args);
            (r, srv.reload_needed.get())
        };
        if reload {
            st.reload();
        }
        return json_response_private(200, &reply);
    }
    // a person's decision on a proposal: Accept, Not now, Look again, Take it back
    if matches!(path, "/api/proposals/accept" | "/api/proposals/dismiss" | "/api/proposals/reopen" | "/api/proposals/take_back" | "/api/proposals/sign") {
        return decide(st, edit, path, body);
    }
    if !edit.allow {
        return json_response_private(403, &json!({ "error": "edit mode is off; start the server with --allow-edit", "editable": false }));
    }
    let mut extra = json!({});
    let by = page_by(body);
    let outcome: Outcome = if path == "/api/trust" {
        let kind_word = str_field(body, "kind");
        let kind = crate::trust::Kind::parse(kind_word);
        if !kind_word.trim().is_empty() && kind.is_none() {
            return json_response_private(400, &json!({ "error": format!("kind is metric, binding, word or guardrail, not `{}`", kind_word) }));
        }
        let op = match str_field(body, "op") {
            "vouch_all" => Ok(edit::GradeOp::VouchAll),
            "accept" => edit::find_graded(&st.ont, kind, str_field(body, "name")).map(|(kind, name)| edit::GradeOp::Accept { kind, name }),
            "check" => edit::find_graded(&st.ont, kind, str_field(body, "name")).map(|(kind, name)| edit::GradeOp::Check { kind, name, against: str_field(body, "against").to_string(), note: str_field(body, "note").to_string() }),
            "sign" => edit::find_graded(&st.ont, kind, str_field(body, "name")).map(|(kind, name)| edit::GradeOp::Sign { items: vec![(kind, name)] }),
            "distinct" if crate::trust::who_said(&by) == "a person" => Err("type your name first: saying two things differ on purpose needs one".into()),
            "distinct" => Ok(edit::GradeOp::Distinct { a: str_field(body, "a").to_string(), b: str_field(body, "b").to_string(), why: str_field(body, "why").to_string() }),
            other => Err(format!("op is accept, check, sign, distinct or vouch_all, not `{}`", other)),
        };
        match op {
            Ok(op) => {
                let out = edit::grade(&st.dir, edit, &st.report, &op, &by);
                if out.written {
                    let what = match &op {
                        edit::GradeOp::Accept { kind, name } => format!("accepted {}", kind.name_words(name)),
                        edit::GradeOp::Check { kind, name, against, .. } => format!("checked {} against {}", kind.name_words(name), against),
                        edit::GradeOp::VouchAll => format!("vouched for {} definitions", out.name),
                        edit::GradeOp::Sign { .. } => format!("{} put their name on {}", edit::sentence(&crate::trust::who_said(&by)), out.stamped.join(" and ")),
                        edit::GradeOp::Distinct { a, b, why } => format!("{} said {} and {} are different on purpose{}", edit::sentence(&crate::trust::who_said(&by)), crate::trust::side_words(a), crate::trust::side_words(b), if why.trim().is_empty() { String::new() } else { format!(": \"{}\"", why.trim()) }),
                    };
                    if matches!(op, edit::GradeOp::Sign { .. } | edit::GradeOp::Distinct { .. }) {
                        extra["summary"] = json!(what);
                    }
                    proposals::log(&st.dir, &by, &what, crate::trust::FILE);
                }
                out
            }
            Err(e) => Outcome { file: crate::trust::FILE.into(), refused: Some(e), ..Default::default() },
        }
    } else if *method == Method::Delete {
        let file = params.get("file").cloned().unwrap_or_else(|| str_field(body, "file").to_string());
        // the entities it defined also leave every area's list, in the same save
        let files = patch::delete_files(&st.ont, &file);
        if files.len() > 1 {
            let summary = format!("Deleted {} (and took it off its area's list)", file);
            edit::apply_files(&st.dir, edit, &st.report, &files, &Note { via: &by, summary: &summary, ..Default::default() }, None)
        } else {
            edit::apply(&st.dir, edit, &st.report, &file, Change::Delete, &Note::via(&by), None)
        }
    } else if path == "/api/create" {
        let (kind, name, content) = (str_field(body, "kind"), str_field(body, "name"), str_field(body, "content"));
        let file = body.get("file").and_then(|f| f.as_str()).filter(|f| !f.trim().is_empty());
        match edit::plan_create(&st.dir, kind, name, content, file) {
            Ok((rel, text)) => edit::apply(&st.dir, edit, &st.report, &rel, Change::Write(text), &Note::via(&by), Some((kind, name))),
            Err(e) => Outcome { file: file.unwrap_or("").to_string(), refused: Some(e), ..Default::default() },
        }
    } else if path == "/api/people" {
        let who = edit::sentence(&crate::trust::who_said(&by));
        match str_field(body, "op") {
            "rename" => match crate::people::rename_plan(&st.ont, str_field(body, "from"), str_field(body, "to")) {
                Ok(plan) => {
                    let summary = format!("{} {} {} {} {}: {}", who, if plan.merge { "merged" } else { "renamed" }, plan.from, if plan.merge { "into" } else { "to" }, plan.to, plan.counts_words());
                    extra["summary"] = json!(summary);
                    extra["merge"] = json!(plan.merge);
                    extra["lossy"] = json!(plan.lossy);
                    edit::apply_many(&st.dir, edit, &st.report, &plan.files, &Note { via: &by, summary: &summary, rename: true, ..Default::default() }, Some(("person", &plan.to)))
                }
                Err(e) => Outcome { file: crate::people::FILE.into(), refused: Some(e), ..Default::default() },
            },
            op => match crate::people::edit_file(&st.ont, op, str_field(body, "name"), str_field(body, "other")) {
                Ok((f, what)) => {
                    let summary = format!("{} {}", who, what);
                    extra["summary"] = json!(summary);
                    let empty = f.people.is_empty() && f.not_same.is_empty();
                    let exists = st.dir.join(crate::people::FILE).is_file();
                    let text = if empty { None } else { Some(crate::people::render(&f)) };
                    if empty && !exists {
                        Outcome { file: crate::people::FILE.into(), ok: true, ..Default::default() }
                    } else {
                        edit::apply_files(&st.dir, edit, &st.report, &[(crate::people::FILE.to_string(), text)], &Note { via: &by, summary: &summary, people: true, ..Default::default() }, Some(("person", str_field(body, "name"))))
                    }
                }
                Err(e) => Outcome { file: crate::people::FILE.into(), refused: Some(e), ..Default::default() },
            },
        }
    } else if path == "/api/patch" && (body.get("ops").is_some() || patch::multi_file(body)) {
        // several ops, or one op that writes several files: one change
        let ops: Vec<Json> = body.get("ops").and_then(|o| o.as_array()).cloned().unwrap_or_else(|| vec![body.clone()]);
        match patch::plan_many(&st.ont, &ops) {
            Ok(plan) => {
                let summary = if str_field(body, "op") == "make_parent" {
                    format!("made {} the parent of {}", str_field(body, "name"), body.get("children").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default())
                } else {
                    plan.sentences().join("; ")
                };
                extra["summary"] = json!(summary);
                extra["summaries"] = json!(plan.sentences());
                let label = if ops.len() == 1 { plan.kinds.first().cloned() } else { None };
                let label = label.as_ref().map(|(k, n)| (k.as_str(), n.as_str()));
                let label = if str_field(body, "op") == "make_parent" { Some(("type", str_field(body, "name"))) } else { label };
                if plan.files.len() == 1 {
                    let (f, t) = plan.files[0].clone();
                    edit::apply(&st.dir, edit, &st.report, &f, Change::Write(t), &Note { via: &by, summary: &summary, ..Default::default() }, label)
                } else {
                    edit::apply_files(&st.dir, edit, &st.report, &plan.files.iter().map(|(f, t)| (f.clone(), if t.is_empty() { None } else { Some(t.clone()) })).collect::<Vec<_>>(), &Note { via: &by, summary: &summary, ..Default::default() }, label)
                }
            }
            Err(e) => Outcome { file: String::new(), refused: Some(e), ..Default::default() },
        }
    } else if path == "/api/patch" {
        match patch::plan(&st.ont, body) {
            Ok(p) => {
                extra["summary"] = json!(p.summary);
                edit::apply(&st.dir, edit, &st.report, &p.file, Change::Write(p.text), &Note { via: &by, summary: &p.summary, ..Default::default() }, Some((&p.kind, &p.name)))
            }
            Err(e) => Outcome { file: str_field(body, "type").to_string(), refused: Some(e), ..Default::default() },
        }
    } else {
        edit::apply(&st.dir, edit, &st.report, str_field(body, "file"), Change::Write(str_field(body, "content").to_string()), &Note { via: &by, reason: str_field(body, "reason"), ..Default::default() }, None)
    };
    if outcome.written {
        if path == "/api/patch" || path == "/api/people" {
            proposals::log(&st.dir, &by, extra["summary"].as_str().unwrap_or("edited"), &outcome.file);
        } else if path == "/api/edit" || path == "/api/create" {
            proposals::log(&st.dir, &by, &format!("edited {}", outcome.file), &outcome.file);
        } else if *method == Method::Delete {
            proposals::log(&st.dir, &by, &format!("deleted {}", outcome.file), &outcome.file);
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
        j["trust"] = trust_json(&st.ont);
        // the words history and trust.yaml got: a name the page sent may not have been usable
        j["by"] = json!(by);
    }
    json_response_private(outcome.status(), &j)
}

const REVIEW_READ_ONLY: &str = "This Lorefile server is read-only (started without --allow-edit), so nobody can accept or set aside here. Restart it with --allow-edit.";

/// The decisions on a proposal, each under the space's decision lock, in the order the spec
/// gives (specs/2026-11-27-review-flow.md, Part 3). Every refusal says what happened in words.
fn decide(st: &mut State, edit: EditConfig, path: &str, body: &Json) -> Response<std::io::Cursor<Vec<u8>>> {
    if !edit.allow {
        return json_response_private(403, &json!({ "error": REVIEW_READ_ONLY, "editable": false }));
    }
    let by = page_by(body);
    let name = edit::sentence(&crate::trust::who_said(&by));
    let id = str_field(body, "id").to_string();
    // 1. one decision at a time per space, across processes; held until the log line is written
    let _lock = match edit::SpaceLock::take(&st.dir, edit::DECIDE_LOCK) {
        Ok(l) => l,
        Err(e) => return json_response_private(409, &json!({ "error": e })),
    };
    // the files may have moved on in another process since this request began
    st.reload();
    // 2. read the proposal again
    let Some(p) = proposals::get(&st.dir, &id) else {
        return json_response_private(404, &json!({ "error": format!("There is no proposal {} in this space.", id), "missing": true }));
    };
    match path {
        "/api/proposals/dismiss" => {
            let changes = review::own_changes(&st.ont, &p);
            match proposals::set_aside(&st.dir, &id, &by, str_field(body, "why")) {
                Ok(p) => {
                    let why = p.why.as_ref().map(|w| format!(": \"{}\"", w)).unwrap_or_default();
                    proposals::log(&st.dir, &by, &format!("{} set aside {}{}", name, review::object_words(&p, &changes), why), &p.file);
                    json_response_private(200, &json!({ "ok": true, "proposal": p, "summary": "Set aside" }))
                }
                Err(e) => json_response_private(409, &json!({ "error": e, "state": p.status })),
            }
        }
        "/api/proposals/reopen" => {
            let changes = review::own_changes(&st.ont, &p);
            match proposals::reopen(&st.dir, &id, &by) {
                Ok(p) => {
                    proposals::log(&st.dir, &by, &format!("{} looked again at {}", name, review::object_words(&p, &changes)), &p.file);
                    json_response_private(200, &json!({ "ok": true, "proposal": p, "summary": "Waiting for review again" }))
                }
                Err(e) => json_response_private(409, &json!({ "error": e, "state": p.status })),
            }
        }
        "/api/proposals/take_back" => take_back(st, edit, &p, &by, &name),
        "/api/proposals/sign" => sign_proposal(st, edit, &p, &by, &name),
        _ => accept(st, edit, &p, body, &by, &name),
    }
}

/// "Put your name on it" for a decision made with no name: the proposal's `accepted_by` (or
/// `decided_by`) and every anonymous grade of the things it graded take the name, in one save.
/// Named records are left alone and listed as already named.
fn sign_proposal(st: &mut State, edit: EditConfig, p: &proposals::Proposal, by: &str, name: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let who = crate::trust::who_said(by);
    if who == "a person" || who == "an agent" {
        return json_response_private(400, &json!({ "error": "Type your name first: putting your name on a decision needs one." }));
    }
    let anon = |b: &Option<String>| b.as_deref().map(|b| crate::trust::who_said(b) == "a person").unwrap_or(false);
    let (field_anon, when) = match p.status.as_str() {
        "accepted" => (anon(&p.accepted_by), "accepted"),
        "set_aside" => (anon(&p.decided_by), "set aside"),
        _ => return json_response_private(409, &json!({ "error": format!("{} is {}; only a decision can take a name.", p.id, p.state_words()) })),
    };
    let graded = if p.status == "accepted" { review::graded_by(&st.ont, p) } else { vec![] };
    let mut items = Vec::new();
    let mut named = Vec::new();
    for (k, n) in graded {
        match edit::sign_refusal(&st.ont, k, &n) {
            None => items.push((k, n)),
            Some(why) => named.push(why),
        }
    }
    if items.is_empty() && !field_anon {
        return json_response_private(409, &json!({ "error": if named.is_empty() { format!("{} already says who decided it.", p.id) } else { format!("Nothing here is missing a name: {}.", named.join("; ")) } }));
    }
    let mut out = Outcome { written: true, ok: true, file: crate::trust::FILE.into(), ..Default::default() };
    let trust_was = std::fs::read_to_string(st.dir.join(crate::trust::FILE)).ok();
    if !items.is_empty() {
        out = edit::grade(&st.dir, edit, &st.report, &edit::GradeOp::Sign { items }, by);
        if !out.written {
            return json_response_private(409, &out.to_json());
        }
    }
    // a name changes no meaning: Take it back stays possible when it was before
    let trust_now = std::fs::read_to_string(st.dir.join(crate::trust::FILE)).ok();
    if let Err(e) = proposals::sign(&st.dir, &p.id, field_anon.then_some(by), trust_was, trust_now) {
        return json_response_private(409, &json!({ "error": e }));
    }
    let changes = review::own_changes(&st.ont, p);
    let date = crate::trust::short_date(p.resolved_at.get(..10).unwrap_or(""));
    let what = format!("{} put their name on {} ({} {} with no name)", name, review::object_words(p, &changes), when, date);
    proposals::log(&st.dir, by, &what, &p.file);
    st.reload();
    let mut j = out.to_json();
    j["written"] = json!(true);
    j["summary"] = json!(what);
    j["already_named"] = json!(named);
    json_response_private(200, &j)
}

/// Accept: fresh or replayed, the person's fixes on top, then the save gate.
fn accept(st: &mut State, edit: EditConfig, p: &proposals::Proposal, body: &Json, by: &str, name: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    if p.status != "pending" {
        return json_response_private(409, &json!({ "error": format!("{} is {}.", p.id, p.state_words()), "state": p.status }));
    }
    // an agent's report of a check: the person's click makes it checked, in their name
    if let Some(c) = &p.check {
        let note = format!("{} (reported by {})", c.note, if p.via.is_empty() { "an agent" } else { p.via.as_str() });
        let op = edit::find_graded(&st.ont, crate::trust::Kind::parse(&c.kind), &c.name).and_then(|(kind, n)| {
            // the agent compared one recipe; if the definition changed since, that comparison says nothing about the new one
            let now = crate::trust::things(&st.ont, kind).get(&n).map(crate::trust::fingerprint).unwrap_or_default();
            if c.of.is_empty() || c.of != now {
                return Err(format!("{} has changed since the agent compared it, so this report is about an older definition; set it aside and ask the agent to compare again", kind.name_words(&n)));
            }
            Ok(edit::GradeOp::Check { kind, name: n, against: c.against.clone(), note })
        });
        let out = match op {
            Ok(op) => edit::grade(&st.dir, edit, &st.report, &op, by),
            Err(e) => Outcome { file: crate::trust::FILE.into(), refused: Some(e), ..Default::default() },
        };
        if out.written {
            let _ = proposals::mark_accepted(&st.dir, &p.id, by, vec![], None, out.commit.clone(), vec![]);
            proposals::log(&st.dir, by, &format!("{} confirmed the check of {} {} against {} reported by {}", name, c.kind, c.name, c.against, review::agent_name(&p.via)), crate::trust::FILE);
            st.reload();
        }
        let mut j = out.to_json();
        j["proposal_id"] = json!(p.id);
        if out.written {
            j["summary"] = json!(format!("Marked {} {} as checked", c.kind, c.name));
            j["trust"] = trust_json(&st.ont);
            j["by"] = json!(by);
        }
        return json_response_private(out.status(), &j);
    }
    // 3. fresh, or changed since: replayed on today's files, or refused in words
    let files = match review::prepare(&st.ont, p) {
        review::Prepared::Fresh { files, .. } => files,
        review::Prepared::Replayed { files, .. } => {
            // the person accepts exactly what the screen showed them
            if str_field(body, "seen") != review::fingerprint(&files) {
                return json_response_private(409, &json!({ "error": "The space changed since this was proposed, and the screen you accepted from is older than that. Review it again.", "stale": true }));
            }
            files
        }
        review::Prepared::Stale { why, .. } => return json_response_private(409, &json!({ "error": why, "stale": true })),
    };
    let seen = str_field(body, "seen");
    if !seen.is_empty() && seen != review::fingerprint(&files) {
        return json_response_private(409, &json!({ "error": "The space changed while you were looking. Review it again.", "stale": true }));
    }
    // 4. the fixes the screen offered, planned on top
    let mut fix_words: Vec<String> = Vec::new();
    // "different on purpose" goes to trust.yaml after the save, not into a file
    let mut distinct: Vec<(String, String, String)> = Vec::new();
    let files = match body.get("fixes").and_then(|f| f.as_array()).filter(|f| !f.is_empty()) {
        None => files,
        Some(fixes) => {
            let ov: Vec<(String, Option<String>)> = files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect();
            let Some((after, after_report)) = review::overlay(&st.dir, &ov) else {
                return json_response_private(400, &json!({ "error": "the proposed files do not load" }));
            };
            let (qs, _, _) = review::questions(&st.report, &after, &after_report, p);
            let mut ops = Vec::new();
            for f in fixes {
                match review::fix_op(f, &qs, &after) {
                    Ok((op, words)) if op["op"] == "distinct" => {
                        if crate::trust::who_said(by) == "a person" {
                            return json_response_private(400, &json!({ "error": "Type your name first: saying two things differ on purpose needs one." }));
                        }
                        distinct.push((str_field(&op, "a").to_string(), str_field(&op, "b").to_string(), str_field(&op, "why").to_string()));
                        fix_words.push(words);
                    }
                    Ok((op, words)) => {
                        ops.push(op);
                        fix_words.push(words);
                    }
                    Err(e) => return json_response_private(400, &json!({ "error": format!("That fix can't ride along with Accept: {}.", e) })),
                }
            }
            if ops.is_empty() {
                files
            } else {
                match patch::plan_many_over(&st.ont, &files, &ops) {
                    Ok(plan) => plan.files,
                    Err(e) => return json_response_private(400, &json!({ "error": format!("That fix does not apply: {}.", e) })),
                }
            }
        }
    };
    // 5. the save gate: one change, the agent as author and the person as committer
    let changes = {
        let ov: Vec<(String, Option<String>)> = files.iter().map(|(f, t)| (f.clone(), Some(t.clone()))).collect();
        review::overlay(&st.dir, &ov).map(|(a, _)| review::compare(&st.ont, &a)).unwrap_or_default()
    };
    let headline = review::headline(p, &changes);
    let mut message_body = p.reason.clone();
    if !fix_words.is_empty() {
        message_body = format!("{}{}Fixes by {}: {}.", message_body, if message_body.is_empty() { "" } else { "\n\n" }, name, fix_words.join("; "));
    }
    let via = if p.via.trim().is_empty() { "agent" } else { p.via.as_str() };
    let trust_before = std::fs::read_to_string(st.dir.join(crate::trust::FILE)).ok();
    let errors_before: Vec<proposals::KnownError> = st.report.diags.iter().filter(|d| d.level == crate::validate::Level::Error).map(|d| proposals::KnownError { code: d.code.to_string(), file: d.file.clone(), message: d.message.clone() }).collect();
    let befores: Vec<Option<String>> = files.iter().map(|(f, _)| std::fs::read_to_string(st.dir.join(f)).ok()).collect();
    let note = Note { via, summary: &headline, reason: &message_body, accepted_by: Some(by), ..Default::default() };
    let label = if files.len() == 1 && !p.kind.is_empty() && p.kind != "batch" { Some((p.kind.as_str(), p.name.as_str())) } else { None };
    let out = edit::apply_many(&st.dir, edit, &st.report, &files, &note, label);
    let mut j = out.to_json();
    j["proposal_id"] = json!(p.id);
    if out.written {
        // the pairs the person said are different on purpose, in trust.yaml, before the snapshot
        for (a, b, why) in &distinct {
            let g = edit::grade(&st.dir, edit, &st.report, &edit::GradeOp::Distinct { a: a.clone(), b: b.clone(), why: why.clone() }, by);
            if !g.written {
                j["distinct_error"] = json!(g.refused.unwrap_or_else(|| "not saved".into()));
            }
        }
        // 6. accepted, with what Take it back needs when there is no history to undo from
        let snapshot = if out.commit.is_none() {
            Some(proposals::Snapshot {
                files: files.iter().zip(befores).map(|((f, t), b)| proposals::SnapFile { file: f.clone(), before: b, after: Some(t.clone()) }).collect(),
                trust_before,
                trust_after: std::fs::read_to_string(st.dir.join(crate::trust::FILE)).ok(),
            })
        } else {
            None
        };
        let _ = proposals::mark_accepted(&st.dir, &p.id, by, fix_words.clone(), snapshot, out.commit.clone(), errors_before);
        // 7. the log, in words
        let fixes = if fix_words.is_empty() { String::new() } else { format!(" ({})", fix_words.join("; ")) };
        proposals::log(&st.dir, by, &format!("{} accepted {}{}", name, review::object_words(p, &changes), fixes), &out.file);
        st.reload();
        j["summary"] = json!(format!("Accepted: {}", review::object_words(p, &changes)));
        j["git"] = edit::git_status(&st.dir).unwrap_or(Json::Null);
        j["stats"] = json!(st.ont.stats());
        j["trust"] = trust_json(&st.ont);
        j["by"] = json!(by);
    }
    json_response_private(out.status(), &j)
}

/// Take it back: history off puts the snapshot's texts back; history on is that commit's Undo.
fn take_back(st: &mut State, edit: EditConfig, p: &proposals::Proposal, by: &str, name: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    if p.status != "accepted" {
        return json_response_private(409, &json!({ "error": format!("{} is {}, so there is nothing to take back.", p.id, p.state_words()), "state": p.status }));
    }
    let (ok, why) = review::take_back_state(&st.ont, p, edit.commit);
    if !ok {
        return json_response_private(409, &json!({ "error": why.unwrap_or_else(|| "this can't be taken back".into()) }));
    }
    let changes = review::own_changes(&st.ont, p);
    let what = review::object_words(p, &changes);
    let summary = format!("Took back {}", what);
    // the errors the space had before Accept may come back with its files; nothing else may
    let mut baseline = crate::validate::Report { diags: st.report.diags.clone() };
    baseline.diags.extend(p.errors_before.iter().filter_map(review::known_diag));
    let out = match (&p.snapshot, &p.commit) {
        (Some(snap), _) => {
            let mut files: Vec<(String, Option<String>)> = snap.files.iter().filter(|f| f.before != f.after).map(|f| (f.file.clone(), f.before.clone())).collect();
            if snap.trust_before != snap.trust_after {
                files.push((crate::trust::FILE.to_string(), snap.trust_before.clone()));
            }
            edit::apply_files(&st.dir, edit, &baseline, &files, &Note { via: by, summary: &summary, keep_grades: true, ..Default::default() }, Some(("take back", &p.id)))
        }
        (None, Some(hash)) => edit::undo(&st.dir, edit, &baseline, hash, by),
        (None, None) => Outcome { refused: Some("nothing recorded to take back".into()), ..Default::default() },
    };
    let mut j = out.to_json();
    j["proposal_id"] = json!(p.id);
    if out.written {
        // a folder the Accept made and the take-back emptied goes too (remove_dir leaves a folder with anything in it)
        if let Some(snap) = &p.snapshot {
            for f in snap.files.iter().filter(|f| f.before.is_none()) {
                if let Some(parent) = st.dir.join(&f.file).parent().filter(|d| *d != st.dir.as_path()) {
                    let _ = std::fs::remove_dir(parent);
                }
            }
        }
        let _ = proposals::mark_taken_back(&st.dir, &p.id, by);
        proposals::log(&st.dir, by, &format!("{} took back {}", name, what), &p.file);
        st.reload();
        j["summary"] = json!(format!("{}; it waits for review again", summary));
        j["git"] = edit::git_status(&st.dir).unwrap_or(Json::Null);
        j["stats"] = json!(st.ont.stats());
        j["trust"] = trust_json(&st.ont);
        j["by"] = json!(by);
    }
    json_response_private(out.status(), &j)
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

/// `+`, `/` and `=` would change meaning in a query string; base64url tokens have none.
pub(crate) fn url_token(t: &str) -> String {
    t.replace('+', "%2B").replace('/', "%2F").replace('=', "%3D")
}

/// What the command line said about OAuth for connectors.
#[derive(Default)]
pub struct OAuthFlags {
    /// `--no-oauth`
    pub off: bool,
    /// `--public-url`, checked and reduced to an origin
    pub public_url: Option<String>,
    /// `--oauth-redirect`, checked
    pub redirects: Vec<String>,
}

/// Why OAuth is off: a word for the page, and the start-up line's words.
fn oauth_off_reason(login: &Login, flags: &OAuthFlags) -> Option<(&'static str, String)> {
    match &login.token {
        None => Some(("no-auth", "off (--no-auth): ChatGPT and Claude connect with \"No authentication\", so anyone with the address gets in".into())),
        Some(_) if flags.off => Some(("no-oauth", "off (--no-oauth)".into())),
        Some(t) if t.len() < oauth::MIN_LOGIN => Some(("short-token", format!("off: the login token is shorter than {} characters, too easy to guess on an approval page the internet can reach (`openssl rand -base64 32` makes a good one)", oauth::MIN_LOGIN))),
        Some(_) => None,
    }
}

fn plural_word(n: usize, word: &str) -> String {
    format!("{} {}{}", n, word, if n == 1 { "" } else { "s" })
}

pub fn run(reg: Registry, host: &str, port: u16, edit: EditConfig, login: Login, flags: OAuthFlags) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let server = Server::http(&addr).map_err(|e| anyhow::anyhow!("cannot bind {}: {}", addr, e))?;
    let mine = reg.user_spaces().len();
    let shipped = reg.spaces.len() - mine;
    // the address to open from this machine: 0.0.0.0 listens everywhere, 127.0.0.1 reaches it
    let shown = if host == "0.0.0.0" || host == "::" || host == "[::]" { "127.0.0.1".to_string() } else { host.to_string() };
    let base = format!("http://{}:{}", shown, port);
    let examples = if shipped > 0 { format!(", plus {} shipped examples", shipped) } else { String::new() };
    let what = if reg.single { format!("one ontology folder, {}{}", reg.root.display(), examples) } else { format!("{} space{} of yours in {}{}", mine, if mine == 1 { "" } else { "s" }, reg.root.display(), examples) };
    eprintln!("Lorefile {} · {} · edit mode {}", env!("CARGO_PKG_VERSION"), what, if edit.allow { "on" } else { "off" });
    let mcp_url = format!("{}/mcp", base);
    match (&login.token, &login.from) {
        (Some(t), Some(from)) => {
            let (open, bearer, note) = match from {
                TokenFrom::Env => (format!("{}/", base), format!("${}", TOKEN_ENV), format!("sign in with the token in {}", TOKEN_ENV)),
                TokenFrom::Flag => (format!("{}/", base), "<the token given to --token>".to_string(), "sign in with the token given to --token".to_string()),
                TokenFrom::File(p) => (format!("{}/?token={}", base, url_token(t)), t.clone(), format!("(token from {})", p.display())),
                TokenFrom::New(p) => (format!("{}/?token={}", base, url_token(t)), t.clone(), format!("(first run: token saved to {})", p.display())),
                TokenFrom::Unsaved(p, e) => (format!("{}/?token={}", base, url_token(t)), t.clone(), format!("(could not save the token to {}: {}; it lasts until the server stops)", p.display(), e)),
            };
            eprintln!("Open    {}   {}", open, note);
            eprintln!("MCP     {}   Authorization: Bearer {}", mcp_url, bearer);
            eprintln!("        claude mcp add --transport http lorefile {} --header \"Authorization: Bearer {}\"", mcp_url, bearer);
            if matches!(from, TokenFrom::Env | TokenFrom::Flag) && t.len() < 16 {
                eprintln!("Warning: a token shorter than 16 characters is easy to guess; `openssl rand -base64 32` makes a good one");
            }
        }
        _ => {
            eprintln!("Open    {}/", base);
            eprintln!("Login   off (--no-auth): anyone who can reach this port can read these spaces{}", if edit.allow { " and, with edit mode on, change them" } else { "" });
            eprintln!("MCP     {}", mcp_url);
            eprintln!("        claude mcp add --transport http lorefile {}", mcp_url);
        }
    }
    eprintln!("        or over stdio on this machine: lorefile mcp --dir {}", reg.root.display());
    let oauth_off = oauth_off_reason(&login, &flags);
    let oauth_srv = match (&oauth_off, &login.token) {
        (None, Some(t)) => {
            let link_note = match &login.from {
                Some(TokenFrom::Env) => Some(format!("sign in with the token in {}", TOKEN_ENV)),
                Some(TokenFrom::Flag) => Some("sign in with the token given to --token".to_string()),
                _ => None,
            };
            let cfg = oauth::Config { login: t.clone(), pinned: flags.public_url.clone(), extra_redirects: flags.redirects.clone(), edit_allowed: edit.allow, link_note };
            Some(Arc::new(oauth::Server::open(&reg.root, cfg, oauth::Clock::system())))
        }
        _ => None,
    };
    match (&oauth_srv, &oauth_off) {
        (Some(o), _) => {
            let n = o.connection_count();
            let count = if n == 0 { "no connections yet".to_string() } else { plural_word(n, "connection") };
            match o.pinned() {
                Some(pin) => eprintln!("OAuth   on at {} only (--public-url): give ChatGPT or Claude {}/mcp · {}", pin, pin, count),
                None => eprintln!("OAuth   on for any HTTPS address that reaches this server (a tunnel or a proxy): give ChatGPT or Claude <that address>/mcp · {}", count),
            }
            if o.dropped > 0 {
                eprintln!("        the login token changed: {} {} disconnected", plural_word(o.dropped, "connection"), if o.dropped == 1 { "was" } else { "were" });
            }
            if let Some(e) = &o.load_error {
                eprintln!("        {}", e);
            }
        }
        (None, Some((_, why))) => eprintln!("OAuth   {}{}", why, if flags.public_url.is_some() { "; --public-url is not used" } else { "" }),
        (None, None) => {}
    }
    if edit.allow {
        let commits = match edit.policy {
            edit::CommitPolicy::Auto => " and saves each change to the space's git history (a space inside another repository stays uncommitted)",
            edit::CommitPolicy::Always => " and commits each change (--commit-edits)",
            edit::CommitPolicy::Never => " (changes stay uncommitted: --no-commit-edits)",
        };
        eprintln!("Edit    on: the page and agents write YAML into the spaces{}", commits);
    } else {
        eprintln!("Edit    off (start with --allow-edit to edit from the page)");
    }
    let root_s = std::fs::canonicalize(&reg.root).unwrap_or_else(|_| reg.root.clone()).display().to_string();
    let single = reg.single;

    let ctx = Arc::new(Ctx { edit, root_s, single, token: login.token, cookie: format!("lorefile-{}", port), oauth: oauth_srv, oauth_off, ghosts: Mutex::new(HashMap::new()) });
    let reg = Arc::new(Mutex::new(reg));
    for req in server.incoming_requests() {
        let (reg, ctx) = (Arc::clone(&reg), Arc::clone(&ctx));
        // a thread per request: a slow or stalled client holds its own thread, not the server
        std::thread::spawn(move || handle(req, &reg, &ctx));
    }
    Ok(())
}

/// What every request handler needs besides the registry.
struct Ctx {
    edit: EditConfig,
    root_s: String,
    single: bool,
    /// The login token; `None` with `--no-auth`.
    token: Option<String>,
    /// The page's cookie name, `lorefile-<port>`.
    cookie: String,
    /// The OAuth sign-in for connectors; `None` when it is off.
    oauth: Option<Arc<oauth::Server>>,
    /// Why it is off: a word for the page and the start-up line's words.
    oauth_off: Option<(&'static str, String)>,
    /// The home page's ghosts of waiting proposals, by space, proposal and the space's files,
    /// so a refresh does not compile them again.
    ghosts: Mutex<HashMap<String, Json>>,
}

/// Who is calling `/mcp`: the owner (the login token, Claude Code's header), or a
/// connection that signed in with OAuth.
enum Caller {
    Owner,
    Connection(oauth::Grant),
}

/// Login token first, then an OAuth access token issued for exactly this MCP address
/// on the public address this request came through.
fn caller(req: &Request, ctx: &Ctx, public: Option<&str>, space: Option<&str>) -> Option<Caller> {
    let Some(want) = &ctx.token else { return Some(Caller::Owner) };
    let given = bearer(req)?;
    if same_token(&given, want) {
        return Some(Caller::Owner);
    }
    let (o, origin) = (ctx.oauth.as_ref()?, public?);
    let resource = match space {
        None => oauth::Resource::All,
        Some(id) => oauth::Resource::Space(id.to_string()),
    };
    o.check(&given, origin, &resource).map(Caller::Connection)
}

fn reply_response(r: oauth::Reply) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut resp = Response::from_string(r.body).with_status_code(StatusCode(r.status));
    for (k, v) in &r.headers {
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp = resp.with_header(h);
        }
    }
    resp
}

/// Settings › Connections: list, and Disconnect (one, or `all`).
fn oauth_api(req: &Request, ctx: &Ctx, method: &Method, path: &str, body: &Json) -> Response<std::io::Cursor<Vec<u8>>> {
    let list = path == "/api/oauth/connections";
    let Some(o) = &ctx.oauth else {
        let (reason, why) = ctx.oauth_off.clone().unwrap_or(("off", "off".into()));
        return if list && *method == Method::Get {
            json_response(200, &json!({ "on": false, "reason": reason, "why": why, "connections": [] }))
        } else {
            json_response_private(404, &json!({ "error": format!("OAuth is {}", why) }))
        };
    };
    match method {
        Method::Get if list => json_response(200, &o.connections_json()),
        Method::Post if path.ends_with("/writes") && !list => {
            if !write_allowed(req) {
                return json_response_private(403, &json!({ "error": WRITE_REFUSED }));
            }
            let id = path.strip_prefix("/api/oauth/connections/").and_then(|r| r.strip_suffix("/writes")).map(percent_decode).unwrap_or_default();
            if id.is_empty() || id.contains('/') {
                return json_response_private(400, &json!({ "error": "say which connection: POST /api/oauth/connections/<id>/writes" }));
            }
            match o.set_writes(&id, str_field(body, "writes")) {
                Ok(()) => {
                    let mut j = o.connections_json();
                    j["ok"] = json!(true);
                    json_response_private(200, &j)
                }
                Err(e) => json_response_private(400, &json!({ "error": e })),
            }
        }
        Method::Post if path.ends_with("/revoke") && !list => {
            if !write_allowed(req) {
                return json_response_private(403, &json!({ "error": WRITE_REFUSED }));
            }
            // /api/oauth/connections/<id>/revoke, with one non-empty id (or `all`)
            let id = path.strip_prefix("/api/oauth/connections/").and_then(|r| r.strip_suffix("/revoke")).map(percent_decode).unwrap_or_default();
            if id.is_empty() || id.contains('/') {
                return json_response_private(400, &json!({ "error": "say which connection: POST /api/oauth/connections/<id>/revoke, or all" }));
            }
            match o.disconnect(&id) {
                Ok(n) => {
                    let mut j = o.connections_json();
                    j["ok"] = json!(true);
                    j["disconnected"] = json!(n);
                    json_response_private(200, &j)
                }
                Err(e) => json_response_private(400, &json!({ "error": e })),
            }
        }
        _ => json_response(405, &json!({ "error": "method not allowed" })),
    }
}

fn handle(mut req: Request, reg: &Mutex<Registry>, ctx: &Ctx) {
    let (edit, single) = (ctx.edit, ctx.single);
    let root_s: &str = &ctx.root_s;
    let (full_path, params) = query_params(req.url());
    let (prefix_space, path) = split_space(&full_path);
    let method = req.method().clone();
    let is_page = method == Method::Get && (path == "/" || path == "/index.html" || path.is_empty());

    // open to everyone: the health check, and signing in and out
    if method == Method::Get && full_path == "/healthz" {
        let _ = req.respond(json_response(200, &json!({ "ok": true })));
        return;
    }
    // OAuth for connectors: the public address this request came through, when OAuth answers
    // on it (HTTPS-marked, or the pinned address). Plain http://127.0.0.1 gets None and no new routes.
    let hdrs: Vec<(String, String)> = if ctx.oauth.is_some() { req.headers().iter().map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string())).collect() } else { Vec::new() };
    let get = |name: &'static str| hdrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let public = ctx.oauth.as_ref().and_then(|o| o.public_origin(&get));
    if let (Some(o), Some(origin)) = (&ctx.oauth, &public) {
        if oauth::is_route(&full_path) {
            let mut body = String::new();
            if method == Method::Post {
                if req.body_length().map(|n| n > oauth::MAX_BODY).unwrap_or(false) {
                    let _ = req.respond(json_response(413, &json!({ "error": "invalid_request", "error_description": "the body is larger than 16 KB" })));
                    return;
                }
                let _ = req.as_reader().take(oauth::MAX_BODY as u64 + 1).read_to_string(&mut body);
                if body.len() > oauth::MAX_BODY {
                    let _ = req.respond(json_response(413, &json!({ "error": "invalid_request", "error_description": "the body is larger than 16 KB" })));
                    return;
                }
            }
            // how the request already proves the login: the bearer header is the token itself, the cookie a signed-in browser
            let approver = match &ctx.token {
                Some(want) if bearer(&req).map(|t| same_token(&t, want)).unwrap_or(false) => Some("login token"),
                Some(want) if cookie_values(&req, &ctx.cookie).iter().any(|t| same_token(t, want)) => Some("signed-in browser"),
                _ => None,
            };
            let r = oauth::Req { method: method.as_str(), path: &full_path, query: &params, header: &get, body: &body, origin, approver };
            let reply = o.handle(&r);
            let sign_in = reply.sign_in;
            let mut resp = reply_response(reply);
            if sign_in {
                // a correct token on the approval page also signs this browser in, as the login screen does
                resp = resp.with_header(cookie_header(&req, ctx, ctx.token.as_deref()));
            }
            let _ = req.respond(resp);
            return;
        }
    }
    // a discovery address Lorefile does not serve (openid-configuration and the like): 404 for
    // everyone, before the login. A 401 here tells a client to sign in to learn how to sign in.
    if full_path.starts_with("/.well-known/") || path.starts_with("/.well-known/") {
        let _ = req.respond(json_response(404, &json!({ "error": "not found" })));
        return;
    }
    if method == Method::Get && path == "/api/auth" {
        let me = signed_in(&req, ctx, true);
        let mut j = json!({ "required": ctx.token.is_some(), "signed_in": me });
        if me {
            // the page shows it masked in Settings, with a copy button; only someone who already has it gets it
            j["token"] = json!(ctx.token);
        }
        let _ = req.respond(json_response(200, &j));
        return;
    }
    // the link the terminal printed: keep the token as a cookie and take it out of the address bar
    if is_page && ctx.token.is_some() {
        if let Some(t) = params.get("token") {
            let resp = if ctx.token.as_deref().map(|want| same_token(t.trim(), want)).unwrap_or(false) {
                redirect(&full_path).with_header(cookie_header(&req, ctx, ctx.token.as_deref()))
            } else {
                login_page(&full_path, Some(WRONG_TOKEN))
            };
            let _ = req.respond(resp);
            return;
        }
    }
    let is_login = method == Method::Post && path == "/api/login";
    let is_logout = method == Method::Post && path == "/api/logout";
    let mut who = None;
    if path == "/mcp" {
        // MCP clients send the bearer header (the login token, or an OAuth access token); the page's cookie does not open it
        who = caller(&req, ctx, public.as_deref(), prefix_space.as_deref());
        if who.is_none() {
            let resp = match (&ctx.oauth, &public) {
                // on an OAuth address the 401 says where to sign in (RFC 9728)
                (Some(o), Some(origin)) => json_response(401, &json!({ "error": "sign in: ChatGPT and Claude connect here with OAuth; Claude Code can also send Authorization: Bearer <token>, the token the server printed when it started" }))
                    .with_header(header("WWW-Authenticate", &o.challenge(origin, &full_path, bearer(&req).is_some())))
                    .with_header(header("Vary", "Host, X-Forwarded-Host, X-Forwarded-Proto, Forwarded")),
                _ => json_response(401, &json!({ "error": "send Authorization: Bearer <token>, the token the server printed when it started" })).with_header(header("WWW-Authenticate", "Bearer")),
            };
            let _ = req.respond(resp);
            return;
        }
    } else if !is_login && !is_logout && !signed_in(&req, ctx, true) {
        // checked before the body is read: no one without the token gets 4 MB of our time
        // the page's address keeps a review link's `?review=<id>` through the sign-in form
        let next = match params.get("review").filter(|r| r.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
            Some(r) => format!("{}?review={}", full_path, r),
            None => full_path.clone(),
        };
        let resp = if is_page { login_page(&next, None) } else { json_response(401, &json!({ "error": SIGN_IN_FIRST })).with_header(header("WWW-Authenticate", "Bearer")) };
        let _ = req.respond(resp);
        return;
    }

    let has_body = method == Method::Post || method == Method::Delete || method == Method::Put;
    let mut body_text = String::new();
    if has_body {
        if req.body_length().map(|n| n > MAX_BODY).unwrap_or(false) {
            let _ = req.respond(json_response(413, &json!({ "error": "the request body is larger than 4 MB" })));
            return;
        }
        let _ = req.as_reader().take(MAX_BODY as u64 + 1).read_to_string(&mut body_text);
        if body_text.len() > MAX_BODY {
            let _ = req.respond(json_response(413, &json!({ "error": "the request body is larger than 4 MB" })));
            return;
        }
    }
    let body: Json = if has_body { serde_json::from_str(&body_text).unwrap_or(json!({})) } else { json!({}) };
    let content_type = header_value(&req, "Content-Type").map(|c| c.to_ascii_lowercase()).unwrap_or_default();
    let json_body = content_type.contains("application/json");

    // sign in: the login screen's form (a redirect back), or a script's JSON (a JSON answer)
    if is_login || is_logout {
        let form = content_type.contains("application/x-www-form-urlencoded");
        let fields = if form { query_params(&format!("?{}", body_text)).1 } else { HashMap::new() };
        let next = fields.get("next").map(|n| local_path(n)).unwrap_or_else(|| "/".to_string());
        let resp = if is_logout {
            let r = if form { redirect("/") } else { json_response_private(200, &json!({ "ok": true, "signed_in": false })) };
            r.with_header(cookie_header(&req, ctx, None))
        } else {
            let given = if form { fields.get("token").cloned().unwrap_or_default() } else { str_field(&body, "token").to_string() };
            match &ctx.token {
                None => if form { redirect(&next) } else { json_response_private(200, &json!({ "ok": true, "required": false })) },
                Some(want) if same_token(given.trim(), want) => {
                    let r = if form { redirect(&next) } else { json_response_private(200, &json!({ "ok": true, "signed_in": true })) };
                    r.with_header(cookie_header(&req, ctx, Some(want)))
                }
                Some(_) => if form { login_page(&next, Some(WRONG_TOKEN)) } else { json_response_private(401, &json!({ "error": WRONG_TOKEN })) },
            }
        };
        let _ = req.respond(resp);
        return;
    }

    // Settings › Connections: the agents that signed in with OAuth
    if path == "/api/oauth/connections" || path.starts_with("/api/oauth/connections/") {
        let resp = oauth_api(&req, ctx, &method, &path, &body);
        let _ = req.respond(resp);
        return;
    }

    // the page itself, for any space
    if is_page {
        let _ = req.respond(text_response(200, "text/html; charset=utf-8", INDEX_HTML.to_string()));
        return;
    }

    // MCP over Streamable HTTP: one JSON-RPC message or a batch per POST, answered as plain JSON
    if path == "/mcp" {
        let resp = match method {
            Method::Post => {
                // where a person opens the page from here: the public address, else this machine's own
                let base = public.clone().or_else(|| header_value(&req, "Host").filter(|h| is_loopback(host_only(h))).map(|h| format!("http://{}", h)));
                mcp_http(&req, reg, ctx, prefix_space.as_deref(), json_body, &body_text, who.as_ref().unwrap_or(&Caller::Owner), base)
            }
            // no server-to-client stream and no sessions: this server only answers what it is asked
            Method::Get => json_response(405, &json!({ "error": "this MCP server sends no event stream; POST JSON-RPC messages to /mcp" })).with_header(header("Allow", "POST, DELETE")),
            Method::Delete => Response::from_string(String::new()).with_status_code(StatusCode(204)),
            _ => json_response(405, &json!({ "error": "method not allowed" })).with_header(header("Allow", "POST, DELETE")),
        };
        let _ = req.respond(resp);
        return;
    }

    // everything from here touches the registry: one request at a time, bodies already read
    let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());

    // the list of spaces, and creating one
    if path == "/api/spaces" {
        let resp = match method {
            Method::Get => json_response(200, &spaces_json(&mut reg, edit)),
            Method::Post if !write_allowed(&req) => json_response_private(403, &json!({ "error": WRITE_REFUSED })),
            Method::Post if !json_body => json_response_private(415, &json!({ "error": "send the body as application/json" })),
            Method::Post if !edit.allow => json_response_private(403, &json!({ "error": "edit mode is off; start the server with --allow-edit to create spaces", "editable": false })),
            Method::Post => match reg.create(str_field(&body, "name"), str_field(&body, "from")) {
                Ok(space) => {
                    proposals::log(&space.dir, PAGE, &format!("created the space {}{}", space.id, if str_field(&body, "from").is_empty() { String::new() } else { format!(" from {}", str_field(&body, "from")) }), "ontology.yaml");
                    json_response_private(200, &json!({ "ok": true, "written": true, "space": space, "history": space.history, "summary": format!("Created the space {}", space.id), "spaces": spaces_json(&mut reg, edit) }))
                }
                Err(e) => json_response_private(400, &json!({ "error": e })),
            },
            _ => json_response(405, &json!({ "error": "method not allowed" })),
        };
        let _ = req.respond(resp);
        return;
    }

    // which space does this request mean?
    let want = prefix_space.clone().or_else(|| params.get("space").cloned()).or_else(|| reg.default_id());
    let Some(space_id) = want else {
        let _ = req.respond(json_response(404, &json!({ "error": "no space yet: create one in the page (or start the server on an ontology folder)", "no_spaces": true })));
        return;
    };
    let Some(space) = reg.info(&space_id).cloned() else {
        let _ = req.respond(json_response(404, &json!({ "error": format!("no space named '{}'", space_id), "spaces": reg.spaces.keys().collect::<Vec<_>>() })));
        return;
    };
    // shipped examples never take writes, whatever the flags say
    let base_edit = if space.builtin { EditConfig::default() } else { edit };
    let spaces_root = reg.root.clone();
    // what the flags mean for this space (commit or not) asks git, so it is only worked out where it matters
    let space_edit = || if base_edit.allow { base_edit.for_space(&space.dir, &spaces_root) } else { base_edit };

    // history: start it, commit, undo a change, share with a remote, push and pull
    if method == Method::Post && path.starts_with("/api/git/") {
        let what = path["/api/git/".len()..].to_string();
        let resp = if !write_allowed(&req) {
            json_response_private(403, &json!({ "error": WRITE_REFUSED }))
        } else if !json_body {
            json_response_private(415, &json!({ "error": "send the body as application/json" }))
        } else if space.builtin {
            json_response_private(403, &json!({ "error": SHIPPED_READ_ONLY, "editable": false, "builtin": true }))
        } else if !base_edit.allow {
            json_response_private(403, &json!({ "error": "edit mode is off; start the server with --allow-edit", "editable": false }))
        } else if what == "undo" {
            let cfg = space_edit();
            match reg.get(&space_id) {
                Ok(st) => {
                    let out = edit::undo(&st.dir, cfg, &st.report, str_field(&body, "hash"), PAGE);
                    let mut j = out.to_json();
                    if out.written {
                        j["summary"] = json!(format!("Put back how things were before '{}'", out.name));
                        proposals::log(&st.dir, PAGE, &format!("undid '{}'", out.name), &out.file);
                        st.reload();
                        j["git"] = edit::git_status(&st.dir).unwrap_or(Json::Null);
                        j["stats"] = json!(st.ont.stats());
                    }
                    json_response_private(out.status(), &j)
                }
                Err(e) => json_response_private(500, &json!({ "error": format!("{} could not be loaded: {:#}", space_id, e) })),
            }
        } else {
            let r = match what.as_str() {
                "init" => match edit::init_space(&space.dir, &spaces_root, PAGE).as_str() {
                    "on" => Ok("History is on: every change from now on is saved as a commit".to_string()),
                    other => Err(format!("history is {}", other)),
                },
                "commit" => edit::git_commit_all(&space.dir, str_field(&body, "message")).map(|h| format!("Committed {}", h)),
                "push" => edit::git_push(&space.dir),
                "remote" => edit::git_set_remote(&space.dir, &spaces_root, str_field(&body, "url")),
                "pull" => edit::git_pull(&space.dir, &spaces_root),
                _ => Err(format!("unknown git action `{}`", what)),
            };
            match r {
                Ok(s) => {
                    proposals::log(&space.dir, PAGE, &s, "");
                    if what == "pull" {
                        // the files changed under us: read them again now rather than on the next request
                        if let Ok(st) = reg.get(&space_id) {
                            st.reload();
                        }
                    }
                    json_response_private(200, &json!({ "ok": true, "written": true, "summary": s }))
                }
                Err(e) => json_response_private(400, &json!({ "error": e })),
            }
        };
        reg.touched(&space_id);
        let _ = req.respond(resp);
        return;
    }

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
        return;
    }

    // the People screen: names from this space, plus (on the page only) names from the other spaces
    // this server serves, never shipped examples
    if method == Method::Get && (path == "/api/people" || path == "/api/people/plan") {
        let mut other: Vec<String> = Vec::new();
        if path == "/api/people" {
            let ids: Vec<String> = reg.user_spaces().iter().filter(|s| s.id != space_id && !s.builtin).map(|s| s.id.clone()).collect();
            for id in ids {
                if let Ok(o) = reg.get(&id) {
                    for p in crate::people::derive(&o.ont, &[]).people {
                        if other.len() < 200 && !other.iter().any(|n| crate::people::fold(n) == crate::people::fold(&p.name)) {
                            other.push(p.name);
                        }
                    }
                }
            }
        }
        let history_on = !space.builtin && space_edit().commit;
        let editable = base_edit.allow && !space.builtin;
        let resp = match reg.get(&space_id) {
            Ok(st) if path == "/api/people/plan" => match crate::people::rename_plan(&st.ont, params.get("from").map(|s| s.as_str()).unwrap_or(""), params.get("to").map(|s| s.as_str()).unwrap_or("")) {
                Ok(p) => json_response_private(200, &json!({ "from": p.from, "to": p.to, "merge": p.merge, "counts": p.counts, "counts_words": p.counts_words(), "lossy": p.lossy, "files": p.files.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>() })),
                Err(e) => json_response_private(400, &json!({ "error": e })),
            },
            Ok(st) => {
                let d = people_derived(st, history_on);
                let here: Vec<String> = d.people.iter().map(|p| crate::people::fold(&p.name)).collect();
                other.retain(|n| !here.contains(&crate::people::fold(n)) && !d.people.iter().any(|p| p.also.iter().any(|a| crate::people::fold(a) == crate::people::fold(n))));
                let mut j = d.to_json();
                j["other_spaces"] = json!(other);
                j["editable"] = json!(editable);
                j["builtin"] = json!(space.builtin);
                json_response_private(200, &j)
            }
            Err(e) => json_response(500, &json!({ "error": format!("{} could not be loaded: {:#}", space_id, e) })),
        };
        reg.touched(&space_id);
        let _ = req.respond(resp);
        return;
    }

    let st = match reg.get(&space_id) {
        Ok(st) => st,
        Err(e) => {
            let _ = req.respond(json_response(500, &json!({ "error": format!("{} could not be loaded: {:#}", space_id, e), "space": space_id })));
            return;
        }
    };

    // writes: edit, create, delete, and propose_change through /api/tool
    let as_tool = method == Method::Post && path == "/api/tool" && matches!(str_field(&body, "name"), "propose_change" | "propose_patch" | "learn_file");
    let is_write = as_tool
        || (method == Method::Post && (path == "/api/edit" || path == "/api/create" || path == "/api/patch" || path == "/api/proposals/accept" || path == "/api/proposals/dismiss" || path == "/api/proposals/reopen" || path == "/api/proposals/take_back" || path == "/api/proposals/sign" || path == "/api/trust" || path == "/api/people"))
        || (method == Method::Delete && path == "/api/file");
    if is_write {
        let resp = if !write_allowed(&req) {
            json_response_private(403, &json!({ "error": WRITE_REFUSED }))
        } else if method == Method::Post && !json_body {
            json_response_private(415, &json!({ "error": "send the body as application/json" }))
        } else {
            mutate(st, space.builtin, space_edit(), &method, &path, &params, &body, as_tool)
        };
        reg.touched(&space_id);
        let _ = req.respond(resp);
        return;
    }

    let ont = &st.ont;
    let mcp = McpServer::with_data(ont, st.data()).with_history(if space.builtin { None } else { Some(spaces_root.clone()) });
    let resp = match (method.clone(), path.as_str()) {
        (Method::Get, "/api/summary") => {
            let mut j = overview(st, &space, root_s, single, space_edit());
            j["history"] = json!(if space.builtin { SHIPPED_HISTORY } else { edit::home(&st.dir, &spaces_root).word() });
            json_response(200, &j)
        }
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
        (Method::Get, "/api/export") => match params.get("format").map(|f| f.as_str()).unwrap_or("ossie") {
            "ossie" => match crate::ossie::export(ont) {
                Ok(ex) => json_response(200, &ex.bundle()).with_header(header("Content-Disposition", &format!("attachment; filename=\"{}.ossie.json\"", ex.base))),
                Err(e) => json_response(500, &json!({ "error": e.to_string() })),
            },
            #[cfg(feature = "rdf")]
            f @ ("turtle" | "ttl" | "jsonld" | "json-ld" | "ntriples" | "nt" | "rdfxml") => {
                let fmt = crate::rdf::Format::parse(f).unwrap_or(crate::rdf::Format::Turtle);
                match crate::rdf::export(ont).write(fmt) {
                    Ok(text) => text_response(200, fmt.mime(), text)
                        .with_header(header("Content-Disposition", &format!("attachment; filename=\"{}.{}\"", crate::iri::slug(&ont.manifest.name), fmt.extension()))),
                    Err(e) => json_response(500, &json!({ "error": e.to_string() })),
                }
            }
            other => json_response(400, &json!({ "error": format!("unknown export format `{}`: ossie (Apache Ossie), or turtle, jsonld, ntriples, rdfxml (OWL, SKOS and SHACL)", other) })),
        },
        (Method::Get, "/api/git") if space.builtin => json_response(200, &json!({ "repo": false, "builtin": true, "history": SHIPPED_HISTORY, "commits": 0 })),
        (Method::Get, "/api/git") => json_response(200, &edit::git_info(&st.dir, &spaces_root)),
        (Method::Get, "/api/git/log") | (Method::Get, "/api/git/show") if space.builtin => json_response(403, &json!({ "error": SHIPPED_READ_ONLY, "builtin": true, "history": SHIPPED_HISTORY })),
        (Method::Get, "/api/git/log") => {
            let home = edit::home(&st.dir, &spaces_root);
            let limit = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50);
            match home {
                edit::Home::NoGit | edit::Home::Nowhere => json_response(200, &json!({ "history": home.word(), "commits": [], "total": 0 })),
                _ => match edit::git_log(&st.dir, limit, params.get("file").map(|f| f.as_str())) {
                    Ok(mut j) => {
                        j["history"] = json!(home.word());
                        json_response(200, &j)
                    }
                    Err(e) => json_response(400, &json!({ "error": e, "history": home.word() })),
                },
            }
        }
        (Method::Get, "/api/git/show") => match edit::git_show(&st.dir, params.get("hash").map(|h| h.as_str()).unwrap_or("")) {
            Ok(j) => json_response(200, &j),
            Err(e) => json_response(404, &json!({ "error": e })),
        },
        (Method::Get, "/api/trust") => json_response(200, &trust_json(ont)),
        (Method::Get, "/api/proposals") => {
            let all = proposals::list(&st.dir);
            let history_on = space_edit().commit;
            let ctx = review::Ctx { editable: base_edit.allow, history_on, space: &space_id, all: &all };
            // the short cards: what each waiting proposal adds, in words (decided ones keep their line)
            let cards: serde_json::Map<String, Json> = all.iter().filter(|p| p.status == "pending" && p.check.is_none()).take(50).map(|p| (p.id.clone(), review::card(ont, &st.report, p, &ctx))).collect();
            json_response(200, &json!({ "proposals": all, "cards": cards, "activity": proposals::activity(&st.dir, 50), "editable": base_edit.allow, "history_on": history_on }))
        }
        (Method::Get, p) if p.starts_with("/api/proposals/") && p.ends_with("/review") => {
            let id = percent_decode(p.trim_start_matches("/api/proposals/").trim_end_matches("/review"));
            let all = proposals::list(&st.dir);
            match proposals::get(&st.dir, &id) {
                Some(prop) => {
                    let ctx = review::Ctx { editable: base_edit.allow, history_on: space_edit().commit, space: &space_id, all: &all };
                    json_response(200, &review::review(ont, &st.report, &prop, &ctx))
                }
                None => json_response(404, &json!({ "error": format!("There is no proposal {} in {}.", id, space_id), "missing": true, "id": id })),
            }
        }
        (Method::Get, "/api/plane") => {
            let conns: Vec<Json> = ctx.oauth.as_ref().map(|o| o.connections_json()["connections"].as_array().cloned().unwrap_or_default()).unwrap_or_default();
            let version = crate::plane::version(&st.dir, &st.fp, &conns);
            if params.get("version").map(|v| *v == version).unwrap_or(false) {
                json_response(200, &json!({ "same": true, "version": version }))
            } else {
                let history_on = !space.builtin && space_edit().commit;
                let git = if history_on { Some(crate::plane::git_changed(&st.dir)) } else { None };
                let props = proposals::list(&st.dir);
                let activity = proposals::activity(&st.dir, 500);
                let fp = format!("{:?}", st.fp);
                let pending: Vec<String> = props.iter().filter(|p| p.status == "pending").map(|p| format!("{}|{}|", space_id, p.id)).collect();
                let mut cache = ctx.ghosts.lock().unwrap_or_else(|e| e.into_inner());
                // a proposal that left pending, or another version of the files: dropped
                cache.retain(|k, _| !k.starts_with(&format!("{}|", space_id)) || pending.iter().any(|p| k.starts_with(p) && k.ends_with(&fp)));
                let inp = crate::plane::Input {
                    ont,
                    report: &st.report,
                    space: crate::plane::SpaceInfo { id: &space_id, name: &space.name, builtin: space.builtin, editable: space_edit().allow && !space.builtin },
                    proposals: &props,
                    activity: &activity,
                    connections: &conns,
                    git_changed: git.as_ref(),
                    today: chrono::Local::now().date_naive(),
                    version,
                };
                let mut ghost = |p: &proposals::Proposal| cache.entry(format!("{}|{}|{}", space_id, p.id, fp)).or_insert_with(|| crate::plane::ghost(ont, &st.report, p)).clone();
                json_response(200, &crate::plane::plane(&inp, &mut ghost))
            }
        }
        (Method::Get, "/api/graph") => match params.get("proposal").and_then(|id| proposals::get(&st.dir, id)) {
            Some(prop) => json_response(200, &review::graph(ont, &prop)),
            None => json_response(404, &json!({ "error": "say which proposal: /api/graph?proposal=<id>" })),
        },
        (Method::Get, "/api/activity") => json_response(200, &json!({ "activity": proposals::activity(&st.dir, 100) })),
        (Method::Get, "/api/tools") => json_response(200, &crate::mcp::tool_definitions()),
        (Method::Get, "/api/tables") => match st.data().sql_catalog() {
            Ok(j) => json_response(200, &j),
            Err(e) => json_response(500, &json!({ "error": e.to_string() })),
        },
        (Method::Post, "/api/query") => {
            let sql = body.get("sql").and_then(|s| s.as_str()).unwrap_or("").to_string();
            let limit = body.get("limit").and_then(|l| l.as_u64()).unwrap_or(crate::data::MAX_QUERY_ROWS as u64) as usize;
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

/// `127.0.0.1:7171` → `127.0.0.1`; `[::1]:7171` → `::1`.
fn host_only(h: &str) -> &str {
    let h = h.trim();
    if let Some(rest) = h.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    h.rsplit_once(':').map(|(a, _)| a).unwrap_or(h)
}

/// `POST /mcp`: the same `Hub` the stdio server runs, one body at a time, so
/// `initialize`, `tools/list` and `tools/call` (with `space`, and with
/// `--allow-edit` deciding whether proposals are written) answer identically.
/// `/s/<id>/mcp` serves that one space, like `lorefile mcp --space <id>`.
/// A connection that signed in with OAuth gets the propose tools only with `lorefile:propose`,
/// and is named in proposals and the activity log.
#[allow(clippy::too_many_arguments)] // the request, its parts and who sent it; a struct would only rename them
fn mcp_http(req: &Request, reg: &Mutex<Registry>, ctx: &Ctx, space: Option<&str>, json_body: bool, body: &str, who: &Caller, link_base: Option<String>) -> Response<std::io::Cursor<Vec<u8>>> {
    // the MCP spec asks servers to check Origin; the bearer header already keeps other sites out
    if !origin_matches(req) {
        return json_response(403, &json!({ "error": "cross-site requests are refused: the Origin does not match the Host" }));
    }
    if !json_body {
        return json_response(415, &json!({ "error": "send the JSON-RPC message as application/json" }));
    }
    let msg: Json = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(e) => return json_response(400, &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {}", e) } })),
    };
    let mut reg = reg.lock().unwrap_or_else(|e| e.into_inner());
    let guest = matches!(who, Caller::Connection(_));
    let only = match space {
        None => None,
        Some(id) => match reg.resolve(Some(id)) {
            Ok(s) => Some(s),
            Err(_) => {
                // a space created since the server started: look again before giving up
                reg.rescan();
                match reg.resolve(Some(id)) {
                    Ok(s) => Some(s),
                    // the owner is told which spaces there are; a connection is not
                    Err(_) if guest => return json_response(404, &json!({ "error": format!("no space named '{}'", id) })),
                    Err(e) => return json_response(404, &json!({ "error": e })),
                }
            }
        },
    };
    let (can_propose, via) = match who {
        Caller::Owner => (true, crate::mcp::VIA.to_string()),
        Caller::Connection(g) => (g.can_propose(), g.label()),
    };
    // a connection set to "wait for me" never writes, whatever --allow-edit says; the owner's own agents keep the flag
    let edit = match who {
        Caller::Connection(g) if !g.direct => EditConfig { allow: false, commit: false, ..ctx.edit },
        _ => ctx.edit,
    };
    let mut hub = Hub { reg: &mut reg, edit, only, can_propose, via, guest, link_base };
    match hub.handle_body(&msg) {
        Some(reply) => json_response(200, &reply),
        // notifications only: accepted, nothing to answer
        None => Response::from_string(String::new()).with_status_code(StatusCode(202)),
    }
}

#[cfg(test)]
mod tests {
    use super::{base64url, is_loopback, local_path, same_token, split_space, token_ok};

    #[test]
    fn tokens_are_base64url_compared_whole_and_checked_for_safe_characters() {
        assert_eq!(base64url(b"Man"), "TWFu");
        assert_eq!(base64url(b"Ma"), "TWE");
        assert_eq!(base64url(b"M"), "TQ");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(super::new_token().unwrap().len(), 43, "32 bytes, unpadded");
        assert!(same_token("abc", "abc") && !same_token("abc", "abd") && !same_token("abc", "abcd"));
        assert!(token_ok("k3f9-_.~+/=").is_ok());
        assert!(token_ok("has space").is_err() && token_ok("semi;colon").is_err() && token_ok("").is_err());
        assert!(is_loopback("127.0.0.1") && is_loopback("localhost") && is_loopback("::1") && is_loopback("[::1]"));
        assert!(!is_loopback("0.0.0.0") && !is_loopback("192.168.1.4") && !is_loopback("example.com"));
        assert_eq!(local_path("/s/acme/"), "/s/acme/");
        assert_eq!(local_path("//evil.example"), "/");
        assert_eq!(local_path("https://evil.example"), "/");
    }

    #[test]
    fn the_space_prefix_is_split_off_the_path() {
        assert_eq!(split_space("/s/acme/api/summary"), (Some("acme".into()), "/api/summary".into()));
        assert_eq!(split_space("/s/%40shopify/"), (Some("@shopify".into()), "/".into()));
        assert_eq!(split_space("/s/acme"), (Some("acme".into()), "/".into()));
        assert_eq!(split_space("/api/summary"), (None, "/api/summary".into()));
        assert_eq!(split_space("/"), (None, "/".into()));
    }
}
