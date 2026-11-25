# Stella Ontology — Architecture Options

> Status: **approved 2026-11-25 — Option A core + DuckDB as the data door, in Rust.** Phase 1 is built; see [README.md](README.md) to run it. Sections below are the design as approved.

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

### Before / after: Stella's own apps

**Before.** Stella's MMM, experiments, and attribution tools each carry their own idea of "channel", "campaign", "KPI", "pre-period". Agents using Stella's MCP learn these by trial and error.

**After.** One `marketing-measurement` ontology defines `Channel`, `Campaign`, `Experiment`, `KPI`, `Model`, with rules like *"a geo experiment needs at least 4 weeks of pre-period data"* and *"a Channel's spend has exactly one source of truth"*. Every Stella tool and every agent reads the same definitions.

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
        ▼  stella-ontology serve ./ontology
   in-memory graph + small snapshot file (.stella/ontology.db)
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
 │  stella-ontology   (one Rust binary, ~50 MB, starts in <100ms) │
 │                                                               │
 │  1. parse      every YAML file                                │
 │  2. validate   types exist · rules reference real properties  │
 │                · no duplicate IDs · cardinalities make sense  │
 │  3. graph      in RAM (10k nodes → microsecond lookups)       │
 │  4. snapshot   .stella/ontology.db  (fast cold start, UI)      │
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
- **No lock-in, ever.** A customer who leaves Stella Cloud still has a folder of YAML and a free binary.
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

One page, three columns. Boots with `stella-ontology serve` and opens in the browser. Same page is embedded in Stella Cloud.

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

Editing from the UI is opt-in (`serve --allow-edit`): an Edit button on every page and a "+ New" menu write the same YAML files, after the compiler has accepted the change (errors block, warnings pass). With `--commit-edits` each save is one git commit; otherwise the footer shows the uncommitted files for review. Agents get the same path as the `propose_change` MCP tool. Next: edit in UI → opens a PR.

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

## 10. Hosting: laptop → any cloud → Stella Cloud

**Local.** `brew install stella-ontology` (or one `curl`), then `stella-ontology serve ./ontology`. UI at `localhost:7070`, MCP on stdio or `localhost:7070/mcp`.

**Any cloud.** The same binary in a container. The folder comes from a git clone at boot, or an S3/GCS bucket mount. Stateless, so it scales horizontally and restarts freely. Fly.io, Cloud Run, ECS, a $5 VPS, all fine.

**Stella Cloud (SOC2).**

```
Customer                                 Stella Cloud
────────                                 ────────────
github.com/acme/ontology ───push───►     sync → compile → serve
                                         ├─ acme.ontology.stella.app          UI with SSO
                                         ├─ acme.ontology.stella.app/mcp      API key per agent
                                         ├─ audit log: which person or agent read/changed what
                                         ├─ connectors with secrets held by Stella (Snowflake, BQ…)
                                         ├─ versions, rollback, scheduled freshness checks
                                         └─ backups, uptime, support
```

Customers connect a repo through a GitHub app, or upload a folder. We run the open-source binary for them and add the things that are hard to do alone: identity, secrets, audit, uptime, multi-tenant isolation. Pricing can be a flat fee per ontology plus seats, with an enterprise tier for SSO and private connectors.

---

## 11. Open source strategy

**Open core, Apache 2.0.** Everything in this document except the cloud layer is open.

| Open source (free, forever) | Stella Cloud (paid) |
|---|---|
| File format and compiler | Multi-tenant hosting |
| MCP server, API, CLI | SSO, roles, per-agent API keys |
| UI | Audit log and SOC2 reports |
| DuckDB connectors | Managed secrets for warehouse connections |
| Starter ontologies | Versioning UI, rollback, scheduled syncs |
| | Support and SLAs |

How people benefit:

- **No lock-in.** Their ontology is a folder they own.
- **Community connectors and starter packs.** `supply-chain`, `saas-metrics`, `marketing-measurement`, `healthcare-claims`. Each a repo anyone can fork.
- **Agent framework integrations** contributed by the community: Claude Agent SDK, OpenAI Agents, LangChain, CrewAI.
- **Interop.** Export to OWL/RDF/JSON-LD/LinkML for the academic and enterprise world; import from dbt, LookML, OpenAPI, Protobuf, and raw database schemas.
- **Trust.** Companies adopt ontology tooling slowly. Being able to read every line of the thing that governs their agents' behaviour matters.

Why it helps Stella: the format becomes a standard, the hosted version is the obvious place to run it, and every Stella product gets a shared vocabulary for free.

---

## 12. The 5 to 10 year view

- **The ontology becomes the contract between humans, data, and agents.** Every agent action is validated against it. "Did the agent follow policy?" becomes a query.
- **Bootstrapping from what exists.** `stella-ontology init --from postgres://…` reads a schema and drafts the types. An LLM fills in descriptions. Day-one onboarding goes from weeks to an afternoon.
- **Agents improve the ontology.** "40 suppliers have a `sustainability_score` column that isn't in the ontology. Add it?" arrives as a PR.
- **Time travel for free.** Git gives "what did our rules look like in Q2?" without building anything.
- **Standards gravity.** Palantir Foundry proved the ontology-as-operating-layer idea is worth billions, but closed. There is no open, lightweight, agent-first equivalent today. This is that gap.

Tools we are deliberately not becoming: Neo4j (instance graph DB, heavy JVM), Protégé/OWL (academic), dbt semantic layer and Cube (metrics only, not rules or actions), LinkML (schema language with no runtime; we borrow its ideas).

---

## 13. Phased roadmap

| Phase | Weeks | Deliverable |
|---|---|---|
| **1 · Core** | 1–3 | File format, Rust compiler with validation, in-memory graph, CLI (`validate`, `serve`, `describe`), MCP server with `search / describe / neighbors / constraints_for / where_is / validate`. Supply-chain example. Read-only. |
| **2 · Doors** | 4–8 | **Built:** web UI; DuckDB inside the binary (cargo feature `duckdb`, on by default) reading Parquet, CSV, `.duckdb` files, Postgres and S3/HTTP Parquet, with the ontology as SQL tables; `query(sql)` as MCP tool, CLI command, `/api/query` and a SQL box in the UI; `examples/supply-chain-parquet`; edit mode in the UI (`--allow-edit`, `--commit-edits`) and `propose_change` in MCP, both validated by the compiler before a file is written. **Open:** MySQL, `init --from <db>`, Stella `marketing-measurement` ontology in production use internally. |
| **3 · Cloud** | 9–16 | Snowflake, BigQuery, Databricks. `propose_change` opening PRs (today it validates and writes locally). Semantic search. Stella Cloud multi-tenant alpha with SSO and audit. |
| **4 · Ecosystem** | ongoing | Starter packs, Python/TS SDKs, OWL/LinkML export, dbt/LookML import, GraphQL. Open-source launch. |

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
