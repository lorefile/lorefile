# Deploying Stella Ontology

Three ways to run it, from a laptop to a hosted service, and how data access and approvals work in each. Plain English, with the honest status of each path.

---

## 1. The mental model first

Two kinds of MCP servers sit next to an agent:

```
                 ┌─────────────────────────┐
   "What is a   │  Ontology MCP            │   knows the MEANING: types, rules, actions,
    supplier,   │  (this project)          │   who approves, and WHERE each table lives.
    what rules  │                          │   Never holds warehouse credentials in files.
    apply?"     └─────────────────────────┘
  Agent ──┤
   "Give me     ┌─────────────────────────┐
    the rows    │  Data MCPs               │   hold the DATA and the user's own permissions:
    for Q3"     │  BigQuery, Snowflake,    │   BigQuery MCP, Snowflake MCP, Meta Ads MCP,
                │  Meta Ads, Google Ads…   │   Google Ads MCP, Postgres MCP …
                └─────────────────────────┘
```

The ontology MCP tells the agent *what things mean and which table to look in*. The data MCP, with the user's own service account or OAuth login, *fetches the rows*. Permissions stay where they already are: in the warehouse or the ad platform. The ontology never widens them.

DuckDB inside the binary is a convenience for the cases where you want the ontology server itself to read rows, so `validate` can check a rule against a real record without a second MCP. It is a connector, not a database. CSV, Parquet, S3 and Postgres work today; it is optional and can be compiled out.

---

## 2. Three ways to run it

### A. On a laptop (today, works)

```bash
stella-ontology serve --dir ./ontology          # web page on http://127.0.0.1:7070
stella-ontology mcp --dir ./ontology            # MCP over stdio, for Claude Code / Claude Desktop / Cursor
```

The folder is the deployment. Copy it, zip it, `git clone` it. Nothing else to install.

### B. In your own cloud (today, works; Docker image untested on our side)

The [Dockerfile](Dockerfile) builds a small image: the binary plus your ontology folder.

```bash
docker build -t stella-ontology .
docker run -p 7070:7070 -v $(pwd)/ontology:/ontology stella-ontology
```

Runs anywhere a container runs: Cloud Run, Fly.io, ECS, Kubernetes, a $5 VPS. It is stateless, so run two for high availability and restart freely. The folder comes from a volume, a `git clone` at boot, or a bucket mount. Warehouse credentials come from environment variables or your secret manager, never from the folder.

Two gaps for a real cloud deployment, both on the roadmap:

- **MCP over HTTP.** Today MCP is stdio only, which suits a laptop. ChatGPT, hosted agents, and anything on another machine need the HTTP transport. That is Phase 3 work.
- **Authentication.** The web page has no login. Put it behind your SSO proxy (Cloudflare Access, IAP, Tailscale) until the hosted version exists.

### C. Stella Cloud (not built yet; this is the plan)

Stella runs the same open-source binary for you and adds the parts that are hard to do alone:

```
You                                    Stella Cloud (SOC2)
───                                    ───────────────────
github.com/you/ontology ──push──►      sync → validate → serve
                                       ├─ you.ontology.stella.app         web page, SSO
                                       ├─ you.ontology.stella.app/mcp     one URL per agent, API key each
                                       ├─ audit log: which person or agent read or changed what
                                       ├─ read-only warehouse credentials held in Stella's secret manager
                                       ├─ every change is a pull request, approvers from the `owner:` fields
                                       └─ versions, rollback, uptime, support
```

You connect a GitHub repository (or upload a folder), and Stella gives you the URL to paste into Claude, ChatGPT, or your own agents. Free to run yourself, paid when Stella runs it for you.

---

## 3. How a new business would start

Say you have just started an app business and want agents to understand it.

1. `stella-ontology init ./ontology` writes a starter folder with two types, two rules and one action.
2. Open the folder in Claude Code and say: *"This is a subscription app. Add Customer, Plan, Subscription, Invoice. A customer on a trial cannot be invoiced. Refunds over $500 need Finance."* Claude edits the YAML files directly, because they are just files. `stella-ontology validate` catches anything misspelled.
3. Point the data bindings at where the rows live: a Postgres `source:` for customers, a Stripe export in Parquet, a BigQuery table for events.
4. Register the MCP server with Claude Desktop or Claude Code (one command, in [README.md](README.md)). From now on, every conversation starts with the same understanding of the business.
5. Later, in Stella Cloud, the same folder gets a URL that ChatGPT and hosted agents can use too. Everyone reads from one place.

What the ontology **ingests** is structure and meaning, not rows. Phase 3 adds `init --from postgres://…` to draft types from an existing database schema so step 2 starts from a filled-in list instead of a blank page.

---

## 4. How approvals work

There are two different approvals, and they should not be confused.

| Approval | What it protects | How it works |
|---|---|---|
| **Changing the ontology** (a new rule, a changed threshold) | The definitions themselves | A file edit, reviewed as a pull request. Each type, rule and action has an `owner:`; they are the natural reviewers. Merge, and the server reloads. In Stella Cloud the same flow runs through the web page. |
| **Taking an action** (switching a supplier, approving a contract) | The business | `validate` says pass / warn / fail and names `requires_approval_from`. The agent reports that to the human. The ontology records the policy; it does not sign off on behalf of anyone. |

Agents can read everything through MCP. Writing through MCP (`propose_change`, which opens a pull request) is Phase 3 and will go through the same review.

---

## 5. Warehouse permissions, concretely

- **Credentials never live in the folder.** A `source:` file names an environment variable, nothing more. The repository is safe to share.
- **Read-only.** When the binary reads through DuckDB it attaches sources read-only and refuses anything but a single `SELECT`. Use a read-only service account anyway.
- **Row-level security stays in the warehouse.** If BigQuery hides EU rows from a user, the ontology MCP does not un-hide them, because it never fetched them; the user's own BigQuery MCP did, under the user's own login.
- **In Stella Cloud**, each tenant gets its own secret store entry and its own isolated process. The hosted audit log records every tool call: which agent, which tool, which type, when.

---

## 6. Ontology packs for common MCPs (idea, not built)

Meta Ads, Google Ads, Shopify, Stripe, HubSpot and similar MCP servers return data but no business meaning: what is a "conversion", which campaign objective maps to which KPI, what "state-level" reporting can and cannot tell you. A pack is a ready-made ontology folder for one of those sources, with the type descriptions, glossary, data bindings and the gotchas written down, so an agent that has both MCPs knows what it is looking at. Stella could ship the measurement-relevant ones first. This is on the list, not in the code.

---

## 7. Status of each path

| Path | Status |
|---|---|
| Laptop: `serve`, `mcp` over stdio | Built, verified |
| Docker image | Dockerfile written, not yet built on our side (no Docker on the build machine) |
| `--host 0.0.0.0` for containers | Built |
| MCP over HTTP | Not built (Phase 3) |
| Login for the web page | Not built; use an SSO proxy |
| Stella Cloud | Design only |
| `init --from <database>` | Not built (Phase 3) |
| `propose_change` through MCP | Not built (Phase 3) |
