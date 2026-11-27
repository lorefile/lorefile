//! lorefile: a lightweight, file-based, agent-first ontology graph.
//! YAML folder in → compiled graph → CLI, MCP server, web UI out.

mod data;
#[cfg(feature = "duckdb")]
mod duck;
mod edit;
mod iri;
mod learn;
mod review;
mod loader;
mod mcp;
mod oauth;
mod ontology;
mod ossie;
mod patch;
mod people;
mod plane;
mod proposals;
#[cfg(feature = "rdf")]
mod rdf;
mod rules;
mod schema;
mod serve;
mod spaces;
mod trust;
mod validate;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use data::DataLayer;
use edit::EditConfig;
use indexmap::IndexMap;
use ontology::Ontology;
use serde_json::{json, Value as Json};
use spaces::Registry;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "lorefile", version, about = "Lorefile: a team's business knowledge in plain YAML files, for AI agents. YAML in; MCP, CLI and a web page out.")]
struct Cli {
    /// An ontology folder, or a folder of spaces (one sub-folder per ontology). Defaults to ./ontology, else the current folder.
    #[arg(short, long, global = true)]
    dir: Option<PathBuf>,
    /// Which space to work on when --dir holds several (shipped examples start with @, e.g. @supply-chain).
    #[arg(short, long, global = true)]
    space: Option<String>,
    /// Leave out the examples and packs that ship inside the binary.
    #[arg(long, global = true)]
    no_examples: bool,
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile the folder (every space, on a folder of spaces) and report errors and warnings.
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
    Mcp {
        /// Let `propose_change` write YAML files (default: it only validates).
        #[arg(long)]
        allow_edit: bool,
        /// Commit each accepted change even when the space sits inside some other repository.
        /// (Spaces with a history of their own commit every change by default.)
        #[arg(long, requires = "allow_edit", conflicts_with = "no_commit_edits")]
        commit_edits: bool,
        /// Never commit: leave accepted changes uncommitted, even in a space's own history.
        #[arg(long)]
        no_commit_edits: bool,
    },
    /// Serve the web UI and JSON API.
    Serve {
        #[arg(long, default_value_t = 7070)]
        port: u16,
        /// Address to bind. Use 0.0.0.0 inside a container.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Let the page create, edit and delete YAML files (default: read-only).
        #[arg(long)]
        allow_edit: bool,
        /// Commit each accepted change even when the space sits inside some other repository.
        /// (Spaces with a history of their own commit every change by default.)
        #[arg(long, requires = "allow_edit", conflicts_with = "no_commit_edits")]
        commit_edits: bool,
        /// Never commit: leave accepted changes uncommitted, even in a space's own history.
        #[arg(long)]
        no_commit_edits: bool,
        /// The login token. Else LOREFILE_TOKEN, else the one saved in <dir>/.lorefile/auth.toml,
        /// else a new one is made, saved there and printed.
        #[arg(long, conflicts_with = "no_auth")]
        token: Option<String>,
        /// No login: anyone who can reach the port can read (and with --allow-edit, write).
        /// For a laptop, or behind a proxy that already signs people in.
        #[arg(long)]
        no_auth: bool,
        /// Allow --no-auth on an address other machines can reach (e.g. --host 0.0.0.0).
        #[arg(long, requires = "no_auth")]
        i_know: bool,
        /// Turn off the OAuth sign-in that ChatGPT's and Claude's connectors use. (It is on by
        /// itself with the login token, for requests that arrive over HTTPS through a tunnel or proxy.)
        #[arg(long)]
        no_oauth: bool,
        /// Answer OAuth sign-ins on this one address only, e.g. https://acme-lorefile.fly.dev.
        /// Optional: without it the address is read from each request. Else LOREFILE_PUBLIC_URL.
        #[arg(long, value_name = "URL")]
        public_url: Option<String>,
        /// Allow one more OAuth return address, matched exactly (repeatable). ChatGPT's, Claude's
        /// and this machine's loopback addresses are always allowed.
        #[arg(long = "oauth-redirect", value_name = "URL")]
        oauth_redirect: Vec<String>,
    },
    /// Who stands behind each definition: list the grades, accept, or record a check.
    Trust {
        #[command(subcommand)]
        op: Option<TrustCmd>,
    },
    /// Write a small starter ontology into a folder.
    Init { path: Option<PathBuf> },
    /// Write the space for other tools: `--ossie` (Apache Ossie) or `--rdf` (OWL, SKOS and SHACL).
    Export {
        /// Apache Ossie (draft 0.3.0.dev0), the format dbt, Snowflake, Cube and NVIDIA's Auto Ontology read.
        #[arg(long, conflicts_with = "rdf")]
        ossie: bool,
        /// One RDF graph: OWL classes and properties, SKOS labels, SHACL shapes, for Protégé, GraphDB, Stardog, TopBraid.
        #[arg(long)]
        rdf: bool,
        /// With --rdf: turtle (default), jsonld, ntriples or rdfxml.
        #[arg(long, requires = "rdf")]
        format: Option<String>,
        /// Ossie: a folder for the three files (YAML, or JSON with --json), or a `.json` file for all three in one.
        /// RDF: the file to write. Without it the export is printed.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Create or extend a space from Apache Ossie documents or an RDF file (OWL, RDFS, SKOS, SHACL).
    /// Nothing is written unless the result compiles.
    Import {
        /// The Ossie semantic model (YAML or JSON).
        #[arg(long, value_name = "SEMANTIC_MODEL", required_unless_present = "from_rdf", conflicts_with = "from_rdf")]
        from_ossie: Option<PathBuf>,
        /// An RDF file: Turtle, N-Triples, RDF/XML or JSON-LD (OWL classes become entities, SHACL shapes guardrails).
        #[arg(long, value_name = "FILE")]
        from_rdf: Option<PathBuf>,
        /// Ossie: the ontology document (concepts become entities, relationships links).
        #[arg(long, requires = "from_ossie")]
        ontology: Option<PathBuf>,
        /// Ossie: the mapping document (which fields identify each concept and hold each relationship).
        #[arg(long, requires = "from_ossie")]
        mapping: Option<PathBuf>,
        /// The space folder to create or extend. For RDF it defaults to a folder named after the file.
        #[arg(long)]
        into: Option<PathBuf>,
        /// RDF: import only these classes, comma-separated: names, `prefix:Name`, IRIs, or a namespace such as `schema:`.
        #[arg(long, value_delimiter = ',', requires = "from_rdf")]
        only: Vec<String>,
        /// RDF, with --only: also import the classes this many links away.
        #[arg(long, default_value_t = 0, requires = "only")]
        depth: usize,
        /// List everything that was guessed, dropped or kept for a person.
        #[arg(long)]
        report: bool,
        /// Restore the grades (who stands behind what) a Lorefile export carries, where each still
        /// matches its definition. Only for your own export: by default imported facts start ungraded.
        #[arg(long)]
        keep_grades: bool,
        /// With --from-rdf: keep rdfs:subClassOf as subclass-of: tags instead of `extends`, and do not bring parent classes along.
        #[arg(long)]
        no_parents: bool,
    },
}

#[derive(Subcommand)]
enum TrustCmd {
    /// Every metric, data binding, glossary word and guardrail, grouped by grade (the default).
    List,
    /// Say you stand behind a definition: proposed or changed becomes accepted.
    /// `--all` vouches for every ungraded one in one change.
    Accept {
        /// A metric, an entity (for its data binding), a glossary word or a guardrail id.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        name: Option<String>,
        #[arg(long)]
        all: bool,
        /// metric, binding, word or guardrail, when the name is used by more than one.
        #[arg(long)]
        kind: Option<String>,
        /// Your name, shown on what you accept.
        #[arg(long)]
        by: Option<String>,
    },
    /// Record that you ran a definition against real rows and it was right.
    /// Lorefile records your word; it runs nothing.
    Check {
        name: String,
        /// The source under sources/ whose rows you used.
        #[arg(long)]
        against: String,
        /// What you compared, e.g. "412 paid orders for 16–22 Nov, the same as Shopify's own report".
        #[arg(long)]
        note: String,
        /// metric, binding, word or guardrail, when the name is used by more than one.
        #[arg(long)]
        kind: Option<String>,
        /// Your name, shown on what you checked.
        #[arg(long)]
        by: Option<String>,
    },
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

pub(crate) fn print_report(ont: &Ontology, report: &validate::Report) {
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
    if let Cmd::Import { from_ossie, from_rdf, ontology, mapping, into, only, depth, report, keep_grades, no_parents } = &cli.cmd {
        if let Some(file) = from_rdf {
            return import_rdf(file, into.as_deref(), only, *depth, *report, *keep_grades, *no_parents, cli.json);
        }
        let Some(sm) = from_ossie else { bail!("say what to import: --from-ossie <semantic model> or --from-rdf <file>") };
        let Some(into) = into else { bail!("--into <folder> says which space to create or extend") };
        return import_ossie(sm, ontology.as_deref(), mapping.as_deref(), into, *report, *keep_grades, cli.json);
    }

    let dir = resolve_dir(cli.dir.clone());
    // refused before anything is written: no login on an address the network can reach
    if let Cmd::Serve { host, no_auth: true, i_know: false, .. } = &cli.cmd {
        if !serve::is_loopback(host) {
            bail!(
                "--no-auth on {} lets anyone who can reach this machine read these spaces (and change them with --allow-edit). \
                 Leave it off and use the token (set LOREFILE_TOKEN, or let the server make one), or add --i-know when a proxy in front already signs people in.",
                host
            );
        }
    }
    let is_server = matches!(cli.cmd, Cmd::Serve { .. } | Cmd::Mcp { .. });
    if !is_server && !dir.exists() {
        bail!("folder not found: {}", dir.display());
    }
    // only the servers write out the shipped examples; a read command touches nothing
    let mut reg = Registry::discover(&dir, is_server && !cli.no_examples, is_server)?;

    // the long-running servers take the whole registry
    match cli.cmd {
        Cmd::Mcp { allow_edit, commit_edits, no_commit_edits } => {
            return mcp::run_stdio(reg, edit_config(allow_edit, commit_edits, no_commit_edits), cli.space);
        }
        Cmd::Serve { port, host, allow_edit, commit_edits, no_commit_edits, token, no_auth, no_oauth, public_url, oauth_redirect, .. } => {
            let public_url = public_url.or_else(|| std::env::var("LOREFILE_PUBLIC_URL").ok().filter(|v| !v.trim().is_empty()));
            let flags = serve::OAuthFlags {
                off: no_oauth,
                public_url: public_url.as_deref().map(oauth::pin).transpose().map_err(|e| anyhow::anyhow!(e))?,
                redirects: oauth_redirect.iter().map(|u| oauth::extra_redirect(u)).collect::<Result<Vec<_>, _>>().map_err(|e| anyhow::anyhow!(e))?,
            };
            let login = serve::resolve_login(&reg.root, token, no_auth)?;
            return serve::run(reg, &host, port, edit_config(allow_edit, commit_edits, no_commit_edits), login, flags);
        }
        _ => {}
    }

    // `validate` on a folder of spaces checks each of them
    if matches!(cli.cmd, Cmd::Validate) && cli.space.is_none() && !reg.single {
        let ok = if cli.json {
            let ids: Vec<String> = reg.user_spaces().iter().map(|s| s.id.clone()).collect();
            let mut all = Vec::new();
            let mut ok = true;
            for id in ids {
                let l = reg.get(&id)?;
                ok &= l.report.ok();
                all.push(json!({ "space": id, "ok": l.report.ok(), "stats": l.ont.stats(), "diagnostics": l.report.diags }));
            }
            print_json(&all);
            ok
        } else {
            spaces::validate_all(&mut reg)?
        };
        if !ok {
            std::process::exit(1);
        }
        return Ok(());
    }

    let space_id = reg.resolve(cli.space.as_deref()).map_err(|e| anyhow::anyhow!("{}. Pass --space <id>.", e))?;
    if let Cmd::Trust { op } = &cli.cmd {
        let builtin = reg.info(&space_id).map(|s| s.builtin).unwrap_or(false);
        let root = reg.root.clone();
        let loaded = reg.get(&space_id)?;
        return trust_cmd(&loaded.ont, &loaded.report, &loaded.dir, &root, builtin, op.as_ref().unwrap_or(&TrustCmd::List), cli.json);
    }
    let loaded = reg.get(&space_id)?;
    let (ont, report) = (&loaded.ont, &loaded.report);

    match cli.cmd {
        Cmd::Validate => {
            if cli.json {
                print_json(&json!({ "ok": report.ok(), "space": space_id, "stats": ont.stats(), "diagnostics": report.diags }));
            } else {
                print_report(ont, report);
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
            let data = DataLayer::new(ont);
            print_json(&data.where_is(&name)?);
        }
        Cmd::Instances { name, limit } => {
            let data = DataLayer::new(ont);
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
            let data = DataLayer::new(ont);
            let raw = parse_inputs(&inputs)?;
            let rep = rules::check_action(ont, &data, &action, &raw)?;
            if cli.json {
                print_json(&rep);
            } else {
                let mark = match rep.verdict.as_str() { "pass" => "✓", "warn" => "⚠", "unknown" => "?", _ => "✗" };
                println!("{} {}", mark, rep.summary);
                for r in &rep.results {
                    let m = if r.passed { "  ok  " } else { match r.severity { schema::Severity::Block => " FAIL ", schema::Severity::Warn => " warn ", schema::Severity::Info => " info " } };
                    let g = match r.grade.as_deref() { Some(g @ ("proposed" | "checked")) => format!("     [{}]", g), _ => String::new() };
                    println!("{} {:<8} {} — {}{}", m, r.id, r.name, r.message, g);
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
                for u in &rep.unverified {
                    println!("note: {}", u);
                }
            }
            if rep.verdict == "fail" {
                std::process::exit(2);
            }
            if rep.verdict == "unknown" {
                std::process::exit(3);
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
            let data = DataLayer::new(ont);
            let res = data.query(&sql, limit)?;
            if cli.json {
                print_json(&res);
            } else {
                print_table(&res);
            }
        }
        Cmd::Tables => {
            let data = DataLayer::new(ont);
            print_json(&data.sql_catalog()?);
        }
        Cmd::Export { ossie: false, rdf: true, format, out } => export_rdf(ont, report, format.as_deref(), out)?,
        Cmd::Export { ossie, out, .. } => export_ossie(ont, report, ossie, out, cli.json)?,
        Cmd::Mcp { .. } | Cmd::Serve { .. } | Cmd::Init { .. } | Cmd::Import { .. } | Cmd::Trust { .. } => unreachable!(),
    }
    Ok(())
}

/// `lorefile trust`: list the grades, or set one as a person at the command line.
fn trust_cmd(ont: &Ontology, report: &validate::Report, dir: &Path, spaces_root: &Path, builtin: bool, op: &TrustCmd, as_json: bool) -> Result<()> {
    use trust::Kind;
    let kind_of = |k: &Option<String>| -> Result<Option<Kind>> {
        match k {
            None => Ok(None),
            Some(s) => Kind::parse(s).map(Some).ok_or_else(|| anyhow::anyhow!("--kind is metric, binding, word or guardrail, not `{}`", s)),
        }
    };
    let (gop, by) = match op {
        TrustCmd::List => {
            if as_json {
                print_json(&json!({ "file": ont.graded.has_file, "counts": ont.graded.counts(), "items": ont.graded.list() }));
                return Ok(());
            }
            if !ont.graded.has_file {
                println!("No trust.yaml yet: every definition is ungraded. Accept one with `lorefile trust accept <name>`, or vouch for all with `lorefile trust accept --all`.");
            }
            fn group(s: &trust::Status) -> usize {
                match (s.grade, s.changed) {
                    (Some(schema::Grade::Checked), _) => 0,
                    (Some(schema::Grade::Accepted), _) => 1,
                    (Some(schema::Grade::Proposed), _) => 2,
                    (None, true) => 3,
                    (None, false) => 4,
                }
            }
            let titles = ["checked", "accepted", "proposed by an agent, not reviewed", "changed since a person graded it", "ungraded"];
            for (i, title) in titles.iter().enumerate() {
                let test = |s: &trust::Status| group(s) == i;
                let rows: Vec<String> = ont
                    .graded
                    .all
                    .iter()
                    .filter(|(k, n)| test(&ont.grade_of(*k, n)))
                    .map(|(k, n)| {
                        let st = ont.grade_of(*k, n);
                        format!("  {:<12} {:<28} {}", k.word(), n, st.say(*k).unwrap_or_default())
                    })
                    .collect();
                if !rows.is_empty() {
                    println!("{} ({})", title, rows.len());
                    for r in rows {
                        println!("{}", r);
                    }
                }
            }
            for (k, n, r) in &ont.graded.orphans {
                println!("no longer exists: {} ({} in trust.yaml)", k.name_words(n), r.grade.word());
            }
            return Ok(());
        }
        TrustCmd::Accept { all: true, by, .. } => (edit::GradeOp::VouchAll, trust::by_words(by.as_deref(), "CLI")),
        TrustCmd::Accept { name, kind, by, .. } => {
            let (k, n) = edit::find_graded(ont, kind_of(kind)?, name.as_deref().unwrap_or("")).map_err(|e| anyhow::anyhow!(e))?;
            (edit::GradeOp::Accept { kind: k, name: n }, trust::by_words(by.as_deref(), "CLI"))
        }
        TrustCmd::Check { name, against, note, kind, by } => {
            let (k, n) = edit::find_graded(ont, kind_of(kind)?, name).map_err(|e| anyhow::anyhow!(e))?;
            (edit::GradeOp::Check { kind: k, name: n, against: against.clone(), note: note.clone() }, trust::by_words(by.as_deref(), "CLI"))
        }
    };
    if builtin {
        bail!("this is a shipped example and stays read-only; copy it into a space of your own to grade it");
    }
    let cfg = EditConfig { allow: true, commit: false, policy: edit::CommitPolicy::Auto }.for_space(dir, spaces_root);
    let out = edit::grade(dir, cfg, report, &gop, &by);
    if as_json {
        print_json(&out.to_json());
    }
    if let Some(r) = &out.refused {
        bail!("{}", r);
    }
    for d in &out.diagnostics {
        eprintln!("{} {}: {} [{}]", if d.level == validate::Level::Error { "error" } else { "warn " }, d.code, d.message, d.file);
    }
    if !out.written {
        bail!("nothing was written");
    }
    if !as_json {
        let what = match &gop {
            edit::GradeOp::Accept { kind, name } => format!("{} is accepted, by {}", kind.name_words(name), by),
            edit::GradeOp::Check { kind, name, against, .. } => format!("{} is checked against {}, by {}", kind.name_words(name), against, by),
            edit::GradeOp::VouchAll => format!("vouched for {} definitions, by {}", out.name, by),
            edit::GradeOp::Sign { .. } => format!("{} signed, by {}", out.stamped.join(", "), by),
            edit::GradeOp::Distinct { a, b, .. } => format!("{} and {} are different on purpose, by {}", a, b, by),
        };
        eprintln!("✓ {}{}", what, out.commit.as_ref().map(|h| format!(" (saved to history as {})", h)).unwrap_or_else(|| " (trust.yaml written, not committed)".into()));
    }
    Ok(())
}

/// `lorefile export --ossie`: three documents, printed or written.
fn export_ossie(ont: &Ontology, report: &validate::Report, ossie: bool, out: Option<PathBuf>, as_json: bool) -> Result<()> {
    if !ossie {
        bail!("say which format: --ossie (Apache Ossie) or --rdf (OWL, SKOS and SHACL)");
    }
    let ex = ossie::export(ont)?;
    if !report.ok() {
        eprintln!("note: this space has {} compile errors; the export shows it as it is", report.errors().len());
    }
    let header = |name: &str| format!("# {}: Apache Ossie {}, written by `lorefile export --ossie` from {}.\n", name, ossie::VERSION, ont.manifest.name);
    match out {
        None if as_json => print_json(&ex.bundle()),
        None => {
            for (name, doc) in ex.documents("yaml") {
                print!("--- # {}\n{}", name, ossie::to_yaml(&doc));
            }
        }
        Some(p) if p.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) => {
            if let Some(parent) = p.parent().filter(|x| !x.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&p, serde_json::to_string_pretty(&ex.bundle())? + "\n")?;
            eprintln!("wrote {}", p.display());
        }
        Some(dir) => {
            std::fs::create_dir_all(&dir)?;
            for (name, doc) in ex.documents(if as_json { "json" } else { "yaml" }) {
                let text = if as_json { serde_json::to_string_pretty(&doc)? + "\n" } else { header(&name) + &ossie::to_yaml(&doc) };
                std::fs::write(dir.join(&name), text)?;
                eprintln!("wrote {}", dir.join(&name).display());
            }
        }
    }
    for n in &ex.notes {
        eprintln!("note: {}", n);
    }
    Ok(())
}

/// `lorefile export --rdf`: one graph, printed or written.
#[cfg(feature = "rdf")]
fn export_rdf(ont: &Ontology, report: &validate::Report, format: Option<&str>, out: Option<PathBuf>) -> Result<()> {
    let fmt = match (format, out.as_deref().and_then(rdf::Format::from_path)) {
        (Some(f), _) => rdf::Format::parse(f)?,
        (None, Some(f)) => f,
        (None, None) => rdf::Format::Turtle,
    };
    if !report.ok() {
        eprintln!("note: this space has {} compile errors; the export shows it as it is", report.errors().len());
    }
    let graph = rdf::export(ont);
    let text = graph.write(fmt)?;
    match out {
        None => print!("{}", text),
        Some(p) => {
            if let Some(parent) = p.parent().filter(|x| !x.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&p, &text)?;
            eprintln!("wrote {}", p.display());
        }
    }
    for n in &graph.notes {
        eprintln!("note: {}", n);
    }
    Ok(())
}

#[cfg(not(feature = "rdf"))]
fn export_rdf(_: &Ontology, _: &validate::Report, _: Option<&str>, _: Option<PathBuf>) -> Result<()> {
    bail!("this lorefile was built without RDF support (the `rdf` feature); use a full or slim release build, or `cargo build --features rdf`")
}

/// `lorefile import --from-rdf`: read the graph, build the files, compile, write only if clean.
#[cfg(feature = "rdf")]
#[allow(clippy::too_many_arguments)]
fn import_rdf(file: &Path, into: Option<&Path>, only: &[String], depth: usize, report: bool, keep_grades: bool, no_parents: bool, as_json: bool) -> Result<()> {
    let into = match into {
        Some(p) => p.to_path_buf(),
        None => {
            let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "imported".into());
            PathBuf::from(stem)
        }
    };
    let w = rdf::import_into(file, &into, &rdf::ImportOptions { only: only.to_vec(), depth, keep_grades, no_parents })?;
    let im = &w.imported;
    if as_json {
        print_json(&json!({ "into": w.dir, "files": w.files, "counts": im.counts, "warnings": w.warnings, "notes": im.notes }));
        return Ok(());
    }
    for f in &w.files {
        eprintln!("wrote {}", w.dir.join(f).display());
    }
    let counts: Vec<String> = im.counts.iter().map(|(k, n)| format!("{} {}", n, k.replace('_', " "))).collect();
    eprintln!("✓ imported into {}: {} · {} warnings", w.dir.display(), counts.join(" · "), w.warnings);
    if report {
        for n in &im.notes {
            eprintln!("  · {}", n);
        }
    } else if !im.notes.is_empty() {
        eprintln!("{} notes on what was guessed, dropped or kept for a person; add --report to list them", im.notes.len());
    }
    eprintln!("Next: lorefile validate --dir {}", w.dir.display());
    Ok(())
}

#[cfg(not(feature = "rdf"))]
#[allow(clippy::too_many_arguments)]
fn import_rdf(_: &Path, _: Option<&Path>, _: &[String], _: usize, _: bool, _: bool, _: bool, _: bool) -> Result<()> {
    bail!("this lorefile was built without RDF support (the `rdf` feature); use a full or slim release build, or `cargo build --features rdf`")
}

/// `lorefile import --from-ossie`: build the files, compile, write only if clean.
#[allow(clippy::too_many_arguments)]
fn import_ossie(sm: &Path, ontology: Option<&Path>, mapping: Option<&Path>, into: &Path, report: bool, keep_grades: bool, as_json: bool) -> Result<()> {
    let w = ossie::import_into(sm, ontology, mapping, into, keep_grades)?;
    let im = &w.imported;
    if as_json {
        print_json(&json!({
            "into": w.dir, "files": w.files, "entities": im.entities, "links": im.links, "metrics": im.metrics,
            "glossary_terms": im.terms, "warnings": w.warnings, "notes": im.notes,
        }));
        return Ok(());
    }
    for f in &w.files {
        eprintln!("wrote {}", w.dir.join(f).display());
    }
    eprintln!(
        "✓ imported into {}: {} entities · {} links · {} metrics · {} glossary terms · {} warnings",
        w.dir.display(), im.entities, im.links, im.metrics, im.terms, w.warnings
    );
    if report {
        for n in &im.notes {
            eprintln!("  · {}", n);
        }
    } else if !im.notes.is_empty() {
        eprintln!("{} notes on what was guessed or dropped; add --report to list them", im.notes.len());
    }
    eprintln!("Next: lorefile validate --dir {}", w.dir.display());
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

/// The flags as one config. A space commits by default when its history is its own;
/// `--commit-edits` commits everywhere, `--no-commit-edits` nowhere.
fn edit_config(allow: bool, commit: bool, no_commit: bool) -> EditConfig {
    let policy = if no_commit {
        edit::CommitPolicy::Never
    } else if commit {
        edit::CommitPolicy::Always
    } else {
        edit::CommitPolicy::Auto
    };
    EditConfig { allow, commit: false, policy }
}

fn init(target: &Path) -> Result<()> {
    if target.join("ontology.yaml").exists() {
        bail!("{} already has an ontology.yaml", target.display());
    }
    spaces::write_starter(target)?;
    for (rel, _) in spaces::STARTER {
        eprintln!("wrote {}", target.join(rel).display());
    }
    let history = edit::init_space(target, target, "person via CLI");
    match history.as_str() {
        "on" => eprintln!("history on: a git repository with one commit, \"Start the space\""),
        "off (inside another repository)" => eprintln!("history: the folder is inside another git repository, so commit it with the rest of that project"),
        other => eprintln!("history {}", other),
    }
    eprintln!("\nNext: lorefile validate --dir {}\n      lorefile serve --dir {} --allow-edit", target.display(), target.display());
    Ok(())
}
