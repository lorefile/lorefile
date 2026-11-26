//! The data door. Inline `instances:` and `csv` connections are read in plain
//! Rust so `validate` can check real rows with no engine at all. Every other
//! readable kind (`parquet`, `duckdb`, `postgres`, `s3`/`httpfs`) goes through
//! DuckDB (`duck.rs`), opened lazily on first use and shared for the life of the
//! process. The interface is the same whichever path a type takes.

use crate::ontology::Ontology;
use crate::schema::*;
use anyhow::{anyhow, Context, Result};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

#[cfg(feature = "duckdb")]
use crate::duck::{self, Db};
#[cfg(feature = "duckdb")]
use std::{cell::OnceCell, sync::Arc};

pub type Record = IndexMap<String, Json>;

/// Hard cap on rows a `query` returns.
pub const MAX_QUERY_ROWS: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// What `query` returns: rows as JSON objects, capped.
#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Record>,
    pub row_count: usize,
    pub truncated: bool,
    pub limit: usize,
    pub elapsed_ms: u128,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub struct DataLayer<'a> {
    ont: &'a Ontology,
    cache: RefCell<HashMap<String, Vec<Record>>>,
    #[cfg(feature = "duckdb")]
    db: OnceCell<std::result::Result<Arc<Db>, String>>,
}

fn yaml_to_json(v: &serde_yaml::Value) -> Json {
    serde_json::to_value(v).unwrap_or(Json::Null)
}

/// Coerce a raw string (from CSV) into the JSON type the property declares.
pub fn coerce(raw: &str, prop: &Property) -> Json {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("null") || s.eq_ignore_ascii_case("na") {
        return Json::Null;
    }
    let as_int = || s.parse::<i64>().ok().map(Json::from);
    let as_float = || s.parse::<f64>().ok().map(|f| json!(f));
    let as_bool = || match s.to_lowercase().as_str() {
        "true" | "yes" | "y" | "1" => Some(Json::Bool(true)),
        "false" | "no" | "n" | "0" => Some(Json::Bool(false)),
        _ => None,
    };
    match prop.kind.as_str() {
        "integer" | "int" => as_int().or_else(as_float).unwrap_or_else(|| json!(s)),
        "number" | "float" | "decimal" => as_float().unwrap_or_else(|| json!(s)),
        "boolean" | "bool" => as_bool().unwrap_or_else(|| json!(s)),
        "json" => serde_json::from_str(s).unwrap_or_else(|_| json!(s)),
        "enum" => {
            let all_ints = !prop.values.is_empty() && prop.values.iter().all(|v| matches!(v, serde_yaml::Value::Number(n) if n.is_i64()));
            let all_bools = !prop.values.is_empty() && prop.values.iter().all(|v| matches!(v, serde_yaml::Value::Bool(_)));
            if all_ints {
                as_int().unwrap_or_else(|| json!(s))
            } else if all_bools {
                as_bool().unwrap_or_else(|| json!(s))
            } else {
                json!(s)
            }
        }
        _ => json!(s),
    }
}

/// Align a typed value (from Parquet, DuckDB, Postgres…) with the property's
/// declared kind so both data paths hand rules identical records: strings are
/// coerced like CSV cells, `number` properties are always floats, `integer`
/// properties are integers when the value is whole.
#[cfg(feature = "duckdb")]
pub fn coerce_json(v: Json, prop: &Property) -> Json {
    match (&v, prop.kind.as_str()) {
        (Json::String(s), _) => coerce(s, prop),
        (Json::Number(n), "number" | "float" | "decimal") => n.as_f64().map(|f| json!(f)).unwrap_or(v),
        (Json::Number(n), "integer" | "int") => match n.as_f64() {
            Some(f) if n.as_i64().is_none() && f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
            _ => v,
        },
        (Json::Number(n), "enum") => {
            let all_ints = !prop.values.is_empty() && prop.values.iter().all(|x| matches!(x, serde_yaml::Value::Number(m) if m.is_i64()));
            match (all_ints, n.as_f64()) {
                (true, Some(f)) if n.as_i64().is_none() && f.fract() == 0.0 => json!(f as i64),
                _ => v,
            }
        }
        _ => v,
    }
}

/// Is this source kind read through DuckDB (everything readable except csv)?
fn duck_kind(kind: &str) -> bool {
    #[cfg(feature = "duckdb")]
    {
        !kind.eq_ignore_ascii_case("csv") && duck::is_duck_kind(kind)
    }
    #[cfg(not(feature = "duckdb"))]
    {
        let _ = kind;
        false
    }
}

/// Rename columns to the property names they map to (case-insensitively) and
/// coerce values to the declared kinds. Extra columns are kept as they are.
#[cfg(feature = "duckdb")]
fn normalise(rec: Record, props: &IndexMap<String, Property>) -> Record {
    let mut out = Record::new();
    let mut used: Vec<String> = Vec::new();
    for (p, prop) in props {
        if let Some((k, v)) = rec.iter().find(|(k, _)| k.eq_ignore_ascii_case(p)) {
            out.insert(p.clone(), coerce_json(v.clone(), prop));
            used.push(k.clone());
        }
    }
    for (k, v) in rec {
        if !used.contains(&k) {
            out.insert(k, v);
        }
    }
    out
}

impl<'a> DataLayer<'a> {
    pub fn new(ont: &'a Ontology) -> Self {
        DataLayer {
            ont,
            cache: RefCell::new(HashMap::new()),
            #[cfg(feature = "duckdb")]
            db: OnceCell::new(),
        }
    }

    /// Reuse an already opened DuckDB (the web server shares one across requests).
    #[cfg(feature = "duckdb")]
    pub fn with_db(ont: &'a Ontology, db: Arc<Db>) -> Self {
        let layer = Self::new(ont);
        let _ = layer.db.set(Ok(db));
        layer
    }

    /// The process-wide DuckDB, opened on first use. Opening never retries:
    /// a failure is remembered and reported on every later call.
    #[cfg(feature = "duckdb")]
    pub fn db(&self) -> Result<Arc<Db>> {
        self.db
            .get_or_init(|| Db::open(self.ont).map(Arc::new).map_err(|e| format!("{:#}", e)))
            .clone()
            .map_err(|e| anyhow!("DuckDB is unavailable: {}", e))
    }

    /// Run one read-only SELECT/WITH statement over the ontology tables and the
    /// data views. Rows come back as JSON objects, at most `MAX_QUERY_ROWS`.
    #[cfg(feature = "duckdb")]
    pub fn query(&self, sql: &str, limit: usize) -> Result<QueryResult> {
        self.db()?.query(sql, limit)
    }

    #[cfg(not(feature = "duckdb"))]
    pub fn query(&self, _sql: &str, _limit: usize) -> Result<QueryResult> {
        anyhow::bail!("this binary was built without the `duckdb` feature; rebuild with default features to use query")
    }

    fn file_path(&self, src: &SourceDef, table: &str) -> PathBuf {
        let folder = src.path.clone().unwrap_or_else(|| "data".into());
        let mut p = self.ont.root.join(folder);
        if table.ends_with(".csv") {
            p.push(table);
        } else {
            p.push(format!("{}.csv", table));
        }
        p
    }

    fn props_of(t: &TypeDef) -> IndexMap<String, Property> {
        t.properties.iter().map(|(k, v)| (k.clone(), v.normalize())).collect()
    }

    /// The source a type is bound to, if the binding resolves.
    fn binding_of<'b>(&'b self, t: &'b TypeDef) -> Option<(&'b SourceDef, &'b TypeSource)> {
        let b = t.source.as_ref()?;
        self.ont.sources.get(&b.connection).map(move |s| (s, b))
    }

    /// Can rows of this type be read here (inline, csv, or a kind DuckDB reads)?
    pub fn is_local(&self, t: &TypeDef) -> bool {
        if !t.instances.is_empty() {
            return true;
        }
        match self.binding_of(t) {
            Some((src, _)) => src.kind.eq_ignore_ascii_case("csv") || duck_kind(&src.kind),
            None => false,
        }
    }

    fn inline_rows(t: &TypeDef) -> Vec<Record> {
        t.instances
            .iter()
            .map(|inst| inst.iter().map(|(k, v)| (k.clone(), yaml_to_json(v))).collect())
            .collect()
    }

    fn csv_rows(&self, src: &SourceDef, binding: &TypeSource, props: &IndexMap<String, Property>) -> Result<Vec<Record>> {
        let path = self.file_path(src, &binding.table);
        let mut rdr = csv::ReaderBuilder::new()
            .trim(csv::Trim::All)
            .flexible(true)
            .from_path(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_string()).collect();
        // property → column (default: same name)
        let col_for: IndexMap<String, String> = props
            .keys()
            .map(|p| (p.clone(), binding.columns.get(p).cloned().unwrap_or_else(|| p.clone())))
            .collect();
        let mut rows = Vec::new();
        for result in rdr.records() {
            let row = result?;
            let get = |col: &str| -> Option<&str> {
                headers.iter().position(|h| h.eq_ignore_ascii_case(col)).and_then(|i| row.get(i))
            };
            let mut rec = Record::new();
            for (p, col) in &col_for {
                if let Some(raw) = get(col) {
                    rec.insert(p.clone(), coerce(raw, &props[p]));
                }
            }
            // keep extra columns too, as strings, so rules can use them
            for (i, h) in headers.iter().enumerate() {
                if !col_for.values().any(|c| c.eq_ignore_ascii_case(h)) {
                    if let Some(raw) = row.get(i) {
                        rec.insert(h.clone(), json!(raw));
                    }
                }
            }
            rows.push(rec);
        }
        Ok(rows)
    }

    /// All rows for a type: inline instances first, then its source if readable here.
    pub fn instances(&self, type_name: &str) -> Result<Vec<Record>> {
        let t = self.ont.type_(type_name).ok_or_else(|| anyhow!("unknown type {}", type_name))?;
        if let Some(rows) = self.cache.borrow().get(&t.name) {
            return Ok(rows.clone());
        }
        let props = Self::props_of(t);
        let mut rows = Self::inline_rows(t);
        if let Some((src, binding)) = self.binding_of(t) {
            if src.kind.eq_ignore_ascii_case("csv") {
                rows.extend(self.csv_rows(src, binding, &props)?);
            } else if duck_kind(&src.kind) {
                #[cfg(feature = "duckdb")]
                {
                    let db = self.db()?;
                    rows.extend(db.rows(&t.name, duck::MAX_INSTANCE_ROWS)?.into_iter().map(|r| normalise(r, &props)));
                }
            }
        }
        self.cache.borrow_mut().insert(t.name.clone(), rows.clone());
        Ok(rows)
    }

    /// Find one row by its key value (or by `name` / `id` as a fallback).
    pub fn find(&self, type_name: &str, key: &str) -> Result<Option<Record>> {
        let t = self.ont.type_(type_name).ok_or_else(|| anyhow!("unknown type {}", type_name))?;
        let key_prop = t.key_property();
        let matches = |v: &Json| -> bool {
            match v {
                Json::String(s) => s.eq_ignore_ascii_case(key),
                Json::Number(n) => n.to_string() == key,
                _ => false,
            }
        };
        let scan = |rows: &[Record]| -> Option<Record> {
            if let Some(kp) = &key_prop {
                if let Some(r) = rows.iter().find(|r| r.get(kp).map(matches).unwrap_or(false)) {
                    return Some(r.clone());
                }
            }
            rows.iter()
                .find(|r| r.get("name").map(matches).unwrap_or(false) || r.get("id").map(matches).unwrap_or(false))
                .cloned()
        };

        #[cfg(feature = "duckdb")]
        if let Some((src, binding)) = self.binding_of(t) {
            if duck_kind(&src.kind) {
                // inline rows first, then one indexed lookup instead of loading the table
                if let Some(r) = scan(&Self::inline_rows(t)) {
                    return Ok(Some(r));
                }
                let mut cols: Vec<String> = Vec::new();
                for c in [key_prop.clone().unwrap_or_default(), binding.key.clone(), "name".into(), "id".into()] {
                    if !c.is_empty() && !cols.iter().any(|x| x.eq_ignore_ascii_case(&c)) {
                        cols.push(c);
                    }
                }
                let props = Self::props_of(t);
                return Ok(self.db()?.find(&t.name, &cols, key)?.map(|r| normalise(r, &props)));
            }
        }
        Ok(scan(&self.instances(&t.name)?))
    }

    /// Rows for an edge table, if the relationship has a readable source.
    pub fn edges(&self, rel_name: &str) -> Result<Vec<Record>> {
        let rels = self.ont.relationship(rel_name);
        let rel = rels.first().ok_or_else(|| anyhow!("unknown relationship {}", rel_name))?;
        let Some(binding) = &rel.source else { return Ok(vec![]) };
        let Some(src) = self.ont.sources.get(&binding.connection) else { return Ok(vec![]) };
        let rename = |h: &str| -> String {
            if h.eq_ignore_ascii_case(&binding.from_key) {
                "from".to_string()
            } else if h.eq_ignore_ascii_case(&binding.to_key) {
                "to".to_string()
            } else {
                h.to_string()
            }
        };
        if duck_kind(&src.kind) {
            #[cfg(feature = "duckdb")]
            {
                let db = self.db()?;
                let raw = db.rows(&rel.qualified(), duck::MAX_INSTANCE_ROWS)?;
                return Ok(raw
                    .into_iter()
                    .map(|r| {
                        r.into_iter()
                            .map(|(k, v)| {
                                let v = match rel.properties.iter().find(|(p, _)| p.eq_ignore_ascii_case(&k)) {
                                    Some((_, p)) => coerce_json(v, p),
                                    None => v,
                                };
                                (rename(&k), v)
                            })
                            .collect()
                    })
                    .collect());
            }
        }
        if !src.kind.eq_ignore_ascii_case("csv") {
            return Ok(vec![]);
        }
        let path = self.file_path(src, &binding.table);
        let mut rdr = csv::ReaderBuilder::new().trim(csv::Trim::All).flexible(true).from_path(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_string()).collect();
        let mut rows = Vec::new();
        for result in rdr.records() {
            let row = result?;
            let mut rec = Record::new();
            for (i, h) in headers.iter().enumerate() {
                if let Some(raw) = row.get(i) {
                    let v = match rel.properties.get(h) {
                        Some(p) => coerce(raw, p),
                        None => json!(raw),
                    };
                    rec.insert(rename(h), v);
                }
            }
            rows.push(rec);
        }
        Ok(rows)
    }

    /// Where the rows live, plus a live row count when readable here.
    pub fn where_is(&self, type_name: &str) -> Result<Json> {
        let t = self.ont.type_(type_name).ok_or_else(|| anyhow!("unknown type {}", type_name))?;
        let mut j = json!({ "type": t.name, "key": t.key_property() });
        match &t.source {
            Some(b) => {
                let src = self.ont.sources.get(&b.connection);
                j["connection"] = json!(b.connection);
                j["kind"] = json!(src.map(|s| s.kind.clone()).unwrap_or_else(|| "unknown".into()));
                j["table"] = json!(b.table);
                j["key_column"] = json!(b.key);
                j["columns"] = json!(b.columns);
                // who stands behind this binding: an agent's guess at a table is not a fact yet
                if self.ont.graded.has_file {
                    j["trust"] = self.ont.grade_of(crate::trust::Kind::Binding, &t.name).to_json(crate::trust::Kind::Binding);
                }
                if let Some(s) = src {
                    if s.kind.eq_ignore_ascii_case("csv") {
                        j["path"] = json!(self.file_path(s, &b.table).display().to_string());
                        j["readable_here"] = json!(true);
                        j["row_count"] = json!(self.instances(&t.name)?.len());
                        #[cfg(feature = "duckdb")]
                        {
                            j["sql"] = json!(format!("select * from {} limit 10", t.name));
                        }
                    } else if duck_kind(&s.kind) {
                        j["config"] = json!(s.config);
                        #[cfg(feature = "duckdb")]
                        match self.db() {
                            Ok(db) => match db.view(&t.name) {
                                Some(v) => {
                                    j["path"] = json!(v.source);
                                    j["readable_here"] = json!(true);
                                    j["row_count"] = json!(db.count(&t.name)?);
                                    j["sql"] = json!(format!("select * from {} limit 10", v.view));
                                    j["hint"] = json!(format!("Readable here through DuckDB ({}): use `query` or `instances`.", s.kind));
                                }
                                None => {
                                    j["readable_here"] = json!(false);
                                    j["hint"] = json!(db.view_error(&t.name).unwrap_or_else(|| "no view".into()));
                                }
                            },
                            Err(e) => {
                                j["readable_here"] = json!(false);
                                j["hint"] = json!(e.to_string());
                            }
                        }
                    } else if s.kind.eq_ignore_ascii_case("mcp") {
                        let server = s.server.clone().unwrap_or_default();
                        j.as_object_mut().map(|o| o.remove("table"));
                        j["server"] = json!(server);
                        j["tool"] = json!(b.tool);
                        j["params"] = json!(b.params);
                        j["readable_here"] = json!(false);
                        let params = serde_json::to_string(&b.params).unwrap_or_else(|_| "{}".into());
                        j["hint"] = json!(format!("Call tool `{}` on the `{}` MCP server with params {}; that server returns the rows, this ontology says what they mean.", b.tool, server, params));
                    } else {
                        j["readable_here"] = json!(false);
                        j["config"] = json!(s.config);
                        j["hint"] = json!(format!("Query {} with your own {} connector; the ontology only holds the binding.", b.table, s.kind));
                    }
                }
            }
            None => {
                j["connection"] = Json::Null;
                j["readable_here"] = json!(!t.instances.is_empty());
                j["row_count"] = json!(t.instances.len());
                if t.instances.is_empty() {
                    j["hint"] = json!("This type has no data binding; it is a concept only.");
                } else {
                    #[cfg(feature = "duckdb")]
                    {
                        j["sql"] = json!(format!("select * from {} limit 10", t.name));
                    }
                }
            }
        }
        Ok(j)
    }

    /// Everything SQL can see: ontology tables plus one view per readable type/relationship.
    #[cfg(feature = "duckdb")]
    pub fn sql_catalog(&self) -> Result<Json> {
        let db = self.db()?;
        let views: Vec<Json> = db.views().into_iter().map(|(name, view, source)| json!({ "name": name, "view": view, "source": source })).collect();
        Ok(json!({
            "duckdb": db.version(),
            "fenced": db.fenced,
            "tables": ["nodes", "properties", "edges", "constraints", "actions", "sources"],
            "views": views,
            "skipped": db.skipped,
            "not_readable": db.unreadable,
            "max_rows": MAX_QUERY_ROWS,
        }))
    }

    #[cfg(not(feature = "duckdb"))]
    pub fn sql_catalog(&self) -> Result<Json> {
        anyhow::bail!("this binary was built without the `duckdb` feature")
    }
}
