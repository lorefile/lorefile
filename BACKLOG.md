# Keep, freeze, bin, backlog

A plain list of what is in the repository, what it is for, and what to do with it. Written 2026-11-25 after the question "what are we overdoing?". The test of each line is the product sentence: *the ontology keeps what matters and where it lives, so any agent with the user's own MCPs can act on it the same way tomorrow.*

## Keep (this is the product)

| Piece | Why it stays |
|---|---|
| The YAML format: entities, links, guardrails, actions, sources, glossary, metrics, functions | The whole value. Everything else is a view of it. |
| The compiler and `validate` codes | What makes agent-written files safe to accept. |
| Spaces, with the shipped examples and packs in the picker | One server per team, one space per brand; examples show how it is done. |
| The page: Sources, Entities, Links, Logic, Guardrails, Actions; Map; Agents; How to use it; Files; Settings | The review surface. People accept, read and demo; they do not type YAML. |
| MCP over stdio and over HTTP with `summary`, `describe`, `where_is`, `constraints_for`, `validate`, `search`, `neighbors` | How agents read it. |
| The login token (`--token`, `LOREFILE_TOKEN`, `auth.toml`) | What a container or a team server needs before anyone else can reach it. |
| Apache Ossie and OWL / SKOS / SHACL import and export, an IRI on everything | Teams start from what they have; nothing is locked in. |
| `propose_patch` and the offer-to-save instruction | The loop: the first report writes the definition down. |
| Proposals inbox, activity log, git commit from Settings | Who learned what, who accepted it, when. |
| Packs for Meta Ads, Google Ads, Shopify | The measurement meaning behind the data MCPs people already have. |
| Tests: unit, MCP end to end, CLI over every example; CI | So all of the above keeps working. |

## Freeze (works; stop investing)

| Piece | Why it is frozen |
|---|---|
| DuckDB inside the binary (CSV, Parquet, `.duckdb`, Postgres, S3) and the `query` tool | Useful for demos and for `validate` against real rows in the examples. It contradicts the principle that agents fetch rows through their own MCPs, it is the slowest part of the build, and it is the only part with a security surface (now fenced). Keep it as the optional feature it already is; add no connectors. |
| Data & SQL page | Stays for engineers; not promoted to marketers. |
| `propose_change` (full-file text) | Needed by Claude Code and power users; `propose_patch` is the path for chat agents. |
| Edge sources on relationships, inline `instances:` on entities | Demo conveniences. Fine as they are. |
| Action `effects: create / remove` | Correct but rarely used; `validate` honours it. |
| `--commit-edits` (one commit per change) | Keep for the hosted path; the Settings commit button covers local use. |

## Bin

| Piece | Why |
|---|---|
| The server executing actions with a credential vault (first Phase 3 draft) | Withdrawn by design: the ontology never runs anything and never holds credentials. |
| Dark theme | Removed. |
| "Things" as the word for entities | Renamed. |
| Separate example folders in the picker's place | Replaced by shipped read-only spaces. |

## From the MCP trial (2026-11-25, `NOTES-mcp-trial.md`)

Fixed the same day: the first glossary word in a blank space produced broken YAML; `validate` said "blocked" when the record did not exist (now "unknown", exit 3); no `add_source` op; misses in the implied space said nothing about other spaces; `summary`, `describe` and `search` descriptions forgot metrics and functions; `propose_change` proposals had no sentence; sources did not count metrics and functions as users; the e-commerce example's `success` filtered on a status that does not exist, named no date column or time zone, and its forecast function did not match the real tool on the measurement MCP; a function shared its name with an action.

Still open, in the order they hurt:

1. ~~**One sitting can finish teaching.**~~ Built 2026-11-26: `propose_patch` with `ops: [...]`.
2. **Accepting a stale proposal overwrites newer ones.** Two proposals built from the same file text: accepting the second drops the first. Fix: refuse or rebase an accept when the file changed since the proposal's `before`, and re-check a proposal's `ok` when it is listed.
3. **The `space` list in tool schemas is fixed at start.** A space created in the page cannot be named until the MCP restarts. Fix: `listChanged` notifications, or validate `space` against a fresh scan.
4. **No MCP way to see a space's own errors.** `validate` means "check an action"; the compile errors are only in the page and the CLI. Fix: a `health` tool, or errors in `summary`.
5. **`propose_change` has no way to read a file first.** Add a `read_file` tool, or make `describe` return the YAML for a thing.
6. **`propose_patch` describes its fields in prose with `additionalProperties`.** Strict-schema clients may drop fields. Fix: a schema per op (oneOf).
7. **Claude Desktop needs absolute paths** and the same flags as the Claude Code line; the connect card now says so, a generated config file would be better.

## From the live ChatGPT test (2026-11-27)

ChatGPT on the web connected over OAuth and called tools. Then "@Lorefile save this file" on a promo-sales CSV gave a proposal that could never read a row.

1. ~~**`add_type` makes only a generated key** (`<name>_id`), and no op adds a field, so none of the file's 14 columns became fields.~~ Built 2026-11-27: `add_type` takes `properties` and `key`; a new `add_property` op adds a field, with `column` when the header differs; `set_source` takes `columns`.
2. ~~**"Save this file" has no recipe**: ChatGPT looked for an upload tool, split a flat export into six entities, made up keys and never asked which number counts.~~ Built 2026-11-27: `match_file` and `learn_file` (`specs/2026-11-27-learn-file.md`).
3. ~~**ChatGPT asks to confirm every read**~~, ~~**`/.well-known/openid-configuration` answers 401**~~, ~~**`offline_access` not listed**~~. Fixed 2026-11-27 (whether ChatGPT then stops asking is still to check live).
4. Item 6 above (strict schemas) is done for `match_file` and `learn_file`; `propose_patch` still describes its ops in prose.


## From the three reviews (2026-11-26)

Three independent reviewers on the Fable model read the working tree, built it, ran the tests and probed a scratch server: a chief engineer, a chief ontologist and a chief security officer. Their full reports are in this session's history; this is the triage.

**Fixed the same day**

| Finding | Fix |
|---|---|
| The DuckDB fence switched off for any space with a Postgres or S3 source, even a broken one, and `query` then read any file on the machine (security S1, engineer M2) | The SQL door reads local files only; the fence is set for every space and a space whose fence cannot be set gets no door. Test. |
| Every read sent `Access-Control-Allow-Origin: *`, and SQL was accepted over GET, so any website in the user's browser could read the API (S2, M1) | Header removed; SQL is POST-only. Test. |
| The write check compared `Origin` with the bind address, so it broke behind a proxy and passed when `Origin` was absent (S5, H5) | Writes need the page's `X-Lorefile: write` header and an `Origin` matching the `Host`. Test. |
| One stalled client blocked the whole server; bodies were unbounded (S3, M6) | A thread per request; bodies over 4 MB refused. Test. |
| A misspelled key such as `severty` compiled clean and silently changed meaning (ontologist F1) | Unknown keys are errors everywhere except source config; the message names the key. Test. |
| Proposal ids collided across processes and overwrote each other (H1) | Files are created, never overwritten; a taken id gets a suffix. Test. |
| Accepting a stale proposal dropped newer accepted changes (H2) | Accept refuses with 409 when the file changed since the proposal. Test. |
| A rule that errored (a missing field) was reported as "blocked" (H4) | Errored rules make the verdict `unknown` with the fields to fetch. Test. |
| `implies` was rewritten inside string literals (M5) | Quote-aware. Test. |
| Names were not escaped for `'` in the page and not validated on save: script injection through an entity name (M3) | `'` escaped; names are plain in every save operation. |
| The MCP process never saw the page's writes; the page never saw git's (H3) | Each space carries a fingerprint of its YAML files and reloads on change; tool lists rescan spaces. |
| A file with a YAML error vanished from Files (M9); `validate --json` failed on a folder of spaces (M13); read commands created folders and extracted examples (M12); a name used by two kinds (M8); summary did not count metrics as source users (L4); CI tested neither the Rust floor nor the CSV-only build (L2) | All done. |
| Docs claimed credentials "never" in files and the fence as absolute | Reworded to what the code enforces. |

**Open, in the order they matter**

1. **Metric semantics** (ontologist F2, F3, F7): an entity anchor (`over:`), a structured measure, a CEL filter checked against the entity's fields, `time: {column, zone}`, `dimensions`, `attribution`, ratio metrics, money and percent property kinds. (`basis` is done: built with evidence grades on 2026-11-26.) Then move ROAS, CPA, GMV and AOV from the packs' glossaries into `logic/`, each with a `basis`. This is the change that lets the compiler catch a wrong status filter and lets a marketing team keep its definitions in fields instead of prose.
2. **Rules that see time, hops and aggregates** (F6, F9): timestamp coercion, `now()`, one-hop traversal, metric references in rules; `kind: reading` with a `message:` for notes that are not guardrails.
3. **Links with identity** (F5): `via:` for the foreign key, `inverse:`, `join:`, composite keys, cardinality as an enum.
4. **Actions finished** (F4, F8): scalar inputs, `approvals:` as evaluated conditions with roles, `does:` to a function, `mutable: false` on entities.
5. ~~**Teaching in one sitting**~~ Built 2026-11-26: `propose_patch` takes `ops: [...]`, each planned against the space as it would be after the previous ones, compiled together, written or queued as one proposal with every file, accepted as one. Still open from the same finding: proposals re-checked when listed; one kind per file is now an error, not a silent drop.
6. **Type-file edits keep the file's shape** (M4): targeted text edits for descriptions and bindings instead of a serde round-trip.
7. **Pack imports and shared enums** (F10); one placeholder syntax with declared params (F11); roles (F12); `entity:` in YAML with `type:` as an alias (F13); richer summary (F14); the missing W-codes (F15).
8. **Performance** (M7): row caches in the loaded space, DuckDB reused when sources are unchanged.
9. **Supply chain** (S7): SHA-pinned actions, `cargo audit`, SBOM, signed releases, a plan off the deprecated `serde_yaml`.
10. **Secrets** (S4, S6): enforce `${ENV}` shapes on credential-bearing fields; keep `.lorefile/` out of syncs; identity on every audit line.
11. **CEL budget** (S8): a wall-clock limit per evaluation; a size cap on inline records.
12. ~~**MCP over HTTP with a login token**~~ Built 2026-11-26 (Part B of the release spec): `POST /mcp` behind the same token as the page. One container per customer belongs to the hosted version, below.

**Readiness, after the fixes.** A laptop: yes. A team on one server with the token and HTTPS in front: yes for reading, teaching and reviewing, one replica, and the open items above as known limits; Claude Code on other machines connects over HTTP with the token, and ChatGPT's and Claude's connectors sign in with OAuth (built 2026-11-27, not yet tried with real accounts). Hosting for customers: no, until items 9 to 11 and accounts.

## Backlog (in order)

1. **`does` on actions, `mutable: false` on entities, `may_be_called_by` honoured by `validate`** (small). The "order table never gets updated" rule as one line.
2. **Derived fields in `logic/`** (medium). `Customer.is_churned` as CEL or SQL, visible in `describe` and `query`.
3. ~~**MCP over HTTP with a login token**~~ Built 2026-11-26.
4. **Table browser and `init --from`** for BigQuery, Snowflake, Postgres (medium). Draft entities from columns.
5. **A marketing-measurement space** as a shipped example (content): MMM, geo experiments, attribution, surveys, with `success` and the forecasting function pointed at a measurement MCP.
6. **More packs**: GA4, TikTok Ads, Klaviyo, Stripe, HubSpot.
7. **Semantic search** over descriptions (nice to have).
8. **Python and TypeScript readers** for the format (ecosystem).
9. **`--push-edits`** (small): when a space has a remote, push after every commit, with the same no-prompt, words-for-errors handling as the Push button. Today a person presses Push.
10. **A branch per experiment** (medium): try a set of changes on a branch, with the MCP pinned to `main` so agents never read a half-made idea. Not in v1 on purpose: the proposals inbox is the local pull request and Accept is the merge; teams that want branch review push and review on GitHub or GitLab.
11. **A `history` MCP tool** (small): the space's recent changes and who made them, so an agent can say "Finance changed `success` yesterday". Today `summary` has one History line.
12. **Embedded git for hosts without git** (large, only if needed): `git2` or `gix` compiled in. The system `git` is the engine because it uses the person's own login for push; embedding would make push credentials Lorefile's problem. The Docker image has git, so this waits for a host that cannot.

### Left for later by the open-source release (2026-11-26)

From Parts B to E of `specs/2026-11-26-open-source-release.md`, in the order they matter:

1. **Accounts and single sign-on** (large). One shared token is the Jupyter pattern and fine for "a team's copy behind one URL"; per-person accounts with email and password, then SSO (the Langfuse, Metabase, Grafana pattern), give each change a named author and let a token be revoked per person.
2. ~~**OAuth for Claude Desktop's and ChatGPT's connectors**~~ Built 2026-11-27 (`specs/2026-11-27-oauth-connectors.md`, option B): a small authorization server in the binary, on any HTTPS address that reaches the server. Still to do by hand: a run with real ChatGPT and Claude accounts and through each tunnel (DEPLOY.md §8). What comes next:
   - **Client ID metadata documents (CIMD)** (small to medium). The MCP spec now prefers them to DCR, which it deprecated on 2026-07-28 (kept for at least 12 months). Fetch a client's document from a short host list (chatgpt.com, claude.ai) with a timeout, a size cap and caching. It is Lorefile's first outbound request. Do it when either vendor stops using DCR, or by mid-2027.
   - **An outside issuer** (large; with item 1). `--oauth-issuer https://acme.okta.com`: publish resource metadata pointing at Okta, Entra, Auth0, Keycloak or Google and check their signed tokens, so a change names a person. The `/mcp` check is already one function (`caller` in `serve.rs`) that can learn this answer.
   - **Approval from the local page** (small), if pasting the token on the approval page proves annoying: requests queued in the page on `127.0.0.1`, like the proposals inbox.
   - **Tool annotations** (small). `readOnlyHint: true` on the read tools may stop ChatGPT asking for confirmation on every read (unverified).
3. **DNS-rebinding protection for `--no-auth`** (small). A `Host` allowlist (127.0.0.1, localhost, the bound name) so a web page cannot rebind a name to a laptop's port and read an unauthenticated server.
4. ~~**npm and crates.io trusted publishing**~~ Built 2026-11-27: `release.yml` publishes to npm (npm 11, `--provenance`) and crates.io (`rust-lang/crates-io-auth-action`) with OIDC trusted publishing, so `NPM_TOKEN` and `CARGO_REGISTRY_TOKEN` are gone; a tag in a fork skips both. Each site trusts `lorefile/lorefile` and `release.yml` once, in the package's or crate's settings. npm drops a new trusted publisher that has not published within 2 days, so add it on the day of the release. Later, if wanted: a GitHub environment (`release`) on both jobs and in both settings, and once a tag has published this way, npm's "Require two-factor authentication and disallow tokens" and crates.io's Trusted Publishing Only mode.
5. **Homebrew tap** (small). `brew install lorefile/tap/lorefile` from the release archives; also musl / Alpine binaries, and macOS signing and notarisation if a download path ever needs it (curl and npm do not).
6. **Sub-kinds: `extends`** (medium). An entity that is a kind of another, inheriting its fields. Imported `rdfs:subClassOf` hierarchies and Ossie's `extends` are tags today (`subclass-of:Intangible`); with sub-kinds they would come in as what they are, and schema.org's `name` and `description` on `Thing` would reach `Person` and `Product`.
7. **Other tools' extensions and Ossie dialects on import** (small). Ossie `nonportable_extensions` from other vendors are dropped, as are dialect-only SQL forms when a portable one exists; an `ossie:` passthrough key would keep them.
8. **`--push-edits`** (small; item 9 above).
9. **More from the RDF bridge** (each small to medium):
   - `owl:inverseOf` and `inverse:` on links, once links have an inverse (the ontologist's F5);
   - OWL restrictions (`owl:someValuesFrom`, cardinalities on a class) read as field facts instead of dropped;
   - SHACL `sh:or`, `sh:node`, `sh:class` on fields, `sh:qualifiedValueShape`, SPARQL constraints and node-level `sh:in` turned into CEL where they can be, instead of listed for a person;
   - SHACL 1.2's `sh:agentInstruction` behind a flag, once the draft settles;
   - `iri:` on metrics, functions and sources (they get derived IRIs today) and on domains;
   - a SKOS concept scheme with no entity to mean (a company glossary on its own) imported as words pointing at entities chosen with `--only`;
   - `--prefix <iri>` on `import --from-rdf` (in the spec, not built): adopt the source's namespace as the new space's `namespace:`, for a space that becomes the company's ontology. Today imported things keep their IRIs as `iri:` keys and the space keeps its own namespace;
   - Turtle output for SHACL lists of non-scalar enum values (written as `lorefile:values` JSON today), and LinkML export.
10. **Two pieces of import code are copied** (small): `src/ossie.rs` and `src/rdf.rs` each have the layout, compile-then-write and never-overwrite logic. One shared module would do.

### Apache Ossie: what `export --ossie` and `import --from-ossie` leave out (2026-11-26)

Built: export of the semantic model, ontology and mapping (all three pass Ossie's validator on every shipped example), import of all three, and a lossless round trip for every example and pack. Left out:

- **Other tools' `nonportable_extensions`** are dropped on import, with a note. Ossie asks readers to keep them; Lorefile has no slot for another vendor's data. A small `ossie:` passthrough key on entities and metrics would fix it.
- **Dialects.** Export writes only the portable `expression.sql`; import takes the first dialect only when there is no portable form.
- **Packs** bind entities to MCP tools, not tables, so their semantic model has no datasets and Ossie's validator rejects it (the ontology document is fine). Waits for Ossie to say how a non-table source is written.
- **Metrics with variants or MCP-tool sources** have no Ossie form and travel in the extension only. A metric's `measure` becomes SQL by a small translation (`count distinct x` → `COUNT(DISTINCT x)`, `filter` → `FILTER (WHERE …)`); the `, by …` grouping hint is left out of the expression, since Ossie groups at query time.
- **Joins** are inferred: a link becomes an Ossie relationship when the one side's key also names a property on the many side. Lorefile links name no columns; a `join:` key on links would make this exact.
- **On import:** compound identifiers (Flights' `Runway`) give no key; `unique_keys`, field-level `ai_context` and `ai_context.examples` are dropped; relationships with three or more roles and the ontology's `requires`/`derived_by` expressions are not turned into links or guardrails; `extends` (sub-kinds) is not read; unary link mappings with a `WHERE` are not read. Each is listed by `--report`.
- ~~**IRIs**~~ Built with Part E: concepts carry each entity's resolved IRI (its own `iri:`, or the namespace plus the name), and an imported concept's or relationship's absolute IRI becomes the entity's, field's or link's `iri:`.

## Hosted version, later

Decided 2026-11-26: the project launches as Lorefile, open source only, and the hosted plan (then called Stella Cloud) waits. Nothing in this section is being built. It is collected here from ARCHITECTURE.md (§10, §11, §17.6 to §17.8), DEPLOY.md (§2C and the status table), PRODUCTION.md (§2) and the backlog above, so the design is not lost. The rules stand: a hosted copy runs the same open-source binary, never executes, and holds no third-party credentials. What it would add is one safe copy of each space that agents read, SSO and roles, an audit log, and GitHub sync.

**What it would be.** The one copy, kept:

```
 your repo (main) ──push──► GitHub App ──► the service pulls, validates, reloads the space
 page edits, accepted proposals ◄──commit / PR──┘        (author = the person, one commit per change)
 agents ──► https://acme.<hosted domain>/mcp/brand-two    one URL per space, one key per agent
```

| | Open source (free) | Hosted |
|---|---|---|
| Format, compiler, page, MCP, DuckDB, examples, packs, `propose_patch`, logic, `validate` | yes | same binary |
| MCP transport | stdio, and HTTP with a login token | HTTPS with a key per agent, so ChatGPT and hosted agents connect |
| Login and roles | the login token, or your SSO proxy | SSO; who may edit, who may accept proposals |
| The one copy | your git, your branch protection | your git through a GitHub App, or a repository held for you, exportable any time |
| Rule changes | commits and proposals in the page | the same, plus Slack or email to the owner named on the rule |
| Audit | `.lorefile/activity.jsonl` | kept, searchable, exportable; part of your SOC2 evidence |
| Credentials | none by convention (not enforced) | none: the MCPs agents call hold their own |
| Isolation | your container | one container per customer |

Updating the hosted MCP would be a push to `main` or an accepted proposal: no deploy, and nothing of the customer's data passing through the service. Backups would be a push to an internal mirror, which needs no customer credentials; syncing with a customer's own GitHub or GitLab would keep its token in the hosted secret manager, never in a space and never in the open-source binary. The earlier sketches (ARCHITECTURE §10, DEPLOY §2C) also had the service hold read-only warehouse credentials; that is withdrawn by the keeper principle.

**Pricing shape.** With no execution and no credentials, what is sold is the kept copy: the hosted MCP URL, the keys, SSO, sync, audit, and the packs with the measurement definitions. Per space is the natural unit.

| Tier | For | Price idea | Includes |
|---|---|---|---|
| Self-hosted | Anyone | Free, Apache-2.0 | Everything |
| Team | One marketing team, a few brands | ~$49 per space per month | Hosted MCP URL, keys, SSO, GitHub sync, audit, table browser |
| Business | Teams with governance needs | ~$199 per space per month | Roles for who accepts rule changes, Slack and email to rule owners, audit export, SOC2 evidence, SLA |
| Enterprise | Several business units | custom | Dedicated containers, data residency, procurement |

The hosted cost per tenant is cents a day, so even the Team tier carries margin. What makes the Business tier worth more is not compute but accountability: who changed a rule, who accepted it, proof for an auditor.

**Hosted roadmap and backlog**, in order:

| Item | Size | Notes |
|---|---|---|
| Keys per agent on top of the open-source HTTP transport, with an agent name | 1 to 2 weeks | The key's name is what `may_be_called_by` matches in hosted. |
| GitHub App sync: a push to `main` reloads the space; page edits become pull requests | large | Local history, commits per change, Share, Push and Pull are built (2026-11-26); this is the hosted, token-holding part. |
| SSO, roles, the control plane | 4 to 6 weeks, with the two rows around it | Who may edit, who may accept proposals; replicas and one container per customer handled by the control plane. |
| Slack and email to rule owners when a proposal waits | small | |
| One container per customer, data residency | with the control plane | A process boundary is what an auditor wants to see between customers. |
| Audit log kept, searchable and exportable as SOC2 evidence | medium | Every change and every tool call: which agent or person, what, when. |

Status: design only.

### Left for later by evidence grades (2026-11-26, `specs/2026-11-27-evidence-grades.md`)

1. **Grades on entities, fields, links, functions, actions and sources** (medium). The first version grades metrics, data bindings, glossary words and guardrails; the mechanism (`trust.yaml`, the fingerprint, the save gate) takes the other kinds as they are.
2. **Rules that read a grade** (medium), with metric references in rules: "a budget change needs a checked success metric".
3. **`calibrated_against:` on modelled metrics** (small): tie a modelled number to the experiment that calibrated it.
4. **A check that expires or is asked for again after N days** (small). Today a check from a year ago still reads "checked", with its date.
5. **`lorefile trust --audit`** (small): compare each line of `trust.yaml` with who committed it, and flag a line an agent's commit wrote.
6. **Named, signed-in people** once accounts exist: "Priya" is a typed name today.
7. **A machine check of a binding for local files** (the table exists, the key is unique), if the frozen DuckDB door is ever reopened.

### Left for later by `learn_file` (2026-11-27, `specs/2026-11-27-learn-file.md`)

1. **A `from_user` source readable here** (medium): `path:` and a glob, read through the frozen DuckDB door, so Claude Code users who drop each month's file in a folder can use `query` (option C in the spec's Part 3).
2. **A `format:` on date fields** (small) for `11/20/2026` and the like; today they are saved as text with a note.
3. **A `set_property` op** (small) to change a field's type or unit.
4. **Splitting a flat entity into several**, on request (medium).
5. **Every sheet of a workbook in one call** (small); today it is one sheet per call.
6. **A totals check against last month** (medium), "revenue is 3× last month's", to catch a meaning that changed under the same headers. Today `match_file` says `same` and the caveat and the person's review are the only guard.
7. **Grades per field** (medium).
8. **An owner for a learned number** (small): a learned metric has none, so it carries warning W109 until a person adds one. Lorefile does not invent who decides what a number means.
9. **Empty flags** (small): `refunded = false` also leaves out rows where `refunded` is empty. Fine for these files; a file with empty flags may want `refunded is distinct from true`.
