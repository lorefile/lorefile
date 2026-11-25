//! stella-ontology: a lightweight, file-based, agent-first ontology graph.
//! YAML folder in → compiled graph → CLI, MCP server, web UI out.

mod data;
#[cfg(feature = "duckdb")]
mod duck;
mod loader;
mod mcp;
mod ontology;
mod rules;
mod schema;
mod serve;
mod validate;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use data::DataLayer;
use indexmap::IndexMap;
use ontology::Ontology;
use serde_json::{json, Value as Json};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "stella-ontology", version, about = "Lightweight, file-based, AI-agent-first ontology graph.")]
struct Cli {
    /// Ontology folder (defaults to ./ontology or the current folder).
    #[arg(short, long, global = true)]
    dir: Option<PathBuf>,
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile the folder and report errors and warnings.
    Validate,
    /// Print the compact Markdown summary agents load into their context.
    Summary,
    /// Describe a type, relationship, action, rule, or source.
    Describe { name: String },
    /// Search names, descriptions, rules, and glossary.
    Search {
        query: Vec<String>,
        #[arg(long, default_value_t = 15)]
        limit: usize,
    },
    /// Types reachable from a type within N hops.
    Neighbors {
        name: String,
        #[arg(long, default_value_t = 1)]
        depth: usize,
    },
    /// Shortest relationship path between two types.
    Path { from: String, to: String },
    /// Rules that apply to a type, relationship, or action.
    Rules { name: String },
    /// Where a type's data lives.
    Where { name: String },
    /// List rows for a type (inline or csv sources).
    Instances {
        name: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Check whether an action may be taken: `check switch_supplier part=BRK-220 to=V-10422`.
    Check {
        action: String,
        /// input=value pairs; value may be a key or inline JSON like to='{"tier":3}'
        inputs: Vec<String>,
    },
    /// Run one read-only SQL statement (SELECT/WITH) over the ontology tables and data views, via DuckDB.
    Query {
        /// The SQL, or `-` to read it from stdin. Tables: nodes, properties, edges, constraints, actions, sources; one view per type.
        sql: String,
        /// Row cap (hard maximum 500).
        #[arg(long, default_value_t = 500)]
        limit: usize,
    },
    /// List what SQL can see: ontology tables, data views, and what could not be attached.
    Tables,
    /// Run the MCP server over stdio (for Claude, Cursor, ChatGPT, agents).
    Mcp,
    /// Serve the web UI and JSON API.
    Serve {
        #[arg(long, default_value_t = 7070)]
        port: u16,
    },
    /// Write a small starter ontology into a folder.
    Init { path: Option<PathBuf> },
}

fn resolve_dir(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(d) = explicit {
        return d;
    }
    for cand in ["ontology", "."] {
        let p = Path::new(cand);
        if p.join("ontology.yaml").exists() || p.join("ontology.yml").exists() {
            return p.to_path_buf();
        }
    }
    PathBuf::from(".")
}

pub fn load_ontology(dir: &Path) -> Result<(Ontology, validate::Report)> {
    let raw = loader::load(dir)?;
    let ont = Ontology::from_raw(raw);
    let report = validate::validate(&ont);
    Ok((ont, report))
}

fn print_json<T: serde::Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn print_report(ont: &Ontology, report: &validate::Report) {
    let st = ont.stats();
    for d in &report.diags {
        let tag = match d.level {
            validate::Level::Error => "error",
            validate::Level::Warning => "warn ",
        };
        let file = if d.file.is_empty() { String::new() } else { format!(" [{}]", d.file) };
        eprintln!("{} {}: {}{}", tag, d.code, d.message, file);
    }
    let status = if report.ok() { "✓ valid" } else { "✗ invalid" };
    eprintln!(
        "{} · {} types · {} relationships · {} rules · {} actions · {} sources · {} errors · {} warnings",
        status, st.types, st.relationships, st.constraints, st.actions, st.sources,
        report.errors().len(), report.warnings().len()
    );
}

fn parse_inputs(pairs: &[String]) -> Result<IndexMap<String, Json>> {
    let mut out = IndexMap::new();
    for p in pairs {
        let Some((k, v)) = p.split_once('=') else {
            bail!("input `{}` must look like name=value", p);
        };
        let v = v.trim();
        let val = if v.starts_with('{') || v.starts_with('[') {
            serde_json::from_str(v).map_err(|e| anyhow::anyhow!("input `{}` is not valid JSON: {}", k, e))?
        } else {
            json!(v)
        };
        out.insert(k.trim().to_string(), val);
    }
    Ok(out)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Cmd::Init { path } = &cli.cmd {
        let target = path.clone().unwrap_or_else(|| PathBuf::from("ontology"));
        return init(&target);
    }

    let dir = resolve_dir(cli.dir.clone());
    let (ont, report) = load_ontology(&dir)?;

    match cli.cmd {
        Cmd::Validate => {
            if cli.json {
                print_json(&json!({ "ok": report.ok(), "stats": ont.stats(), "diagnostics": report.diags }));
            } else {
                print_report(&ont, &report);
            }
            if !report.ok() {
                std::process::exit(1);
            }
        }
        Cmd::Summary => print!("{}", ont.summary_markdown()),
        Cmd::Describe { name } => match (ont.describe(&name), ont.describe_markdown(&name)) {
            (Some(j), Some(md)) => {
                if cli.json { print_json(&j) } else { print!("{}", md) }
            }
            _ => {
                let hits = ont.search(&name, 5);
                bail!("nothing named `{}`. Did you mean: {}", name, hits.iter().map(|h| h.name.clone()).collect::<Vec<_>>().join(", "));
            }
        },
        Cmd::Search { query, limit } => {
            let hits = ont.search(&query.join(" "), limit);
            if cli.json {
                print_json(&hits)
            } else {
                for h in hits {
                    println!("{:<12} {:<28} {}", format!("{:?}", h.kind).to_lowercase(), h.name, h.snippet);
                }
            }
        }
        Cmd::Neighbors { name, depth } => {
            let Some(sg) = ont.neighbors(&name, depth) else { bail!("unknown type `{}`", name) };
            if cli.json {
                print_json(&sg)
            } else {
                for e in &sg.edges {
                    println!("{} ─{}→ {}  ({})", e.from, e.relationship, e.to, e.cardinality);
                }
            }
        }
        Cmd::Path { from, to } => match ont.path(&from, &to) {
            Some(p) if cli.json => print_json(&p),
            Some(p) if p.is_empty() => println!("{} is {}", from, to),
            Some(p) => {
                for e in p {
                    println!("{} ─{}→ {}", e.from, e.relationship, e.to);
                }
            }
            None => bail!("no path between `{}` and `{}`", from, to),
        },
        Cmd::Rules { name } => {
            let cs = ont.constraints_for(&name);
            if cli.json {
                print_json(&cs)
            } else {
                for c in cs {
                    println!("{:<8} {:<6} {} — {}  [{}]", c.id, c.severity, c.name, c.rule, c.applies_to);
                }
            }
        }
        Cmd::Where { name } => {
            let data = DataLayer::new(&ont);
            print_json(&data.where_is(&name)?);
        }
        Cmd::Instances { name, limit } => {
            let data = DataLayer::new(&ont);
            let rows = if ont.type_(&name).is_none() && !ont.relationship(&name).is_empty() { data.edges(&name)? } else { data.instances(&name)? };
            let total = rows.len();
            let shown: Vec<_> = rows.into_iter().take(limit).collect();
            if cli.json {
                print_json(&json!({ "type": name, "total": total, "rows": shown }))
            } else {
                for r in &shown {
                    println!("{}", serde_json::to_string(r).unwrap_or_default());
                }
                if total > shown.len() {
                    println!("… {} more", total - shown.len());
                }
            }
        }
        Cmd::Check { action, inputs } => {
            let data = DataLayer::new(&ont);
            let raw = parse_inputs(&inputs)?;
            let rep = rules::check_action(&ont, &data, &action, &raw)?;
            if cli.json {
                print_json(&rep);
            } else {
                let mark = match rep.verdict.as_str() { "pass" => "✓", "warn" => "⚠", _ => "✗" };
                println!("{} {}", mark, rep.summary);
                for r in &rep.results {
                    let m = if r.passed { "  ok  " } else { match r.severity { schema::Severity::Block => " FAIL ", schema::Severity::Warn => " warn ", schema::Severity::Info => " info " } };
                    println!("{} {:<8} {} — {}", m, r.id, r.name, r.message);
                    if let Some(e) = &r.error {
                        println!("         error: {}", e);
                    }
                    if !r.passed && !r.rationale.is_empty() {
                        println!("         why: {}", r.rationale);
                    }
                }
                if !rep.suggestions.is_empty() {
                    println!("suggestions:");
                    for s in &rep.suggestions {
                        println!("  {} → {}", s.input, s.label);
                    }
                }
                if !rep.requires_approval_from.is_empty() {
                    println!("approval: {}", rep.requires_approval_from);
                }
                for n in &rep.notes {
                    println!("note: {}", n);
                }
            }
            if rep.verdict == "fail" {
                std::process::exit(2);
            }
        }
        Cmd::Query { sql, limit } => {
            let sql = if sql == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                s
            } else {
                sql
            };
            let data = DataLayer::new(&ont);
            let res = data.query(&sql, limit)?;
            if cli.json {
                print_json(&res);
            } else {
                print_table(&res);
            }
        }
        Cmd::Tables => {
            let data = DataLayer::new(&ont);
            print_json(&data.sql_catalog()?);
        }
        Cmd::Mcp => {
            print_report(&ont, &report);
            mcp::run_stdio(ont)?;
        }
        Cmd::Serve { port } => {
            print_report(&ont, &report);
            serve::run(dir, port)?;
        }
        Cmd::Init { .. } => unreachable!(),
    }
    Ok(())
}

/// Fixed-width text table for `query` results; long cells are cut, nested values are shown as JSON.
fn print_table(res: &data::QueryResult) {
    let cell = |v: &Json| -> String {
        let s = match v {
            Json::String(s) => s.clone(),
            Json::Null => String::new(),
            other => other.to_string(),
        };
        let one: String = s.replace('\n', " ");
        if one.chars().count() > 48 { format!("{}…", one.chars().take(47).collect::<String>()) } else { one }
    };
    let names: Vec<String> = res.columns.iter().map(|c| c.name.clone()).collect();
    let grid: Vec<Vec<String>> = res.rows.iter().map(|r| names.iter().map(|n| r.get(n).map(cell).unwrap_or_default()).collect()).collect();
    let widths: Vec<usize> = names
        .iter()
        .enumerate()
        .map(|(i, n)| grid.iter().map(|r| r[i].chars().count()).chain(std::iter::once(n.chars().count())).max().unwrap_or(0))
        .collect();
    let line = |cells: &[String]| cells.iter().enumerate().map(|(i, c)| format!("{:<w$}", c, w = widths[i])).collect::<Vec<_>>().join("  ");
    if !names.is_empty() {
        println!("{}", line(&names));
        println!("{}", widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("  "));
    }
    for r in &grid {
        println!("{}", line(r));
    }
    let more = if res.truncated { format!(" (truncated at {})", res.limit) } else { String::new() };
    eprintln!("{} row{}{} · {} ms", res.row_count, if res.row_count == 1 { "" } else { "s" }, more, res.elapsed_ms);
    for w in &res.warnings {
        eprintln!("note: {}", w);
    }
}

fn init(target: &Path) -> Result<()> {
    if target.join("ontology.yaml").exists() {
        bail!("{} already has an ontology.yaml", target.display());
    }
    let files: [(&str, &str); 5] = [
        ("ontology.yaml", include_str!("starter/ontology.yaml")),
        ("glossary.yaml", include_str!("starter/glossary.yaml")),
        ("types/customer.yaml", include_str!("starter/types/customer.yaml")),
        ("constraints/basics.yaml", include_str!("starter/constraints/basics.yaml")),
        ("actions/close_account.yaml", include_str!("starter/actions/close_account.yaml")),
    ];
    for (rel, content) in files {
        let p = target.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, content)?;
        eprintln!("wrote {}", p.display());
    }
    eprintln!("\nNext: stella-ontology validate --dir {}\n      stella-ontology serve --dir {}", target.display(), target.display());
    Ok(())
}
