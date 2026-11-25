//! Spaces: one server, many ontologies. A "space" is one ontology folder.
//!
//! * Pointed at a folder that has an `ontology.yaml`, the server has one space
//!   (the old behaviour).
//! * Pointed at any other folder, every sub-folder with an `ontology.yaml` is a
//!   space: `spaces/acme/`, `spaces/brand-two/`. New spaces are created there.
//! * The examples and packs compiled into the binary are shipped spaces: their
//!   ids start with `@` (`@supply-chain`, `@meta-ads`), they are read-only, and
//!   they are written out under `<root>/.stella/examples/` at start-up so the
//!   loader and DuckDB can read them like any folder.

use crate::data::DataLayer;
#[cfg(feature = "duckdb")]
use crate::duck::Db;
use crate::load_ontology;
use crate::ontology::Ontology;
use crate::validate::Report;
use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "duckdb")]
use std::sync::Arc;

mod builtin {
    include!(concat!(env!("OUT_DIR"), "/builtin.rs"));
}

/// Files `stella-ontology init` writes, also the "starter" choice for a new space.
pub const STARTER: [(&str, &str); 5] = [
    ("ontology.yaml", include_str!("starter/ontology.yaml")),
    ("glossary.yaml", include_str!("starter/glossary.yaml")),
    ("types/customer.yaml", include_str!("starter/types/customer.yaml")),
    ("constraints/basics.yaml", include_str!("starter/constraints/basics.yaml")),
    ("actions/close_account.yaml", include_str!("starter/actions/close_account.yaml")),
];

#[derive(Debug, Clone, Serialize)]
pub struct Space {
    /// URL- and tool-safe id: the folder name, or `@name` for a shipped one.
    pub id: String,
    /// Shown in the picker: the folder name until the manifest is read.
    pub name: String,
    pub dir: PathBuf,
    /// Shipped inside the binary; always read-only.
    pub builtin: bool,
    /// `space` (yours), `example` or `pack`.
    pub group: String,
    /// Filled lazily from the manifest: title, description, counts, validity.
    #[serde(flatten)]
    pub stats: Option<Json>,
}

/// One compiled space plus its DuckDB, when that feature is on.
pub struct Loaded {
    pub dir: PathBuf,
    pub ont: Ontology,
    pub report: Report,
    #[cfg(feature = "duckdb")]
    pub db: Option<Arc<Db>>,
    with_db: bool,
}

impl Loaded {
    pub fn open(dir: &Path, with_db: bool) -> Result<Loaded> {
        let (ont, report) = load_ontology(dir)?;
        #[cfg(feature = "duckdb")]
        let db = if with_db {
            match Db::open(&ont) {
                Ok(db) => Some(Arc::new(db)),
                Err(e) => {
                    eprintln!("warn  DuckDB could not open for {}: {:#}", dir.display(), e);
                    None
                }
            }
        } else {
            None
        };
        Ok(Loaded {
            dir: dir.to_path_buf(),
            ont,
            report,
            #[cfg(feature = "duckdb")]
            db,
            with_db,
        })
    }

    pub fn data(&self) -> DataLayer<'_> {
        #[cfg(feature = "duckdb")]
        if let Some(db) = &self.db {
            return DataLayer::with_db(&self.ont, db.clone());
        }
        DataLayer::new(&self.ont)
    }

    /// Re-read the folder after a write. Keeps the old state if the reload fails.
    pub fn reload(&mut self) {
        match Loaded::open(&self.dir, self.with_db) {
            Ok(l) => *self = l,
            Err(e) => eprintln!("warn  reload of {} failed: {:#}", self.dir.display(), e),
        }
    }
}

pub struct Registry {
    pub root: PathBuf,
    /// True when `root` itself is an ontology folder (one space, no creating).
    pub single: bool,
    pub spaces: IndexMap<String, Space>,
    /// Where shipped examples were written, if they were.
    pub examples_dir: Option<PathBuf>,
    loaded: HashMap<String, Loaded>,
    with_db: bool,
}

fn has_manifest(dir: &Path) -> bool {
    dir.join("ontology.yaml").is_file() || dir.join("ontology.yml").is_file()
}

/// `<root>/.stella`, created with a `.gitignore` that hides itself from git.
pub fn stella_dir(root: &Path) -> PathBuf {
    let d = root.join(".stella");
    if std::fs::create_dir_all(&d).is_ok() {
        let gi = d.join(".gitignore");
        if !gi.exists() {
            let _ = std::fs::write(gi, "*\n");
        }
    }
    d
}

/// Lower-case letters, digits and dashes; what a folder and a URL both accept.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let s = out.trim_end_matches('-').to_string();
    s.chars().take(48).collect::<String>().trim_end_matches('-').to_string()
}

/// Write the shipped examples and packs under `dest`. Only files whose bytes
/// differ are rewritten, so a restart is cheap. Returns the shipped spaces.
fn extract_builtin(dest: &Path) -> Result<Vec<Space>> {
    let mut seen: IndexMap<String, String> = IndexMap::new();
    for (group, rel, bytes) in builtin::FILES {
        let (name, _) = rel.split_once('/').ok_or_else(|| anyhow!("bad embedded path {}", rel))?;
        seen.entry(name.to_string()).or_insert_with(|| group.to_string());
        let p = dest.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let same = std::fs::read(&p).map(|cur| cur.as_slice() == *bytes).unwrap_or(false);
        if !same {
            std::fs::write(&p, bytes).with_context(|| format!("writing {}", p.display()))?;
        }
    }
    Ok(seen
        .into_iter()
        .map(|(name, group)| Space { id: format!("@{}", name), name: name.clone(), dir: dest.join(&name), builtin: true, group, stats: None })
        .collect())
}

impl Registry {
    /// Find the spaces under `root`. With `with_examples`, the shipped examples
    /// are written out and listed too. `with_db` says whether `get` opens DuckDB.
    pub fn discover(root: &Path, with_examples: bool, with_db: bool) -> Result<Registry> {
        let single = has_manifest(root);
        if !single && !root.exists() {
            std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
        }
        let mut reg = Registry { root: root.to_path_buf(), single, spaces: IndexMap::new(), examples_dir: None, loaded: HashMap::new(), with_db };
        reg.scan_user_spaces();
        if with_examples {
            let mut dest = stella_dir(root).join("examples");
            let mut shipped = extract_builtin(&dest);
            if shipped.is_err() {
                // a read-only folder: fall back to the temp dir
                dest = std::env::temp_dir().join("stella-ontology").join("examples");
                shipped = extract_builtin(&dest);
            }
            match shipped {
                Ok(list) => {
                    reg.examples_dir = Some(dest);
                    for s in list {
                        reg.spaces.insert(s.id.clone(), s);
                    }
                }
                Err(e) => eprintln!("warn  shipped examples are unavailable: {:#}", e),
            }
        }
        Ok(reg)
    }

    /// Re-read the user spaces: picks up folders added or removed outside the server.
    pub fn rescan(&mut self) {
        let builtins: Vec<Space> = self.spaces.values().filter(|s| s.builtin).cloned().collect();
        self.spaces.retain(|_, s| s.builtin);
        self.scan_user_spaces();
        for b in builtins {
            self.spaces.entry(b.id.clone()).or_insert(b);
        }
        self.loaded.retain(|id, _| self.spaces.contains_key(id));
    }

    fn scan_user_spaces(&mut self) {
        let mut found: Vec<Space> = Vec::new();
        if self.single {
            let name = self.root.canonicalize().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "ontology".into());
            let id = { let s = slug(&name); if s.is_empty() { "ontology".to_string() } else { s } };
            found.push(Space { id, name, dir: self.root.clone(), builtin: false, group: "space".into(), stats: None });
        } else if let Ok(rd) = std::fs::read_dir(&self.root) {
            let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            for d in dirs {
                let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if name.starts_with('.') || !has_manifest(&d) {
                    continue;
                }
                found.push(Space { id: name.clone(), name, dir: d, builtin: false, group: "space".into(), stats: None });
            }
        }
        // user spaces first, shipped ones after
        let mut rebuilt: IndexMap<String, Space> = IndexMap::new();
        for s in found {
            rebuilt.insert(s.id.clone(), s);
        }
        for (id, s) in self.spaces.drain(..) {
            rebuilt.entry(id).or_insert(s);
        }
        self.spaces = rebuilt;
    }

    pub fn user_spaces(&self) -> Vec<&Space> {
        self.spaces.values().filter(|s| !s.builtin).collect()
    }

    pub fn info(&self, id: &str) -> Option<&Space> {
        self.spaces.get(id)
    }

    /// The space a request gets when it names none: the first of yours, else the first example.
    pub fn default_id(&self) -> Option<String> {
        self.user_spaces().first().map(|s| s.id.clone()).or_else(|| self.spaces.keys().next().cloned())
    }

    /// Pick a space for a caller that may not have named one. Exactly one space
    /// of yours means it is implied; otherwise the caller has to say.
    pub fn resolve(&self, want: Option<&str>) -> Result<String, String> {
        if let Some(w) = want.map(str::trim).filter(|w| !w.is_empty()) {
            if self.spaces.contains_key(w) {
                return Ok(w.to_string());
            }
            let lower = w.to_ascii_lowercase();
            if let Some(s) = self.spaces.values().find(|s| s.id.to_ascii_lowercase() == lower || s.name.to_ascii_lowercase() == lower || s.id.trim_start_matches('@') == lower) {
                return Ok(s.id.clone());
            }
            return Err(format!("no space named '{}'. Spaces here: {}", w, self.ids_sentence()));
        }
        let mine = self.user_spaces();
        match mine.len() {
            1 => Ok(mine[0].id.clone()),
            0 => match self.spaces.keys().next() {
                Some(first) if self.spaces.len() == 1 => Ok(first.clone()),
                _ => Err(format!("no space of your own yet. Say which shipped example you mean (space: \"@supply-chain\") or create a space. Spaces here: {}", self.ids_sentence())),
            },
            _ => Err(format!("say which space you mean (space: \"{}\"). Spaces here: {}", mine[0].id, self.ids_sentence())),
        }
    }

    fn ids_sentence(&self) -> String {
        self.spaces.keys().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// The compiled space, loaded on first use.
    pub fn get(&mut self, id: &str) -> Result<&mut Loaded> {
        let space = self.spaces.get(id).ok_or_else(|| anyhow!("no space named '{}'", id))?.clone();
        if !self.loaded.contains_key(id) {
            let l = Loaded::open(&space.dir, self.with_db)?;
            self.loaded.insert(id.to_string(), l);
        }
        Ok(self.loaded.get_mut(id).unwrap())
    }

    /// Drop the cached stats of one space (after a write).
    pub fn touched(&mut self, id: &str) {
        if let Some(s) = self.spaces.get_mut(id) {
            s.stats = None;
        }
    }

    /// Every space with its manifest title, counts and validity, for the picker.
    pub fn summaries(&mut self) -> Vec<Json> {
        let ids: Vec<String> = self.spaces.keys().cloned().collect();
        let mut out = Vec::new();
        for id in ids {
            if self.spaces[&id].stats.is_none() {
                let stats = match self.loaded.get(&id) {
                    Some(l) => Some(stats_json(&l.ont, &l.report)),
                    None => match load_ontology(&self.spaces[&id].dir) {
                        Ok((o, r)) => Some(stats_json(&o, &r)),
                        Err(e) => Some(json!({ "title": self.spaces[&id].name, "description": "", "types": 0, "rules": 0, "ok": false, "error": e.to_string() })),
                    },
                };
                self.spaces.get_mut(&id).unwrap().stats = stats;
            }
            let s = &self.spaces[&id];
            let mut j = json!({ "id": s.id, "name": s.name, "dir": s.dir.display().to_string(), "builtin": s.builtin, "group": s.group });
            if let Some(Json::Object(st)) = &s.stats {
                for (k, v) in st {
                    j[k] = v.clone();
                }
            }
            out.push(j);
        }
        out
    }

    /// Create a space folder under the root. `from` is `blank`, `starter`, or
    /// the id of a space to copy (a shipped example, or one of yours).
    pub fn create(&mut self, name: &str, from: &str) -> Result<Space, String> {
        if self.single {
            return Err(format!(
                "this server was started on one ontology folder ({}). To hold several spaces, start it on a folder that contains them: stella-ontology serve --dir ./spaces",
                self.root.display()
            ));
        }
        let id = slug(name);
        if id.is_empty() || !id.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
            return Err("give the space a name that starts with a letter".into());
        }
        if self.spaces.contains_key(&id) || self.root.join(&id).exists() {
            return Err(format!("a space called '{}' already exists", id));
        }
        let dir = self.root.join(&id);
        let result = match from {
            "" | "blank" => write_blank(&dir, &id, name.trim()),
            "starter" => write_starter(&dir).and_then(|_| rename_manifest(&dir, &id, name.trim())),
            other => match self.spaces.get(other).cloned() {
                Some(src) => copy_tree(&src.dir, &dir).and_then(|_| rename_manifest(&dir, &id, name.trim())),
                None => Err(anyhow!("nothing to copy: no space named '{}'", other)),
            },
        };
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("{:#}", e));
        }
        // compile it once so a broken copy never becomes a space (a blank one
        // is allowed to fail validation until its first thing is added)
        if let Err(e) = load_ontology(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("the new space does not compile: {:#}", e));
        }
        let space = Space { id: id.clone(), name: id.clone(), dir, builtin: false, group: "space".into(), stats: None };
        let at = self.user_spaces().len();
        self.spaces.shift_insert(at, id, space.clone());
        Ok(space)
    }
}

fn stats_json(ont: &Ontology, report: &Report) -> Json {
    let st = ont.stats();
    json!({
        "title": ont.manifest.name,
        "description": ont.manifest.description,
        "types": st.types, "relationships": st.relationships, "rules": st.constraints, "actions": st.actions, "sources": st.sources,
        "ok": report.ok(), "errors": report.errors().len(), "warnings": report.warnings().len(),
    })
}

fn write_blank(dir: &Path, id: &str, title: &str) -> Result<()> {
    std::fs::create_dir_all(dir.join("types"))?;
    let text = format!(
        "name: {id}\nversion: 0.1.0\ndescription: >\n  {title}. Say in one or two sentences what this business does and what the\n  agents reading this should know first.\ndomains:\n  - name: Core\n    description: The main things this business works with.\n    types: []\n",
    );
    std::fs::write(dir.join("ontology.yaml"), text)?;
    std::fs::write(dir.join("glossary.yaml"), "# Business words and what they mean here, e.g. `basket` means Order.\nterms: []\n")?;
    Ok(())
}

/// The small starter ontology `init` writes.
pub fn write_starter(dir: &Path) -> Result<()> {
    for (rel, content) in STARTER {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, content)?;
    }
    Ok(())
}

/// Point the copied manifest's `name:` at the new space, leaving the rest of the file as it was.
fn rename_manifest(dir: &Path, id: &str, title: &str) -> Result<()> {
    let p = if dir.join("ontology.yaml").exists() { dir.join("ontology.yaml") } else { dir.join("ontology.yml") };
    let text = std::fs::read_to_string(&p)?;
    let mut out = String::new();
    let mut renamed = false;
    for line in text.lines() {
        if !renamed && line.starts_with("name:") {
            out.push_str(&format!("name: {}\n", id));
            renamed = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !renamed {
        out = format!("name: {}\n{}", id, out);
    }
    let _ = title;
    std::fs::write(p, out)?;
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    Ok(())
}

/// Validate every space of yours; the exit code of `validate` on a spaces folder.
pub fn validate_all(reg: &mut Registry) -> Result<bool> {
    let ids: Vec<String> = reg.user_spaces().iter().map(|s| s.id.clone()).collect();
    if ids.is_empty() {
        bail!("{} holds no space yet. Create one in the web page, or validate a shipped example with --space @supply-chain", reg.root.display());
    }
    let mut all_ok = true;
    for id in ids {
        let l = reg.get(&id)?;
        eprintln!("── {} ({})", id, l.dir.display());
        crate::print_report(&l.ont, &l.report);
        all_ok &= l.report.ok();
    }
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stella-spaces-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn slugs_are_folder_and_url_safe() {
        assert_eq!(slug("Brand Two!"), "brand-two");
        assert_eq!(slug("  ACME   Retail  "), "acme-retail");
        assert_eq!(slug("@shipped"), "shipped");
        assert_eq!(slug("---"), "");
    }

    #[test]
    fn a_folder_with_a_manifest_is_one_space_and_cannot_grow() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/supply-chain");
        let mut reg = Registry::discover(&root, false, false).unwrap();
        assert!(reg.single);
        assert_eq!(reg.user_spaces().len(), 1);
        assert_eq!(reg.default_id().as_deref(), Some("supply-chain"));
        assert!(reg.create("Other", "blank").is_err());
        assert_eq!(reg.resolve(None).unwrap(), "supply-chain");
    }

    #[test]
    fn an_empty_folder_holds_shipped_examples_and_grows_by_creating() {
        let root = scratch("multi");
        let mut reg = Registry::discover(&root, true, false).unwrap();
        assert!(!reg.single);
        assert!(reg.user_spaces().is_empty());
        assert!(reg.spaces.contains_key("@supply-chain"), "shipped examples are listed");
        assert!(reg.spaces.contains_key("@meta-ads"), "packs too");
        assert!(root.join(".stella/.gitignore").exists(), ".stella hides itself from git");
        // nothing of yours yet: a caller has to name the example
        assert!(reg.resolve(None).is_err());
        assert_eq!(reg.resolve(Some("supply-chain")).unwrap(), "@supply-chain");

        let s = reg.create("Brand One", "blank").unwrap();
        assert_eq!(s.id, "brand-one");
        assert!(root.join("brand-one/ontology.yaml").exists());
        assert_eq!(reg.resolve(None).unwrap(), "brand-one", "one space of yours is implied");
        assert_eq!(reg.spaces.keys().next().unwrap(), "brand-one", "yours come first");

        let c = reg.create("Brand Two", "@ecommerce").unwrap();
        let manifest = std::fs::read_to_string(root.join("brand-two/ontology.yaml")).unwrap();
        assert!(manifest.starts_with("name: brand-two\n"), "copied manifest is renamed: {}", manifest);
        assert!(root.join("brand-two/types").is_dir());
        assert!(!root.join("brand-two/.stella").exists(), "state folders are not copied");
        assert_eq!(c.group, "space");
        assert!(reg.resolve(None).is_err(), "two of yours: the caller must say which");
        assert!(reg.create("brand two", "blank").is_err(), "same slug twice is refused");
        assert!(reg.create("Nope", "@nothing").is_err());

        let loaded = reg.get("brand-two").unwrap();
        assert!(loaded.report.ok());
        assert!(reg.get("@shopify").unwrap().report.ok());
        let summaries = reg.summaries();
        assert_eq!(summaries[0]["id"], "brand-one");
        assert!(summaries.iter().any(|s| s["id"] == "@google-ads" && s["group"] == "pack"));

        // a second discovery sees the folders that were created
        let reg2 = Registry::discover(&root, false, false).unwrap();
        assert_eq!(reg2.user_spaces().len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
