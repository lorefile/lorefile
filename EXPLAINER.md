# Lorefile, explained in plain English

This page answers three questions: what did we build, how do the pieces fit together, and what does it change for a person or an AI agent. No code knowledge needed. The technical reference is [README.md](README.md); the design decisions are in [ARCHITECTURE.md](ARCHITECTURE.md).

---

## 1. The one-paragraph version

You describe your business in a folder of small text files: what a Supplier is, how it relates to a Part, which rules apply, what an agent is allowed to do, and where the real rows live. One small program reads that folder and opens three doors into it: a **web page** for people, an **MCP server** for AI agents like Claude, and a **command line** for scripts. The program never copies your data. It knows where the data is and reaches in only when a rule needs a real row.

---

## 2. Before and after

### A supply-chain manager and Claude

**Before.**
Maya, an ops manager, types into Claude: *"Can we move part BRK-220 from Acme Corp to Beta Ltd?"*
Claude has no idea what "part" or "tier" mean at her company. It does not know BRK-220 is a brake part, that Beta Ltd is a tier-3 spot-buy vendor, or that Procurement must sign off. It either guesses, or asks Maya ten questions she would have to look up in three systems.

**After.**
Claude is connected to the ontology. The same question now runs like this:

```
1. summary                       → learns the vocabulary: Supplier, Part, tiers, who approves what
2. describe("Part")              → BRK-220 is safety_critical = true
3. constraints_for("switch_supplier")
                                 → 10 rules apply; 5 of them block
4. validate("switch_supplier", {part: BRK-220, from: V-10001, to: V-10422})
                                 → FAIL
                                   SC-041  safety-critical parts need a tier-1 supplier   (Beta Ltd is tier 3)
                                   SC-063  safety-critical parts need IATF 16949          (Beta Ltd is not certified)
                                   would pass instead: Gamma Inc, Delta Co, Kappa Precision …
                                   approval needed: Procurement Lead (policy PR-007)
```

Claude answers in one turn: *"No. Beta Ltd is tier 3 and not IATF-certified, which rules SC-041 and SC-063 block for a safety-critical part. Gamma Inc or Delta Co would pass. Procurement Lead has to approve either way."*

Nobody wrote a prompt for that. The rules, the rationale, and the approver all came from the folder.

### A measurement company's own tools

**Before.** A marketing-measurement company's MMM, geo-experiment, and attribution products each carry their own private definition of "channel", "campaign", "pre-period". An agent using its tools learns the differences by trial and error, and a customer's agent never learns them at all.

**After.** One `marketing-measurement` folder defines Channel, Campaign, Experiment, KPI, Model, with rules such as *"a geo experiment needs at least 4 weeks of pre-period data"*. Every product it ships and every agent reads the same definitions. When the rule changes, it changes in one file, reviewed in one pull request.

---

## 3. The pieces, and what each one does

| Piece | In plain words | Where it lives |
|---|---|---|
| **The ontology folder** | The source of truth. Text files a human or an agent can read and edit. | `examples/supply-chain/` |
| **The compiler** | Reads every file, checks that nothing is misspelled or missing, builds a graph in memory. Takes milliseconds. | inside the binary |
| **The rules engine** | Evaluates a rule like `tier == 1` against a real row and says pass or fail, and why. | inside the binary |
| **The data door** | Fetches real rows when a rule needs them. CSV files directly; Parquet, Postgres, and others through DuckDB. | inside the binary |
| **The MCP server** | What Claude or any agent talks to. Nine lookup tools plus `validate` and `query`. | `lorefile mcp` |
| **The web page** | What you see in the browser. Same lookups, drawn for people. | `lorefile serve` |
| **The command line** | Same lookups for scripts and CI. | `lorefile check …` |

All three doors call the same functions. If the web page shows a rule, the agent sees the same rule.

```
  WHAT YOU EDIT                 WHAT THE BINARY DOES                   WHO USES IT
  ─────────────                 ────────────────────                   ───────────
  ontology/
  ├─ types/*.yaml      ──┐
  ├─ relationships/    ──┤      read ─► check ─► graph in memory  ──►  web page   (people)
  ├─ constraints/      ──┤                          │                 MCP server (Claude, agents)
  ├─ actions/          ──┤                          │                 command line (scripts, CI)
  ├─ sources/*.yaml    ──┘                          ▼
  └─ data/*.csv  ─────────────────────────►  data door  ◄──── Parquet, Postgres, S3 (via DuckDB)
                                                         ◄──── Snowflake, BigQuery (binding only, for now)
```

---

## 4. Where DuckDB fits, honestly

DuckDB is **not** where the ontology lives. The ontology lives in the YAML files, full stop.

DuckDB is a universal adapter plug for data. The binary carries it inside so one `source:` block in YAML can point at a Parquet file, a Postgres database, or a bucket in S3, and the same `validate` call works. Three things to know:

| Situation | Does DuckDB run? |
|---|---|
| `examples/supply-chain` (rows in `data/*.csv`) | **No.** CSV rows are read directly. The screenshot of the Supplier page is this case. |
| `examples/supply-chain-parquet` (rows in `data/*.parquet`) | **Yes.** DuckDB reads the Parquet files. The result is byte-for-byte the same as the CSV case. |
| The SQL box on the web page, or the `query` tool | **Yes.** DuckDB holds the ontology as tables (`nodes`, `edges`, `constraints`) so SQL can join "rules about Supplier" to "actual suppliers". |

Snowflake and BigQuery are declared as sources today so an agent can learn *where* a table lives (`where_is`) and query it with its own connector. Reading them from inside the binary is Phase 3.

---

## 5. The web page, annotated

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ acme-supply-chain v0.1.0   10 types · 18 rules · 4 actions     [Reload]      │  ← counts come from the compiler
├───────────────┬───────────────────────────────────┬──────────────────────────┤
│ 🔍 search     │  Supplier                          │ RULES ON SUPPLIER (11)   │
│               │  "A company we buy parts from…"    │ ⛔ SC-041 block          │
│ SUPPLY CHAIN  │                                    │ ⛔ SC-043 block          │
│  Assembly     │  [Overview] [Graph] [Data] [Yaml]  │ ⚠  SC-042 warn           │
│  Part         │                                    │ …                        │
│  Supplier ●   │  PROPERTIES                        │                          │
│ QUALITY       │  supplier_id  string  key          │ DATA                     │
│  Auditor      │  tier         enum [1,2,3]         │ erp_files · suppliers    │
│  Incident     │  status       enum [approved…]     │ 24 rows                  │
│ RULES (18)    │                                    │                          │
│  ⛔ CT-001    │  RELATIONSHIPS                     │ FILE                     │
│  ⛔ SC-041    │  Supplier ─supplies→ Part          │ types/supplier.yaml      │
│  ⚠  SC-042    │  Supplier ─located_in→ Region      │                          │
├───────────────┴───────────────────────────────────┴──────────────────────────┤
│ ✓ valid · 0 errors · 0 warnings      MCP: lorefile mcp --dir …        │  ← compiler status
└──────────────────────────────────────────────────────────────────────────────┘
   ▲                        ▲                                   ▲
   ontology.yaml            types/supplier.yaml                 constraints/**/*.yaml,
   (domains) + each         (one file = one type)               filtered to this type
   types/*.yaml file
```

- **Left column** is the table of contents: domains from `ontology.yaml`, one line per type file, then every rule, action, and source.
- **Centre** is one file, drawn nicely. The **Yaml** tab shows the raw text; the **Graph** tab draws the neighbours; the **Data** tab opens the data door and lists real rows.
- **Right column** is every rule that touches this type, pulled from wherever it was written.
- On an **action** page the centre has a "Try it" form. It calls the same `validate` an agent calls and shows pass, warn, or fail with reasons.

---

## 6. What an agent sees

An agent connects once and gets a small set of tools. Think of them as questions it can ask:

| It asks | It gets |
|---|---|
| "Give me the overview" (`summary`) | Two pages of Markdown: types by domain, blocking rules, actions, data sources, glossary |
| "What is a vendor?" (`describe`) | The Supplier type, its properties, relationships, rules, and where its rows live. "Vendor" resolves via the glossary |
| "What rules apply to switching a supplier?" (`constraints_for`) | Ten rules, blocking ones first, each with its rationale |
| "May I do this?" (`validate`) | pass / warn / fail, which rule fired, why, who approves, and inputs that would pass instead |
| "Where are the purchase orders?" (`where_is`) | `snowflake_prod` · `ERP.PURCHASE_ORDERS` · key `PO_ID` · column mapping |
| "Run this SQL" (`query`) | Rows, read-only, joined across the ontology and the data |

The agent never loads all 18 rules at once. It asks for the ones that matter for what it is about to do.

---

## 7. What is real today, and what is not

| Built and verified | Not built yet |
|---|---|
| YAML format, compiler, validation with error codes | Bootstrapping an ontology from an existing database schema |
| Graph queries, search, glossary | Editing in the web page (today: edit the file, press Reload) |
| Rules in CEL, `validate` with suggestions | Agents proposing changes as pull requests |
| MCP over stdio, tested with Claude's protocol | MCP over HTTP (needed for a hosted version) |
| Web page, command line | MySQL, Snowflake, BigQuery read from inside the binary |
| CSV, Parquet, local DuckDB, Postgres, S3 via DuckDB | Hosted multi-tenant version with SSO and audit log |
| Supply-chain example in CSV and Parquet flavours | A marketing-measurement example |

---

## 8. Words used on this page

- **Ontology**: the description of your business. Types, relationships, rules, actions. Small, slow-changing, reviewed.
- **Type**: a kind of thing. Supplier, Part, Contract.
- **Instance / row**: one actual thing. Acme Corp. Lives in your data, not in the ontology.
- **Relationship**: how two types connect. Supplier *supplies* Part.
- **Rule / constraint**: a condition with a severity. `block` stops an action, `warn` flags it, `info` is a note.
- **Action**: something an agent may do, with the rules to check first and who must approve.
- **Source / binding**: where a type's rows live. Connection, table, key column.
- **MCP**: the protocol AI assistants use to call tools. Claude, ChatGPT, and Cursor speak it.
- **CEL**: the small expression language rules are written in. `tier == 1 implies lead_time_days <= 30`.
- **DuckDB**: an embedded SQL engine that reads many formats. Our adapter plug, not our database.
- **Parquet**: a compact file format for tables, common in data warehouses.

---

## 9. Try the whole loop in five commands

```bash
cargo build --release
```

```bash
./target/release/lorefile validate --dir examples/supply-chain
```

```bash
./target/release/lorefile check --dir examples/supply-chain switch_supplier part=BRK-220 from=V-10001 to=V-10422
```

```bash
./target/release/lorefile serve --dir examples/supply-chain-parquet --port 7071
```

```bash
claude mcp add lorefile -- ./target/release/lorefile mcp --dir ./examples/supply-chain
```

The third command is the "after" story from section 2, run by hand. The fourth opens the Parquet flavour, where DuckDB does the reading. The fifth hands the same thing to Claude.
