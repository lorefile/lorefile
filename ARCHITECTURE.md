# Lorefile — Architecture Options

> Status: **approved 2026-11-25 — Option A core + DuckDB as the data door, in Rust.** Phases 1 and 2 are built; see [README.md](README.md) to run it. Phase 3 (any MCP as a source, the capture loop, logic, hosted) is designed in [§17](#17-phase-3-design-any-mcp-is-a-source-the-ontology-keeps-what-matters); its first steps are built. Sections below are the design as approved. Since 2026-11-26 the project is released as **Lorefile**, open source only; the hosted parts of this design are parked in [BACKLOG.md](BACKLOG.md) under "Hosted version, later".

---

## 0. The one idea that shapes everything

**Ontology is not data.**

| | Ontology | Data |
|---|---|---|
| What it is | The rules of the business: what a Supplier *is*, how it relates to a Part, what is allowed | The 5,000 actual suppliers, 2 million purchase orders |
| Size | Small. A complex company with thousands of rules is still a few megabytes of text | Big. Millions of rows |
| Changes | Slowly, deliberately, reviewed | Constantly |
| Lives | In this tool | In your warehouse (Snowflake, Postgres, BigQuery, Parquet) |

"Thousands of suppliers" is **one** `Supplier` type plus a pointer to the table where the suppliers live. Thousands of *constraints* is real ontology, and still fits in memory on a laptop instantly.

Every option below keeps this split. They differ in **what the source of truth is** and **how far the tool reaches into the data**.

---

## 1. What we are building, in plain English

A folder of readable files that describes your business, plus one small program that reads that folder and opens three doors into it:

1. **UI** for people. Browse types, relationships, rules, where data lives.
2. **MCP server** for AI agents (Claude, ChatGPT, Cursor, your own agents).
3. **API and CLI** for code and CI.

### Before / after: supply chain

**Before.** An ops manager asks Claude: *"Can we move part BRK-220 from Acme Corp to Beta Ltd?"*
Claude has no idea what a supplier is at this company. It doesn't know Beta Ltd is tier 3, that BRK-220 is a safety-critical brake part, or that switching needs Procurement sign-off. It either guesses or asks ten questions.

**After.** Claude's MCP connection calls the ontology:

```
describe("Part")             → BRK-220 has safety_critical = true
constraints_for("supplies")  → SC-041: safety-critical parts require a tier-1 supplier (block)
where_is("Supplier")         → snowflake_prod · ERP.VENDORS · key VENDOR_ID
validate(switch_supplier, {part: BRK-220, to: Beta Ltd})
                             → FAIL · SC-041 · Beta Ltd is tier 3
                             → alternatives with tier 1: Gamma Inc, Delta Co
                             → approval needed: Procurement Lead (policy PR-007)
```

Claude answers in one turn, correctly, and cites the rule. Nobody wrote a prompt for this. The ontology *is* the prompt.

### Before / after: a measurement company's own apps

**Before.** A marketing-measurement company's MMM, experiments, and attribution tools each carry their own idea of "channel", "campaign", "KPI", "pre-period". Agents using its MCP learn these by trial and error.

**After.** One `marketing-measurement` ontology defines `Channel`, `Campaign`, `Experiment`, `KPI`, `Model`, with rules like *"a geo experiment needs at least 4 weeks of pre-period data"* and *"a Channel's spend has exactly one source of truth"*. Every tool it ships and every agent reads the same definitions.

---

## 2. Three architectures

### Option A — "Ontology as code"
**Source of truth: YAML files in a git repo. The program compiles them into an in-memory graph.**

```
ontology/            ← humans and agents edit this
   types/*.yaml
   constraints/**/*.yaml
   sources/*.yaml
        │
        ▼  lorefile serve ./ontology
   in-memory graph + small snapshot file (.lorefile/ontology.db)
        │
   UI · MCP · API
```

- Instances (the 5,000 suppliers) are **not stored**. Each type has a `source:` block saying which warehouse table holds them.
- Small reference data (regions, tiers, a few hundred named entities) can be inlined or dropped in as CSV/Parquet in a `data/` folder.
- Change review is a git pull request. An agent proposing a new rule opens a PR like any engineer.

**Good:** zero infrastructure, fully readable by humans and LLMs, git gives versioning, diff, rollback, blame, and branches for free. Runs on a laptop, a container, or any cloud that can mount a folder or clone a repo.
**Bad:** no live data queries out of the box (the agent uses its own warehouse connector, guided by `where_is`). Thousands of files need folder conventions and a good compiler to stay sane.

### Option B — "Embedded graph database"
**Source of truth: a binary graph store (KùzuDB, Oxigraph, or SQLite-with-graph-tables). YAML is import/export only.**

- Real query language (Cypher or SPARQL), real transactions, millions of nodes.
- Can store instances too, so one tool holds ontology *and* suppliers.

**Good:** proper database behaviour, multi-writer, big scale.
**Bad:** the ontology becomes opaque. An agent can't `cat` a rule, a reviewer can't diff a PR, and users must learn Cypher or SPARQL. You've rebuilt a smaller Neo4j and lost the "it's just files" property that makes agents and newcomers productive.

### Option C — "Lakehouse ontology"
**Source of truth: Parquet/Iceberg tables in object storage (`nodes`, `edges`, `constraints`) plus a manifest. DuckDB queries everything.**

- Scales to billions of rows, cloud-native, cheap, loved by data engineers.
- Instances and ontology live side by side in the same lake.

**Good:** huge scale, one SQL engine for everything, natural fit if the company is already Parquet/Iceberg-first.
**Bad:** Parquet isn't human- or agent-readable. Editing one constraint means rewriting a columnar file. The ontology, which is small and precious, gets lost inside the data, which is big and noisy. Overkill for the thing we actually care about.

### Side by side

| | A · Ontology as code | B · Embedded graph DB | C · Lakehouse |
|---|---|---|---|
| Source of truth | YAML in git | Binary DB file | Parquet / Iceberg |
| Agent can read & edit directly | Yes | No, via API only | No, via SQL only |
| Change review | Git PR diff | Custom audit log | Custom |
| Setup | 1 binary + 1 folder | 1 binary + 1 file | Object store + catalog |
| Ontology size it handles | ~100k nodes in RAM (plenty) | Millions | Billions |
| Stores instances | Small reference data only | Yes | Yes, massive |
| Query | Tools + traversal + SQL | Cypher / SPARQL | SQL |
| Cloud hosting | Trivial | Easy | Native |
| Main risk | Folder sprawl without tooling | Opaque to humans and agents | Ontology drowned in data |

---

## 3. Recommendation: A as the core, DuckDB as the data door

Take Option A for the ontology. Borrow only the good part of Option C, DuckDB, as the engine that reaches into data when someone wants a live answer. Don't take Option B at all.

```
  ontology/  (YAML · git)                 ← humans + agents edit; PRs review
        │
        ▼
 ┌───────────────────────────────────────────────────────────────┐
 │  lorefile          (one Rust binary, ~50 MB, starts in <100ms) │
 │                                                               │
 │  1. parse      every YAML file                                │
 │  2. validate   types exist · rules reference real properties  │
 │                · no duplicate IDs · cardinalities make sense  │
 │  3. graph      in RAM (10k nodes → microsecond lookups)       │
 │  4. snapshot   .lorefile/ontology.db  (fast cold start, UI)    │
 │  5. DuckDB     ontology tables + optional live connectors:    │
 │                Parquet · CSV · Postgres · MySQL · S3 ·        │
 │                Snowflake · BigQuery · Iceberg · Delta         │
 └───────┬──────────────────┬──────────────────┬─────────────────┘
         │                  │                  │
        UI               MCP server         HTTP + CLI
      (people)        (Claude, ChatGPT,       (code, CI,
                       Cursor, agents)         dbt hooks)
```

Why this wins for the next 5 to 10 years:

- **Files are the API.** Every LLM already reads YAML perfectly. No SDK required to understand an ontology; `cat` works.
- **Git is the audit log.** Who changed rule SC-041, when, why, and what it looked like last quarter. SOC2 reviewers like this.
- **The agent loop closes.** Agents don't just read the ontology, they propose changes to it as PRs. The ontology gets better the more agents use it.
- **No lock-in, ever.** A team that stops using it, or leaves any hosted copy, still has a folder of YAML and a free binary.
- **DuckDB is the right bet for "connect to everything via config".** One embedded engine already speaks Parquet, Postgres, MySQL, S3, Iceberg, Delta, and more, with Snowflake and BigQuery reachable through Arrow. We don't write connectors; we write a 10-line YAML `source` and DuckDB does the rest.

---

## 4. File format wireframe

```
ontology/
├── ontology.yaml                 # name, version, which folders to load, defaults
├── glossary.yaml                 # plain-English terms → canonical types ("vendor" = Supplier)
├── sources/
│   ├── snowflake_prod.yaml       # connection config, secrets via env vars only
│   └── postgres_erp.yaml
├── types/
│   ├── supplier.yaml
│   ├── part.yaml
│   └── facility.yaml
├── relationships/
│   └── supplies.yaml             # Supplier ─supplies→ Part, with its own properties
├── constraints/
│   └── supply_chain/
│       ├── tier_rules.yaml       # 10–50 rules per file, grouped by topic
│       └── lead_time.yaml
├── actions/
│   └── switch_supplier.yaml      # what an agent is allowed to *do*, and what gets checked
└── data/                         # optional small reference data; DuckDB reads it directly
    ├── regions.csv
    └── tiers.parquet
```

### A type

```yaml
# types/supplier.yaml
type: Supplier
description: >
  A company we buy parts or services from. Every supplier has a tier that
  reflects how critical they are and how deeply we have audited them.
properties:
  supplier_id:    { type: string,  key: true, description: "ERP vendor number, e.g. V-10422" }
  name:           { type: string }
  tier:           { type: enum, values: [1, 2, 3], description: "1 = strategic and audited, 3 = spot buy" }
  country:        { type: string }
  lead_time_days: { type: integer, description: "Average days from PO to dock" }
  approved_until: { type: date }
relationships:
  supplies:    { to: Part,     cardinality: many-to-many }
  located_in:  { to: Facility, cardinality: many-to-one }
  audited_by:  { to: Auditor,  cardinality: many-to-many }
source:
  connection: snowflake_prod
  table: ERP.VENDORS
  key: VENDOR_ID
  columns: { supplier_id: VENDOR_ID, name: VENDOR_NAME, tier: TIER_CODE, lead_time_days: AVG_LEAD_DAYS }
owner: procurement@company.com
tags: [supply-chain, core]
```

Every object has a `description`. That single habit is what makes the ontology "AI-readable": the agent never has to guess what a field means.

### Constraints

```yaml
# constraints/supply_chain/tier_rules.yaml
constraints:
  - id: SC-041
    name: Safety-critical parts need tier-1 suppliers
    applies_to: supplies                       # a relationship, a type, or an action
    rule: part.safety_critical == true implies supplier.tier == 1
    severity: block                            # block | warn | info
    rationale: Regulatory requirement under ISO 26262 and customer contract clause 14.2
    owner: quality@company.com

  - id: SC-042
    name: Tier-1 lead time ceiling
    applies_to: Supplier
    rule: tier == 1 implies lead_time_days <= 30
    severity: warn
```

Rules use **CEL** (Common Expression Language: Google's small, safe, non-Turing-complete policy language with a Rust implementation). Readable by humans, evaluable in microseconds, and an LLM writes it correctly on the first try. Rules that need live data (`"no supplier may exceed 40% of total spend"`) can carry an optional `sql:` predicate that DuckDB runs against the source.

### Actions

```yaml
# actions/switch_supplier.yaml
action: switch_supplier
description: Move a Part from one Supplier to another
inputs: { part: Part, from: Supplier, to: Supplier }
checks: [SC-041, SC-042, PR-007]
requires_approval_from: Procurement Lead
side_effects: Updates ERP.SUPPLIER_PARTS; notifies Planning
```

Actions are what make this "agent-driven first". An agent asks `validate(switch_supplier, …)` before it does anything, and gets a yes/no with reasons.

### A source

```yaml
# sources/snowflake_prod.yaml
connection: snowflake_prod
kind: snowflake
account: ${SNOWFLAKE_ACCOUNT}
database: PROD
auth: env            # credentials never live in the repo
refresh: 15m         # how often row counts / freshness are checked for the UI
```

---

## 5. UI wireframe (plain English)

One page, three columns. Boots with `lorefile serve` and opens in the browser.

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ● acme-ontology    main ✓   142 types · 1,380 rules · 3 sources   [Publish]│
├───────────────┬────────────────────────────────────┬─────────────────────┤
│ 🔍 search…    │  Supplier                          │ Rules on Supplier   │
│               │  "A company we buy parts from…"    │ ───────────────     │
│ ▾ Supply chain│                                    │ ⛔ SC-041  block    │
│   Supplier ●  │  Properties (6)   Relationships (3)│   Safety-critical…  │
│   Part        │  supplier_id key  ─supplies→ Part  │ ⚠  SC-042  warn     │
│   Facility    │  tier       enum  ─located_in→ Fac.│   Lead time ≤ 30 d  │
│   Auditor     │  lead_time  int   ─audited_by→ Aud.│ … 29 more           │
│ ▸ Finance     │                                    │                     │
│ ▸ People      │  [Graph] [Table] [YAML] [History]  │ Data                │
│               │                                    │ snowflake_prod      │
│ Rules         │     (Facility)──(Supplier)──(Part) │ ERP.VENDORS         │
│ Actions       │                   │                │ 5,212 rows          │
│ Sources       │               (Auditor)            │ fresh 2 min ago ✓   │
│ Glossary      │                                    │                     │
├───────────────┴────────────────────────────────────┴─────────────────────┤
│ ✓ valid · 0 errors · 2 warnings     💬 Ask: "what blocks switching a part?"│
└──────────────────────────────────────────────────────────────────────────┘
```

- **Left:** domains and types as a tree, plus rules, actions, sources, glossary.
- **Centre:** the selected thing. Tabs for graph view, table view, raw YAML, and git history.
- **Right:** rules that apply, and where the data lives with live freshness.
- **Bottom:** compiler status and an "Ask" box that is literally the same MCP server the agents use.

Editing from the UI is opt-in (`serve --allow-edit`): an Edit button on every page and a "+ New" menu write the same YAML files, after the compiler has accepted the change (errors block, warnings pass). Each save is one git commit when the space keeps its own history (the default for a space the app created; `--no-commit-edits` turns it off); a space nested inside another project's repository leaves its changes uncommitted for review, or commits them with `--commit-edits`. Agents get the same path as the `propose_change` MCP tool. Next: edit in UI → opens a PR.

---

## 6. What the agent sees (MCP wireframe)

**Tools**

| Tool | Returns |
|---|---|
| `search(query)` | Types, relationships, rules, glossary terms matching plain text |
| `describe(name)` | A type or action in full: properties, relationships, rules, source |
| `neighbors(name, depth)` | The subgraph around a type |
| `constraints_for(name)` | Rules that apply to a type, relationship, or action |
| `validate(action, inputs)` | Pass / fail, which rules fired, why, what would fix it |
| `where_is(type)` | Connection, table, key, column mapping, so the agent can query with its own connector |
| `query(sql)` | *(built in phase 2)* One read-only SELECT over the ontology tables and the data views DuckDB attaches |
| `propose_change(patch, reason)` | *(phase 3)* Opens a PR with a YAML change |

**Resources** (for system prompts)

- `ontology://summary` — a compact Markdown of the whole ontology, under ~4k tokens, meant to be pasted into any agent's context.
- `ontology://types/Supplier` — one type as Markdown.
- `ontology://glossary` — business words → canonical names.

The agent never loads 1,380 rules. It asks for the 31 that apply to what it's doing.

---

## 7. How querying works

Three levels, simplest first. Most agent traffic is level 1.

1. **Lookups.** `describe`, `search`, `constraints_for`, `where_is`. Hash-map reads from the in-memory graph. No query language.
2. **Traversal.** `neighbors`, `path(A, B)`, `subgraph(root, depth)`. Built in, in Rust, over the in-memory graph.
3. **SQL.** DuckDB, for power users and for live data. The ontology itself is exposed as tables: `nodes`, `edges`, `constraints`, `sources` (plus `properties` and `actions`). Every readable type or relationship is a view named after it, reading from its source (Parquet, CSV, a `.duckdb` file, Postgres through the postgres extension, S3/HTTP Parquet through httpfs). One SQL joins "rules about Supplier" to "actual suppliers". Phase 2 built this; the README's "Data sources" section is the user-facing description.

Yes, the compiled snapshot is "just a few metadata tables". That is genuinely all the ontology needs. The graph is an index over them.

No Cypher, no SPARQL, no GraphQL in v1. Add GraphQL later if UI builders want it; it can be generated from the types automatically.

---

## 8. Why Rust

| | Rust | Go | Python |
|---|---|---|---|
| One binary, no runtime to install | Yes | Yes | No |
| Startup | ~50 ms | ~50 ms | ~1 s plus imports |
| Compiles to WASM (validate YAML in the browser, no server) | Yes | Partial | No |
| DuckDB, CEL, MCP, petgraph libraries | All exist | Mostly | All exist |
| Speed to write v1 | Slowest | Medium | Fastest |
| Fits "anyone can `brew install` and run" | Best | Best | Worst |

Rust. The file format is the real product and is language-neutral, so Python and TypeScript SDKs that read the same YAML come later and cheaply. The cost of Rust is development time, which is the thing we have the most leverage on.

---

## 9. Staying sane with thousands of rules

- **Folders by domain, files by topic**, 10 to 50 rules each. 1,000 rules ≈ 30 files.
- **Stable IDs** (`SC-041`) that never change; names can.
- **The compiler is the maintainer.** Every referenced type and property must exist. No duplicate IDs. Flags contradictions it can detect (two `block` rules that can never both pass).
- **Nobody reads the whole list.** UI shows rules per type. Agents call `constraints_for`.
- **Semantic search** over descriptions (local embeddings) once a repo passes a few hundred rules, so `search("lead time")` finds `SC-042` even if the word isn't in the title.
- **Performance:** compiling 10,000 YAML objects takes under a second in Rust. Lookups are microseconds. This is never the bottleneck.

---

## 10. Hosting: laptop → any cloud

**Local.** `npx lorefile`, the Docker image, `cargo install lorefile` or one `curl` (all specified, being built; today `cargo build`), then `lorefile serve ./ontology`. UI at `localhost:7070`, MCP on stdio, and over HTTP at `localhost:7070/mcp` once the login token lands.

**Any cloud.** The same binary in a container. The folder comes from a git clone at boot, or an S3/GCS bucket mount. Stateless, so it scales horizontally and restarts freely. Fly.io, Cloud Run, ECS, a $5 VPS, all fine.

**Hosted, later.** A hosted version may come later. It would run the same open-source binary and add what is hard to do alone: one safe copy that agents read, SSO and roles, an audit log, and GitHub sync. It is not being built now; the earlier design and pricing ideas are in [BACKLOG.md](BACKLOG.md) under "Hosted version, later".

---

## 11. Open source strategy

**Apache-2.0, all of it.** The file format, compiler, MCP server, API, CLI, page, DuckDB door, examples and packs are open and free. A hosted version, if one comes, would add hosting, SSO, roles and audit around the same binary, not features held back from it.

How people benefit:

- **No lock-in.** Their ontology is a folder they own.
- **Community connectors and starter packs.** `supply-chain`, `saas-metrics`, `marketing-measurement`, `healthcare-claims`. Each a repo anyone can fork.
- **Agent framework integrations** contributed by the community: Claude Agent SDK, OpenAI Agents, LangChain, CrewAI.
- **Interop.** Apache Ossie and OWL / SKOS / SHACL (Turtle, JSON-LD, N-Triples, RDF/XML) both ways, built (§18). Still to come: LinkML, and import from dbt, LookML, OpenAPI, Protobuf and raw database schemas.
- **Trust.** Companies adopt ontology tooling slowly. Being able to read every line of the thing that governs their agents' behaviour matters.

Why open: a format becomes a standard only when anyone can read it, run it and fork it.

---

## 12. The 5 to 10 year view

- **The ontology becomes the contract between humans, data, and agents.** Every agent action is validated against it. "Did the agent follow policy?" becomes a query.
- **Bootstrapping from what exists.** `lorefile init --from postgres://…` reads a schema and drafts the types. An LLM fills in descriptions. Day-one onboarding goes from weeks to an afternoon.
- **Agents improve the ontology.** "40 suppliers have a `sustainability_score` column that isn't in the ontology. Add it?" arrives as a PR.
- **Time travel for free.** Git gives "what did our rules look like in Q2?" without building anything.
- **Standards gravity.** Palantir Foundry proved the ontology-as-operating-layer idea is worth billions, but closed. In autumn 2026 NVIDIA (Auto Ontology), Databricks (Genie Ontology), Snowflake (Business Ontology) and Microsoft (Fabric IQ Ontology) all shipped agent ontologies; each either runs the queries itself or is locked to one platform. What stays distinctive here:
  - **a keeper only**: it never runs a query, a function or a write, and holds no credential; agents do the work with their own logins;
  - **across warehouses and beyond them**: one ontology spans several warehouses and the non-warehouse MCPs a team already uses (ad platforms, a shop, a measurement tool);
  - **actions, guardrails and approvers**: what an agent may do, what must hold first and who signs, checked by `validate` before the agent acts;
  - **plain files in git**: a person reviews every change, and leaving costs nothing;
  - **speaking the standards**: Apache Ossie, the draft interchange format backed by Snowflake, dbt and others, and the W3C's OWL, SKOS and SHACL, both ways, with an IRI on every entity, field, link, word, guardrail and action (§18), so a draft made elsewhere can move in and other tools can read it out.

Tools we are deliberately not becoming: Neo4j (instance graph DB, heavy JVM), an OWL editor or reasoner like Protégé (we export to it and import from it; the YAML stays the source of truth), dbt semantic layer and Cube (metrics only, not rules or actions), LinkML (schema language with no runtime; we borrow its ideas).

---

## 13. Phased roadmap

| Phase | Weeks | Deliverable |
|---|---|---|
| **1 · Core** | 1–3 | File format, Rust compiler with validation, in-memory graph, CLI (`validate`, `serve`, `describe`), MCP server with `search / describe / neighbors / constraints_for / where_is / validate`. Supply-chain example. Read-only. |
| **2 · Doors** | 4–8 | **Built:** web UI; DuckDB inside the binary (cargo feature `duckdb`, on by default) reading Parquet, CSV, `.duckdb` files, Postgres and S3/HTTP Parquet, with the ontology as SQL tables; `query(sql)` as MCP tool, CLI command, `/api/query` and a SQL box in the UI; `examples/supply-chain-parquet`; edit mode in the UI (`--allow-edit`, `--commit-edits`) and `propose_change` in MCP, both validated by the compiler before a file is written. **Open:** MySQL, `init --from <db>`, a `marketing-measurement` ontology in production use. |
| **3 · Any MCP is a source** | 9–16 | Designed in §17: agents offer to save what they settle during their work (`propose_patch`), a `logic/` folder (derived fields, metrics, functions as recipes), `does` and `mutable: false` checked by `validate`, the table browser and `init --from`. Built: `propose_patch`, metrics and functions, MCP over HTTP with a login token. A hosted copy (GitHub App sync, SSO, roles, audit) is parked in BACKLOG.md. The ontology never executes and never holds credentials. |
| **4 · Ecosystem** | ongoing | Open-source launch as Lorefile. **Built:** `npx`, Docker and `cargo install` ready for the first tag; Apache Ossie and OWL / SKOS / SHACL import and export with IRIs (§18). Kinds of entities (`extends`, `where`) for 0.2.0. **Open:** starter packs, Python/TS SDKs, LinkML, dbt/LookML import, GraphQL. |

---

## 14. The heavy example (built at the end for power users)

A fictional automotive parts manufacturer:

- **14 types:** Supplier, Part, Assembly, Facility, Auditor, PurchaseOrder, Contract, Region, Certification, Incident, Shipment, Carrier, Customer, Employee
- **22 relationships** including `supplies`, `assembled_from` (recursive bill of materials), `ships_via`, `certified_by`
- **~80 constraints** across tiering, lead time, dual-sourcing, geographic risk, certification expiry, spend concentration
- **6 actions:** switch_supplier, approve_contract, qualify_supplier, raise_incident, reroute_shipment, expedite_order
- **3 sources:** a Parquet folder with 5,000 suppliers and 60,000 parts, a Postgres ERP mock, a CSV of regions
- **Demo:** an agent is asked to reroute around a port closure; it traverses the graph, hits three constraints, proposes a plan, and `validate` approves it.

---

## 15. Decision needed

**Recommended: Option A core with DuckDB as the data door, in Rust (section 3).**

Alternatives if you disagree:
- **Option B** if you expect to store millions of instances *inside* the ontology tool and want Cypher.
- **Option C** if the company is already Parquet/Iceberg-first and you want ontology and data in one lake.

On approval, the first working example is **Phase 1**: file format, compiler, CLI, MCP server, and the supply-chain sample ontology, runnable with one command and connectable to Claude the same day.

---

## 16. Format gaps found while dogfooding

Writing `examples/ecommerce`, `examples/saas-pricing` and `examples/b2b-sales` (37 types, 75 rules, 15 actions) against the Phase 2 binary surfaced the following. Each entry says what we wanted to write, what happened, the workaround the example uses, and the change to the format or compiler that would remove the workaround. None of these changed `src/`.

1. **Reserved CEL words cannot be action inputs or property names.** `refund_order` naturally takes `return: Return`; every rule mentioning `return.order_id` failed with `reserved identifier: return`, and `validate` only reports it as E009 on the rule, not on the action. The example names the input `rma`. CEL also reserves `in`, `as`, `break`, `const`, `continue`, `else`, `for`, `function`, `if`, `import`, `let`, `loop`, `package`, `namespace`, `var`, `void` and `while`. *Fix:* a validator check (new code, say E011) on input and property names, with the message pointing at the action file.

2. **Empty CSV cells are `null`, not `""`, so "is set" tests need both.** `contract_id != ""` passed for a subscription with no contract because `null != ""` is true in CEL; `partner_id == ""` failed for every direct deal. The examples write `x != null && x != ""` and `x == null || x == ""`. *Fix:* either coerce empty cells of `string` properties to `""`, or add a `required: true` enforcement and an `isSet(x)` helper so rules can say what they mean. The `required` flag already exists on properties but nothing reads it.

3. **No `now()` or today's date.** "Close date must be in the future", "promotion not expired as of today" and "trial has ended" cannot be written; rules can only compare dates on the records with each other (`promotion.starts_on <= order.placed_on`). `timestamp()` and `duration()` do work once a `date` string gets a `T00:00:00Z` suffix (`RT-001` uses them for the 30-day return window). *Fix:* expose `now()` (and a `today` string) in the evaluation context, with a `--as-of` override for reproducible checks and tests.

4. **Cross-row rules cannot be expressed.** "No two reps own the same account in one territory", "a part needs at least two approved suppliers", "no supplier above 40% of spend" are uniqueness, count and sum rules over many rows. CEL sees one record (or one edge). `RP-001` in `b2b-sales` checks the row-level half and carries the uniqueness query in `sql:`, which the schema accepts but the engine never runs. *Fix:* evaluate `sql:` through DuckDB when the type is readable here (Phase 2 already has the engine), scoping it with the action's inputs, and make `describe` say whether a rule's `sql:` is live.

5. **Actions cannot take scalar inputs.** `restock(variant, warehouse, qty)`, `issue_credit(invoice, amount)` and `add_seats(subscription, count)` are the natural signatures, but `inputs` only maps a name to a type. The examples move the quantity onto a record (`Invoice.credit_usd`, `Subscription.seats`) and check the record, which means the agent has to pre-write the change into the row it passes inline. *Fix:* allow primitive input kinds (`qty: integer`, `amount_usd: number`) that become plain CEL variables.

6. **Inline `instances:` are not coerced by declared kind, CSV rows are.** A CSV cell is parsed according to the property's `type`, so a `number` column is always a float. An inline value keeps whatever YAML type it was written in: `max_parcel_kg: 30` arrives as an integer. Comparing across the two works (`shipment.weight_kg <= carrier.max_parcel_kg` holds), but arithmetic does not: `to.base_rate_usd <= from.base_rate_usd * 1.25` fails with `Unsupported binary operator 'mul': Int, Float` as soon as one side is an integer. The examples write every `number` value in inline rows with a decimal point (`30.0`). *Fix:* run inline instance values through the same `coerce_json` path as DuckDB rows, and add a warning for an inline value whose JSON type disagrees with the property kind.

7. **Relationship rules only bind to edges an action creates.** `add_seats` has no `effects`, so it cannot reuse the `on_plan` seat-cap rule `SP-002`; the engine would evaluate it against the action inputs with a note. The example duplicates the rule as an action-level `AS-003`. *Fix:* an optional `binds:` list on an action (`- { relationship: on_plan, from: subscription, to: plan }`) that names an existing edge without creating or removing it.

8. **Type rules are opt-in per action, and easy to forget.** Every action must list the type-level rules it wants in `checks` (`PR-002` on Promotion, `SH-006` on Carrier), while rules whose `applies_to` is the action are added automatically. Forgetting one means an agent is told "pass" for a suspended carrier. *Fix:* `checks: auto` (or a manifest default) that includes every rule on an input's type, with an explicit `skip:` list for exceptions, and a W-code when a bound type has blocking rules none of its actions check.

9. **Suggestions only swap the `to`/`from` side of a created edge.** For `restock` (no effects) and for a bad `rma` in `refund_order`, nothing is suggested even though another local row would pass. *Fix:* try alternatives for any input whose type is readable here, ranked by how many failing rules the swap fixes, and cap the work.

10. **Unresolved inputs fail as "could not evaluate", not as "unknown".** When `PaymentMethod` cannot be read, `PM-002` reports `Undeclared reference to 'status'` and blocks. That is the right verdict, but the message should say "input payment_method not loaded" rather than leak the CEL error. *Fix:* mark results for unresolved inputs as `unknown` with the fetch hint attached, and let `--strict` decide whether unknown blocks.

Smaller observations: `sql:` and `required:` are accepted silently and do nothing (see 2 and 4); a plain `description:` containing `: ` or starting with a quote breaks YAML parsing and E001 points at the line but not the key, which newcomers hit on their first rationale; the glossary's `means:` must name a type or action, so business words for a property (`EB` for `economic_buyer_contact_id`) have to be hung on the type instead.

---

## 17. Phase 3 design: any MCP is a source, the ontology keeps what matters

> Status: **design, written 2026-11-25, revised the same day after discussion, not built.** The first draft made the ontology server execute actions and hold credentials. That is withdrawn. The ontology is the keeper of information: what matters to this business, where it lives, what may and may not be done. Agents do the work through the MCPs they already have, and those tools authenticate every request themselves.

### 17.1 The principle: a Yellow Pages, not a switchboard

| The ontology is responsible for | The ontology is not responsible for |
|---|---|
| What the entities, links, metrics and words mean | Fetching rows (the data MCP does, with the user's login) |
| Where each thing lives: which MCP server, which tool, which table, which filter | Running functions (the tool or API the function names does) |
| Which guardrails an agent must check before it acts, and `validate` to check them | Authenticating or authorising anything (every API and MCP does that per request) |
| Recording what agents learned and proposed, for a person to accept | Enforcing anything inside Snowflake, BigQuery, Shopify or Meta |
| Recording who stands behind each definition (proposed, accepted, checked) | Checking a definition against data itself: "checked" is a person's word |
| Being the one copy agents read, so a definition is the same tomorrow as today | Being in the path of a write |

This is the competitive position against Foundry: the same vocabulary (entities, links, functions, actions with criteria) without owning the data, the code or the credentials. Anyone whose agent runs in Claude, ChatGPT or Cursor with a data MCP installed can use it. There is nothing to host to get value; a hosted version, if one comes, would be about keeping the one copy safe, not about executing.

### 17.2 Any MCP is a source, and the first run writes the ontology

**Before.** Monday, a marketer in Claude: "Give me last week's campaign performance." Claude has the Meta Ads MCP, the Google Ads MCP and the BigQuery MCP installed. It explores: finds the Meta insights tool, picks a date range, finds the GA4 export in BigQuery, guesses that `purchase` is the conversion that matters, joins by campaign name, reports. Thirty tool calls and three clarifying questions. Tuesday, a colleague asks the same thing and Claude explores again, guesses differently, and the numbers do not match Monday's.

**After.** Same Monday, but the ontology MCP is installed too and its instructions say: *when the user settles a definition during the work, offer to save it.* At the end Claude asks:

```
Claude: I used three things worth remembering for next time. Save them to the ontology?
  • Success metric: "purchase" events in bigquery analytics.ga4_events, count distinct transaction_id
  • Spend: Meta Ads MCP → get_insights(level: campaign, fields: spend, date_preset: last_7d)
                           Google Ads MCP → campaign_report(metrics: cost_micros)
  • Join: Meta campaign name ↔ ga4 utm_campaign (lower-cased)
User: yes, and the success metric is different for the SaaS product, there it's trial_started
Claude: Saved as a proposal in the space "acme"; someone with edit rights accepts it in the page.
```

Tuesday: "show me yesterday's performance." Claude calls `summary`, sees `Metric: paid_performance`, sees where each piece comes from, makes four tool calls, and the numbers match Monday's because the definition is the same file.

What gets written, in the file format that exists today plus the Logic folder from 17.3:

```yaml
# sources/meta_ads.yaml                     (kind: mcp exists today)
connection: meta_ads
kind: mcp
server: meta-ads
description: The Meta Ads MCP the team has installed in Claude. Spend and results by campaign.

# types/campaign.yaml
type: Campaign
source: { connection: meta_ads, tool: get_insights, params: { level: campaign, fields: [campaign_name, spend, impressions] }, key: campaign_id }

# logic/measurement.yaml                    (new)
metrics:
  - metric: success
    description: What counts as a win for this business. Different per product line.
    variants:
      - when: product_line == "store"
        source: { connection: warehouse, table: analytics.ga4_events, filter: "event_name = 'purchase'", measure: "count distinct transaction_id" }
      - when: product_line == "saas"
        source: { connection: warehouse, table: analytics.ga4_events, filter: "event_name = 'trial_started'", measure: "count distinct user_pseudo_id" }
    owner: growth@acme
  - metric: paid_performance
    description: Spend by channel next to success events, joined on campaign name.
    needs: [Campaign.spend, success]
    join: lower(meta.campaign_name) = lower(ga4.utm_campaign)
    caveats: Platform-reported spend; Meta conversions are attributed, not incremental.
```

Three businesses, three success metrics: an e-commerce store counts purchases, a SaaS product counts trials or activations, a B2B company counts qualified opportunities. The metric has the same name in every space, `success`, so a pack, a prompt or a report can ask for it without knowing the business. Only the definition differs.

**The mechanics that make the loop work:**

1. **Instructions.** The MCP `initialize` text already tells agents to load `summary` first. It gains one sentence: *when the user settles a definition (a metric, a table, a filter, a word), offer to save it with `propose_patch`.* That sentence is the product.
2. **`propose_patch`.** Today an agent proposes a change by sending the full text of one YAML file. That is fine for Claude Code and hard for a chat agent. `propose_patch` takes the same small operations the page uses (`add_type`, `set_source`, `set_property_description`, `add_relationship`, `add_constraint`, plus new `add_metric`, `add_function`, `add_term`), builds the file, compiles it, and writes or queues it exactly like `propose_change`. Agents stop writing YAML by hand.
3. **`where_is` for `kind: mcp`** already answers "which server, which tool, which params". A metric source adds `filter` and `measure` so it is a complete recipe; `join` comes with step 2.
4. **The page.** The Agents inbox shows each proposal in words: *"Claude learned: the success metric for the store is purchase events in ga4_events. Accept?"* with the YAML one click away. Accepting is what makes it true for everyone.
5. **Warehouses are the best case, not the only one.** With BigQuery or Snowflake the ontology sits on the whole business and DuckDB or the warehouse MCP can run the metric SQL. With only ad-platform MCPs it still works: the recipe says which tool and which params, and the agent runs it.

The table browser from the first draft stays, as a convenience for warehouses: list tables, tick, draft entities from columns, `init --from bigquery://…`. It is the batch version of the same loop.

### 17.3 Logic: recipes with typed ins and outs

The `logic/` folder holds three kinds of declaration. None of them run inside the ontology server.

**Derived fields**, a computed property on an entity, as CEL or SQL. `Customer.is_churned: days_since(last_order_at) > 90`.

**Metrics**, as in 17.2: a name, the definition, where it comes from, who owns it, the caveats.

**Functions**, a contract and a pointer. Forecasting is the example that came up:

```yaml
# logic/forecast.yaml
functions:
  - function: forecast_spend_outcome
    description: Projects success events for a proposed weekly spend per channel, from the MMM.
    inputs:  { plan: SpendPlan }                       # entities in, so an agent knows what to gather
    outputs: { forecast: Forecast }                    # entities out, so it knows what comes back
    runs:    { kind: mcp, server: measurement, tool: mmm_run_budget_optimizer, args: { study_id: "{{plan.study_id}}", budget: "{{plan.weekly_budget}}" } }
    writes_back: { entity: Forecast, to: { connection: warehouse, table: forecasts } }   # where the result lands, if anywhere
    may_be_called_by: [growth_agent]
```

When the user says "run the forecast on this plan", the agent reads the function, gathers a `SpendPlan`, calls the measurement MCP tool named, gets a `Forecast` back, and writes it where `writes_back` says, through the warehouse MCP. The ontology told it what to run, how, and where the result goes. That MCP authenticated the call. Nothing passed through the ontology server.

Who writes functions: the vendor whose MCP has the tool (a measurement vendor, Meta, Shopify), or the team's engineers as an HTTP endpoint the agent can call, or a pack. The ontology only needs the contract.

### 17.4 Actions and the guardrails that are for agents

An action says what may be done, what it changes, which guardrails to check, who must approve, and from Phase 3 which function does it:

```yaml
# actions/cancel_order.yaml
action: cancel_order
inputs: { order: Order, reason: string }
checks: [ORD-004, ORD-011]
requires_approval_from: support-lead@acme
does: cancel_order                    # a function in logic/, which names the Shopify MCP tool
```

`validate(action, inputs)` stays the gate: the agent calls it, gets pass, warn or fail with reasons and the approver's name, and only then calls the tool the function names. The ontology server never calls the tool.

Two small additions make the common policies one line:

- `mutable: false` on an entity means no action may change it. *"The order table never gets updated"* becomes `type: Order` + `mutable: false`, and `validate` fails any action whose effects touch an Order. The compiler also refuses an action file that declares such an effect (`E014`).
- `may_be_called_by` on a function, matched against the agent's declared name, so a reporting agent cannot run a budget change even if a person could.

**Execution as an option, not the core.** A team that wants the server to call the tool for them (one door, one audit line, credentials in one place) can have a thin runner later, off by default, for `runs: http` and `runs: mcp`. It is not needed for the product to work and it is not where enforcement comes from.

### 17.5 Enforcement: three layers, and which one is ours

| Layer | Who enforces | What it stops | Whose job |
|---|---|---|---|
| **The agent** | The agent, by reading the guardrails and calling `validate` before acting. Model vendors' agents do this reliably when the instructions and tools make it the obvious path. | An agent updating the order table, moving a budget past the cap, skipping an approver. | **Ours.** The ontology exists for this layer. |
| **The tools** | Every API and MCP, by authenticating and authorising each request: the warehouse's grants, Shopify's API scopes, a measurement vendor's own auth. | A human or an agent doing what their login does not allow. | **The customer's and the vendors'.** Already there; the ontology never widens it. |
| **The one copy** | Whoever holds the ontology agents read: a protected `main` with review and CI, or the hosted space where rules cannot be changed casually and every change is logged. | A local edit becoming everyone's rule. | **The team's, with git** (and a hosted copy's, if one comes). |

Someone with a local copy can still edit a guardrail and run their own agent against it. What they cannot do is change what everyone else's agent reads, and they cannot do anything their own logins do not already allow. That is the right boundary for a keeper of information. Teams that are serious put the hard limits in their APIs and functions, and write them into the ontology so agents stop before the API has to.

### 17.6 Hosted, later

A hosted version may come later, and with no execution and no credentials it would sell only the kept copy: one safe copy of each space that agents read through one HTTPS URL, SSO and roles for who may edit and who may accept proposals, an audit log kept for SOC2 evidence, and GitHub sync so page edits and accepted proposals become commits. It would run the same binary, and nothing of a customer's data would pass through it. It is not being built now; the earlier design (the sync diagram, the open-versus-hosted table and the pricing tiers) is in [BACKLOG.md](BACKLOG.md) under "Hosted version, later".

### 17.8 Build order

| Step | What | Size | Where |
|---|---|---|---|
| 1 | **The loop**: the "offer to save" instruction, `propose_patch` with the page's operations plus `add_metric` and `add_term`; metrics as the first `logic/` kind with a Logic step in the page; proposals shown in words in the Agents inbox | **built 2026-11-25** | open source |
| 2 | `logic/` folder: derived fields, metrics with variants, functions with `runs` and `writes_back`; validation codes; Logic step in the page | **metrics and functions built 2026-11-25**; derived fields open | open source |
| 3 | `does` on actions, `mutable: false`, `may_be_called_by`; `validate` honouring both | 1 week | open source |
| 4 | MCP over HTTP with a token and an agent name | **token and HTTP built 2026-11-26**; agent name open | open source |
| 5 | Table browser and `init --from` for BigQuery, Snowflake, Postgres | 2 weeks | open source |

The hosted control plane (GitHub App sync, SSO, roles, Slack and email) is parked in BACKLOG.md. Step 1 is the product in miniature and can be built this week on what exists: `kind: mcp` sources, `propose_change`, the proposals inbox and the patch operations are all there.

### 17.9 Decisions needed

1. **The name of the universal metric.** `success` is proposed so packs and prompts can ask for it in any space. Alternatives: `conversion`, `primary_outcome`.
2. **Expression languages.** CEL for derived fields and guardrails (in), SQL for metrics. Python stays out of the open-source path.
3. **Does the server ever call a tool?** No in the core. A thin optional runner later, off by default, for teams that want one audit line.
4. **How an agent identifies itself.** A name in the MCP command (`--agent growth_agent`) on stdio; over HTTP, a name given with the token. Needed for `may_be_called_by`.

---

## 18. Standards: Apache Ossie, OWL, SKOS and SHACL

The YAML stays the source of truth: agents write it, people read it, git diffs it, and even LinkML's own FAQ says OWL should not be used as a schema language. Lorefile then speaks the two families of standards an organisation already has, both ways, and joins them with one identity per thing. [docs/standards-explained.md](docs/standards-explained.md) walks one order through all of it.

**IRIs** (`src/iri.rs`). `ontology.yaml` may set `namespace:` (ends in `#` or `/`; default `https://lorefile.dev/ns/<space>#`) and `prefixes:`; common ones (`rdf`, `rdfs`, `owl`, `xsd`, `skos`, `sh`, `dcterms`, `schema`, `foaf`, `lorefile`) are built in. Entities, fields, links, glossary terms, guardrails and actions take an optional `iri:` (absolute, `prefix:local`, or `:local` for the space's own namespace). Without one the IRI is derived: entity `<ns>Order`, field `<ns>Order.total_usd`, link `<ns>Order.placed_by`, guardrail `<ns>EC-001`, action `<ns>cancel_order`, term `<ns>term.basket`, metric and function `<ns>name`, source `<ns>source.shop_files`, domain `<ns>domain.<slug>`, the space itself the namespace without its `#`. Fields are per entity because two entities' `status` rarely mean the same thing, and OWL would intersect their domains; to say they do, give both the same `iri:` (fields of different entities may share one, and so may links). The compiler resolves every IRI into `Ontology::iris`; E016 refuses one IRI on two things when either was written with `iri:`, W112 warns when two derived ones collide, E017 refuses an IRI that does not resolve. Files without these keys compile exactly as before.

**Apache Ossie** (`src/ossie.rs`) is the semantic-layer world (Snowflake, dbt, Cube, AtScale, Atlan, DataHub, Collibra, NVIDIA's Auto Ontology), a draft at `0.3.0.dev0`. Export writes its three documents: the semantic model (entities with a table become datasets, fields fields, metrics SQL, links with key columns joins), the ontology (every entity a concept carrying its IRI, every link a relationship whose `verbalizes` pattern comes from its sentence) and the mapping between them. Each object is rebuilt from its portable parts the way import will, and only the difference goes into `nonportable_extensions` under `vendor_name: lorefile`, which Ossie readers must keep; things Ossie has no word for (guardrails, actions, functions, sources, owners) ride there whole. So a portable edit made in another tool wins on the way back, and a Lorefile export imported back is identical, which `tests/ossie.rs` checks for every shipped folder.

**OWL, SKOS and SHACL** (`src/rdf.rs`, cargo feature `rdf`, on by default and kept in the slim build) are the W3C's ontology world (Protégé, GraphDB, Stardog, TopBraid, LinkML). `export --rdf` writes one graph as Turtle (written by our own pretty printer), JSON-LD, N-Triples or RDF/XML (Oxigraph's `oxttl`, `oxjsonld`, `oxrdfxml`); `import --from-rdf` reads all four with the same crates. No reasoner, no triple store. The mapping:

| Lorefile | Standard | Notes |
|---|---|---|
| Space (`name`, `description`, `version`, `namespace`, `prefixes`) | `owl:Ontology` with `rdfs:label`, `rdfs:comment`, `owl:versionInfo`, `vann:preferredNamespaceUri`, `sh:declare` per prefix | `lorefile:formatVersion` marks a graph Lorefile wrote |
| Domain | `skos:ConceptScheme` with `rdfs:label`, `rdfs:comment`; entities `skos:inScheme` it | `lorefile:types` keeps the order |
| Entity | `owl:Class` with `rdfs:label`, `skos:prefLabel`, `rdfs:comment` | `tags` → `dcterms:subject`; `owner` → `lorefile:owner`; `source` binding and inline `instances` → `lorefile:source`, `lorefile:instances` (`rdf:JSON`) |
| Field | `owl:DatatypeProperty` with `rdfs:label`, `rdfs:comment`, `rdfs:domain` the entity (an `owl:unionOf` when shared), `rdfs:range` an XSD type | string and enum `xsd:string`, integer `xsd:integer`, number `xsd:decimal`, boolean, date, datetime `xsd:dateTime`, json `rdf:JSON` |
| How an entity uses its fields and links | one `sh:NodeShape` (also `lorefile:EntityShape`) per entity, `sh:targetClass` it, one `sh:property` per field and link: `sh:path`, `sh:name`, `sh:description`, `sh:order`, `sh:datatype` or `sh:class` | `required` → `sh:minCount 1`; enum `values` → `sh:in`; many-to-one or one-to-one → `sh:maxCount 1`; `key` → `lorefile:key`; `unit` → `lorefile:unit`; the cardinality text → `lorefile:cardinality` |
| Link | `owl:ObjectProperty` with `rdfs:label`, the sentence as `rdfs:comment`, `rdfs:domain`, `rdfs:range` | many-to-one and one-to-one also `owl:FunctionalProperty`; one-to-one and one-to-many also `owl:InverseFunctionalProperty`; edge properties and edge tables → `lorefile:properties`, `lorefile:edgeSource` |
| Glossary term | `skos:Concept` with `skos:prefLabel`, `skos:definition`, `lorefile:means` (and `lorefile:meansName`), and `skos:altLabel` on what it means | |
| Guardrail | `sh:NodeShape` with `sh:severity` (`sh:Violation`, `sh:Warning`, `sh:Info`), `sh:message` (its name), `rdfs:comment`, `sh:targetClass` (or `sh:targetSubjectsOf` the link) | the rule always as `lorefile:cel`, with `lorefile:appliesTo`, `lorefile:rationale`, `lorefile:owner`; a conjunction of `x in [...]`, `x >= n` and friends, `x == v` and `has(self.x)` also becomes native `sh:in`, `sh:minInclusive` …, `sh:hasValue`, `sh:minCount`; anything else is `lorefile:celOnly`. A guardrail an action checks (its `applies_to` is the action, or an action lists it) gets `sh:deactivated true` and `lorefile:action`, because SHACL checks every row at any time and Lorefile checks before an action |
| Action, metric, function, source | `lorefile:Action`, `lorefile:Metric`, `lorefile:Function`, `lorefile:Source` with `rdfs:label`, `rdfs:comment` and the object as `lorefile:definition` (`rdf:JSON`) | valid RDF that other tools store; a changed `rdfs:comment` wins on import |
| File layout and order | `lorefile:file`, `lorefile:order` on each node | so an export imported back is identical, file for file |

The `lorefile:` vocabulary lives at `https://lorefile.dev/vocab#`. Import has two modes per node: a node Lorefile wrote (it carries `lorefile:file`, or it is a `lorefile:EntityShape`) is restored exactly; anything else is read the way the standards mean it, with `rdfs:Class` and schema.org's `domainIncludes` / `rangeIncludes` understood, `owl:hasKey` as the key, `rdfs:subClassOf` as `extends` with the parents brought along (`--no-parents` keeps a `subclass-of:` tag), SHACL property shapes as guardrails (`sh:minCount` → `has(self.x)`, `sh:in`, ranges, lengths, `sh:pattern` → `matches`), and a report of what was guessed, dropped (statements for reasoning, links to classes not imported) or kept for a person (SHACL with no CEL form). `--only` and `--depth` cut a large vocabulary down. Like the Ossie import, the files are compiled over the target folder first and nothing is written unless that compiles; no existing file is overwritten. `tests/rdf.rs` round-trips every shipped folder, checks the four formats carry the same graph, imports a slice of schema.org, and checks that a SHACL shape becomes a guardrail `check` enforces.

## 19. Evidence grades: who stands behind each definition

Built 2026-11-26 from `specs/2026-11-27-evidence-grades.md`. Agents read a definition another agent guessed yesterday in the same voice as one Finance checked against the ledger; grades make the difference visible, without letting an agent promote itself.

| Grade | Means | Set by |
|---|---|---|
| (none) ungraded | nobody has said | existing spaces, hand-written files, imports, copies |
| proposed | an agent wrote it, no person has looked | the save gate, for an agent's direct write (`--allow-edit`) |
| accepted | a person stands behind the words | a person's edit or Accept in the page, Looks right, Vouch for all (never-graded things only), `lorefile trust accept` |
| checked | a person ran it against real rows and it was right (who, when, which source, what was compared) | Mark as checked…, `lorefile trust check`, or a person confirming an agent's `record_check` |

**Store.** One `trust.yaml` per space (`src/trust.rs`), one flow-style line per graded thing under `metrics`, `bindings`, `terms`, `guardrails`, written only by Lorefile. Not a key on each fact: agents write the fact files, so a grade there would have to be filtered out of every agent's text; inserting it would also rewrite hand-written files and turn every waiting proposal stale. `edit::apply` and `apply_files` refuse `trust.yaml` unless the write is Undo, which covers agents and the page's raw editor in one place; a `grade:` key in a fact file is an unknown key (E001).

**Fingerprint.** Each line carries `of`, the first 12 hex characters of an FNV-1a hash (written in the crate, stable across Rust versions) of the thing as compiled, with keys sorted and `file` left out; a binding's covers its entity name, key and `source:`. A metric's and a binding's also carry the definition of every source they read (its description left out), so re-pointing a source stops, or for an agent queues, every grade that depends on it; a word written twice is one graded thing covering every meaning. A thing that no longer matches counts as ungraded and `validate` warns W113; an entry naming nothing warns W114; nonsense (checked without a source, a word marked checked, an empty `of`) is E018. A space with no `trust.yaml` compiles with exactly the diagnostics it had.

**The gate.** Inside the save gate (`edit::stamp_for`), the ontology before and after a change is compared kind by kind: new and changed things get the saver's grade (agent → proposed, person → accepted; the person who accepted a proposal, not the agent who wrote it), deleted ones lose their line, and a person's words-only change to a checked metric or guardrail keeps the check with a fresh `of`. The entries are laid over `trust.yaml` as it is on disk, under a lock file in `.lorefile/` (stdio `mcp` and `serve` are separate processes), and the file is written before the facts in the same commit, so a crash leaves a harmless W114 orphan and never an unflagged fact. Undo does not restamp: the file goes back entry by entry with the facts (`trust::merge_undo`). `mcp::Server::propose` asks `edit::graded_overwrites` first and queues, even with `--allow-edit`, any agent change that would alter or remove an accepted or checked thing, saying which and why. `record_check` (`patch::plan_check`) is always queued, as a proposal with a `check` claim instead of a file.

**What agents read.** `summary` gets one Trust line (only when the space has a `trust.yaml`) and marks proposed things UNVERIFIED and checked things with date and source; `describe` returns `trust` with `say`, a sentence the agent can use as it is; the server's instructions tell agents to say "unverified" before relying on a proposed definition and never to add numbers of different `basis`. `validate` verdicts do not change (an unreviewed blocking rule still blocks, failing safe); `unverified` names the unreviewed rules and data bindings the answer relied on, each rule result carries `grade` once the space has a `trust.yaml`, and `where_is` returns the binding's `trust`. An agent's `record_check` carries the fingerprint of what it compared, and a person's click is refused when the definition has changed since.

**basis.** A separate optional key on metrics and variants (`transactional`, `attributed`, `modelled`, `experimental`): what kind of number it is, part of the definition and graded with it.

**Export and import.** RDF: `lorefile:trust` on the graded thing (`lorefile:bindingTrust` on an entity), plus a `lorefile:Grading` record that is also a `prov:Activity` (`prov:wasAssociatedWith` a named `prov:Person` or `prov:SoftwareAgent`, `prov:endedAtTime`, `prov:used` the thing and the source). Ossie: the whole book in the root `lorefile` extension, and `ai_context.instructions` "Unverified: …" on a proposed metric or dataset (Ossie 0.3.0.dev0 has no native review-status field). Imports start ungraded; the claims are counted in the report (unreadable ones too); no imported object may name `trust.yaml` as its file; `--keep-grades` restores, for a team's own export, the grades that still match and that the compiler accepts. A copy of a space leaves `trust.yaml` behind.

**Not done.** It grades definitions, not answers; it runs no check; it is as tamper-proof as the repository's review; "Priya" is a typed name until accounts exist. Grades on entities, fields, links, functions, actions and sources, and rules that read a grade, are in the backlog.

## 20. A file is learned, not stored

Built 2026-11-27 from `specs/2026-11-27-learn-file.md`. When a person gives an agent a file (a monthly export uploaded to ChatGPT, say) and says "save this", Lorefile saves what the file *means*, so the next file of the same shape is read the same way by any agent. It never sees a row. The agent sends the file's name, its column names, up to five example values per column and some counts; `src/learn.rs`, pure functions with no I/O, turns that into a draft by fixed rules (field names from the headers, types from the examples, the key from the counts, a `table` from the file name without its date), and the examples are dropped when the call ends: they never reach YAML, a proposal, the activity log or history, and a description that quotes one is left out. Two tools: `match_file` reads only and answers same, changed, waiting or new, by column names alone, over every bound entity and over `learn_file` proposals still waiting (each records the entity and the normalized headers, `Proposal.learned`); `learn_file` turns the user's answers into ordinary `propose_patch` ops (`add_source`, `add_type` with `properties`, `set_source`, `add_metric`, `add_term`, or `add_property` for a changed file) and saves them through `patch::plan_many` and `propose_many` as one proposal, so the existing gate, grades and Accept apply unchanged. The rows' home is a new source kind, `from_user`: never readable here, and every surface (`where_is`, `instances`, `query`, `summary`, `describe`, the page) says the rows arrive as a file a person gives the agent. What does not count goes in the metric's `filter` and `caveats`, not a guardrail. Lorefile never computes the number: `read_it` is a recipe, and the agent applies it to the file with its own tools.

## 21. Reviewing by meaning

Built 2026-11-27 from `specs/2026-11-27-review-flow.md`. A proposal is reviewed by what it means, not by its lines. `src/review.rs` compiles the space as it is and with the proposal's files laid over it (the same `check_many` the save gate uses), and compares the two thing by thing: entities and their columns, links, metrics, rules, glossary words, sources, functions and actions, with renames found by shape. It words the result for people (types as "money (USD)", recipes as "add up revenue, leaving out lines where refunded is true", rules as "refunded must be false") and turns the warnings a proposal adds into questions, three of them with a fix the screen offers. The page only renders `GET /api/proposals/<id>/review`.

Accept runs in a fixed order under a per-space decision lock (`.lorefile/decide.lock`, outside the save gate's own `trust.lock`): read the proposal again; if its files moved on, plan it again from its stored ops (or `learn_file`'s input without examples) on today's files, or refuse in words when it cannot be; check the fingerprint of what the screen showed; plan the person's fixes on top; then the save gate, one change with the agent as author and the person as committer. History off, it keeps the before and after texts so Take it back can put them back while nothing else has changed them; history on, Take it back is that commit's Undo. A connection approved with "wait for me" gets edit mode off whatever `--allow-edit` says, which is what lets one server take ChatGPT's proposals and the page's Accept.


## 22. The home page and the Map

`GET /s/<space>/api/plane` (`src/plane.rs`) answers with one picture of a space: each entity with its traffic light and the reasons for it in words, how important it is and why, a glyph guessed from its name, the metrics that count it (matched on the source's connection and table) and the fields they use, the sources underneath, the agents above (OAuth connections that can reach the space, plus channels seen in its activity), and up to five waiting proposals as ghosts (`review::graph`, cached by proposal and the space's files). Lights roll up: an entity shows the worst of itself, its source, its metrics and its guardrails. `?version=` answers `{same: true}` when nothing changed, so the page can ask every 30 seconds.

Lights come from diagnostics, so every diagnostic that names one item carries it as `subject` (`validate::Diag`), and `review::questions` reads the subject instead of parsing messages.

The page draws Home and Map with the same code (`// ---------- plane ----------` in `index.html`): one SVG, a deterministic layout (best-first walk from the most important entity, on a square spiral per area), two projections (isometric and flat) and one camera transform for pan and zoom. Map's dragged cells and pins live in the browser's `localStorage`, never in the space. The plane carries names, words and grades only, never a row value or a metric's value.
