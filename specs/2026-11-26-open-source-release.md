# Spec: the open-source release, as Lorefile

Written 2026-11-26 for hand-off to build sessions. Vinay's decisions this round: pick a neutral name and launch under a new GitHub org; focus on the open-source version only (the hosted version comes later); support `npx` and Docker for local use, with a login like other open-source tools have; support Apache Ossie. This spec also records the answers to his questions on Rust and DuckDB, because they decide what gets built.

Read [BACKLOG.md](../BACKLOG.md) for the standing rules (the ontology never executes and never holds third-party credentials; YAML is written by agents and builders, people review; light theme only; honest status). Build notes for every session: `cargo build --offline` works, run cargo with the sandbox off, zsh wants `$?` not `PIPESTATUS`; `cargo test` runs 50 tests (after Parts B and C) and must stay green; every folder under `examples/` and `packs/` must compile with no errors and no warnings (`tests/cli.rs`); the preview server `ontology-spaces` in `.claude/launch.json` runs the debug binary on port 7076 (rebuild, then restart it, or run your own on another port). Commit nothing; Vinay commits.

---

## Decisions

### The name: Lorefile

**Lore** is what a team knows and newcomers have to be told: what counts as a sale, where the orders live, which number the business is run on, what an agent may never do. **Lorefile** is that lore written down in files that agents read. It says what the product is, it is one word, and it is free today on npm, crates.io, PyPI and GitHub, with `lorefile.dev` and `lorefile.io` not resolving (`lorefile.com` does). Runners-up, also free everywhere: **nounbook** (from "start with the nouns of the business"; reads as entities only, and the product has metrics, guardrails and actions too) and **ontofile** (accurate, flat). Vinay checks the trademark classes and registers the domain; this is not a legal clearance.

How the name appears, so every session spells it the same way:

| Where | Spelling |
|---|---|
| Product, docs, page header | Lorefile |
| Binary, crate, npm package, Docker image, GitHub org and repo | `lorefile` |
| Per-space state folder (was `.stella/`) | `.lorefile/` |
| Write header (was `X-Stella: write`) | `X-Lorefile: write` |
| Environment variables | `LOREFILE_TOKEN`, `LOREFILE_DIR` |
| MCP server name and `.mcp.json` entry | `lorefile` |
| Attribution line in README and NOTICE | "Built by the team at Stella." |

The shipped example ontology called `lumen-ecommerce` is a fictional shop and stays as it is.

### Open source only, for now

Every "Stella Cloud" section shrinks to one paragraph that says a hosted version may come later and what it would add (one safe copy, SSO, audit, GitHub sync). Pricing tables, the hosted roadmap and the hosted rows of status tables go to BACKLOG.md under one heading, "Hosted version, later". The research found that NVIDIA, Databricks, Snowflake and Microsoft all shipped agent ontologies this autumn, each either running the queries or locked to one platform; ARCHITECTURE.md's claim that no open, lightweight, agent-first equivalent exists is rewritten to say what is still distinctive: a keeper only, spanning warehouses and non-warehouse MCPs, with actions, guardrails and approvers, in plain files in git, speaking Ossie.

### Rust stays. Why, in plain words

| | Rust (today) | Next.js instead |
|---|---|---|
| What a user installs | One file, no runtime. `npx lorefile` downloads it and runs it, the same way esbuild, Biome and Turborepo ship. | Node.js first, then the package and its dependencies. |
| Docker image | About 40 MB, starts in milliseconds. | 150 to 300 MB, starts in seconds. |
| CLI, MCP over stdio, web page and API | One process does all four from the same compiled ontology. | A web framework; the CLI and the stdio MCP would be separate programs. |
| DuckDB, CEL, YAML | In-process libraries. | Possible through Node bindings; CEL support is thinner. |
| Who can contribute | Fewer people write Rust. | Many more write TypeScript. |
| The page | One HTML file served by the binary. Plain, fast, no build step. | A richer UI framework. |

Rust is not there for DuckDB; it is there so the whole tool is one file that anyone can run anywhere, which is what `npx` and a small Docker image need. The honest cost is the contributor pool and a plainer UI. Decision: keep the engine in Rust; keep the page a single file for now; if the UI ever needs a framework it becomes a separate front end talking to the same API, with no change to the engine.

### DuckDB: what it is here, and what it is not

Lorefile stores no rows. DuckDB is an in-process SQL engine (think SQLite for analytics) used for exactly one thing, the **local-files door**: when a source is a CSV, Parquet or `.duckdb` file on disk, the binary can read it so that the `query` tool and the Data & SQL page can run a `SELECT` over those files, and `validate` can check a guardrail against a real row without a second MCP. It reads only those files, fenced to the space's folders, read-only, no network, and it is compiled in as an optional feature (`--no-default-features` drops it). It never reads BigQuery, Snowflake, Meta Ads or anything remote; agents fetch those rows with their own MCPs and their own logins. Graph questions (`neighbors`, `describe`, `search`, `where_is`) run on the in-memory graph in Rust, not in DuckDB.

Decision: keep it, as the optional feature it is, because the shipped examples use it for demos and it costs the user nothing. The cost is to the build (DuckDB compiles from source, about 5 to 10 minutes) and to the download (the binary is about 70 MB instead of 10). The release workflow therefore builds both: `lorefile` (full) and `lorefile-slim` (no DuckDB) as separate assets; `npx` ships the full one.

### Login, the way other open-source tools do it

Three patterns exist: no login and "put a proxy in front" (Lorefile today); a token, generated on first run and printed in the terminal (Jupyter, code-server, Ollama's remote mode); accounts with email and password plus optional single sign-on (Langfuse, Metabase, Grafana). Decision: **the token pattern now**, accounts and SSO later (backlog). One token is enough while the tool is "a team's shared copy behind one URL", it needs no database, and it is what `npx` and Docker users expect. Details in Part B.

### MCP over HTTP

Docker users cannot run the MCP over stdio from inside a container, so the server gains a second transport: MCP's Streamable HTTP at `POST /mcp`, protected by the same token. Claude Code connects with `claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer <token>"`. Details in Part B.

### Apache Ossie

[Apache Ossie](https://github.com/apache/ossie) (incubating, formerly Open Semantic Interchange) is a draft YAML/JSON interchange format, version `0.3.0.dev0`, Apache-2.0, backed by Snowflake, dbt, Cube, AtScale, Atlan, DataHub and Collibra; NVIDIA's Auto Ontology imports and exports it. Supporting it makes Lorefile the open home a draft can be moved into, and the format others can read Lorefile from. Decision: build `export --ossie` and `import --from-ossie`. Details in Part D.

---

## Part A · Rename, and open source only

One session, first, alone: it touches nearly every file.

1. **Code.** `Cargo.toml` package and `[[bin]]` name `lorefile`; `src/main.rs` command name and help text; `.stella` → `.lorefile` everywhere (`spaces.rs` `stella_dir`, proposals, activity log, examples extraction path `.lorefile/examples`, the self-ignoring `.gitignore`, tests); `X-Stella` → `X-Lorefile` in `serve.rs`, `index.html` and `tests/http.rs`; MCP server name and instructions in `mcp.rs`; git identity in `edit.rs` (`Lorefile <lorefile@localhost>`); page title, header and footer text in `index.html`; `.mcp.json`; `.claude/launch.json` (binary path); `Dockerfile` (binary name, user `lorefile`, volume stays `/spaces`); `fly.toml`, `vercel.json`, `.dockerignore` as needed; `.github/workflows/ci.yml`; `build.rs` unchanged. The repository is also the crate, so `cargo build --offline`, `cargo test` and the three example folders must pass after the rename.
2. **Docs.** README.md (title, every command, every path, the status line, the attribution line "Built by the team at Stella."), ARCHITECTURE.md (name; §12 rewritten as decided above; §17 hosted parts shortened to one paragraph), PRODUCTION.md and DEPLOY.md (name; hosted sections to one paragraph each), BACKLOG.md (new heading "Hosted version, later" holding the pricing table, the hosted roadmap rows and the hosted backlog items), NOTES-mcp-trial.md (name only), `specs/` (leave as history), LICENSE (unchanged), new `NOTICE` file. Rebuild the tutorial last: `python3 scripts/build-tutorial.py`.
3. **Do not** rename the fictional `lumen-ecommerce` example, change the YAML format, or touch behaviour.

Done looks like: `grep -ri stella` over the repository finds only the attribution line, the `specs/` history, NOTES-mcp-trial.md's title, BACKLOG's hosted heading and the example's Stella MCP functions (`sources/stella.yaml` in the ecommerce example names the Stella MCP as a data source, which is correct and stays); the binary answers `lorefile --help`; 41 tests pass; the page header says Lorefile; the tutorial is rebuilt.

---

## Part B · Login and MCP over HTTP

Files: `src/serve.rs`, `src/mcp.rs`, `src/main.rs`, `src/spaces.rs` (state folder), `src/ui/index.html`, `tests/http.rs`, new `tests/mcp_http.rs`, PRODUCTION.md and DEPLOY.md (their auth sections). Not README.md (Part D owns it).

### Before

```
 $ lorefile serve --dir spaces --allow-edit
 http://127.0.0.1:7070                         anyone who can reach the port can read and, with the header, write
 MCP: stdio only                               a container cannot be used from Claude Desktop or Claude Code
```

### After

```
 $ npx lorefile serve --dir spaces --allow-edit
 Lorefile 0.2.0 · spaces in ./spaces · edit mode on
 Open  http://127.0.0.1:7070/?token=k3f9…        (first run: token saved to spaces/.lorefile/auth.toml)
 MCP   http://127.0.0.1:7070/mcp                 Authorization: Bearer k3f9…
       claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer k3f9…"

 ┌─ page, first visit without the token ────────────────┐
 │  Lorefile                                             │
 │  Paste the token printed in the terminal  [ …… ] [Go] │
 │  Running in Docker? It is in LOREFILE_TOKEN.          │
 └───────────────────────────────────────────────────────┘
```

### What to build

1. **Token.** On `serve`, resolve the token in this order: `--token`, `LOREFILE_TOKEN`, `<dir>/.lorefile/auth.toml` (`token = "…"`, file mode 0600), else generate 32 random bytes as base64url, save it there, and print the login URL once. `--no-auth` turns it off and prints a one-line warning; it is refused together with `--host 0.0.0.0` unless `--i-know` is also given (the Docker command in DEPLOY.md passes a token instead).
2. **Login.** `GET /?token=…` and `POST /api/login {token}` set an `HttpOnly; SameSite=Strict` cookie holding the token (or a session id mapped to it) and redirect to `/`; `POST /api/logout` clears it. Every other route requires the cookie or `Authorization: Bearer <token>`; without one, page routes get the login screen and API routes get 401 with a one-line JSON error. The existing `X-Lorefile: write` plus matching `Origin` check stays as the cross-site guard for writes; the token does not replace it. Compare tokens in constant time. Nothing else changes in the write gate: `--allow-edit` still decides whether writes are possible at all.
3. **MCP over HTTP.** `POST /mcp` takes one JSON-RPC message (a request, a notification, or a batch), runs it through the same `Hub` as stdio, and answers with `application/json` (the Streamable HTTP transport allows a plain JSON response; no SSE stream and no `Mcp-Session-Id` are needed for this stateless server). `GET /mcp` answers 405, `DELETE /mcp` 204. `initialize` reports the same capabilities and instructions as stdio; `tools/list` and `tools/call` behave identically, including `space`. `--allow-edit` applies the same way. Body cap 4 MB as elsewhere. Document the Claude Code command above and note that Claude Desktop's custom connectors need HTTPS, so for Desktop on a laptop stdio stays the way (`.mcp.json`), and for a server put HTTPS in front (Caddy or the platform's proxy).
4. **Docker.** `DEPLOY.md`'s run command passes `-e LOREFILE_TOKEN=…`; the Dockerfile needs no change beyond the name.
5. **Page.** A login screen (the box above), a Sign out link in Settings, and Settings' "MCP for agents" row shows both commands (stdio for laptops, HTTP for containers) with the token masked and a copy button.
6. **Tests.** `tests/http.rs`: no token → 401 on `/api/summary` and the login screen on `/`; bearer works; cookie works after `POST /api/login`; wrong token → 401; `--no-auth` keeps the old behaviour; writes still need the header and Origin. `tests/mcp_http.rs`: `initialize`, `tools/list`, `summary` and a `propose_patch` over HTTP with the bearer token, the same assertions as `tests/mcp_loop.rs` makes over stdio; without the token → 401.

Done looks like: `npx`-style first run prints the URL, the page asks for the token once and remembers it, Claude Code connects over HTTP with the command printed, Docker works with the env var, all tests green.

---

## Part C · Distribution: `npx`, Docker, `cargo install`

Files: new `npm/` folder, new `.github/workflows/release.yml`, `Dockerfile`, `.dockerignore`, new `scripts/install.sh`. Not README.md (Part D owns it); write the install text into `npm/README.md`, which is what npm shows.

### What to build

1. **GitHub Releases.** `release.yml` runs on tags `v*`: a build matrix of macOS arm64 and x86_64, Linux x86_64 and aarch64 (build on `ubuntu-22.04` so the glibc floor is old), Windows x86_64 (best effort: if the bundled DuckDB build fails on Windows, ship only the slim variant there and say so). Two variants per target: `lorefile` (default features) and `lorefile-slim` (`--no-default-features`). Each asset is a `.tar.gz` (`.zip` on Windows) named `lorefile-<version>-<target>[-slim].tar.gz` with a `.sha256` file next to it, plus one `checksums.txt`. The release notes are the tag's annotation. A dry run on a branch (`workflow_dispatch`) must pass before the first tag.
2. **npm launcher.** `npm/package.json` name `lorefile`, `bin: { lorefile: "bin/lorefile.js" }`, no dependencies, Node 18 or newer. `bin/lorefile.js`: on first run, work out the platform and architecture, download the matching full asset for the package's own version from GitHub Releases into `~/.cache/lorefile/<version>/` (respect `XDG_CACHE_HOME` and `LOREFILE_BINARY` to point at a local build), verify the sha256 against the published checksum, mark it executable, then `spawn` it with the user's arguments and stdio inherited, exiting with its code. Print one progress line while downloading. No postinstall script. `npm/README.md` is the install page. Publishing is a job in `release.yml` guarded by the `NPM_TOKEN` secret existing (Vinay adds it); until then the job is skipped with a notice.
3. **Docker.** `release.yml` builds a multi-arch image (`linux/amd64`, `linux/arm64`) with buildx and pushes `ghcr.io/<org>/lorefile:<version>` and `:latest`, using the repository's `GITHUB_TOKEN`; a `workflow_dispatch` run pushes `:edge`. The Dockerfile already runs as a non-root user with `/spaces` as the volume.
4. **`cargo install lorefile`.** A `release.yml` job runs `cargo publish` when the `CARGO_REGISTRY_TOKEN` secret exists; otherwise skipped with a notice. `Cargo.toml` gets `repository`, `homepage`, `readme`, `keywords`, `categories`, and `include` limited to what the crate needs (`src`, `examples`, `packs`, `build.rs`, LICENSE, README).
5. **`scripts/install.sh`.** The `curl -fsSL https://raw.githubusercontent.com/<org>/lorefile/main/scripts/install.sh | sh` path for people without Node: detect platform, download the latest full asset, verify the checksum, put the binary in `~/.local/bin` (or `/usr/local/bin` with sudo), print what it did. Homebrew tap: backlog.
6. **CI.** `ci.yml` gains a job that runs the npm launcher against a locally built binary (`LOREFILE_BINARY=target/debug/lorefile node npm/bin/lorefile.js --help`) so the launcher is tested without a release.

Done looks like: a `workflow_dispatch` dry run of `release.yml` produces the assets for every target in the matrix (the Windows full build may be marked best effort); `node npm/bin/lorefile.js --help` works with `LOREFILE_BINARY` set; `docker build` succeeds (if Docker is available on the machine; otherwise say so); the publish jobs are skipped cleanly without the secrets. Vinay then adds `NPM_TOKEN` and `CARGO_REGISTRY_TOKEN`, pushes a tag, and `npx lorefile` works for the world.

---

## Part D · Apache Ossie import and export, then the docs

Files: new `src/ossie.rs`, `src/main.rs` (two subcommands), `src/serve.rs` (one GET route), `src/ui/index.html` (one Settings row), new `tests/ossie.rs`, `tests/fixtures/ossie/` (vendored examples with their ASF headers), README.md, BACKLOG.md, the tutorial rebuild. This session runs last and also owns the README for everything Parts B and C built.

### The mapping

Ossie has two documents. The **semantic model** (`core-spec/spec.md`): root `version`, `name`, `description`, `ai_context`, `datasets[]` (each `name`, `source` as a dotted string like `sales.public.orders`, `primary_key`, `unique_keys`, `description`, `ai_context`, `fields[]` with `name`, `expression: {sql}`, `datatype`, `dimension: {is_time}`, `label`, `description`, and dataset-scoped `metrics[]`), model-scoped `metrics[]` (`name`, `expression: {sql, dialects}`, `description`, `datatype`), `relationships[]` (`name`, `from`, `to`, `from_columns`, `to_columns`), and `nonportable_extensions[]` (`{vendor_name, data}`) at every level, which consumers must preserve. The **ontology** document (`ontology/ontology.md`): `ontology[]` of concepts (`concept`, `type: EntityType|ValueType`, `description`, `extends`, `identify_by`, `relationships[]` with `name`, `multiplicity`, `roles[{concept, name}]`, `verbalizes`, `annotations`) and a **mapping** document linking concepts to datasets (`concept_mappings[{concept, object_mappings, link_mappings}]`). Datatypes: String, Integer, Decimal, Float, Boolean, Date, Time, DateTime, DateTimeTz, Opaque. There is no glossary; synonyms go in `ai_context`. Read the spec and the examples in the repository before writing a line; the format is a draft and may have moved since this was written.

| Lorefile | Ossie semantic model | Ossie ontology |
|---|---|---|
| Entity with a data binding | `dataset` (`source` = `<connection>.<table>`, `primary_key` = the key field, `fields` from properties with `expression: {sql: <column>}`, datatype mapped, descriptions carried) | concept `EntityType` with `description`, `identify_by` |
| Entity without a binding | nothing in the model | concept `EntityType` |
| Property | `field` | (annotation) |
| Metric with `source` + `measure`/`sql` | dataset-scoped `metric` when its table is one dataset; model-scoped otherwise, `expression.sql` from `sql` or built from `measure` + `filter` | |
| Metric variants, caveats, owner, grain | `nonportable_extensions` `{vendor_name: "lorefile", data: {...}}` | |
| Relationship with key columns | `relationship` with `from_columns`/`to_columns` | relationship with `verbalizes` = the link's sentence, `multiplicity` from cardinality |
| Relationship without columns | `nonportable_extensions` | relationship with `verbalizes` |
| Glossary term | `ai_context` on the dataset it means (synonyms) | annotation |
| Guardrails, actions, functions, sources of kind `mcp` | `nonportable_extensions` on the root, one entry per kind, verbatim | annotations |
| Space `ontology.yaml` name and description | root `name`, `description` | ontology `name`, `description` |
| Identity | | every concept carries `iri`: until Part E lands, the derived default `https://lorefile.dev/ns/<space>#<Name>`; after it, the entity's own `iri:` when set |

Ossie's ontology document requires a `verbalizes` sentence on every relationship. A link that has no sentence in Lorefile gets a generated one ("An Order is placed_by a Customer.") marked in the report, and the shipped ecommerce example's links get real sentences written in as part of this session (ten lack one today). Everything that has no Ossie home goes into `nonportable_extensions` under `vendor_name: "lorefile"` so an export followed by an import is lossless, and a file that passed through another tool keeps what it did not understand.

### What to build

1. `lorefile export --ossie [--dir <space>] [--out <file-or-folder>]` writes `<name>.semantic_model.yaml`, `<name>.ontology.yaml` and `<name>.mapping.yaml` (or `--json`). `GET /api/export?format=ossie` returns the three as one JSON object; Settings gets an "Export for other tools (Apache Ossie)" row with a download link.
2. `lorefile import --from-ossie <semantic_model.yaml> [--ontology <file>] [--mapping <file>] --into <space-folder>` creates or extends a space: datasets → entities and sources (one source per distinct connection prefix, kind `bigquery`/`snowflake`/`postgres` guessed from the prefix when obvious, else declared with a note), fields → properties, metrics → `logic/metrics.yaml`, relationships → `relationships/`, `ai_context` → descriptions and glossary terms, `lorefile` extensions restored verbatim. The result is compiled with the strict compiler; errors are reported with the file and line, and nothing is written when it does not compile. A `--report` line lists what was dropped or guessed.
3. **Tests.** Round-trip: export each shipped example, import it into a fresh folder, compile, and compare the compiled summaries (types, fields, metrics, relationships, terms) for equality. Fixtures: `minimal.yaml`, `flights.semantic_model.yaml`, `flights.ontology.yaml`, `flights.mapping.yaml` vendored from the Ossie repository with their headers; import each and assert the entity and metric counts. If `uv` and Python are present in CI, run Ossie's own `validation/validate.py` against an export as an optional job.
4. **Docs, the final pass** (done by the Part E session, after the RDF bridge, so the README is written once). README.md: the name throughout; an Install section with `npx lorefile`, Docker, `cargo install` and the install script; the login paragraph and the MCP over HTTP command; an "Other tools: Apache Ossie" section; the page tour's Settings line; the status line at the top; `scripts/build-tutorial.py` rebuild. BACKLOG.md: accounts and SSO, OAuth for Claude Desktop's and ChatGPT's connectors, DNS-rebinding protection for `--no-auth` servers, npm trusted publishing, Homebrew tap, Ossie dialects, `--push-edits`, sub-kinds (`extends`) so imported class hierarchies stop being tags, and whatever Parts B and C left out. `docs/standards-explained.md` (written 2026-11-26) is linked from the README and its "planned" markers are updated to what was built. ARCHITECTURE.md gets a short Ossie paragraph.

Done looks like: `lorefile export --ossie` on the ecommerce example validates with Ossie's own validator; importing it back gives an identical summary; the README reads as the tutorial for Lorefile with the four install paths and the login; the tutorial page is rebuilt.

---

## Part E · W3C standards: OWL, RDFS, SKOS and SHACL, with an IRI on everything

Vinay asked whether Lorefile uses the Web Ontology Language, and said he wants to use the existing standards first and make it all compatible. The honest answer today: no. Lorefile has no RDF, no OWL, no IRIs; its YAML is its own format. That is the right authoring format (agents write it, people read it, it diffs in git, and even LinkML's own FAQ says OWL should not be used as a schema language), but every entity, field, link and term should have a standard identity and the whole thing should round-trip with the W3C stack, so that Protégé, GraphDB, Stardog, TopQuadrant, LinkML and any company's existing OWL or SKOS files can read Lorefile and be read by it. Decision: **keep YAML as the source of truth, give everything an IRI, and build a two-way RDF bridge**, next to the Ossie one. Ossie covers the semantic-layer world (datasets, metrics); OWL, SKOS and SHACL cover the ontology world; Ossie's own ontology document carries `iri` fields and ships a `foaf_owl_import.yaml` example, so the two line up.

### What maps to what

| Lorefile | Standard | Notes |
|---|---|---|
| Space (`ontology.yaml` `name`, `description`) | `owl:Ontology` with `rdfs:label`, `rdfs:comment`, `owl:versionInfo` | base IRI from a new optional `namespace:` in `ontology.yaml`, default `https://lorefile.dev/ns/<space>#` |
| Entity | `owl:Class` (`rdfs:label`, `rdfs:comment`, `skos:definition`) | `tags` → `dcterms:subject` or `skos:note`; `domain` → `skos:inScheme` of a `skos:ConceptScheme` per domain; `owner` → `lorefile:owner` |
| Field (property with a primitive type) | `owl:DatatypeProperty` with `rdfs:domain` the entity and `rdfs:range` an XSD type (`xsd:string`, `xsd:integer`, `xsd:decimal`, `xsd:boolean`, `xsd:date`, `xsd:dateTime`) | `required` → SHACL `sh:minCount 1`; `enum` → `sh:in`; `description` → `rdfs:comment` |
| Link (relationship) | `owl:ObjectProperty` with `rdfs:domain` and `rdfs:range`; the link's sentence → `rdfs:comment`; cardinality → SHACL `sh:maxCount` on the from-side shape (and `owl:FunctionalProperty` when it is "exactly one") | an inverse, when the file names one, → `owl:inverseOf` |
| Glossary term | `skos:altLabel` on the entity it means (the entity's name is `skos:prefLabel`); a term with its own meaning → `skos:Concept` | |
| Guardrail (CEL rule with `applies_to`, `severity`, `owner`) | SHACL `sh:NodeShape` targeting the entity's class, with `sh:severity` (`sh:Violation`, `sh:Warning`, `sh:Info`), `sh:message` = the plain-words description, and the rule carried as `lorefile:cel` (CEL is not SHACL; only cardinality, datatype, enum and simple comparisons become native SHACL constraints such as `sh:minInclusive`; everything else stays a CEL annotation and is marked so) | SHACL 1.2's non-validating `sh:agentInstruction` is a draft; use it only behind a flag. Timing differs: a Lorefile guardrail is checked before an action, SHACL checks every row at any time, so a guardrail that belongs to an action (its `applies_to` is an action, or an action lists it) is exported with `sh:deactivated true` and `lorefile:action` naming the action, and a SHACL tool will not flag every delivered order for a cancel rule |
| Actions, functions, metrics, sources | a small `lorefile:` vocabulary (`lorefile:Action`, `lorefile:Function`, `lorefile:Metric`, `lorefile:Source`) in the same graph, verbatim structure as JSON-LD-style literals or nested nodes | valid RDF; other tools ignore it; Lorefile reads it back losslessly |
| Everything | an `iri:` key, optional on entity, field, link, term, guardrail and action; derived from the namespace plus the name when absent | lets a space reuse schema.org, FIBO, GoodRelations or a company ontology's IRIs directly |

### What to build

1. **IRIs in the format.** `ontology.yaml` gets optional `namespace:` and `prefixes:` (a map, e.g. `schema: https://schema.org/`); entities, fields, links, terms, guardrails and actions get optional `iri:` (an absolute IRI or `prefix:local`). The compiler resolves every IRI, refuses duplicates (new error code) and exposes them in `describe` and `summary`; the Ossie export from Part D switches from the derived default to the resolved IRI so the same identifier ties the Lorefile entity, the Ossie concept and the OWL class together. Nothing else in the format changes; files without these keys behave exactly as before.
2. **`lorefile export --rdf [--format turtle|jsonld|ntriples] [--dir <space>] [--out <file>]`** writes the OWL, SKOS and SHACL graph described above, with the `lorefile:` vocabulary for what has no standard home. `GET /api/export?format=turtle|jsonld` serves it; the Settings export row offers both Ossie and RDF.
3. **`lorefile import --from-rdf <file> [--into <space-folder>] [--prefix <iri>]`** reads Turtle, N-Triples, RDF/XML (most published OWL files are RDF/XML) or JSON-LD and creates or extends a space: `owl:Class` and `rdfs:Class` → entities (label → name, comment or `skos:definition` → description, `rdfs:subClassOf` → `extends` if the schema has it, else a tag and a note); datatype properties → fields; object properties → links with the comment as the sentence; `skos:altLabel` → glossary terms; SHACL shapes → guardrails where expressible (cardinality, datatype, `sh:in`, `sh:minInclusive` and friends become CEL), the rest listed in the report; `lorefile:` nodes restored verbatim. Large upstream ontologies (schema.org, FIBO) are big: `--only <prefix-or-class-list>` and `--depth` limit the import to the classes named and their neighbours. The result compiles with the strict compiler or nothing is written; a `--report` line lists what was dropped, guessed or kept as annotation.
4. **Crates.** Prefer the Oxigraph family (`oxrdf`, `oxttl` for Turtle and N-Triples, `oxrdfxml` for RDF/XML, `oxjsonld` if it builds on rustc 1.87; else `sophia`) over writing a parser; no reasoner. Adding them needs the sandbox off for the fetch; keep them behind a cargo feature `rdf`, on by default, so the slim build can drop them.
5. **Tests.** Round-trip each shipped example through Turtle and back and compare compiled summaries; import a vendored slice of schema.org (`Organization`, `Person`, `Order`, `Product` and their properties, as Turtle, with its CC BY licence noted), assert the entities and fields; import `flights.ontology.yaml`'s OWL equivalent if Ossie ships one; a SHACL shape with `sh:minCount` and `sh:in` becomes a guardrail that `validate` enforces; a duplicate IRI is an error.
6. **Docs.** README gets the login paragraph, the `claude mcp add --transport http …` command and the Settings tour line from Part B; the install section from Part C (`npx lorefile`, Docker with `LOREFILE_TOKEN`, `cargo install`, the curl script, full versus slim); a "Before the first release" note for Vinay (the org name `lorefile/lorefile` sits in six places, make the ghcr package public after the first push, npm tokens expire so trusted publishing is a backlog item); and a section "Standards: Ossie, OWL, SKOS, SHACL" in plain words (what you can import, what you can export, what is carried as annotation), and the vocabulary table above moves to ARCHITECTURE.md. The final docs and tutorial pass of Part D moves here, since this is now the last session.

Done looks like: `lorefile export --rdf` on the ecommerce example is valid Turtle that Protégé or `rapper` opens; importing schema.org's `Order` gives an entity with its fields and descriptions; a Lorefile export imported back is identical; the README explains it in one screen.

---

## Hand-off plan

| Order | Session | Scope (files) | Starts |
|---|---|---|---|
| 1 | **rename** (Part A) | everything | now |
| 2 | **auth-http** (Part B) | `src/serve.rs`, `src/mcp.rs`, `src/main.rs`, `src/spaces.rs`, `src/ui/index.html`, `tests/`, PRODUCTION.md, DEPLOY.md | after 1 |
| 2 | **distribution** (Part C) | `npm/`, `.github/workflows/release.yml`, `.github/workflows/ci.yml` (one job), `Dockerfile`, `.dockerignore`, `scripts/install.sh`, `Cargo.toml` metadata | after 1, in parallel with auth-http: no files shared |
| 3 | **ossie** (Part D) | `src/ossie.rs`, `src/main.rs`, `src/serve.rs`, `src/ui/index.html`, `tests/`, BACKLOG.md | after both 2s |
| 4 | **rdf-docs** (Part E, plus the final README and tutorial pass from Part D item 4) | `src/rdf.rs`, `src/schema.rs`, `src/loader.rs`, `src/validate.rs`, `src/main.rs`, `src/serve.rs`, `src/ui/index.html`, `Cargo.toml`, `tests/`, README.md, ARCHITECTURE.md, BACKLOG.md, tutorial | after 3 |

All sessions work in this checkout on `master`; none commits, none switches branches; README.md is edited only by sessions 1 and 4. Vinay creates the GitHub org and repository, adds the `NPM_TOKEN` and `CARGO_REGISTRY_TOKEN` secrets, and pushes the first tag.
