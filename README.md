# Stella Ontology

A lightweight, file-based, **AI-agent-first** ontology graph.

> New here? Read [EXPLAINER.md](EXPLAINER.md) first: what this is, how the pieces fit, and a before/after story in plain English.

You describe your business in a folder of YAML files: types, relationships, rules, actions, and where the data lives. One small Rust binary compiles that folder into an in-memory graph and opens three doors into it:

- **MCP server** for Claude, ChatGPT, Cursor, and your own agents
- **CLI** for people and CI
- **Web UI** that boots locally with one command

The ontology is small and precious; the data is big and lives in your warehouse. This tool holds the first and points at the second. See [ARCHITECTURE.md](ARCHITECTURE.md) for the why.

> Status: Phase 2 working example. YAML format, compiler, graph, CLI, MCP (stdio), web UI, and the **DuckDB data door**: CSV, Parquet, `.duckdb` files, Postgres and S3/HTTP Parquet are read inside the binary; `query(sql)` joins rules to rows. Snowflake/BigQuery connectors and `init --from <db>` are Phase 3.

## Try it in two minutes

```bash
cargo build --release
```

```bash
./target/release/stella-ontology validate --dir examples/supply-chain
```

```bash
./target/release/stella-ontology check --dir examples/supply-chain switch_supplier part=BRK-220 from=V-10001 to=V-10422
```

That last command is what an agent does before telling a user "yes, you can switch suppliers". The answer, from real rows in `data/*.csv`:

```
✗ switch_supplier is blocked by SC-041 (block), SC-063 (block), SC-050 (warn), SC-081 (warn).
 FAIL  SC-041   Safety-critical parts need tier-1 suppliers — violated for to=V-10422, part=BRK-220
         why: ISO 26262 and customer contract clause 14.2 require audited, strategic suppliers …
 FAIL  SC-063   Safety-critical parts need IATF 16949 — violated for to=V-10422, part=BRK-220
 …
suggestions:
  to → Gamma Inc (V-10105)
  to → Delta Co (V-10210)
approval: Procurement Lead (policy PR-007)
```

The same check against Parquet instead of CSV, read by DuckDB inside the binary, gives the same verdict:

```bash
./target/release/stella-ontology check --dir examples/supply-chain-parquet switch_supplier part=BRK-220 from=V-10001 to=V-10422
```

And SQL can join the ontology to the rows it governs:

```bash
./target/release/stella-ontology query --dir examples/supply-chain-parquet "select s.name, s.tier from Supplier s join nodes n on n.name='Supplier' limit 3"
```

Open the UI (the **SQL** button in the header runs the same queries):

```bash
./target/release/stella-ontology serve --dir examples/supply-chain
```

Then visit http://127.0.0.1:7070. Add `--allow-edit` to create and change types, rules and actions from the page (see [Editing from the page](#editing-from-the-page)).

## Connect it to Claude

**Claude Code** (from this folder):

```bash
claude mcp add stella-ontology -- ./target/release/stella-ontology mcp --dir ./examples/supply-chain
```

**Claude Desktop** — add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "stella-ontology": {
      "command": "/absolute/path/to/stella-ontology",
      "args": ["mcp", "--dir", "/absolute/path/to/examples/supply-chain"]
    }
  }
}
```

Then ask: *"Can we move part BRK-220 from Acme Corp to Beta Ltd?"* Claude will call `describe`, `constraints_for`, and `validate`, and answer with the rule IDs that block it and the suppliers that would pass.

### What the agent sees

| Tool | Returns |
|---|---|
| `summary` | Compact Markdown of the whole ontology (load this first) |
| `search(query)` | Types, rules, actions, glossary terms matching plain text |
| `describe(name)` | Full detail of a type, relationship, action, rule, or source |
| `neighbors(name, depth)` / `path(from, to)` | Graph traversal |
| `constraints_for(name)` | Every rule that applies, sorted by severity |
| `validate(action, inputs)` | Pass / warn / fail with reasons, approver, and alternatives that would pass |
| `where_is(type)` | Connection, table, key, column mapping for the data, and whether it is readable here |
| `instances(type, key?)` | Rows when readable here (inline, csv, parquet, duckdb, postgres, s3) |
| `query(sql)` | One read-only SELECT/WITH over the ontology tables and the data views, rows as JSON, max 500 |
| `tables` | What `query` can see: ontology tables, data views, and bindings that could not be attached |

Resources: `ontology://summary`, `ontology://glossary`, `ontology://types/{Name}`, `ontology://actions/{name}`.

## Examples

Each folder under `examples/` is a complete ontology with CSV rows, a glossary, a warehouse source the binary cannot read (to show `where_is` and inline records), and a README whose `check` outputs were pasted from real runs.

| Folder | Business | What it shows |
|---|---|---|
| [`supply-chain`](examples/supply-chain) | Automotive parts manufacturer buying from tiered suppliers | The reference example: safety-critical sourcing rules, `switch_supplier` with suggestions, Snowflake-only PurchaseOrder rows |
| [`supply-chain-parquet`](examples/supply-chain-parquet) | The same manufacturer, Parquet instead of CSV | The DuckDB door: identical verdicts from Parquet files, `query` joining rules to rows |
| [`ecommerce`](examples/ecommerce) | Online store with hazardous goods and three warehouses | Return windows with real date math, promotion stacking caps, hazmat carrier rules, PCI-scoped PaymentMethod fetched inline |
| [`saas-pricing`](examples/saas-pricing) | Subscription software company with four plans | Seat caps, enterprise-needs-annual, CFO discount threshold, trial accounts that cannot be invoiced, Invoice rows fetched inline |
| [`b2b-sales`](examples/b2b-sales) | Long-lead B2B sales team with channel partners | Stage exit criteria read from data, Deal Desk and VP Sales thresholds, territory ownership, Quote rows fetched inline |

Serve any of them with `stella-ontology serve --dir examples/<folder>`; `.claude/launch.json` maps them to ports 7070, 7071 and 7073 to 7075.

## Start your own

```bash
./target/release/stella-ontology init ./ontology
```

```bash
./target/release/stella-ontology validate --dir ./ontology
```

## The file format

```
ontology/
├── ontology.yaml          # name, version, domains
├── glossary.yaml          # "vendor" means Supplier
├── sources/*.yaml         # connections: csv, parquet, duckdb, postgres, s3 read here; snowflake/… declared; mcp points at another MCP server
├── types/*.yaml           # one type per file, or a `types:` list
├── relationships/*.yaml   # relationships with their own properties / edge tables
├── constraints/**/*.yaml  # rules, grouped however you like
├── actions/*.yaml         # what agents may do, and which rules are checked
└── data/*.csv|*.parquet   # optional local rows for csv / parquet sources
```

Files are classified by their keys, not their folder, so organise them however suits your team.

### A type

```yaml
type: Supplier
description: A company we buy parts from. Tier 1 is strategic and audited yearly.
properties:
  supplier_id: { type: string, key: true, description: "ERP vendor number" }
  tier:        { type: enum, values: [1, 2, 3] }
  status:      { type: enum, values: [approved, probation, suspended] }
  lead_time_days: integer
relationships:
  located_in: { to: Region, cardinality: many-to-one }
source:
  connection: erp_files
  table: suppliers
  key: supplier_id
owner: procurement@acme.example
```

### A rule

Rules are [CEL](https://cel.dev) expressions. `A implies B` is accepted as sugar for `!(A) || (B)`.

```yaml
constraints:
  - id: SC-041
    name: Safety-critical parts need tier-1 suppliers
    applies_to: supplies          # a relationship: rule sees `supplier` and `part`
    rule: part.safety_critical == true implies supplier.tier == 1
    severity: block               # block | warn | info
    rationale: ISO 26262 and customer contract clause 14.2.
```

What a rule can see depends on `applies_to`:

| applies_to | Variables available |
|---|---|
| a type, e.g. `Supplier` | its properties as top-level names (`tier`, `status`), plus `self` |
| a relationship, e.g. `supplies` | `supplier`, `part` (snake_case of the two type names), plus `from`, `to` |
| an action, e.g. `switch_supplier` | each action input by name (`part`, `from`, `to`) |

### An action

```yaml
action: switch_supplier
description: Move a Part from one approved Supplier to another.
inputs: { part: Part, from: Supplier, to: Supplier }
effects:
  - remove: { relationship: supplies, from: from, to: part }
  - create: { relationship: supplies, from: to, to: part }
checks: [SC-041, SC-042, SC-043]
requires_approval_from: Procurement Lead
```

`validate` evaluates every rule in `checks`, every rule whose `applies_to` is the action, and binds relationship rules to the edges the action `create`s.

## Data sources: the DuckDB door

The ontology is small and lives in git; the rows are big and live elsewhere. A `source` says where. Kinds DuckDB reads inside the binary are **readable here**: `validate` checks real rows, `instances` lists them, and `query` joins them. Any other kind (`snowflake`, `bigquery`, …) is declared for agents, which fetch the row with their own connector and pass it inline to `validate`.

| Kind | YAML | What the binary does |
|---|---|---|
| `csv` | `path: data` (folder) | Reads `data/<table>.csv` in plain Rust; also a DuckDB view for SQL |
| `parquet` | `path: data` (folder) | `read_parquet('data/<table>.parquet')`; a table may also be a glob like `parts/*.parquet` |
| `duckdb` | `path: data/erp.duckdb` (file) | `ATTACH … (READ_ONLY)`; table is `name` or `schema.name` |
| `postgres` | `env: ORDERS_PG_URL`, `schema: public` | Loads the `postgres` extension and attaches read-only using the libpq connection string in that env var |
| `s3` / `httpfs` | `path: s3://bucket/prefix` or an `https://…` base | Loads `httpfs`; `read_parquet('<path>/<table>.parquet')` |
| `mcp` | `server: meta-ads` | Nothing is read here. `where_is` tells the agent which tool to call on that MCP server, with which params |

```yaml
# sources/parquet_files.yaml
connection: parquet_files
kind: parquet
path: data              # Supplier.source.table: suppliers → data/suppliers.parquet
```

```yaml
# sources/orders_pg.yaml — credentials never live in the repo
connection: orders_pg
kind: postgres
env: ORDERS_PG_URL      # export ORDERS_PG_URL='postgresql://user:pw@host:5432/erp'
schema: public
```

```yaml
# sources/lake.yaml
connection: lake
kind: s3
path: s3://acme-erp/extracts
region: eu-west-1       # optional; endpoint / url_style for MinIO or R2
# credentials: AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_SESSION_TOKEN in the environment,
# or `credential_chain: true` to use the AWS SDK chain (profiles, SSO, instance roles).
```

```yaml
# sources/meta-ads.yaml — the rows live behind another MCP server the agent already has
connection: meta_ads
kind: mcp
server: meta-ads         # the server name as the agent has it configured
```

A type binds to a source with `source: { connection, table, key, columns }`. `columns` maps property names to column names when they differ; the view renames them so rules and SQL see property names. Relationships bind edge tables the same way with `from_key` / `to_key`.

A type on an `mcp` source binds with `tool` and an optional `params` map instead of a table:

```yaml
source:
  connection: meta_ads
  tool: get_campaigns
  params: { account_id: "act_<Account.account_id>", limit: 100 }
```

`where_is("Campaign")` then answers `readable_here: false` with `server`, `tool`, `params` and a `hint` that spells out the call: "Call tool `get_campaigns` on the `meta-ads` MCP server with params {…}". The agent fetches the row there and passes it inline to `validate`. The validator requires `server` on every `kind: mcp` source (E011) and a `tool` on every type bound to one (E012). This is how the [packs](#packs) work.

**What SQL sees.** `query` runs DuckDB SQL over:

- the ontology as tables: `nodes` (types), `properties`, `edges` (relationships), `constraints` (rules), `actions`, `sources`;
- one view per readable type or relationship, named after it: `Supplier`, `Part`, `supplies`, … Inline `instances:` become small tables too.

```sql
-- which rules govern each type, and how many rows each type has
select n.name, n.source_kind, c.id, c.severity
from nodes n join constraints c on c.applies_to = n.name
where n.readable_here order by 1, 3;

-- rules joined to rows: tier-1 suppliers that break the lead-time warning
select s.name, s.lead_time_days, c.id
from Supplier s join constraints c on c.id = 'SC-042'
where s.tier = 1 and s.lead_time_days > 30;
```

Rules of the door: exactly one `SELECT` or `WITH` statement (anything else is rejected before DuckDB sees it), sources are attached read-only, results are capped at 500 rows, and dates, timestamps, decimals and nested values come back as JSON strings, numbers and arrays. Column discovery: `select * from information_schema.columns where table_name = 'Supplier'`, or the `tables` tool. The `postgres`, `httpfs` and `aws` extensions are downloaded from extensions.duckdb.org the first time they are needed (about 20 MB, cached in `~/.duckdb`); `parquet` and `json` are compiled in. A SELECT can still read any file the process can read (`read_csv('/etc/hosts')`), so treat `serve` as a local tool: it binds to 127.0.0.1 only.

**Build notes.** DuckDB is compiled from source by the `duckdb` cargo feature, on by default. On a 12-core Apple Silicon Mac with rustc 1.87: `cargo build` 2m50s and `cargo build --release` 2m57s from a clean checkout (DuckDB's C++ is the bulk of both; incremental rebuilds of this crate take seconds). The release binary grows from about 6 MB to about 40 MB, and still starts and answers a `check` in under 50 ms. `cargo build --no-default-features` gives a small CSV-only binary in under a minute, with `query` reporting that it was built without DuckDB. The crate pins `rust-version = "1.87"` with `resolver = "3"` so cargo picks dependency versions that compile on that toolchain (`duckdb` 1.10506 = DuckDB v1.5.6; `uuid` is pinned in `Cargo.lock` for the same reason). No system DuckDB is needed; if the bundled C++ build ever fails on a platform, `libduckdb-sys` also supports linking a system library via `pkg-config`.

DuckDB is **fenced**: once a space's views are built, the catalog may touch only the folders those views read from, external access is switched off and the configuration is locked, so a `query` cannot read other files on the machine, install extensions or reach the network (`tables` reports `fenced`). A space with a Postgres or S3 source keeps the fence open, since those need external access at query time.

## The web page

`stella-ontology serve --dir ./ontology` opens a guided workspace at http://127.0.0.1:7070. It is organised around putting the ontology in place, not around the data model:

- **Build**, five steps with live status: **Sources** (where the rows live), **Things** (each type with a plain-English description, its fields, links, guardrails, data and YAML), **Links** (a relationship builder and one sentence per link), **Guardrails** (rules grouped by what they apply to, a builder that writes the expression for you, and the expression itself one click away), **Actions** (what agents may do, with a try-it form that runs the same `validate` an agent runs).
- **Use**: **Map** (the whole shape, with what to do next computed from the gaps), **Agents** (the MCP tools, connect commands, an inbox of changes agents proposed, and an activity log), **Data & SQL** (DuckDB over the ontology and every readable source).
- A readiness meter in the header rises as fields get words, links get sentences and things get tables.

### Editing from the page

Start the server with `--allow-edit` and every description box, link sentence, builder and "Edit YAML" button writes a file. Each change is a small operation (`POST /api/patch`: describe a field, describe a link, add a link, add a guardrail, create a thing, bind a table) that becomes the full new text of one YAML file and goes through the same compiler check as any other write: errors refuse the save and are shown, warnings are allowed. Guardrails are appended as text, so their file keeps its comments; type files are rewritten through the YAML library, which keeps key order but drops comments in that one file. Add `--commit-edits` to commit each accepted change when the folder is a git repository; otherwise changes stay uncommitted for `git diff`.

Without `--allow-edit` the page is read-only and says so. Keep it that way on anything reachable from the internet until authentication exists.

### Proposals from agents

An agent calls `propose_change` with the full new text of one file and a reason. When the server allows edits and the change is clean, it is written and logged. Otherwise it is queued under `<ontology>/.stella/proposals/` and shows up on the Agents page with a diff, where a person accepts or sets it aside; accepting writes it through the same checked path. Activity (who changed or proposed what, when) is kept in `<ontology>/.stella/activity.jsonl`. The `.stella/` folder is local state and is ignored by git.

### Spaces: one server, many ontologies

Point `serve` or `mcp` at a folder that has no `ontology.yaml` of its own and every sub-folder with one becomes a **space**: one ontology per brand, business unit or client. The page gets a picker at the top; each space has its own URL (`/s/brand-two/`), API (`/s/brand-two/api/…`), DuckDB catalog, proposals inbox and activity log. With `--allow-edit`, "＋ New space…" creates one as blank, from the starter, or as a copy of any other space.

```bash
stella-ontology serve --dir ./spaces --allow-edit     # spaces/brand-one, spaces/brand-two, …
stella-ontology validate --dir ./spaces               # checks every space (for CI)
stella-ontology describe Campaign --dir ./spaces --space brand-two
```

The examples and packs ship inside the binary and are listed as read-only spaces whose ids start with `@` (`@ecommerce`, `@supply-chain-parquet`, `@meta-ads`), so you can open one to see how it is done and copy it. They are written out under `<dir>/.stella/examples/` at start (that folder hides itself from git); `--no-examples` leaves them out.

Over MCP, agents name the space in each call. With exactly one space of yours it is implied; with several, every tool takes `space:` (the tool list names them) and a `spaces` tool describes each one. `mcp --space brand-two` dedicates a server to one space. Resource URIs carry the space too: `ontology://brand-two/summary`.

## CLI

```
stella-ontology validate              compile and report errors / warnings (exit 1 on errors)
stella-ontology summary               the Markdown agents load into context
stella-ontology describe <name>       type / relationship / action / rule / source
stella-ontology search <words>
stella-ontology neighbors <type> --depth 2
stella-ontology path <from> <to>
stella-ontology rules <name>
stella-ontology where <type>
stella-ontology instances <type|relationship>
stella-ontology check <action> k=v …  exit 2 when blocked
stella-ontology query "<sql>"         one read-only SELECT/WITH via DuckDB (--limit, max 500; `-` reads stdin)
stella-ontology tables                what SQL can see: ontology tables, data views, bindings not attached
stella-ontology mcp [--allow-edit]    MCP server on stdio; --allow-edit lets propose_change write
stella-ontology serve --port 7070 [--allow-edit [--commit-edits]]   web UI + JSON API
stella-ontology init [path]           starter ontology
```

`--dir` picks the folder (defaults to `./ontology`, then `.`): an ontology folder, or a folder of spaces. `--space <id>` picks a space when there are several (`@…` for a shipped example); `--no-examples` leaves the shipped ones out. `--json` switches any command to machine output.

## Validation codes

| Code | Meaning |
|---|---|
| E001 | A YAML file did not parse |
| E002 | Duplicate type, rule id, action, or source |
| E003 | Relationship points to an unknown type |
| E004 | Rule `applies_to` is not a type, relationship, or action |
| E005–E007 | Action input type, check id, or effect relationship unknown |
| E008 | Unknown connection |
| E009 | Rule does not parse |
| E011 | `kind: mcp` source has no `server` |
| E012 | Type binding has no `table` (or no `tool`, for an `mcp` source) |
| W101 | Missing description |
| W102 | Blocking rule without a rationale |
| W103 | Type bound to data but no `key: true` property |
| W104 | Rule references a name not available on its `applies_to` |
| W106 | CSV / Parquet / DuckDB file not found |

## Packs

An ontology pack is a ready-made folder for one data source an agent already has an MCP server for. The data MCP returns rows; the pack says what they mean, which rules apply, and what the rows cannot tell you. Load both in the same agent. Each pack has a `README.md` with the Claude Desktop config and three worked questions, a `sources/<server>.yaml` of `kind: mcp`, and validates with zero errors and zero warnings.

| Pack | Types | Rules | Actions | What it fixes |
|---|---|---|---|---|
| [`packs/meta-ads`](packs/meta-ads) | Account, Campaign, AdSet, Ad, Creative, Audience, Placement, Insight, Conversion, AttributionWindow, LiftStudy, BudgetChange | 16 | pause_campaign, change_budget, create_lookalike, read_performance, compare_conversions | Conversions are attributed counts under one window (7-day click, 1-day view by default), booked on the click day, modelled and late on iOS; awareness campaigns are not judged on CPA; CBO ad-set budgets are advisory; geo breakdowns stop at region/DMA and are not incrementality; test cells are frozen |
| [`packs/google-ads`](packs/google-ads) | Account, Campaign, AdGroup, Keyword, Ad, ConversionAction, BiddingStrategy, Audience, GeoTarget, Readout, BudgetChange, TargetChange | 16 | pause_campaign, change_budget, change_bid_target, read_performance, compare_readouts, audit_conversion_action | `conversions` counts primary actions under each action's model (data-driven by default), by click date; counting rules per category; CPM/CPV strategies are not judged on CPA; geo segments are matched location, not presence; shared budgets and experiment arms |
| [`packs/shopify`](packs/shopify) | Shop, Order, LineItem, Product, Variant, Customer, Fulfillment, Refund, DiscountCode | 15 | cancel_order, refund_order, create_discount_code, attribute_order | Test and cancelled orders are not revenue; `processed_at` and the shop currency are the reporting basis; financial versus fulfillment status; `source_name` is the sales channel, not attribution; codes are proof of use |

```bash
./target/release/stella-ontology validate --dir packs/meta-ads
```

```bash
./target/release/stella-ontology mcp --dir packs/meta-ads
```

The three read-out guards (`read_performance`, `compare_conversions`, `attribute_order` and their siblings) are actions with no side effects: an agent calls `validate` on them before quoting a number, and gets the same block / warn / info verdict it would get before a write. Where a platform detail was uncertain the YAML says `# TODO verify` rather than guessing. Tool names in the bindings follow the common open-source MCP servers for each platform; change `tool:` in a type file if yours differ.

## Roadmap

1. **Core** (done) — format, compiler, graph, CLI, MCP, UI, CSV data door.
2. **Doors** (this) — DuckDB inside the binary: Parquet, `.duckdb`, Postgres, S3/HTTP; `query(sql)` and the ontology as SQL tables; SQL box in the UI. Editing from the page and `propose_change` for agents, both compiler-checked. Still open: MySQL, `init --from <db>` to draft types from an existing schema, opening a PR from an edit.
3. **Cloud** — Snowflake, BigQuery, Databricks connectors; `propose_change` opening PRs instead of local commits; semantic search; Stella Cloud hosted version with SSO and audit log.
4. **Ecosystem** — more packs (TikTok Ads, GA4, Klaviyo), Python/TS SDKs, OWL/LinkML export, dbt/LookML import.

## License

Apache-2.0, see [LICENSE](LICENSE). Use it, change it, host it, sell what you build on it; keep the notice. "Stella" is a trademark of Stella: a hosted copy can be anything but Stella Cloud. Why Apache-2.0 and not MIT, AGPL or a source-available licence is in [PRODUCTION.md](PRODUCTION.md#6-licence).
