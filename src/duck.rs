//! DuckDB inside the binary: the Phase 2 data door.
//!
//! One in-memory DuckDB per process holds the ontology as tables (`nodes`,
//! `properties`, `edges`, `constraints`, `actions`, `sources`) plus one view per
//! data-bound type or relationship, named after it, that reads the rows where
//! they live: a CSV or Parquet file, a `.duckdb` file, a Postgres table through
//! DuckDB's postgres extension, or Parquet on S3/HTTP through httpfs. One SQL
//! statement can then join "rules about Supplier" to "actual suppliers".
//!
//! Everything here is read-only: sources are attached `READ_ONLY`, and `query`
//! accepts exactly one SELECT/WITH statement.

use crate::data::{Column, QueryResult, Record};
use crate::ontology::{Ontology, Relationship};
use crate::schema::*;
use anyhow::{anyhow, bail, Context, Result};
use duckdb::types::ValueRef;
use duckdb::Connection;
use indexmap::IndexMap;
use serde_json::{json, Value as Json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

/// Hard cap on rows a `query` returns.
pub const MAX_ROWS: usize = 500;
/// Cap on rows `instances()` loads from a DuckDB-backed source.
pub const MAX_INSTANCE_ROWS: usize = 10_000;
/// Source kinds DuckDB reads directly.
pub const KINDS: &[&str] = &["csv", "parquet", "duckdb", "postgres", "s3", "httpfs"];
/// Names of the ontology tables; a type or relationship with one of these names gets no view.
const RESERVED: &[&str] = &["nodes", "properties", "edges", "constraints", "actions", "sources"];

pub fn is_duck_kind(kind: &str) -> bool {
    let k = kind.to_ascii_lowercase();
    KINDS.contains(&k.as_str())
}

/// A view DuckDB created for a type or relationship.
#[derive(Debug, Clone)]
pub struct ViewInfo {
    /// Name of the view inside DuckDB (usually the type name).
    pub view: String,
    /// Where it reads from, for `where_is`: a file path, a `read_parquet(...)` call, or `alias.schema.table`.
    pub source: String,
    pub columns: Vec<String>,
}

pub struct Db {
    conn: Mutex<Connection>,
    /// True when DuckDB was fenced to the folders the views read from (no
    /// other file, no network) before any query ran. Off when a source is
    /// remote (postgres, s3) because those need external access at query time.
    pub fenced: bool,
    /// Type name, or `From.relationship` for relationships → view.
    views: IndexMap<String, ViewInfo>,
    /// Type/relationship name → why its view could not be created (a real failure: missing file, bad credentials…).
    pub skipped: IndexMap<String, String>,
    /// Type/relationship name → source kind DuckDB does not read (snowflake, bigquery…). Expected, not an error.
    pub unreadable: IndexMap<String, String>,
    has_json: bool,
}

// ---------------- SQL text helpers ----------------

fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn lit_opt(s: &str) -> String {
    if s.is_empty() { "NULL".into() } else { lit(s) }
}

fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn lit_list(v: &[String]) -> String {
    format!("[{}]::VARCHAR[]", v.iter().map(|s| lit(s)).collect::<Vec<_>>().join(", "))
}

fn yaml_lit(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::Null => "NULL".into(),
        serde_yaml::Value::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => lit(s),
        other => lit(&serde_json::to_value(other).map(|j| j.to_string()).unwrap_or_default()),
    }
}

/// `${VAR}` → the environment value. Errors name the missing variable.
pub fn expand_env(s: &str) -> Result<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else { bail!("unterminated ${{ in `{}`", s) };
        let var = &after[..end];
        match std::env::var(var) {
            Ok(v) => out.push_str(&v),
            Err(_) => bail!("environment variable {} is not set", var),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn cfg_str(src: &SourceDef, key: &str) -> Option<String> {
    match src.config.get(key)? {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn cfg_bool(src: &SourceDef, key: &str) -> bool {
    matches!(src.config.get(key), Some(serde_yaml::Value::Bool(true)))
        || matches!(cfg_str(src, key).as_deref(), Some("true") | Some("yes") | Some("1"))
}

/// `my-conn.prod` → `src_my_conn_prod`: a safe ATTACH alias.
fn alias_for(connection: &str) -> String {
    let safe: String = connection.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    format!("src_{}", safe)
}

fn has_scheme(s: &str) -> bool {
    ["s3://", "s3a://", "gs://", "gcs://", "r2://", "az://", "http://", "https://", "hf://"].iter().any(|p| s.starts_with(p))
}

fn has_ext(s: &str) -> bool {
    Path::new(s).extension().is_some() || s.contains('*') || s.contains('?')
}

/// `root/<folder>/<table>.<ext>` unless the table already names a file or glob.
fn file_in(root: &Path, folder: Option<&String>, table: &str, ext: &str) -> PathBuf {
    let mut p = root.join(folder.cloned().unwrap_or_else(|| "data".into()));
    if has_ext(table) { p.push(table) } else { p.push(format!("{}.{}", table, ext)) }
    p
}

/// `base/table.parquet` for remote sources; the table may already be a full URL.
fn url_in(base: &str, table: &str) -> String {
    if has_scheme(table) {
        return table.to_string();
    }
    let name = if has_ext(table) { table.to_string() } else { format!("{}.parquet", table) };
    format!("{}/{}", base.trim_end_matches('/'), name)
}

/// `a.b.c` → `"a"."b"."c"`.
fn dotted(parts: &[&str]) -> String {
    parts.iter().map(|p| ident(p)).collect::<Vec<_>>().join(".")
}

// ---------------- read-only guard ----------------

/// Strip `--` and `/* */` comments outside string literals and report whether a
/// `;` remains outside strings.
fn strip_comments(sql: &str) -> (String, bool) {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let (mut i, mut semicolon) = (0, false);
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                if i + 1 < chars.len() && chars[i + 1] == q {
                    out.push(q);
                    i += 1;
                } else {
                    quote = None;
                }
            }
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' => { quote = Some(c); out.push(c); i += 1; }
            '-' if i + 1 < chars.len() && chars[i + 1] == '-' => {
                while i < chars.len() && chars[i] != '\n' { i += 1; }
                out.push(' ');
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') { i += 1; }
                i += 2;
                out.push(' ');
            }
            ';' => { semicolon = true; out.push(c); i += 1; }
            _ => { out.push(c); i += 1; }
        }
    }
    (out, semicolon)
}

/// Accept exactly one SELECT or WITH statement; return it trimmed, without the trailing `;`.
pub fn check_read_only(sql: &str) -> Result<String> {
    let (stripped, _) = strip_comments(sql);
    let mut s = stripped.trim().to_string();
    while s.ends_with(';') {
        s.pop();
        s = s.trim_end().to_string();
    }
    if s.is_empty() {
        bail!("empty SQL");
    }
    let (_, semicolon) = strip_comments(&s);
    if semicolon {
        bail!("only one statement is allowed (found a `;` in the middle)");
    }
    let first = s.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or("").to_ascii_lowercase();
    if first != "select" && first != "with" {
        bail!("only a single SELECT or WITH statement is allowed, not `{}`; the data door is read-only", first);
    }
    Ok(s)
}

// ---------------- builder (runs once at open) ----------------

struct Builder<'c> {
    conn: &'c Connection,
    has_json: bool,
    attached: HashSet<String>,
    loaded: HashSet<String>,
    views: IndexMap<String, ViewInfo>,
    skipped: IndexMap<String, String>,
    unreadable: IndexMap<String, String>,
    /// The ontology folder as an absolute path, so view paths and the fence agree.
    root: PathBuf,
    /// Folders the local views read from; the only ones queries may touch afterwards.
    local_dirs: HashSet<String>,
    /// A postgres/s3/http source was attached: the fence has to stay open.
    remote: bool,
}

impl<'c> Builder<'c> {
    fn note_local(&mut self, p: &Path) {
        let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        self.local_dirs.insert(dir.display().to_string());
    }

    fn exec(&self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql).map_err(|e| anyhow!("{}", e)).with_context(|| format!("running `{}`", truncate_sql(sql)))
    }

    fn load_extension(&mut self, name: &str) -> Result<()> {
        if self.loaded.contains(name) {
            return Ok(());
        }
        self.conn
            .execute_batch(&format!("INSTALL {}; LOAD {};", name, name))
            .map_err(|e| anyhow!("could not load the DuckDB `{}` extension: {} (the first use downloads it from extensions.duckdb.org, so network access is needed once)", name, e))?;
        self.loaded.insert(name.to_string());
        Ok(())
    }

    fn attach(&mut self, alias: &str, target: &str, options: &str) -> Result<()> {
        if self.attached.contains(alias) {
            return Ok(());
        }
        let opts = if options.is_empty() { "READ_ONLY".to_string() } else { format!("{}, READ_ONLY", options) };
        self.conn
            .execute_batch(&format!("ATTACH {} AS {} ({});", lit(target), ident(alias), opts))
            .map_err(|e| anyhow!("could not attach {}: {}", alias, e))?;
        self.attached.insert(alias.to_string());
        Ok(())
    }

    /// Credentials for httpfs come from env vars or the AWS credential chain, never from YAML.
    fn s3_secret(&mut self, src: &SourceDef) -> Result<()> {
        let alias = alias_for(&src.connection);
        if self.attached.contains(&format!("secret:{}", alias)) {
            return Ok(());
        }
        let mut opts: Vec<String> = vec!["TYPE S3".into()];
        if cfg_bool(src, "credential_chain") {
            self.load_extension("aws")?;
            opts.push("PROVIDER CREDENTIAL_CHAIN".into());
            if let Some(p) = cfg_str(src, "profile") { opts.push(format!("PROFILE {}", lit(&p))); }
        } else if let (Ok(k), Ok(s)) = (std::env::var("AWS_ACCESS_KEY_ID"), std::env::var("AWS_SECRET_ACCESS_KEY")) {
            opts.push(format!("KEY_ID {}", lit(&k)));
            opts.push(format!("SECRET {}", lit(&s)));
            if let Ok(t) = std::env::var("AWS_SESSION_TOKEN") { opts.push(format!("SESSION_TOKEN {}", lit(&t))); }
        }
        for (key, opt) in [("region", "REGION"), ("endpoint", "ENDPOINT"), ("url_style", "URL_STYLE")] {
            if let Some(v) = cfg_str(src, key).or_else(|| if key == "region" { std::env::var("AWS_REGION").ok() } else { None }) {
                opts.push(format!("{} {}", opt, lit(&v)));
            }
        }
        if let Some(v) = cfg_str(src, "use_ssl") { opts.push(format!("USE_SSL {}", v)); }
        if opts.len() > 1 {
            let scope = cfg_str(src, "path").or_else(|| cfg_str(src, "url"));
            let scope_sql = scope.map(|s| format!(", SCOPE {}", lit(&s))).unwrap_or_default();
            self.exec(&format!("CREATE OR REPLACE SECRET {} ({}{});", ident(&format!("stella_{}", alias)), opts.join(", "), scope_sql))?;
        }
        self.attached.insert(format!("secret:{}", alias));
        Ok(())
    }

    /// A FROM-able expression for one table of one source, attaching or loading what it needs.
    fn table_ref(&mut self, root: &Path, src: &SourceDef, table: &str) -> Result<String> {
        let kind = src.kind.to_ascii_lowercase();
        match kind.as_str() {
            "csv" => {
                let p = file_in(root, src.path.as_ref(), table, "csv");
                self.note_local(&p);
                Ok(format!("read_csv({}, header = true)", lit(&p.display().to_string())))
            }
            "parquet" => {
                let p = file_in(root, src.path.as_ref(), table, "parquet");
                self.note_local(&p);
                Ok(format!("read_parquet({})", lit(&p.display().to_string())))
            }
            "duckdb" => {
                let Some(file) = src.path.as_ref() else { bail!("source `{}` (duckdb) needs `path: <file>.duckdb`", src.connection) };
                let p = root.join(file);
                self.note_local(&p);
                let alias = alias_for(&src.connection);
                self.attach(&alias, &p.display().to_string(), "")?;
                let mut parts: Vec<&str> = vec![alias.as_str()];
                parts.extend(table.split('.'));
                Ok(dotted(&parts))
            }
            "postgres" => {
                let conn_str = if let Some(var) = cfg_str(src, "env") {
                    std::env::var(&var).map_err(|_| anyhow!("source `{}`: environment variable {} (the Postgres connection string) is not set", src.connection, var))?
                } else if let Some(cs) = cfg_str(src, "connection_string").or_else(|| cfg_str(src, "dsn")) {
                    expand_env(&cs)?
                } else {
                    bail!("source `{}` (postgres) needs `env: <VAR>` naming the environment variable that holds the libpq connection string", src.connection)
                };
                self.load_extension("postgres")?;
                self.remote = true;
                let alias = alias_for(&src.connection);
                self.attach(&alias, &conn_str, "TYPE postgres")?;
                let schema = cfg_str(src, "schema").unwrap_or_else(|| "public".into());
                let mut parts: Vec<&str> = vec![alias.as_str()];
                if table.contains('.') { parts.extend(table.split('.')) } else { parts.push(schema.as_str()); parts.push(table) }
                Ok(dotted(&parts))
            }
            "s3" | "httpfs" => {
                self.load_extension("httpfs")?;
                self.remote = true;
                let base = src.path.clone().or_else(|| cfg_str(src, "url")).or_else(|| cfg_str(src, "bucket").map(|b| format!("s3://{}", b)));
                let url = match base {
                    Some(b) => url_in(&expand_env(&b)?, table),
                    None if has_scheme(table) => table.to_string(),
                    None => bail!("source `{}` ({}) needs `path: s3://bucket/prefix` (or an https:// base URL)", src.connection, kind),
                };
                if url.starts_with("s3") || url.starts_with("r2") || url.starts_with("gs") || url.starts_with("gcs") {
                    self.s3_secret(src)?;
                }
                Ok(format!("read_parquet({})", lit(&url)))
            }
            other => bail!("kind `{}` is not readable here; agents query it with their own connector", other),
        }
    }

    fn columns_of(&self, view: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(&format!("DESCRIBE {}", ident(view)))?;
        let mut rows = stmt.query([])?;
        let mut cols = Vec::new();
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            cols.push(name);
        }
        Ok(cols)
    }

    fn free_view_name(&self, wanted: &str) -> Result<String> {
        if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(wanted)) {
            bail!("`{}` is the name of an ontology table, so it gets no view", wanted);
        }
        if self.views.values().any(|v| v.view.eq_ignore_ascii_case(wanted)) {
            bail!("a view named `{}` already exists", wanted);
        }
        Ok(wanted.to_string())
    }

    fn select_for(&mut self, root: &Path, src: &SourceDef, table: &str, columns: &IndexMap<String, String>) -> Result<(String, String)> {
        let from = self.table_ref(root, src, table)?;
        let renames: Vec<String> = columns
            .iter()
            .filter(|(prop, col)| !prop.eq_ignore_ascii_case(col))
            .map(|(prop, col)| format!("{} AS {}", ident(col), ident(prop)))
            .collect();
        let select = if renames.is_empty() {
            format!("SELECT * FROM {}", from)
        } else {
            format!("SELECT * RENAME ({}) FROM {}", renames.join(", "), from)
        };
        Ok((select, from))
    }

    fn type_view(&mut self, ont: &Ontology, t: &TypeDef) -> Result<()> {
        if let Some(b) = &t.source {
            let src = ont.sources.get(&b.connection).ok_or_else(|| anyhow!("unknown connection `{}`", b.connection))?;
            if !is_duck_kind(&src.kind) {
                self.unreadable.insert(t.name.clone(), src.kind.clone());
                return Ok(());
            }
            let view = self.free_view_name(&t.name)?;
            let root = self.root.clone();
            let (select, from) = self.select_for(&root, src, &b.table, &b.columns)?;
            self.exec(&format!("CREATE VIEW {} AS {}", ident(&view), select))?;
            let columns = self.columns_of(&view)?;
            self.views.insert(t.name.clone(), ViewInfo { view, source: from, columns });
            return Ok(());
        }
        if t.instances.is_empty() {
            return Ok(()); // a concept with no rows
        }
        let view = self.free_view_name(&t.name)?;
        let mut cols: Vec<String> = t.properties.keys().cloned().collect();
        for inst in &t.instances {
            for k in inst.keys() {
                if !cols.iter().any(|c| c == k) {
                    cols.push(k.clone());
                }
            }
        }
        let rows: Vec<String> = t
            .instances
            .iter()
            .map(|inst| format!("({})", cols.iter().map(|c| inst.get(c).map(yaml_lit).unwrap_or_else(|| "NULL".into())).collect::<Vec<_>>().join(", ")))
            .collect();
        self.exec(&format!(
            "CREATE TABLE {} AS SELECT * FROM (VALUES {}) AS t({})",
            ident(&view),
            rows.join(", "),
            cols.iter().map(|c| ident(c)).collect::<Vec<_>>().join(", ")
        ))?;
        let columns = self.columns_of(&view)?;
        self.views.insert(t.name.clone(), ViewInfo { view, source: format!("inline instances in {}", t.file), columns });
        Ok(())
    }

    fn rel_view(&mut self, ont: &Ontology, rel: &Relationship) -> Result<()> {
        let Some(b) = &rel.source else { return Ok(()) };
        let src = ont.sources.get(&b.connection).ok_or_else(|| anyhow!("unknown connection `{}`", b.connection))?;
        if !is_duck_kind(&src.kind) {
            self.unreadable.insert(rel.qualified(), src.kind.clone());
            return Ok(());
        }
        let view = self.free_view_name(&rel.name).or_else(|_| self.free_view_name(&format!("{}_{}", rel.from, rel.name)))?;
        let root = self.root.clone();
        let (select, from) = self.select_for(&root, src, &b.table, &b.columns)?;
        self.exec(&format!("CREATE VIEW {} AS {}", ident(&view), select))?;
        let columns = self.columns_of(&view)?;
        self.views.insert(rel.qualified(), ViewInfo { view, source: from, columns });
        Ok(())
    }

    fn json_type(&self) -> &'static str {
        if self.has_json { "JSON" } else { "VARCHAR" }
    }

    fn json_lit(&self, v: &Json) -> String {
        let l = lit(&v.to_string());
        if self.has_json { format!("{}::JSON", l) } else { l }
    }

    fn insert_rows(&self, table: &str, rows: Vec<String>) -> Result<()> {
        for chunk in rows.chunks(500) {
            self.exec(&format!("INSERT INTO {} VALUES {};", table, chunk.join(", ")))?;
        }
        Ok(())
    }

    /// The ontology itself as tables, so SQL can join rules to rows.
    fn ontology_tables(&self, ont: &Ontology) -> Result<()> {
        let j = self.json_type();
        self.exec(&format!(
            "CREATE TABLE nodes (name VARCHAR, kind VARCHAR, domain VARCHAR, description VARCHAR, key_property VARCHAR, owner VARCHAR, tags VARCHAR[], connection VARCHAR, source_kind VARCHAR, source_table VARCHAR, readable_here BOOLEAN, inline_rows INTEGER, view_name VARCHAR, file VARCHAR, properties {j});
             CREATE TABLE properties (type_name VARCHAR, name VARCHAR, kind VARCHAR, is_key BOOLEAN, required BOOLEAN, description VARCHAR, unit VARCHAR, allowed_values {j}, column_name VARCHAR);
             CREATE TABLE edges (relationship VARCHAR, qualified VARCHAR, from_type VARCHAR, to_type VARCHAR, cardinality VARCHAR, description VARCHAR, connection VARCHAR, source_kind VARCHAR, source_table VARCHAR, from_key VARCHAR, to_key VARCHAR, view_name VARCHAR, file VARCHAR, properties {j});
             CREATE TABLE constraints (id VARCHAR, name VARCHAR, applies_to VARCHAR, applies_to_kind VARCHAR, rule VARCHAR, severity VARCHAR, rationale VARCHAR, description VARCHAR, owner VARCHAR, tags VARCHAR[], sql VARCHAR, file VARCHAR);
             CREATE TABLE actions (name VARCHAR, description VARCHAR, inputs {j}, effects {j}, checks VARCHAR[], requires_approval_from VARCHAR, side_effects VARCHAR, tags VARCHAR[], file VARCHAR);
             CREATE TABLE sources (connection VARCHAR, kind VARCHAR, path VARCHAR, description VARCHAR, readable_here BOOLEAN, config {j}, file VARCHAR);"
        ))?;

        let mut nodes = Vec::new();
        let mut props = Vec::new();
        for t in ont.types.values() {
            let (conn, skind, stable) = match &t.source {
                Some(s) => (lit(&s.connection), lit_opt(&ont.sources.get(&s.connection).map(|x| x.kind.clone()).unwrap_or_default()), lit(&s.table)),
                None => ("NULL".into(), "NULL".into(), "NULL".into()),
            };
            let view = self.views.get(&t.name);
            let readable = view.is_some() || !t.instances.is_empty();
            let plist: Vec<Json> = t
                .properties
                .iter()
                .map(|(n, p)| {
                    let p = p.normalize();
                    let mut j = json!({ "name": n, "type": p.kind });
                    if p.key { j["key"] = json!(true); }
                    if !p.description.is_empty() { j["description"] = json!(p.description); }
                    if !p.values.is_empty() { j["values"] = serde_json::to_value(&p.values).unwrap_or(Json::Null); }
                    if let Some(u) = &p.unit { j["unit"] = json!(u); }
                    j
                })
                .collect();
            nodes.push(format!(
                "({}, 'type', {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                lit(&t.name),
                lit_opt(&ont.domain_of(&t.name).unwrap_or_default()),
                lit(&t.description.trim()),
                lit_opt(&t.key_property().unwrap_or_default()),
                lit_opt(&t.owner),
                lit_list(&t.tags),
                conn,
                skind,
                stable,
                readable,
                t.instances.len(),
                view.map(|v| lit(&v.view)).unwrap_or_else(|| "NULL".into()),
                lit(&t.file),
                self.json_lit(&Json::Array(plist)),
            ));
            for (n, p) in &t.properties {
                let p = p.normalize();
                let col = t.source.as_ref().and_then(|s| s.columns.get(n)).cloned().unwrap_or_else(|| n.clone());
                props.push(format!(
                    "({}, {}, {}, {}, {}, {}, {}, {}, {})",
                    lit(&t.name),
                    lit(n),
                    lit(&p.kind),
                    p.key,
                    p.required,
                    lit(&p.description),
                    p.unit.as_deref().map(lit).unwrap_or_else(|| "NULL".into()),
                    if p.values.is_empty() { "NULL".into() } else { self.json_lit(&serde_json::to_value(&p.values).unwrap_or(Json::Null)) },
                    lit(&col),
                ));
            }
        }
        self.insert_rows("nodes", nodes)?;
        self.insert_rows("properties", props)?;

        let edges: Vec<String> = ont
            .relationships
            .iter()
            .map(|r| {
                let (conn, skind, stable, fk, tk) = match &r.source {
                    Some(s) => (
                        lit(&s.connection),
                        lit_opt(&ont.sources.get(&s.connection).map(|x| x.kind.clone()).unwrap_or_default()),
                        lit(&s.table),
                        lit(&s.from_key),
                        lit(&s.to_key),
                    ),
                    None => ("NULL".into(), "NULL".into(), "NULL".into(), "NULL".into(), "NULL".into()),
                };
                let plist: Vec<Json> = r.properties.iter().map(|(n, p)| json!({ "name": n, "type": p.kind })).collect();
                format!(
                    "({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    lit(&r.name),
                    lit(&r.qualified()),
                    lit(&r.from),
                    lit(&r.to),
                    lit_opt(&r.cardinality),
                    lit(r.description.trim()),
                    conn,
                    skind,
                    stable,
                    fk,
                    tk,
                    self.views.get(&r.qualified()).map(|v| lit(&v.view)).unwrap_or_else(|| "NULL".into()),
                    lit_opt(&r.file),
                    self.json_lit(&Json::Array(plist)),
                )
            })
            .collect();
        self.insert_rows("edges", edges)?;

        let constraints: Vec<String> = ont
            .constraints
            .values()
            .map(|c| {
                format!(
                    "({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    lit(&c.id),
                    lit(&c.name),
                    lit(&c.applies_to),
                    lit_opt(&ont.kind_of(&c.applies_to).map(|k| format!("{:?}", k).to_lowercase()).unwrap_or_default()),
                    lit(&c.rule),
                    lit(&c.severity.to_string()),
                    lit(c.rationale.trim()),
                    lit(c.description.trim()),
                    lit_opt(&c.owner),
                    lit_list(&c.tags),
                    c.sql.as_deref().map(lit).unwrap_or_else(|| "NULL".into()),
                    lit(&c.file),
                )
            })
            .collect();
        self.insert_rows("constraints", constraints)?;

        let actions: Vec<String> = ont
            .actions
            .values()
            .map(|a| {
                format!(
                    "({}, {}, {}, {}, {}, {}, {}, {}, {})",
                    lit(&a.name),
                    lit(a.description.trim()),
                    self.json_lit(&serde_json::to_value(&a.inputs).unwrap_or(Json::Null)),
                    self.json_lit(&serde_json::to_value(&a.effects).unwrap_or(Json::Null)),
                    lit_list(&a.checks),
                    lit_opt(&a.requires_approval_from),
                    lit_opt(&a.side_effects),
                    lit_list(&a.tags),
                    lit(&a.file),
                )
            })
            .collect();
        self.insert_rows("actions", actions)?;

        let sources: Vec<String> = ont
            .sources
            .values()
            .map(|s| {
                format!(
                    "({}, {}, {}, {}, {}, {}, {})",
                    lit(&s.connection),
                    lit(&s.kind),
                    s.path.as_deref().map(lit).unwrap_or_else(|| "NULL".into()),
                    lit(s.description.trim()),
                    is_duck_kind(&s.kind),
                    self.json_lit(&serde_json::to_value(&s.config).unwrap_or(Json::Null)),
                    lit(&s.file),
                )
            })
            .collect();
        self.insert_rows("sources", sources)?;
        Ok(())
    }
}

fn truncate_sql(sql: &str) -> String {
    let s: String = sql.chars().take(120).collect();
    if s.len() < sql.len() { format!("{}…", s) } else { s }
}

// ---------------- the open database ----------------

impl Db {
    /// Open an in-memory DuckDB, build one view per data-bound type and
    /// relationship (best effort; failures land in `skipped`), then load the
    /// ontology tables.
    pub fn open(ont: &Ontology) -> Result<Db> {
        let conn = Connection::open_in_memory().context("opening DuckDB")?;
        let has_json = conn.query_row("SELECT to_json(1)::VARCHAR", [], |_| Ok(())).is_ok();
        let root = std::fs::canonicalize(&ont.root).unwrap_or_else(|_| ont.root.clone());
        let mut b = Builder { conn: &conn, has_json, attached: HashSet::new(), loaded: HashSet::new(), views: IndexMap::new(), skipped: IndexMap::new(), unreadable: IndexMap::new(), root, local_dirs: HashSet::new(), remote: false };
        for t in ont.types.values() {
            if let Err(e) = b.type_view(ont, t) {
                b.skipped.insert(t.name.clone(), format!("{:#}", e));
            }
        }
        for r in &ont.relationships {
            if r.source.is_none() {
                continue;
            }
            if let Err(e) = b.rel_view(ont, r) {
                b.skipped.insert(r.qualified(), format!("{:#}", e));
            }
        }
        b.ontology_tables(ont)?;
        // The fence. Everything the views need is set up; from here on a query
        // may read only the folders those views point at: no other file on
        // the machine, no extension install, no network. `lock_configuration`
        // stops a query from switching it back on.
        let fenced = if b.remote {
            false
        } else {
            let mut dirs: Vec<String> = b.local_dirs.iter().map(|d| format!("{}/", d.trim_end_matches('/'))).collect();
            dirs.sort();
            let sql = format!("SET allowed_directories = {}; SET enable_external_access = false; SET lock_configuration = true;", lit_list(&dirs));
            match conn.execute_batch(&sql) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("warn  DuckDB could not be fenced to the data folders: {}", e);
                    false
                }
            }
        };
        let (views, skipped, unreadable) = (b.views, b.skipped, b.unreadable);
        Ok(Db { conn: Mutex::new(conn), fenced, views, skipped, unreadable, has_json })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn version(&self) -> String {
        self.conn().version().unwrap_or_else(|_| "unknown".into())
    }

    /// The view for a type name or a `From.relationship` qualified name.
    pub fn view(&self, name: &str) -> Option<&ViewInfo> {
        self.views.get(name)
    }

    /// Why a type or relationship has no view, if it has none.
    pub fn view_error(&self, name: &str) -> Option<String> {
        if let Some(why) = self.skipped.get(name) {
            return Some(why.clone());
        }
        self.unreadable.get(name).map(|kind| format!("kind `{}` is not readable here; agents query it with their own connector", kind))
    }

    /// (ontology name, view name, source) for everything SQL can see.
    pub fn views(&self) -> Vec<(String, String, String)> {
        self.views.iter().map(|(k, v)| (k.clone(), v.view.clone(), v.source.clone())).collect()
    }

    fn view_info(&self, name: &str) -> Result<&ViewInfo> {
        self.views.get(name).ok_or_else(|| match self.view_error(name) {
            Some(why) => anyhow!("{} is not readable here: {}", name, why),
            None => anyhow!("{} has no data binding DuckDB can read", name),
        })
    }

    /// Up to `limit` rows of a type or relationship view.
    pub fn rows(&self, name: &str, limit: usize) -> Result<Vec<Record>> {
        let v = self.view_info(name)?;
        Ok(self.select(&format!("SELECT * FROM {}", ident(&v.view)), limit)?.rows)
    }

    /// The first row whose key column (first of `key_cols` that exists) equals `key`, case-insensitively.
    pub fn find(&self, name: &str, key_cols: &[String], key: &str) -> Result<Option<Record>> {
        let v = self.view_info(name)?;
        for col in key_cols {
            let Some(actual) = v.columns.iter().find(|c| c.eq_ignore_ascii_case(col)) else { continue };
            let sql = format!("SELECT * FROM {} WHERE lower(CAST({} AS VARCHAR)) = lower({})", ident(&v.view), ident(actual), lit(key));
            if let Some(rec) = self.select(&sql, 1)?.rows.into_iter().next() {
                return Ok(Some(rec));
            }
        }
        Ok(None)
    }

    pub fn count(&self, name: &str) -> Result<u64> {
        let v = self.view_info(name)?;
        let n: i64 = self.conn().query_row(&format!("SELECT count(*) FROM {}", ident(&v.view)), [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    }

    /// The public door: one read-only statement, rows as JSON, capped at `MAX_ROWS`.
    pub fn query(&self, sql: &str, limit: usize) -> Result<QueryResult> {
        let sql = check_read_only(sql)?;
        let limit = if limit == 0 { MAX_ROWS } else { limit.min(MAX_ROWS) };
        let mut r = self.select(&sql, limit)?;
        // only real failures are worth a warning; kinds DuckDB never reads are listed by `tables`
        for (name, why) in &self.skipped {
            r.warnings.push(format!("{} could not be attached: {}", name, why));
        }
        Ok(r)
    }

    /// Column names and DuckDB types of a SELECT, without running it.
    fn describe(&self, conn: &Connection, sql: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM ({}) AS q", sql)).map_err(|e| anyhow!("{}", e))?;
        let mut rows = stmt.query([]).map_err(|e| anyhow!("{}", e))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            let ty: String = row.get(1)?;
            out.push((name, ty));
        }
        Ok(out)
    }

    /// Run a SELECT and return JSON rows. Exotic types are cast in SQL so the
    /// Rust side only ever sees NULL, booleans, integers, floats and text.
    fn select(&self, sql: &str, limit: usize) -> Result<QueryResult> {
        let started = Instant::now();
        let conn = self.conn();
        // DuckDB renames duplicate output columns (a, a_1) inside the subquery, so names are unique here.
        let desc = self.describe(&conn, sql)?;
        let mut exprs = Vec::new();
        let mut parse_json = Vec::new();
        for (name, ty) in &desc {
            let (e, p) = self.wrap_expr(name, ty);
            exprs.push(e);
            parse_json.push(p);
        }
        let final_sql = format!("SELECT {} FROM ({}) AS q LIMIT {}", exprs.join(", "), sql, limit + 1);
        let mut stmt = conn.prepare(&final_sql).map_err(|e| anyhow!("{}", e))?;
        let mut rows = stmt.query([]).map_err(|e| anyhow!("{}", e))?;
        let mut out: Vec<Record> = Vec::new();
        let mut truncated = false;
        while let Some(row) = rows.next()? {
            if out.len() >= limit {
                truncated = true;
                break;
            }
            let mut rec = Record::new();
            for (i, (name, _)) in desc.iter().enumerate() {
                let mut v = value_to_json(row.get_ref(i)?);
                if parse_json[i] {
                    if let Json::String(s) = &v {
                        if let Ok(parsed) = serde_json::from_str::<Json>(s) {
                            v = parsed;
                        }
                    }
                }
                rec.insert(name.clone(), v);
            }
            out.push(rec);
        }
        Ok(QueryResult {
            columns: desc.into_iter().map(|(name, kind)| Column { name, kind }).collect(),
            row_count: out.len(),
            rows: out,
            truncated,
            limit,
            elapsed_ms: started.elapsed().as_millis(),
            warnings: Vec::new(),
        })
    }

    /// Select-list expression for a column, plus whether the text should be parsed as JSON.
    fn wrap_expr(&self, name: &str, ty: &str) -> (String, bool) {
        let col = ident(name);
        let up = ty.trim().to_ascii_uppercase();
        let base = up.split('(').next().unwrap_or("").trim().to_string();
        let nested = up.ends_with(']') || base.starts_with("STRUCT") || base.starts_with("MAP") || base.starts_with("UNION");
        if nested {
            return if self.has_json {
                (format!("CAST(to_json({}) AS VARCHAR) AS {}", col, col), true)
            } else {
                (format!("CAST({} AS VARCHAR) AS {}", col, col), false)
            };
        }
        match base.as_str() {
            "BOOLEAN" | "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "UTINYINT" | "USMALLINT" | "UINTEGER" | "UBIGINT" | "FLOAT" | "DOUBLE" | "VARCHAR" => (col, false),
            "JSON" => (format!("CAST({} AS VARCHAR) AS {}", col, col), true),
            "DECIMAL" => (format!("CAST({} AS DOUBLE) AS {}", col, col), false),
            _ => (format!("CAST({} AS VARCHAR) AS {}", col, col), false),
        }
    }
}

fn value_to_json(v: ValueRef<'_>) -> Json {
    match v {
        ValueRef::Null => Json::Null,
        ValueRef::Boolean(b) => json!(b),
        ValueRef::TinyInt(i) => json!(i),
        ValueRef::SmallInt(i) => json!(i),
        ValueRef::Int(i) => json!(i),
        ValueRef::BigInt(i) => json!(i),
        ValueRef::HugeInt(i) => i64::try_from(i).map(|x| json!(x)).unwrap_or_else(|_| json!(i.to_string())),
        ValueRef::UHugeInt(i) => u64::try_from(i).map(|x| json!(x)).unwrap_or_else(|_| json!(i.to_string())),
        ValueRef::UTinyInt(i) => json!(i),
        ValueRef::USmallInt(i) => json!(i),
        ValueRef::UInt(i) => json!(i),
        ValueRef::UBigInt(i) => json!(i),
        ValueRef::Float(f) => json!(f),
        ValueRef::Double(f) => json!(f),
        ValueRef::Text(b) => json!(String::from_utf8_lossy(b)),
        ValueRef::Blob(b) => json!(format!("<{} bytes>", b.len())),
        _ => json!("<unsupported value>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader;

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stella-duck-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn guard_accepts_only_one_select() {
        assert!(check_read_only("select 1").is_ok());
        assert!(check_read_only("  WITH x AS (select 1) select * from x; ").is_ok());
        assert!(check_read_only("select 1 -- trailing comment\n").is_ok());
        assert!(check_read_only("select ';' as s").is_ok());
        assert!(check_read_only("select 1; drop table nodes").is_err());
        assert!(check_read_only("drop table nodes").is_err());
        assert!(check_read_only("insert into nodes values (1)").is_err());
        assert!(check_read_only("/* select */ copy nodes to 'x.csv'").is_err());
        assert!(check_read_only("").is_err());
    }

    #[test]
    fn env_expansion_names_missing_vars() {
        std::env::set_var("STELLA_TEST_VAR", "ok");
        assert_eq!(expand_env("a ${STELLA_TEST_VAR} b").unwrap(), "a ok b");
        let e = expand_env("${STELLA_DEFINITELY_UNSET}").unwrap_err().to_string();
        assert!(e.contains("STELLA_DEFINITELY_UNSET"));
    }

    /// CSV, a `.duckdb` file with a renamed column, and inline instances all become views;
    /// the ontology tables join to them; the guard and the row cap hold.
    #[test]
    fn views_and_ontology_tables() {
        let d = tmp("views");
        write(&d, "ontology.yaml", "name: t\n");
        write(&d, "data/suppliers.csv", "supplier_id,name,tier\nV-1,Acme,1\nV-2,Beta,3\n");
        // a real .duckdb file with a differently named key column
        let dbfile = d.join("data/erp.duckdb");
        {
            let c = Connection::open(&dbfile).unwrap();
            c.execute_batch("CREATE TABLE parts (part_no VARCHAR, name VARCHAR, safety_critical BOOLEAN, unit_cost DECIMAL(10,2), since DATE, tags VARCHAR[]);
                             INSERT INTO parts VALUES ('P-1', 'Caliper', true, 48.50, DATE '2021-03-01', ['brake','core']), ('P-2', 'Bolt', false, 0.10, NULL, []);").unwrap();
        }
        write(&d, "sources/s.yaml", "sources:\n  - { connection: files, kind: csv, path: data }\n  - { connection: erp, kind: duckdb, path: data/erp.duckdb }\n  - { connection: wh, kind: snowflake }\n");
        write(
            &d,
            "types/t.yaml",
            "types:\n  - type: Supplier\n    properties: { supplier_id: { type: string, key: true }, name: string, tier: { type: enum, values: [1,2,3] } }\n    source: { connection: files, table: suppliers, key: supplier_id }\n    tags: [core]\n  - type: Part\n    properties: { part_id: { type: string, key: true }, name: string, safety_critical: boolean, unit_cost: number, since: date }\n    source: { connection: erp, table: parts, key: part_id, columns: { part_id: part_no } }\n  - type: Region\n    properties: { code: { type: string, key: true }, name: string }\n    instances:\n      - { code: EMEA, name: Europe }\n      - { code: APAC, name: Asia Pacific }\n  - type: PurchaseOrder\n    properties: { po_id: { type: string, key: true } }\n    source: { connection: wh, table: PO }\n",
        );
        write(&d, "constraints/c.yaml", "constraints:\n  - { id: R-1, name: tier one, applies_to: Supplier, rule: tier == 1, severity: block, rationale: because }\n");
        let ont = Ontology::from_raw(loader::load(&d).unwrap());
        let db = Db::open(&ont).unwrap();

        assert!(db.view("Supplier").is_some());
        assert!(db.view("Part").is_some());
        assert!(db.view("Region").is_some());
        assert!(db.view("PurchaseOrder").is_none());
        assert!(db.view_error("PurchaseOrder").unwrap().contains("snowflake"));
        assert!(db.skipped.is_empty(), "{:?}", db.skipped);

        // the fence: a query reaches the data folder and nothing else on the machine
        assert!(db.fenced);
        let outside = if cfg!(windows) { "C:/Windows/win.ini" } else { "/etc/passwd" };
        let err = db.query(&format!("select * from read_csv_auto('{}')", outside), 5).unwrap_err().to_string();
        assert!(err.to_lowercase().contains("permission") || err.to_lowercase().contains("disabled") || err.to_lowercase().contains("not allowed"), "{}", err);
        assert!(db.query(&format!("select * from read_text('{}')", outside), 5).is_err());
        assert!(db.query("select count(*) as n from Supplier", 5).is_ok(), "the data folder itself still works");
        assert!(db.query("install httpfs", 5).is_err());

        // renamed key column, decimal → number, date and list → text/json
        let p = db.find("Part", &["part_id".into()], "p-1").unwrap().unwrap();
        assert_eq!(p["part_id"], json!("P-1"));
        assert_eq!(p["unit_cost"], json!(48.5));
        assert_eq!(p["since"], json!("2021-03-01"));
        assert_eq!(p["tags"], json!(["brake", "core"]));
        assert!(db.find("Part", &["part_id".into()], "nope").unwrap().is_none());
        assert_eq!(db.count("Supplier").unwrap(), 2);
        assert_eq!(db.rows("Region", 10).unwrap().len(), 2);

        // join rules to rows through the ontology tables
        let r = db
            .query("select s.name, c.id from Supplier s join nodes n on n.name = 'Supplier' join constraints c on c.applies_to = n.name where s.tier = 1", 10)
            .unwrap();
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0]["name"], json!("Acme"));
        assert_eq!(r.rows[0]["id"], json!("R-1"));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let n = db.query("select name, tags, properties from nodes where name = 'Supplier'", 10).unwrap();
        assert_eq!(n.rows[0]["tags"], json!(["core"]));
        assert_eq!(n.rows[0]["properties"][0]["name"], json!("supplier_id"));
        assert_eq!(db.query("select count(*) as n from edges", 10).unwrap().rows[0]["n"], json!(0));
        assert_eq!(db.query("select kind, readable_here from sources where connection = 'wh'", 10).unwrap().rows[0]["readable_here"], json!(false));

        // the cap and the guard
        let big = db.query("select range as i from range(2000)", 0).unwrap();
        assert_eq!(big.rows.len(), MAX_ROWS);
        assert!(big.truncated);
        assert!(db.query("drop table nodes", 10).is_err());
        assert!(db.query("select 1; select 2", 10).is_err());
        let dup = db.query("select a, a from (select 1 a)", 10).unwrap();
        assert_eq!(dup.columns.len(), 2);
        assert_eq!(dup.rows[0]["a"], json!(1));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn postgres_needs_its_env_var() {
        let d = tmp("pg");
        write(&d, "ontology.yaml", "name: t\n");
        write(&d, "sources/s.yaml", "connection: pg\nkind: postgres\nenv: STELLA_TEST_PG_URL_UNSET\n");
        write(&d, "types/t.yaml", "type: Order\nproperties: { id: { type: string, key: true } }\nsource: { connection: pg, table: orders }\n");
        let ont = Ontology::from_raw(loader::load(&d).unwrap());
        let db = Db::open(&ont).unwrap();
        let why = db.view_error("Order").unwrap();
        assert!(why.contains("STELLA_TEST_PG_URL_UNSET"), "{}", why);
        let _ = std::fs::remove_dir_all(&d);
    }
}
