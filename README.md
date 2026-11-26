# Lorefile

Your business, written down once, so every AI agent reads the same thing and acts the same way tomorrow.

**Lore** is what a team knows and newcomers have to be told: what counts as a sale, where the orders live, which number the business is run on, what an agent may never do. A **Lorefile** is that lore written down in plain files that agents read.

> Status, 2026-11-27: version 0.1.0 is out, open source under Apache-2.0. Built and tested: the format and its strict compiler (an unknown key in any file is an error), the web page, the MCP server over stdio and over HTTP, a login token, spaces, the save-what-you-learned loop, metrics and functions, evidence grades (proposed, accepted, checked, kept in `trust.yaml`; the gate, the API, the CLI and the exports are covered by tests, the page's grade buttons were checked by hand in a browser), local git history from day one (every change a commit, History with Undo, Share / Push / Pull to any git address), Apache Ossie import and export, OWL, SKOS and SHACL import and export with an IRI on everything, tests and CI. Install it with `npx lorefile`, the Docker image `ghcr.io/lorefile/lorefile`, `cargo install lorefile` or the install script ([Install](#install)); binaries for macOS, Linux and Windows are on the [releases page](https://github.com/lorefile/lorefile/releases). Not yet: accounts and single sign-on (one shared token for now), OAuth for Claude Desktop's and ChatGPT's connectors. A hosted version may come later. [BACKLOG.md](BACKLOG.md) has the honest list; [PRODUCTION.md](PRODUCTION.md) has what a team server still needs. This file is also a web page: `python3 scripts/build-tutorial.py` writes [docs/tutorial.html](docs/tutorial.html).

## The problem, in one Monday

**Before.** Monday morning a marketer asks Claude: "Give me last week's campaign performance." Claude has the Meta Ads MCP, the Google Ads MCP and the BigQuery MCP installed. It explores. It finds the Meta insights tool and guesses a date range. It finds the GA4 export in BigQuery and decides that `purchase` is the conversion that matters. It joins spend to orders by campaign name, roughly. Thirty tool calls and three clarifying questions later, there is a report. Tuesday a colleague asks the same question. Claude explores again, guesses differently, and the numbers do not match Monday's. Nobody can say which report is right, because the definition of "performance" lived in two chat windows and nowhere else.

**After.** Same Monday, but the Lorefile MCP is installed next to the data MCPs. Claude reads one short summary first: what the entities are, which number counts as success and where it comes from, which guardrails apply, which operations exist. It makes four tool calls instead of thirty. At the end it says: "I used three things worth remembering: the success metric, where spend comes from, and how campaigns are joined. Save them?" The marketer says yes. On Tuesday the colleague's question runs against the same files and gives the same number. Nothing was typed into a config, and nothing left the company's own tools: Claude fetched the rows with the marketer's own BigQuery and Meta logins, as it always did.

```
 BEFORE                                          AFTER
 ┌──────────────┐    explore, guess, join       ┌──────────────┐   summary → four calls
 │ Claude + the │ ───────────────────────────►  │ Claude + the │ ─────────────────────►  same number
 │ data MCPs    │   Mon ≠ Tue                   │ data MCPs    │   Mon = Tue              every day
 └──────────────┘                               └──────┬───────┘
                                                       │ "save what we settled?"
                                                ┌──────▼───────┐
                                                │ the ontology │  entities · links · guardrails · actions
                                                │ (YAML, git)  │  sources · words · metrics · functions
                                                └──────────────┘
```

## What it is, in one picture

A folder of small YAML files describes the business. One Rust binary compiles the folder, checks it, and opens three doors into it. The ontology holds what matters and where it lives; it never holds rows, never runs anything, and never holds a credential.

```
 your folder of YAML ───► lorefile ──────────► MCP server ───► Claude, ChatGPT*, Cursor, your own agents
   entities  links        compiles,          ───► web page   ───► people: build, review, accept, demo
   guardrails  actions    checks,            ───► CLI        ───► scripts and CI
   sources  words         serves
   metrics  functions
                                              agents fetch rows from YOUR data MCPs (BigQuery, Snowflake,
                                              Meta Ads, Shopify…) with their own logins, and run functions
                                              on YOUR tools (a model's MCP, an endpoint). The ontology
                                              only says what things mean, where they are, and what may be done.
 * ChatGPT's connectors reach a server from OpenAI's cloud, so they need a public HTTPS address
   and OAuth sign-in, which is not built yet. Claude Code connects over HTTP with the token today.
```

## The words

| Word on the page | In the files | What it is | Example |
|---|---|---|---|
| **Entity** | `type:` under `types/` | A noun of the business, with its fields in plain words and where its rows live | Order, Customer, Campaign |
| **Link** | `relationships:` inline or under `relationships/` | How two entities connect, with a sentence and a cardinality | an Order *is placed by* one Customer |
| **Guardrail** | `constraints/` | What must hold before an action, as a sentence first and an expression second, with a severity and an owner | only unshipped orders can be cancelled |
| **Action** | `actions/` | What an agent may do: inputs, what changes, which guardrails to check, who must approve | cancel_order |
| **Source** | `sources/` | Where rows live: a warehouse, files, or an MCP server. Never a credential | shop_bigquery, meta_ads |
| **Word** | `glossary.yaml` | What people say and which entity they mean | "basket" means Order |
| **Metric** | `logic/` | What counts, and the recipe for where the number comes from | success = paid orders in shop_bigquery.orders |
| **Function** | `logic/` | What an agent can run: inputs, outputs, and which tool does it | forecast_success → the measurement MCP |
| **Space** | a folder of these folders | One ontology per brand, business unit or client; one server holds many | brand-one, brand-two, @ecommerce |
| **Proposal** | `.lorefile/proposals/` | Something an agent learned and wants to save, waiting for a person to accept | "An agent learned: added metric success" |

## Install

Every way gives you the same `lorefile` command: one file, no runtime to install first.

| Way | Command | Needs |
|---|---|---|
| npx | `npx lorefile serve --dir spaces` | Node.js 18 or newer |
| Docker | `docker run -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN=… ghcr.io/lorefile/lorefile` | Docker |
| cargo | `cargo install lorefile --locked` | Rust 1.87 or newer |
| Install script | the `curl` line below | macOS or Linux |
| From this checkout | `cargo build --release`, then `./target/release/lorefile` | Rust 1.87 or newer |

**npx** downloads the binary built for your computer from the project's GitHub Releases on first use, checks its SHA-256 against the checksum published with the package, and keeps it in `~/.cache/lorefile/<version>/`. Nothing runs at install time. For Claude Code over stdio: `claude mcp add lorefile -- npx -y lorefile mcp --dir /path/to/spaces`.

**Docker** keeps your spaces on the `/spaces` volume and runs as a non-root user. Pick the token yourself with `LOREFILE_TOKEN`; the container starts read-only, and to let the page and agents write you pass the command:

```bash
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN=choose-a-long-secret \
  ghcr.io/lorefile/lorefile serve --dir /spaces --host 0.0.0.0 --allow-edit
```

**The install script**, for machines without Node, downloads the latest release, checks the checksum and puts `lorefile` in `~/.local/bin` (`LOREFILE_SLIM=1` for the slim build, `LOREFILE_VERSION=…` for another version):

```bash
curl -fsSL https://raw.githubusercontent.com/lorefile/lorefile/main/scripts/install.sh | sh
```

**Full or slim.** Each release has two builds per platform. The full one (about 70 MB) includes DuckDB, so the `query` tool and the Data & SQL page can run SQL over local CSV, Parquet and DuckDB files. The slim one (`-slim`, about 10 MB) leaves DuckDB out; everything else works the same, Ossie and RDF export and import included. `npx` uses the full build. From source: `cargo install lorefile --locked --no-default-features --features rdf` builds the slim one in a minute or two.

The rest of this page writes `lorefile`; from a checkout, read it as `./target/release/lorefile`.

## End to end, in ten steps

Everything below was run on a laptop.

### 1. Open the page

```bash
lorefile serve --dir ./spaces --port 7070 --allow-edit
```

The first run prints a login link and saves the token it made in `spaces/.lorefile/auth.toml` (readable by you only):

```
Lorefile 0.1.0 · 0 spaces of yours in ./spaces · edit mode on
Open    http://127.0.0.1:7070/?token=k3f9…   (first run: token saved to ./spaces/.lorefile/auth.toml)
MCP     http://127.0.0.1:7070/mcp   Authorization: Bearer k3f9…
        claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer k3f9…"
```

Open that link once; the browser keeps you signed in (Settings has Sign out). Without the token the page shows a box asking for it, and the API answers 401. This is the Jupyter way of doing it: one token per server, no accounts. To choose the token yourself, set `LOREFILE_TOKEN` or pass `--token`; that is how Docker and servers get theirs. On your own laptop, `--no-auth` turns the login off (it is refused on an address other machines can reach unless you add `--i-know`, for a proxy that already signs people in).

The `spaces` folder is created empty, and the page opens on "Your first space". The picker at the top already lists the examples and packs that ship inside the binary, read-only, so you can look at how things are done before you make anything.

```
┌────────────────────────────────────────────────────────────────────────────┐
│ [Pick a space… ▾]   Lorefile                               edit mode        │
├──────────────┬─────────────────────────────────────────────────────────────┤
│ A space is   │  Your first space                                            │
│ one ontology:│  A space is one ontology: one brand, one business unit, one  │
│ one brand…   │  client. It becomes a folder under spaces/.                  │
│              │                                                              │
│              │  Name        [ Brand One                       ]             │
│              │  Start from  (•) Blank    ( ) Starter                        │
│              │              ( ) Copy of ecommerce (shipped example)         │
│              │              ( ) Copy of meta-ads (shipped pack)             │
│              │                                            [ Create space ]  │
└──────────────┴─────────────────────────────────────────────────────────────┘
```

### 2. Make a space

Name it after the brand and start blank, from the starter, or as a copy of an example. A copy of `ecommerce` is the fastest way to see every page with real content. Each space is a folder under `spaces/`; `.lorefile/` inside it holds the server's own state and hides itself from git.

### 3. Look at an example to see how it is done

Pick `ecommerce` in the picker. It is read-only, which the banner says, and a nudge on the map offers to copy it into a space of your own. The rail on the left is the order of work: Sources, Entities, Links, Logic, Guardrails, Actions. Each step shows what is still missing, in words.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ [ecommerce · 12 entities ▾]  lumen-ecommerce   12 entities · 13 links ·      │
│                              25 guardrails · 5 actions   ▰▰▰▰▰▱▱▱ 55% ready  │
├────────────────────┬─────────────────────────────────────────────────────────┤
│ BUILD              │  shipped example · read-only · copy it to change it     │
│ ✓ Sources  6       │                                                         │
│ 2 Entities 12 · 12 │   ┌─────────┐      ┌─────────┐      ┌─────────┐         │
│   need plain words │   │Customer │──────│ Order   │──────│ Product │         │
│ 3 Links 13 · 10    │   │3 need   │placed│ 6 stop  │has   │3 need   │         │
│   without sentence │   │words    │by    │ rules   │line  │words    │         │
│ ✓ Logic 2 · 2      │   └─────────┘      └────┬────┘      └─────────┘         │
│ 5 Guardrails 25    │                         │ shipped as                     │
│ ✓ Actions 5        │                    ┌────▼────┐          WHAT TO DO NEXT  │
│ USE                │                    │Shipment │          12 entities have │
│ How to use it      │                    │4 stop   │          fields without   │
│ Map · Agents       │                    └─────────┘          plain words      │
│ Data & SQL         │                                         [Add words]      │
│ Files · Settings   │  ● Customers & orders ● Catalogue ● Fulfilment          │
└────────────────────┴─────────────────────────────────────────────────────────┘
```

### 4. Connect your AI tool

Claude Code, on the same machine (stdio; the agent starts its own Lorefile, no token needed):

```bash
claude mcp add lorefile -- lorefile mcp --dir ./spaces
```

Claude Code, against a server or a container (MCP over HTTP, with the server's token):

```bash
claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer <token>"
```

Settings › MCP for agents shows both commands with the token masked and a Copy button that copies the real one.

Claude Desktop, in `claude_desktop_config.json`, with absolute paths:

```json
{
  "mcpServers": {
    "lorefile": {
      "command": "/absolute/path/to/lorefile",
      "args": ["mcp", "--dir", "/absolute/path/to/spaces", "--allow-edit"]
    }
  }
}
```

One server holds every space. With one space of your own the tools use it; with several, every tool takes `space: "brand-one"` and the tool list names the choices. Leave out `--allow-edit` and every save from an agent waits for a person in the page instead of being written. A `.mcp.json` in this repository does the same for any Claude Code session opened here. Claude Desktop's custom connectors and ChatGPT's connect from their vendor's cloud and need public HTTPS with OAuth, which is not built yet: on a laptop, Desktop uses the stdio config above.

### 5. Teach it by using it

Ask your AI tool, in your own words:

> Give me last week's performance for Brand One. Spend is in the Meta Ads MCP; orders are in BigQuery, dataset `shop`.

The agent calls `summary` first, then works. When it settles something, it offers to save it. Say yes, and it calls `propose_patch` once, with everything it learned as one batch of ops, in the order they depend on each other:

```
propose_patch  reason: "First reporting conversation."  ops:
  - op: add_source   connection: shop_bigquery  kind: bigquery  project: acme  dataset: shop
  - op: add_type     name: Order  domain: Core  description: One checkout, paid or not.
  - op: set_source   type: Order  connection: shop_bigquery  table: orders  key: order_id
  - op: add_metric   metric: success  connection: shop_bigquery  table: orders
                     filter: payment_status = 'paid'  measure: count distinct order_id
                     grain: day  owner: growth@brand-one
  - op: add_term     term: basket  means: Order
```

Each op is planned against the space as it would be after the ones before it, so the entity can be bound to a source that is new in the same batch. The whole set is compiled together. In edit mode it is written at once, as one change. Otherwise it is queued as one proposal, and a blank space can be taught in a single sitting.

### 6. Accept what it learned

Open Agents. Every proposal reads as a sentence, with the file change one click away.

```
┌ Agents ─────────────────────────────────────────────────────────────────────┐
│ WAITING FOR YOUR REVIEW                                                      │
│ ┌──────────────────────────────────────────────────────────────────────────┐ │
│ │ An agent learned: added metric success (shop_bigquery.orders)   metric   │ │
│ │ The user said paid orders are the win.      agent via MCP · 11-26 09:14  │ │
│ │ Show what changes                                   [Accept] [Not now]   │ │
│ └──────────────────────────────────────────────────────────────────────────┘ │
│ ACTIVITY                                                                     │
│ ● proposed added metric success (waiting for review) · agent via MCP        │
│ ● created the space brand-one · person via UI                               │
└──────────────────────────────────────────────────────────────────────────────┘
```

Accept writes the file through the same checked path. This is what got written:

```yaml
# logic/metrics.yaml
metrics:
  - metric: success
    description: Paid orders. Not sessions, not platform-reported conversions.
    source:
      connection: shop_bigquery
      table: orders
      filter: payment_status = 'paid'
      measure: count distinct order_id
    grain: day
    owner: growth@brand-one
```

Accepting also records who stands behind the definition. Every metric, data binding, glossary word and guardrail carries a grade, kept in one file Lorefile writes, `trust.yaml`:

```
 ungraded   nobody has said: files from before grades, hand-written YAML, imports, copies
 proposed   an agent wrote it and no person has looked          agents call it "unverified"
 accepted   a person stands behind the words                    Accept, Looks right, or an edit in the page
 checked    a person ran it against real rows and it was right  who, when, which source, what was compared
```

A grade moves up only by a person's click and down by itself: when the definition changes, by hand, in a merge or by another tool, the grade stops counting until a person looks again (warning W113). When the server runs with `--allow-edit`, what an agent saves is written at once as **proposed** and listed under Agents › *Written by agents, not reviewed*, where **Looks right** makes it accepted. An agent's change to something a person accepted or checked is never written directly: it waits for a person, like any proposal. **Mark as checked…** on an accepted metric, binding or guardrail asks which source's rows you used and what you compared ("412 paid orders for 16–22 Nov, the same as Shopify's own report"); Lorefile records your word and runs nothing. An agent that did the comparing with the user can report it (`propose_patch` with `op: record_check`); that is always queued, and the card asks "Did you see this match yourself?". A space with no `trust.yaml` behaves exactly as before; **Vouch for all** on the Map grades everything nobody has graded yet in one change; what an agent proposed, and what changed since a person graded it, still need a look each. A reported check is tied to the definition the agent compared: if that definition changes before a person clicks, the click is refused. Settings › *Your name* puts your name on what you accept and check (`Priya via page`): up to 60 letters, digits, spaces and `. - ' _`, never "agent" or "via", and the page says so when a name cannot be used.

### 7. Ask again tomorrow

> Show me yesterday's performance.

The agent reads `summary`, sees the metric and its recipe, fetches the rows through the BigQuery MCP with the user's own login, and reports. Same files, same definition, same number. The How-to-use-it page lists the questions a space can answer now, with the tool call behind each and a Try-it button that runs it.

### 8. Put a guardrail in

Guardrails are what an agent checks before it acts. The builder writes the expression for you; the sentence comes first.

```
┌ Guardrails · new ───────────────────────────────────────────────────────────┐
│ About which entity   [ Order ▾ ]                                             │
│ Field  [ status ▾ ]  is  [ in ▾ ]  [ placed, paid, packing ]                 │
│ Expression   status in ["placed", "paid", "packing"]            (editable)   │
│ Severity     (•) stop  ( ) warn  ( ) note                                    │
│ Why          Shipped orders are returns, not cancellations.                  │
│ Who signs    support-lead@brand-one                              [ Save ]    │
└──────────────────────────────────────────────────────────────────────────────┘
```

Now an action that names it is checked against real rows:

```bash
lorefile check cancel_order order=ORD-1007 --dir spaces/brand-one
```

```
✗ cancel_order is blocked by OR-001.
 FAIL  OR-001   Only unshipped orders can be cancelled — violated for order=ORD-1007
         why: Shipped orders are returns, not cancellations.
approval: support-lead@brand-one
```

An agent gets the same answer from `validate`, with the reason and the approver, and tells the user "no" before anything is touched. A key that matches no row answers `unknown`, never "blocked".

### 9. Add the numbers and the operations

Logic is two lists the business agrees on once. **Metrics** are the numbers: what counts and where it comes from. **Functions** are the operations: what an agent can run, with what goes in, what comes out and which tool does it. Nothing runs inside the ontology.

```yaml
# logic/forecast.yaml
functions:
  - function: forecast_success
    description: Projects revenue for a weekly budget over N weeks, from the marketing mix model on the measurement MCP.
    inputs: { study_id: string, total_budget: number, weeks: integer }
    outputs: { forecast: json }
    runs: { kind: mcp, server: measurement, tool: mmm_run_budget_optimizer,
            args: { studyId: "{{study_id}}", totalBudget: "{{total_budget}}", timePeriodWeeks: "{{weeks}}" } }
    owner: growth@brand-one
    caveats: Asynchronous; poll with mmm_refresh_study. A model output, not a promise.
```

When someone says "run the forecast for a £40k week", the agent reads the contract, gathers the inputs, calls the measurement MCP itself, and reports with the confidence intervals the caveat asks for.

### 10. Keep the history and share it

History starts with the space. Creating one runs `git init` in its folder (the `git` already on your machine; nothing to set up, no GitHub account) and makes the first commit, "Start the space". From then on every accepted change is one commit: the summary in words is the message ("Described Order.basket", "An agent learned: 5 changes: …") and the author says who, `person via page` or `agent via MCP`, so an agent's change and a person's look different. When a person accepts an agent's proposal, the agent stays the author and the person is the committer.

The **History** page lists those changes, newest first. **Show** opens one line by line; **Undo** puts the files it touched back as they were just before it, through the same compiler check as a save, as a new commit "Undo: …" (so an undo can be undone). Later edits to the same file are kept; when they overlap the change being undone, nothing is written and the page says which file.

To share, paste any git address into **Settings › Share this space**: GitHub, GitLab, Bitbucket, Gitea, or a folder holding a bare repository on a shared drive. Create an empty repository there first. **Push** and **Pull** (fast-forward only) use your own git login, your SSH key or the password helper git already has; Lorefile never asks for or keeps a token, and when git cannot sign in the page says so in words instead of waiting. Branches and pull requests stay where your team already reviews: the inbox on the Agents page is the local pull request, and Accept is the merge.

A space inside some other project's repository (say `serve --dir ontology` inside your app's code) is left to that project: changes stay uncommitted for `git diff`, Settings lists them with a Commit box, and `--commit-edits` commits each one there. `--no-commit-edits` turns commits off everywhere. Without git on the machine everything else works and the page says history is off.

To share the live server instead: run it for the team with a token (`LOREFILE_TOKEN`) behind HTTPS (Caddy, or your platform's proxy), give everyone the HTTP MCP command, and let the spaces picker do the rest. [DEPLOY.md](DEPLOY.md) has Docker, Fly and a VM.

## The page, screen by screen

| Screen | What it is for |
|---|---|
| **Sources** | Where rows live. Warehouses, files, MCP servers. Which entities and metrics use each. |
| **Entities** | Each noun: fields with plain words, links, guardrails, data binding, the YAML. Empty boxes are highlighted; type into them. |
| **Links** | A builder for new links and a sentence box for every existing one. |
| **Logic** | Metrics and functions as cards: the recipe, the owner, the caveat, the file, the metric's `basis`, and its grade chip (grey ungraded, amber proposed, blue accepted, green checked, amber ✎ changed since graded) with Looks right or Mark as checked…. Data bindings, glossary words (on the entity, "People also say") and guardrails carry the same chip. |
| **Guardrails** | Every rule grouped by what it applies to, a builder that writes the expression, the expression one click away. |
| **Actions** | What agents may do, with a try-it form that runs the same check an agent runs. |
| **How to use it** | The questions this space can answer now, which tool answers each, and Try-it buttons. |
| **Map** | The whole shape, by domain, with what to do next on the right: definitions written by agents and not reviewed, and ones changed since a person graded them, come first; ungraded ones get a Vouch for all button. |
| **Agents** | Proposals waiting for review (an agent's reported check gets its own card: "Yes, mark as checked"), Written by agents, not reviewed (with Looks right), the activity log, connect commands, the tool list with a Call-it-from-here box. |
| **History** | Every change to the space, newest first: who (agent or person), when, what. Show gives the diff; Undo puts it back through the compiler check. |
| **Data & SQL** | One read-only SQL door across the ontology tables and any readable source, for engineers. |
| **Files** | Every YAML file with a note on what its folder is for; `trust.yaml` is shown to read, never to edit ("Who stands behind what. Written by Lorefile."). Where a space is read-only (a shipped example, or a server without `--allow-edit`), every Edit YAML button on the page becomes View YAML: the file opens to read, copy, or jump to here. |
| **Settings** | Your name (kept in the browser, shown on what you accept and check), history on or off and why, Share this space (any git address) with Push and Pull, what this server runs with, how agents connect (the stdio and HTTP commands, token masked, Copy), Sign out, and Export for other tools (Apache Ossie; OWL, SKOS and SHACL as Turtle or JSON-LD). |

## What an agent can ask

| The person says | The agent calls | What comes back |
|---|---|---|
| What is an Order, and what do we track about it? | `describe("Order")` | Fields in plain words, links, guardrails, where the rows live |
| What does "basket" mean here? | `describe("basket")` | The entity it means |
| What was success yesterday? | `describe("success")`, then the data MCP | The recipe, its `basis` and its grade, then the number from the real rows |
| How sure are you about that ROAS? | `describe("blended_roas")` | `trust.say`: "Unverified: an agent proposed this definition on 24 Nov and no person has reviewed it." |
| Shopify says 412 too; note that we checked it | `propose_patch(record_check)` | Queued; a person confirms it in the page and it becomes checked in their name |
| Where does Campaign data live? | `where_is("Campaign")` | Server, tool and params, or warehouse and table, and whether it is readable here |
| Can I cancel order ORD-1007? | `constraints_for`, `validate` | pass, warn, blocked or unknown, with reasons and the approver |
| Run the forecast for a £40k week | `describe("forecast_success")`, then the measurement MCP | The contract, then the forecast |
| Remember that "cart" means Order | `propose_patch(add_term)` | Written, or queued for a person |
| Which spaces do you have? | `spaces` | One line per brand with what it holds |

## Standards: Ossie, OWL, SKOS, SHACL

The YAML is Lorefile's own format, because agents write it, people read it and git diffs it. It also speaks the two families of standards the industry uses, both ways, so a company starts from what it already has and nothing is locked in. [docs/standards-explained.md](docs/standards-explained.md) explains all of this with one order, start to finish.

**Every thing has an IRI**, a standard identity written like a web address. Without you doing anything it is the space's namespace plus the name (`https://lorefile.dev/ns/lumen-ecommerce#Order`); set `namespace:` in `ontology.yaml` to use your own. To say "our Order is schema.org's Order", write `iri: schema:Order` on the entity; fields, links, glossary words, guardrails and actions take `iri:` too. Common prefixes (`schema`, `owl`, `skos`, `foaf` …) are built in, and `prefixes:` in `ontology.yaml` adds yours. The compiler resolves every IRI and refuses one IRI on two things (E016) and a prefix nobody declared (E017); fields of different entities may share one, which is how `schema:name` is reused. `describe` shows each IRI; the summary names the namespace.

**Apache Ossie** is the semantic-layer world: Snowflake, dbt, Cube, Atlan, DataHub, Collibra and NVIDIA's Auto Ontology read and write it.

```bash
lorefile export --ossie --dir spaces/brand-one --out ossie/
lorefile import --from-ossie sales.semantic_model.yaml --ontology sales.ontology.yaml --mapping sales.mapping.yaml --into spaces/sales --report
```

Entities with a table become datasets, fields become fields, metrics become SQL, links with key columns become joins, every entity becomes a concept carrying its IRI, and each link's sentence becomes a verbalization. Guardrails, actions, functions, sources, owners and anything else Ossie has no word for ride in a `lorefile` extension that Ossie readers must keep, so an export imported back is identical. Ossie's own validator accepts the exports of the examples; the packs' exports have no datasets, because their entities come from MCP tools, and the export says so.

**OWL, SKOS and SHACL** are the W3C's ontology world: Protégé, GraphDB, Stardog, TopBraid and LinkML read them, and so does any company's existing ontology file.

```bash
lorefile export --rdf --dir spaces/brand-one --out brand-one.ttl          # or --format jsonld | ntriples | rdfxml
lorefile import --from-rdf acme-business.owl --into spaces/acme --report
lorefile import --from-rdf schemaorg.ttl --only schema:Order,schema:Product --depth 1 --into spaces/shop
```

| You can export and import | As |
|---|---|
| The space | an `owl:Ontology`, with its namespace and prefixes |
| Entities | OWL classes with a label and a comment; glossary words as `skos:altLabel` |
| Fields | OWL datatype properties with an XSD type |
| Links | OWL object properties with the sentence as their comment, functional when many-to-one |
| How each entity uses its fields and links | one SHACL shape per entity: name, order, datatype, required (`sh:minCount 1`), allowed values (`sh:in`), at most one |
| Guardrails | SHACL shapes with severity and message. Simple rules (allowed values, required, a number range) are real SHACL checks; every rule also travels as its CEL text in `lorefile:cel`. A guardrail an action checks is exported switched off (`sh:deactivated`), naming the action, so a SHACL tool does not flag every delivered order for a cancel rule |
| Actions, metrics, functions, sources, owners, data bindings | a small `lorefile:` vocabulary in the same graph: other tools store it, Lorefile reads it back |
| Grades (`trust.yaml`) | `lorefile:trust lorefile:Checked` (or `Accepted`, `Proposed`) on the graded thing, `lorefile:bindingTrust` on an entity for its data binding, and a record that is also a PROV-O `prov:Activity`: `prov:wasAssociatedWith` the person or agent, `prov:endedAtTime`, `prov:used` the thing and the source it was checked against, the note as `rdfs:comment`. In Ossie the whole of `trust.yaml` rides in the `lorefile` extension, and a proposed metric or dataset also says "Unverified: …" in `ai_context.instructions`, where other tools' AI reads it |

Coming in from another tool's file, classes become entities, datatype properties fields, object properties links, `owl:hasKey` the key, `skos:altLabel` glossary words, and SHACL property shapes guardrails (`sh:minCount`, `sh:in`, ranges, lengths and patterns become CEL). A class hierarchy (`rdfs:subClassOf`) becomes a tag, because Lorefile has no sub-kinds yet. Statements that exist only for reasoning engines are counted and dropped, and SHACL that has no CEL form (SPARQL, `sh:or` …) is listed for a person. `--report` says what was guessed, dropped or kept. Like every import, nothing is written unless the result compiles, and no existing file is overwritten. Lorefile has no reasoner and no triple store: the Turtle is just a file.

An import starts with every definition ungraded, whatever grades the file claims: a grade is a statement by this team about this copy. The report says how many arrived with a grade, and how many claims it could not read. An object in the file that asks to be written as `trust.yaml` is never written there. `--keep-grades` brings back the grades of a team's own export where each still matches its definition, and lists the ones dropped.

## Reference

## The file format

```
ontology/
├── ontology.yaml          # name, version, domains; optional namespace and prefixes for IRIs
├── glossary.yaml          # "vendor" means Supplier
├── sources/*.yaml         # connections: csv, parquet, duckdb, postgres, s3 read here; snowflake/… declared; mcp points at another MCP server
├── types/*.yaml           # one type per file, or a `types:` list
├── relationships/*.yaml   # relationships with their own properties / edge tables
├── constraints/**/*.yaml  # rules, grouped however you like
├── actions/*.yaml         # what agents may do, and which rules are checked
├── trust.yaml             # who stands behind each definition; written by Lorefile only
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

### IRIs

Optional everywhere. `ontology.yaml` may set the base every derived IRI starts with, and short names for other vocabularies:

```yaml
name: acme-supply-chain
namespace: https://ontology.acme.example/supply#     # ends in # or /; default https://lorefile.dev/ns/<name>#
prefixes:
  fibo: https://spec.edmcouncil.org/fibo/ontology/   # schema, owl, rdfs, xsd, skos, sh, dcterms, foaf are built in
```

An entity, field, link, glossary term, guardrail or action may then name its own identity, absolute or `prefix:local`:

```yaml
type: Supplier
iri: schema:Organization
properties:
  name: { type: string, iri: schema:legalName }
```

Without `iri:`, an entity is `<namespace>Supplier`, a field `<namespace>Supplier.name`, a link `<namespace>Supplier.located_in`, a guardrail `<namespace>SC-041`, an action `<namespace>switch_supplier` and a glossary term `<namespace>term.vendor`.

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

### A metric

```yaml
# logic/measurement.yaml
metrics:
  - metric: success                 # the conventional name for the one number the business is run on
    description: Paid orders. Not sessions, not the conversions the ad platforms report.
    source:
      connection: shop_bigquery     # a source under sources/ …
      table: orders                 # … and a table, or `tool:` + `params:` for an MCP source
      filter: status in ('paid', 'fulfilled')
      measure: count distinct order_id
    grain: day
    basis: transactional            # transactional | attributed | modelled | experimental (optional)
    owner: growth@lumenhome
    caveats: Platform conversions are attributed, not the same thing; compare, never add.
  - metric: paid_performance
    variants:                       # one definition per condition
      - when: channel == "meta"
        source: { connection: meta_ads, tool: get_insights, params: { level: campaign } }
```

A metric is a named number with its definition, where it comes from and who owns it, so every agent computes it the same way tomorrow. A **function** in the same folder is an operation an agent can run: typed inputs and outputs plus `runs: {kind: mcp, server, tool, args}` (or `http` with a `url`, or `manual`). The ontology never runs it; the agent reads the contract and calls the tool with its own login. `examples/ecommerce/logic/forecast.yaml` points `forecast_success` at the budget optimiser of the marketing-measurement MCP the example declares as a source. `success` is the same name in every space: a store counts purchases, a SaaS product counts trials, a B2B company counts qualified opportunities. `summary` lists metrics, `describe` gives the full recipe, and the Logic step of the page shows them. Derived fields and functions follow in the same folder.

`basis:` says what kind of number a metric is: `transactional` (counted from the business's own records), `attributed` (credit a platform or rule assigns to a channel), `modelled` (an estimate from a model) or `experimental` (read from a controlled test). A variant may set its own, which wins. It is optional and nothing warns when it is absent; agents are told never to add numbers of different basis.

### Who stands behind what: `trust.yaml`

```yaml
# trust.yaml · who stands behind what in this space.
trust:
  metrics:
    success: { grade: checked, by: "Priya via page", at: "2026-11-28", of: "3f9a1c2e70b4", against: "shop_bigquery", note: "412 paid orders for 16–22 Nov, the same as Shopify's own report" }
    blended_roas: { grade: proposed, by: "agent via MCP", at: "2026-11-24", of: "9d02b7c15a11" }
  bindings:
    Order: { grade: accepted, by: "person via page", at: "2026-11-26", of: "77b0e1aa0c3d" }
```

One line per graded thing, under `metrics`, `bindings` (by entity name), `terms` and `guardrails`. `of` is a fingerprint of the definition as compiled (comments, spacing and key order do not count; a binding's covers the entity's name, key and `source:` block). A metric's and a binding's also cover the definition of each source they read, without its description, so pointing `shop_bigquery` at another project or dataset stops the grade of everything that reads it; a word written twice is graded as one thing, every meaning counted. A definition edited behind the grade's back stops counting. The save gate writes it in the same commit as the facts: an agent's save is `proposed`, a person's edit or Accept is `accepted`, a person changing only the words of a checked metric (description, caveats, owner, tags) keeps it checked, and changing the recipe makes it accepted again. Agents cannot write the file (`propose_change` refuses it, edits on or off), and a `grade:` key typed into a fact file is an unknown key. Undo puts the file back with the facts. A copy of a space leaves it behind; imports start ungraded unless `--keep-grades`. `summary` gains one Trust line and marks proposed (UNVERIFIED) and checked lines; `describe` returns `trust` with a ready sentence (`say`); `where_is` returns the binding's `trust`; `validate` keeps its verdicts and adds an `unverified` list, and `grade` on each rule result once the space has a `trust.yaml`. It grades definitions, never answers; it is as tamper-proof as the repository's review.

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

DuckDB is **fenced**, always: once a space's views are built, the catalog may touch only the folders those views read from, external access is switched off and the configuration is locked, so a `query` cannot read other files on the machine, install extensions or reach the network. A space whose fence cannot be set gets no SQL door at all. That is why the door reads only local files (csv, parquet, `.duckdb`): Postgres, S3 and HTTP sources are declared so `where_is` can point at them, and agents read them with their own connectors.

## The web page

`lorefile serve --dir ./ontology` opens a guided workspace at http://127.0.0.1:7070. It is organised around putting the ontology in place, not around the data model:

- **Build**, five steps with live status: **Sources** (where the rows live), **Entities** (each type with a plain-English description, its fields, links, guardrails, data and YAML), **Links** (a relationship builder and one sentence per link), **Logic** (metrics: what counts and where the number comes from; functions: what can be run and which tool does it), **Guardrails** (rules grouped by what they apply to, a builder that writes the expression for you, and the expression itself one click away), **Actions** (what agents may do, with a try-it form that runs the same `validate` an agent runs).
- **Use**: **Map** (the whole shape, with what to do next computed from the gaps), **Agents** (the MCP tools, connect commands, an inbox of changes agents proposed, and an activity log), **Data & SQL** (DuckDB over the ontology and every readable source).
- One vocabulary, three places: an **entity** on the page is a `type:` block in YAML and a type to the MCP tools (`describe`, `neighbors`). The page uses the plain word; the files and the tools keep the short one.
- **Use**: **How to use it** (the questions an agent can now answer about this space, which tool answers each, and a Try-it button that runs it), **Map**, **Agents**, **History** (every change, Show and Undo), **Data & SQL**, **Files** (every YAML file, with a note on what each folder is for) and **Settings** (history and sharing: Share to any git address, Push, Pull; what this server runs with; how agents connect over stdio or HTTP; Sign out; Export for other tools: Apache Ossie, and OWL, SKOS and SHACL as Turtle or JSON-LD).
- A readiness meter in the header rises as fields get words, links get sentences and entities get tables.

### Editing from the page

Start the server with `--allow-edit` and every description box, link sentence, builder and "Edit YAML" button writes a file. Each change is a small operation (`POST /api/patch`: describe a field, describe a link, add a link, add a guardrail, create a thing, bind a table) that becomes the full new text of one YAML file and goes through the same compiler check as any other write: errors refuse the save and are shown, warnings are allowed. Guardrails are appended as text, so their file keeps its comments; type files are rewritten through the YAML library, which keeps key order but drops comments in that one file. Each accepted change is committed when the space's history is its own (its repository is the space folder or the folder of spaces, which is how a new space starts); a space nested inside another project's repository stays uncommitted for `git diff` unless the server runs with `--commit-edits`. `--no-commit-edits` never commits.

Without `--allow-edit` the page is read-only and says so. The login is one shared token, not accounts: on anything reachable from the internet, put HTTPS in front and think about who holds the token before turning edits on ([PRODUCTION.md](PRODUCTION.md)).

### Proposals from agents

An agent calls `propose_patch` with one small fact (`add_source`, `add_type`, `set_source`, `add_metric`, `add_function`, `add_term`, `set_property_description`, `add_relationship`, `add_constraint` and the other page operations, or `record_check` to report a check, which is always queued), or with `ops: [...]` for several that depend on each other, planned in order and queued as one proposal with every file in it; or `propose_change` with the full new text of one file, plus a reason. The server's instructions tell agents to offer this the moment the user settles a definition during their work: which event counts as success and where it comes from, which table or tool holds an entity's rows, what a word means. That is the loop: the first report writes the definition down, and the next day's question runs the same way for everyone. When the server allows edits and the change is clean, it is written and logged, and graded proposed; a change to something a person accepted or checked is queued instead. Otherwise it is queued under `<ontology>/.lorefile/proposals/` and shows up on the Agents page with a diff, where a person accepts or sets it aside; accepting writes it through the same checked path. Activity (who changed or proposed what, when) is kept in `<ontology>/.lorefile/activity.jsonl`. The `.lorefile/` folder is local state and is ignored by git.

### Spaces: one server, many ontologies

Point `serve` or `mcp` at a folder that has no `ontology.yaml` of its own and every sub-folder with one becomes a **space**: one ontology per brand, business unit or client. The page gets a picker at the top; each space has its own URL (`/s/brand-two/`), API (`/s/brand-two/api/…`), DuckDB catalog, proposals inbox and activity log. With `--allow-edit`, "＋ New space…" creates one as blank, from the starter, or as a copy of any other space.

```bash
lorefile serve --dir ./spaces --allow-edit     # spaces/brand-one, spaces/brand-two, …
lorefile validate --dir ./spaces               # checks every space (for CI)
lorefile describe Campaign --dir ./spaces --space brand-two
```

The examples and packs ship inside the binary and are listed as read-only spaces whose ids start with `@` (`@ecommerce`, `@supply-chain-parquet`, `@meta-ads`), so you can open one to see how it is done and copy it. They are written out under `<dir>/.lorefile/examples/` at start (that folder hides itself from git); `--no-examples` leaves them out.

Over MCP, agents name the space in each call. With exactly one space of yours it is implied; with several, every tool takes `space:` (the tool list names them) and a `spaces` tool describes each one. `mcp --space brand-two` dedicates a server to one space. Resource URIs carry the space too: `ontology://brand-two/summary`.

## CLI

```
lorefile validate              compile and report errors / warnings (exit 1 on errors)
lorefile summary               the Markdown agents load into context
lorefile describe <name>       type / relationship / action / rule / source
lorefile search <words>
lorefile neighbors <type> --depth 2
lorefile path <from> <to>
lorefile rules <name>
lorefile where <type>
lorefile instances <type|relationship>
lorefile check <action> k=v …  exit 2 when blocked
lorefile query "<sql>"         one read-only SELECT/WITH via DuckDB (--limit, max 500; `-` reads stdin)
lorefile tables                what SQL can see: ontology tables, data views, bindings not attached
lorefile mcp [--allow-edit]    MCP server on stdio; --allow-edit lets propose_patch and propose_change write
lorefile serve --port 7070 [--allow-edit] [--commit-edits | --no-commit-edits] [--token T | --no-auth]
                               web page, JSON API and MCP over HTTP at /mcp, behind the login token
lorefile init [path]           starter ontology, with its own git history when it is not inside a repository
lorefile trust                 every metric, data binding, word and guardrail, grouped by grade
lorefile trust accept <name> [--all] [--by NAME]                   a person stands behind it (--all: every ungraded one)
lorefile trust check <name> --against <source> --note "…" [--by NAME]   you ran it against real rows and it was right
lorefile export --ossie [--out <folder>|<file>.json]          Apache Ossie: semantic model, ontology, mapping
lorefile export --rdf [--format turtle|jsonld|ntriples|rdfxml] [--out <file>]   OWL, SKOS and SHACL
lorefile import --from-ossie <semantic model> [--ontology F] [--mapping F] --into <folder> [--report] [--keep-grades]
lorefile import --from-rdf <file> [--into <folder>] [--only A,B] [--depth N] [--report] [--keep-grades]
```

`--dir` picks the folder (defaults to `./ontology`, then `.`): an ontology folder, or a folder of spaces. `--space <id>` picks a space when there are several (`@…` for a shipped example); `--no-examples` leaves the shipped ones out. `--json` switches any command to machine output.

## Tests

`cargo test` runs 95 tests in these layers: unit tests inside the modules (paths that cannot escape the folder, the save gate, the DuckDB fence, spaces, patch operations, history, IRI resolution, CEL to SHACL, and grades: proposed, accepted and checked through the gate, a hand edit giving W113, Undo taking a grade back, two saves at once both landing their grade), `tests/mcp_loop.rs` (the real binary over stdio: an agent lists tools, reads the summary, saves a metric with `propose_patch`, and the save is queued in words, or written and committed when edits are allowed), `tests/mcp_http.rs` (the same over HTTP with the token, and 401 without it), `tests/http.rs` (the server: the login, write headers, a stale proposal, read-only examples, and History, Undo, Share, Push and Pull against a bare repository, no network), `tests/cli.rs` (every example and pack compiles with no errors and no warnings, and the documented commands answer), `tests/ossie.rs` (every example and pack through Ossie and back, unchanged; Ossie's own example files import) and `tests/rdf.rs` (every example and pack through Turtle and back, unchanged; JSON-LD, N-Triples and RDF/XML carry the same graph; a slice of schema.org imports; a SHACL shape becomes a guardrail that `check` enforces; one IRI on two things is an error), and for grades: an agent can neither write `trust.yaml` nor overwrite an accepted binding, a reported check waits for a person's click, and grades travel in Turtle (as PROV-O) and Ossie and come back only with `--keep-grades`. `.github/workflows/ci.yml` runs the same on every push, then `cargo clippy -- -D warnings`. A `.mcp.json` in the repository gives any Claude Code session opened here the ontology MCP server, pointed at `spaces/`.

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
| E010 | A domain in `ontology.yaml` lists an unknown type |
| E011 | `kind: mcp` source has no `server` |
| E012 | Type or metric binding has no `table` (or no `tool`, for an `mcp` source) |
| E013 | Metric says nothing about where it comes from (no `source`, `variants` or `sql`) |
| E014 | Function `runs` is incomplete, or names an MCP server no `kind: mcp` source declares |
| E015 | One name, two kinds (a metric or function named like an entity or action) |
| E016 | One IRI names two things (fields of different entities may share one) |
| E017 | An IRI, namespace or prefix that does not resolve (an undeclared prefix, a namespace not ending in `#` or `/`) |
| E018 | A grade in `trust.yaml` that makes no sense: `checked` without `against`, `by` or `at`, `against` that is no source, a word marked checked, an empty `of` |
| W101 | Missing description |
| W109 | Metric has no owner |
| W111 | Function does not say where it runs |
| W102 | Blocking rule without a rationale |
| W103 | Type bound to data but no `key: true` property |
| W104 | Rule references a name not available on its `applies_to` |
| W105 | An enum with no `values` |
| W106 | CSV / Parquet / DuckDB file not found |
| W107 | An action's effect runs a link the wrong way round for its inputs |
| W108 | A glossary term means something that does not exist |
| W112 | Two things get the same derived IRI; set `iri:` on one |
| W113 | A graded definition changed since it was graded; a person has to accept it again |
| W114 | `trust.yaml` grades something that no longer exists |

## Packs

An ontology pack is a ready-made folder for one data source an agent already has an MCP server for. The data MCP returns rows; the pack says what they mean, which rules apply, and what the rows cannot tell you. Load both in the same agent. Each pack has a `README.md` with the Claude Desktop config and three worked questions, a `sources/<server>.yaml` of `kind: mcp`, and validates with zero errors and zero warnings.

| Pack | Types | Rules | Actions | What it fixes |
|---|---|---|---|---|
| [`packs/meta-ads`](packs/meta-ads) | Account, Campaign, AdSet, Ad, Creative, Audience, Placement, Insight, Conversion, AttributionWindow, LiftStudy, BudgetChange | 16 | pause_campaign, change_budget, create_lookalike, read_performance, compare_conversions | Conversions are attributed counts under one window (7-day click, 1-day view by default), booked on the click day, modelled and late on iOS; awareness campaigns are not judged on CPA; CBO ad-set budgets are advisory; geo breakdowns stop at region/DMA and are not incrementality; test cells are frozen |
| [`packs/google-ads`](packs/google-ads) | Account, Campaign, AdGroup, Keyword, Ad, ConversionAction, BiddingStrategy, Audience, GeoTarget, Readout, BudgetChange, TargetChange | 16 | pause_campaign, change_budget, change_bid_target, read_performance, compare_readouts, audit_conversion_action | `conversions` counts primary actions under each action's model (data-driven by default), by click date; counting rules per category; CPM/CPV strategies are not judged on CPA; geo segments are matched location, not presence; shared budgets and experiment arms |
| [`packs/shopify`](packs/shopify) | Shop, Order, LineItem, Product, Variant, Customer, Fulfillment, Refund, DiscountCode | 15 | cancel_order, refund_order, create_discount_code, attribute_order | Test and cancelled orders are not revenue; `processed_at` and the shop currency are the reporting basis; financial versus fulfillment status; `source_name` is the sales channel, not attribution; codes are proof of use |

```bash
lorefile validate --dir packs/meta-ads
```

```bash
lorefile mcp --dir packs/meta-ads
```

The three read-out guards (`read_performance`, `compare_conversions`, `attribute_order` and their siblings) are actions with no side effects: an agent calls `validate` on them before quoting a number, and gets the same block / warn / info verdict it would get before a write. Where a platform detail was uncertain the YAML says `# TODO verify` rather than guessing. Tool names in the bindings follow the common open-source MCP servers for each platform; change `tool:` in a type file if yours differ.

## Roadmap

1. **Core** (done) — format, compiler, graph, CLI, MCP, UI, CSV data door.
2. **Doors** (this) — DuckDB inside the binary: Parquet, `.duckdb`, Postgres, S3/HTTP; `query(sql)` and the ontology as SQL tables; SQL box in the UI. Editing from the page and `propose_change` for agents, both compiler-checked. Still open: MySQL, `init --from <db>` to draft types from an existing schema, opening a PR from an edit.
3. **Any MCP is a source** — designed in [ARCHITECTURE.md §17](ARCHITECTURE.md#17-phase-3-design-any-mcp-is-a-source-the-ontology-keeps-what-matters). Step 1 is built: `propose_patch`, metrics in `logic/`, the Logic step, the offer-to-save instruction. Still to come: derived fields in `logic/`; `does` on actions and `mutable: false` on entities, checked by `validate`; a table browser and `init --from` for warehouses. A hosted version that keeps the one copy with SSO, roles and audit may come later ([BACKLOG.md](BACKLOG.md), "Hosted version, later"). The ontology never executes and never holds credentials.
4. **Ecosystem** — Apache Ossie and OWL / SKOS / SHACL import and export are built. Next: more packs (TikTok Ads, GA4, Klaviyo), Python/TS SDKs, sub-kinds (`extends`) so imported class hierarchies stop being tags, LinkML, dbt/LookML import.

## License

Apache-2.0, see [LICENSE](LICENSE). Use it, change it, host it, sell what you build on it; keep the notice ([NOTICE](NOTICE)). Why Apache-2.0 and not MIT, AGPL or a source-available licence is in [PRODUCTION.md](PRODUCTION.md#6-licence).

Built by the team at [Stella](https://stellaheystella.com).
