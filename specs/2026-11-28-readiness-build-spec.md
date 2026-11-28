# Spec: ready for a large space (before the airport example)

Written 2026-11-28 for Vinay to decide on. **Status: not built.**

It comes from his message: "what else is missing, we need to model an airport example with 1000s of entities and object ane links actions guadrails etc. so before that i want the product to be as ready as possible. … so we have a good foundation. we can commit this example to repo but maybe when someone pulls we show a way where they can select the example they want to load, otherwise it'll be too many file just when someone clones or want to use right." In the same message: "how do i switch branch or merge etc?", and a shared show/hide ("layers") control on the Overview and the Graph, to see "guardrails only and more".

One idea: **make Lorefile honest at 2,000 entities before we write 2,000 entities.** Today it is built and tested for spaces of about 10. Measured below: at 500 entities the Overview takes 85 seconds to arrive, and at 2,000 an agent's `summary` is 181,000 tokens.

**Base:** `main` at `643eb56` (v0.2.0), plus the Ledger page, uncommitted on branch `claude/zen-robinson-0bd6f5`.

**Standing rules, unchanged:**
- The ontology never executes and never holds credentials.
- Ontology is not data. Rows stay in their sources.
- Agents and builders write; people review.
- Honest status: a number says where it came from, and "unknown" is allowed.
- Decisions already made in the other specs are built on, not reopened. Where this spec asks to change one, it says so (decisions 6 and 7).

**Words used here.** A *thing* is anything with a name in a space: an entity, a link, a guardrail, an action, a metric, a function, a source, a word. *Realistic shape* means the synthetic spaces described in Part 1.

---

## Before and after

**Today.** An agent drafts 1,500 airport entities. `lorefile validate` takes about 8 seconds (14 at 2,000, a minute and a half at 5,000). Vinay opens the page: the picker waits 14 seconds, the Overview never arrives (the server is working out guardrail counts, and every other request waits behind it). Claude calls `summary` and gets 700 KB, more than its whole context. Each small change the agent saves takes about a minute. Everyone who clones the repository gets the 2,800 airport files, and every install carries them inside the binary.

**After.** The same space validates in about a second. The Overview opens in under two seconds on a map of areas; "Layers → Guardrails only → Area: Baggage → not yet accepted" shows the 40 that need a person. `summary` is a map of about 6,000 tokens that says how to drill down. The agent's draft arrives as one import on its own branch; Vinay accepts it area by area. The repository holds no airport files: `lorefile examples add airport` (or the New space screen) downloads the one example someone asks for, checked against a checksum built into the binary.

---

## Decisions

All for Vinay. The recommendation is in bold.

| # | Question | Options | Recommendation |
|---|---|---|---|
| 1 | What an airport "object" is (Gate A12, Runway 27L) | **A. A row, not YAML: reference rows live in `data/*.csv` (or the real source) and the entity `Gate` points at them.** B. One YAML entity per object. C. Inline `instances:` on the entity | **A.** It is the first rule of the product ("ontology is not data"), it already works (`where_is`, `instances`, `validate` against rows), and it keeps the example at thousands of *definitions*, not thousands of gates. C stays for lists under about 20 rows. |
| 2 | The size we design and test for | **A. 2,000 entities with about 15,000 things is the budgeted size; 5,000 entities must still work, slower.** B. 10,000 and up | **A.** B needs a different loader (parse on demand) and is not what the airport needs. |
| 3 | What "fast enough" means | **A. The budgets in Part 2, checked by tests.** B. No numbers | **A.** Without numbers the next quadratic loop comes back unnoticed. |
| 4 | MCP `summary` in a big space | **A. A map with a fixed size (about 6,000 tokens): counts, areas, the most important entities per area, how to drill down. Small spaces keep today's full text.** B. Today's full list, cut off at a limit. C. Leave it | **A.** Today it is 181,000 tokens at 2,000 entities. An agent cannot read that, and B drops whatever comes last in the file. |
| 5 | A ceiling on every MCP answer | **A. About 24 KB (6,000 tokens). Past it the answer ends with what was left out and the call that gets the next part.** B. No ceiling | **A.** `neighbors` at depth 2 is 704 KB today. |
| 6 | Areas inside areas (changes people-areas-graph "no sub-areas") | **A. Two levels: `Airside / Stands`.** B. Flat, as decided. C. Any depth | **A.** A 2,000-entity space has 40 to 60 natural groups; flat, that is 50 tiles in 8 colours. Two levels give 8 to 12 top areas, each with its own colour. C makes the map a tree browser. |
| 7 | Fields many entities share (changes sub-kinds "no mixins") | **A. Field sets: a named group of fields (`valid_from`, `valid_to`, `recorded_by`) that an entity includes.** B. Only `extends`, as decided | **A.** The sub-kinds spec left this for "if a real space needs them". An airport does: validity windows, locations and audit fields sit on hundreds of entities. Faking it with a parent ("AuditedThing") abuses `extends`, which means "is a kind of, and its rows are part of the parent's". |
| 8 | Names | **A. One flat list, every name unique in the space, as today; add a warning for names that differ only by case or plural.** B. Names per area (`Baggage.Tag`) | **A.** Agents call tools with a bare name. Qualified names break every tool call and every export. |
| 9 | Folder layout for thousands of files | **A. A written convention, checked by an optional warning: one entity per file under `types/<area>/`, guardrails per area in files of at most 50, areas named in each entity's `domain:` not in one long list in `ontology.yaml`.** B. Free, as today | **A.** Files are still classified by content, so nothing breaks. The convention keeps diffs small and stops two branches colliding in one file. |
| 10 | Layers: how far | **A. Kinds of thing (built first version) plus filters by area, kind-of, grade, owner and light, an "only" shortcut, saved views, and the choice carried in the link.** B. Kinds of thing only | **A.** "Guardrails only" in a 3,000-guardrail space is still 3,000; the filters are what make it useful. |
| 11 | Where a layers choice is kept | **A. In this browser per space, and in the page link so it can be sent to someone.** B. In the space's files, shared by everyone | **A.** B is a new file agents would also read; later, if asked for. |
| 12 | The Overview's far view | **A. Areas first: each area a tile with counts and lights, lines between areas bundled ("Airside → Baggage · 42 links"). Entities appear as you zoom in.** B. Today's "top entities" view | **A.** At 500 entities the link lines already cover the plate (see the screenshot note in Part 5). |
| 13 | Branches | **A. Approve `2026-11-28-history-branches-identity.md` as written, and add one thing: a bulk import is also a branch (`imports/<id>`).** B. Hold it | **A.** A 1,500-entity import written straight to `main` is the opposite of "what agents read is always reviewed". |
| 14 | Reviewing a big change | **A. A summary first, then accept by group (area, or kind of thing); the whole change is still checked as one.** B. All or nothing | **A.** Nobody reads 1,500 change cards. |
| 15 | Getting a large model in | **A. `lorefile import --from-sheets`: a folder of CSVs or one workbook with fixed sheets (entities, fields, links, guardrails, actions, words), plus `--from-schema` for a database's column list.** B. Agents only, 40 ops at a time | **A.** B is today: 1,500 entities is 40+ proposals of a minute each. |
| 16 | Where large examples live | A. In this repository and inside the binary (today's way). B. In this repository, not in the binary. **C. A separate repository, `lorefile/examples`, pinned by commit; each example is a file on the GitHub release; the binary carries only the list.** D. An npm or crates package per example. E. Generated from a small seed. F. This repository with a sparse clone | **C.** Compared in Part 9. |
| 17 | How the binary fetches an example | **A. It runs `curl`, then checks the SHA-256 itself against the list built into the binary, and unpacks. `lorefile examples add ./file.tar.gz` works with no network.** B. An HTTP client compiled in. C. `git clone` of the examples repository | **A.** No new network code in a tool whose promise is that it calls nothing. The checksum is what makes it safe, and the binary already has SHA-256. |
| 18 | `lorefile` with nothing after it | **A. On a terminal with no space in the folder: one question (blank, starter, or an example from the list), then it serves. Anywhere else: today's help.** B. Help, as today | **A.** That is "show a way to select the example". Agents and scripts have no terminal, so nothing changes for them. |
| 19 | The eight small examples and packs | **A. Stay inside the binary (369 KB), so a first run works offline.** B. Move everything out | **A.** |
| 20 | The airport example's size and honesty | **A. About 1,500 entities and 10,000 to 12,000 things, built area by area through the import and review path; grades left as they honestly are (mostly "proposed").** B. 5,000 entities, generated | **A.** B is a load test, not an example. The generator in work package 1 covers B. |
| 21 | Order of work | **A. Part 11's order: measure, make it linear, fix the freeze, then agents, page, branches, import, examples, and the airport last.** B. Airport first, fix as we go | **A.** With B the first thing the example teaches a new user is that the page hangs. |

---

## Part 1 · What was measured

**How.** A generator (Python, seeded) made three synthetic spaces with an airport's shape. Each entity has about 6 to 19 fields, a key, an owner, an area and a source. About 20% are kinds of another entity, up to 4 levels. Links: about 3.3 per entity, 70% inside the area, plus 5 hubs that 15% of all entities point at and one that points at 10%. About 1.5 guardrails per entity, an action for 30%, a metric for 15%, a word for half.

| Space | Entities | Links | Guardrails | Actions | Metrics | Sources | Words | Areas | Files | YAML | tar.gz |
|---|---|---|---|---|---|---|---|---|---|---|---|
| a500 | 500 | 1,655 | 765 | 168 | 74 | 5 | 250 | 12 | 712 | 1.2 MB | 157 KB |
| a2000 | 2,000 | 6,635 | 2,988 | 626 | 316 | 20 | 1,000 | 50 | 2,797 | 5.0 MB | 642 KB |
| a5000 | 5,000 | 16,580 | 7,522 | 1,498 | 820 | 50 | 2,500 | 125 | 6,930 | 13.2 MB | 1.6 MB |

**Where.** Apple M2 Max, 12 cores, 32 GB, macOS 26.3. Release build of `main` at `643eb56` (full, with DuckDB, 47.6 MB), `--no-examples`, `--no-auth`, 127.0.0.1. One run each, not averaged; a500 ran while another build was going, so its small numbers are a little high. The page numbers are from a build of the worktree as it was at 15:13 on 2026-11-28 (the Ledger page with the first Layers control), headless Chrome at 1440 × 900.

**Server and CLI**

| What | a500 | a2000 | a5000 |
|---|---|---|---|
| `lorefile validate` | 0.85 s | 14.2 s | 96.5 s |
| Server start until it listens | 0.07 s | 0.02 s | 0.02 s |
| `GET /api/spaces` (first; the picker) | 0.9 s | 14.6 s | 94.6 s |
| `GET /api/summary` (first; loads the space again) | 1.9 s | 17.6 s | 105 s |
| `GET /api/summary` (warm) | 0.45 s · 1.2 MB | 1.33 s · 4.9 MB | 4.6 s · 12.7 MB |
| `GET /api/summary.md` | 0.65 s · 181 KB | 7.5 s · 734 KB | 47.7 s · 1.95 MB |
| `GET /api/plane` (the Overview and Graph) | **85–91 s** · 418 KB | not run: about 1.5 hours by the arithmetic below | not run |
| `GET /api/plane`, nothing changed | 7 ms | 11 ms (g2000) | not run |
| `GET /api/search?q=gate` | 7 ms | 30 ms | 77 ms |
| `GET /api/describe` of a hub | 4 ms · 19 KB | 17 ms · 62 KB | 41 ms · 151 KB |
| `GET /api/neighbors` depth 1 / 2 | 21 KB / 115 KB | 80 KB / 548 KB | 205 KB / 1.3 MB |
| Server memory after these | 41 MB | 130 MB | 111 MB |

**MCP over stdio** (tokens are bytes ÷ 4, an estimate)

| Tool | a500 | a2000 | a5000 |
|---|---|---|---|
| First call of a session (loads the space) | about 1.5 s | 16 s | about 100 s |
| `summary` | 174 KB ≈ 45,000 tokens | 707 KB ≈ **181,000 tokens** | 1.9 MB ≈ 483,000 tokens |
| `search` (15 hits) | 3 KB | 4 KB | 4 KB |
| `describe` a typical entity | 8 KB ≈ 2,000 tokens | 8 KB | 4 KB |
| `describe` a hub (297 links at a2000) | 24 KB | 77 KB ≈ 20,000 tokens | 188 KB ≈ 48,000 tokens |
| `neighbors` depth 1 | 27 KB | 102 KB ≈ 26,000 tokens | 260 KB |
| `neighbors` depth 2 | 148 KB | 704 KB ≈ 180,000 tokens | 1.6 MB |
| `constraints_for` a hub | 1 KB | 1 KB | 1 KB |
| `validate` with a wrong action name | not run | 16.5 KB (it lists all 626 actions) | not run |
| One small save, `propose_patch` with `--allow-edit` (add a word; describe an entity) | not run | **59 s; 57 s** | not run |

**The page** (Ledger build; g2000 is a2000 with one guardrail, so that `/api/plane` answers at all)

| What | a500 | g2000 |
|---|---|---|
| `/api/plane` | 85.7 s | 4.3 s · 1.66 MB (159 KB gzipped) |
| First open until the Overview is drawn | 85 s | 6.5 s |
| Overview, once the data is there | 53 ms · 2,256 SVG nodes | 61 ms · 2,521 SVG nodes |
| Redrawing the Overview (`drawPlane`) | 13 ms | 19 ms |
| Graph | 56 ms | 69 ms |
| List | 71 ms · 5,379 nodes | 187 ms · 21,119 nodes (every row is in the page) |
| Entities screen | 36 ms | 214 ms |
| Links screen | 125 ms · 19,215 nodes | 592 ms · 76,995 nodes |
| Guardrails screen (765 guardrails) | 40 ms | not meaningful (one guardrail) |
| Browser memory (JS heap) | 10 MB | 19 MB |

**Git on a2000** (2,797 files in their own repository): `git status` 28 ms, one-file commit 55 ms, the 7-day log the plane asks for 43 ms. Git is not the problem at this size.

**What the numbers say**

1. **The browser is fine.** The level-of-detail work from the home-plane spec holds: 60 ms to draw at 2,000 entities. Lists are not virtual, but 21,000 nodes still paint in 0.2 s.
2. **The server is not.** Three pieces of code grow with the square or the cube of the space:
   - `validate::said_twice` (`src/validate.rs:521`) compares every entity with every other, and rebuilds both field lists for each pair. A profile of `validate` at a2000 put 92% of the time there (and in the same pass over metrics).
   - `guards_of` inside `plane::plane` (`src/plane.rs:577`) asks, for every entity, every guardrail, about every link, and builds a string each time: entities × guardrails × links. That is 633 million at a500 (85 s, confirmed by profile) and 40 billion at a2000, hence "about 1.5 hours". The home-plane spec's 0.30 s for 2,000 entities was real, but that test space had almost no guardrails.
   - `Ontology::summary_markdown` (`src/ontology.rs:1528`) scans every link for every entity, and `domain_of` (`:930`) walks the manifest's lists for every entity.
3. **Every request waits for the slowest one.** All of this runs under the one registry lock (`src/serve.rs:1426`), the same lock the git-freeze notes describe. An 85-second plane is an 85-second freeze for the page and for every agent.
4. **A space is loaded more often than needed.** The picker loads and validates every space to count it and throws the result away (`Registry::summaries`, `src/spaces.rs:391`); the next request loads it again. One save runs the full load-and-validate about four times (`edit::check_many`, `src/edit.rs:373`, then the reload), which is the 57 seconds.
5. **Agents get answers they cannot read.** `summary`, `neighbors` and `describe` of a hub have no ceiling.

**Not measured:** `/api/plane` on a2000 and a5000 with realistic guardrails (too slow to wait for); MCP over HTTP; accepting a proposal from the page; `import --from-rdf` and `--from-ossie` at this size; Safari, Firefox, a phone; Windows and Linux; a space with thousands of grades in `trust.yaml`; DuckDB with real rows attached. The budgets in Part 2 are targets worked out from the profile, not results.

Scratch files (generator, timings, screenshots): `/private/tmp/claude-501/-Users-vinaykarode-Documents-code-stella-ontology/ca814851-c675-4290-9d2c-e44314646e7f/scratchpad/readiness/` (`gen.py`, `bench.py`, `page.py`, `bench_*.json`, `page_a500.json`). That folder does not survive a reboot; work package 1 moves the generator into the repository.

---

## Part 2 · The budgets (decision 3)

On the machine above, release build, the a2000 space. Each becomes a test in work package 1.

| What | Today | Budget at 2,000 | At 5,000 |
|---|---|---|---|
| `lorefile validate` | 14.2 s | 1.5 s | 5 s |
| First request after start | 14.6 s, then 17.6 s again | 2 s, once | 6 s |
| `/api/plane` | about 1.5 hours | 0.5 s | 1.5 s |
| What the page loads first (`/api/summary` today) | 1.33 s · 4.9 MB | 0.2 s · 300 KB | 0.4 s |
| MCP `summary` | 707 KB | 24 KB at any size | 24 KB |
| Any MCP answer | up to 704 KB | 24 KB, then "more" | 24 KB |
| One small save | 57 s | 3 s | 8 s |
| First open until the Overview is drawn | does not arrive | 2 s | 4 s |
| A request while another is slow | waits for it | does not wait | does not wait |

---

## Part 3 · The gap list

What an airport-sized model needs that Lorefile lacks or does badly today. "WP" is the work package in Part 11.

### Speed and loading

| Gap | Where | Fix | WP |
|---|---|---|---|
| Validation is quadratic | `validate::said_twice` | Build each entity's own-field set once. Find W116 candidates through an index (field name → entities; same source table → entities) instead of all pairs. Group metrics by recipe in a map for W115, words by folded term for W117. | 2 |
| The plane is cubic in guardrails | `plane::plane`, `guards_of` | Build once: guardrail ids by entity, link names by entity. Same for `also` (words per entity, `:769`) and the per-area light scan (`:809`). | 2 |
| No index of links | `Ontology.relationships` is a list; `outgoing`, `incoming`, `neighbors`, `path`, `summary_markdown`, `kind_sentence` all scan it | Two maps built in `Ontology::from_raw`: links by `from`, links by `to`. The duplicate-link check at `src/ontology.rs:334` uses a set. | 2 |
| Area lookups walk lists | `Ontology::domain_of`, `serve::overview` (`placed.contains`), `validate.rs:127` | One map, entity → area, built in `from_raw`. | 2 |
| Spaces are loaded and thrown away | `Registry::summaries`, `other_names` | Keep the `Loaded` they make. Shipped examples carry their counts in the built-in list (WP 11), so the picker never loads one. | 2 |
| A save loads the space four times | `edit::check_many`, `Loaded::reload` | The checked result of the save *is* the new state: keep it instead of reloading. One load before, one after. | 2 |
| One slow request freezes all | `serve::handle` holds the registry lock to the end | Hand each request an `Arc` of the loaded space and let go of the lock before the work. Writes still take turns. This is option C of the git-freeze notes, and it also covers slow non-git work. | 3 |
| One hung `git` freezes all | `edit.rs`, per the freeze notes | Their option B: 5-second limit, 30-second back-off, an honest "unknown". | 3 |
| Every request re-reads every file's date | `spaces::fingerprint` | Fine up to 5,000 (it is inside the 30–77 ms search times). Leave it; note it. | — |
| Whole-folder re-parse on every change | `loader::load_with` | Only if the budgets are missed after WP 2: keep each parsed file by path, size and date, and re-parse the changed ones. | Later |
| A flood of warnings | `lorefile validate`, the page's nudges | Group by code when a code has more than 20: "W101 × 412 · first 5 shown · `--all` for the rest". | 2 |

### What agents get (MCP)

| Gap | Where | Fix | WP |
|---|---|---|---|
| `summary` lists everything | `Ontology::summary_markdown`, `mcp.rs` `summary` | Above about 150 entities it becomes a map (Part 4). | 4 |
| No ceiling, no paging | `neighbors`, `describe`, `constraints_for`, `resources/list` (`mcp.rs:930` stops at 200 without saying so) | One helper that caps an answer and adds the "more" line (Part 4). MCP's own `cursor` for `resources/list`. | 4 |
| No way to list "all guardrails in Baggage" | — | A `list` tool: kind, area, owner, grade, paged. `search` gains the same filters and `offset`. | 4 |
| Search ranking is flat | `Ontology::search` | Ties broken by importance (the plane's score) before the name, so "gate" returns `Gate` before `GateAllocationWindow`. Kind and area filters. | 4 |
| Errors list everything | `validate` with an unknown action returns all 626 names | The 8 nearest names. | 4 |
| Agents can't add an action in a patch | `propose_patch`'s ops (`mcp.rs:246`) have no `add_action` | Add `add_action` (and `add_function` is there already). | 8 |
| 40 ops per patch, one file per change | `patch.rs:82`, `propose_change` | A bundle: many files as one proposal (Part 7). | 8 |

### What people see (the page)

| Gap | Where | Fix | WP |
|---|---|---|---|
| One 4.9 MB load before anything shows | `serve::overview` returns every link, guardrail, action, word | A small first answer (counts, areas, people, totals) and a paged `/api/list`. Screens ask for what they show. | 5 |
| Lists hold every row | List, Entities, Links (77,000 nodes), Guardrails, Actions, Files, People | A filter box on every list, 100 rows drawn at a time, more as you scroll, sort done by the server. | 5 |
| The List has entities, metrics, sources, agents only | `planeList` | Guardrails, actions and links join it, so layers and filters mean the same thing in all three views. | 5, 6 |
| The plane sends everything | `/api/plane` 1.66 MB at 2,000 | Areas and the top entities first; an area's entities when it is opened or zoomed into. Gzip (already in BACKLOG). | 5 |
| Lines cover the Overview from about 500 entities | `drawIso` draws each link | Bundled lines between areas when zoomed out (decision 12). | 7 |
| 50 areas, 8 colours, flat | `schema::AREA_COLORS`, `Domain` | Two levels of area (decision 6). | 7 |
| A hub in the Graph | `columnRows` shows 60 of 297 and says "Find to go to one" | Group a hub's neighbours by area ("Baggage · 41"), open a group, filter within. | 7 |
| No map of where you are | — | A small corner map of the areas with the view marked. | 7 |
| Layers: kinds only | worktree, `LAYERS` | The full version (Part 5). | 6 |

### Modelling

| Gap | Today | Fix | WP |
|---|---|---|---|
| Objects | Entities are types; rows live in sources; `instances:` is frozen | Decision 1: reference rows as CSV with a source. A README section says it in one before/after. | 12 |
| Kinds at depth | One parent, 6 levels, decided | Enough. The airport guide says: at most 3 levels unless there is a reason. | 12 |
| Many areas | Flat | Sub-areas (decision 6). | 7 |
| Shared fields | Only through `extends` | Field sets (decision 7, Part 6). | 7 |
| Names | Unique across the space; a clash is E002 | Keep (decision 8). New warning W122: two names that differ only by case, plural or underscore. | 7 |
| IDs of guardrails | Free text, unique | Convention: an area prefix and a number (`BAG-014`). The import tool assigns them. | 8, 12 |
| Cross-file references | By name, checked by the compiler (E003–E008) | Works. The error should name the file that holds the nearest match. | 4 |
| Files that every change touches | `ontology.yaml`'s `types:` lists; one `glossary.yaml`; one `trust.yaml` | Convention (decision 9): `domain:` on the entity; words in `glossary/<area>.yaml` (the loader already takes `terms:` from any file). `trust.yaml` stays one file for now; **risk, not measured:** with thousands of grades and branches it will be the file that conflicts. If it does, split it by area. | 7, 9 |
| Folder layout | Free | Decision 9, and `lorefile validate --layout` to warn. | 7 |

### History, review, import, docs, tests

| Gap | Today | Fix | WP |
|---|---|---|---|
| Branches | One line, `main` | The history-branches-identity spec, plus imports as branches (Part 8). | 9 |
| Reviewing a big change | One card per change; the plane compiles a "ghost" space per waiting proposal (a full load each) | Part 7. Ghosts only for proposals under 50 changes. | 10 |
| History of a big commit | `git show` of 2,800 files in one answer | Files listed first, a file's diff on click, 200 files per page. | 10 |
| Bulk import | Ossie and RDF only; they write straight to the folder | Sheets and schema import, landing as a proposal (Part 7). | 8 |
| `learn_file` | One file → one entity | Unchanged. It is the right tool for one file, not for a model. | — |
| Docs | The README teaches a 12-entity space | "Modelling a large space": objects, areas, kinds, field sets, layout, import, review. ARCHITECTURE gets the measured table. | 12 |
| Tests | One ignored 2,000-entity plane test with no guardrails | The generator in `tests/`, the budgets as tests, run in CI on the release job. | 1 |

---

## Part 4 · MCP at size (decisions 4, 5)

**`summary`, above 150 entities, becomes a map.** Same tool, same name.

```
# airport — ontology map
1,512 entities · 5,020 links · 2,240 guardrails · 470 actions · 230 metrics · 18 sources
Grades: 212 accepted · 1,840 proposed (unverified) · 31 checked

## Areas (9, with 41 sub-areas)
- Airside (310 entities, 520 guardrails): Stand, Runway, Taxiway, Pushback, …  → summary(area="Airside")
- Baggage (140 entities, 260 guardrails): Bag, BagTag, Belt, …               → summary(area="Baggage")
…
## Start here
- The 20 most linked entities: Flight (212 links), Stand (140), …
- Blocking guardrails: 380. list(kind="guardrail", severity="block", area=…)
## How to go deeper
search(q, kind?, area?) · list(kind, area?, owner?, grade?, offset?) · describe(name) · neighbors(name, limit?, area?)
```

- `summary(area=…)` gives today's full text for that area only. If one area is still over the ceiling, it lists the sub-areas.
- Under 150 entities nothing changes: the shipped examples answer exactly as today.
- The server instructions gain one sentence: "In a large space `summary` is a map; follow its pointers."

**The ceiling.** One function in `mcp.rs`, used by every tool: when the text passes 24 KB it is cut at a whole item and ends with

```
… 262 more links not shown. Next: neighbors(name="Flight", offset=30)
```

Never a silent cut. `describe` of a hub shows its fields in full, then the first 30 links by importance, then counts by area. `neighbors` takes `limit` (default 50), `offset`, `area` and `link`; at depth 2 it returns the first ring and counts per area for the second.

**`list`**, the new tool: `list(kind, area?, owner?, grade?, severity?, q?, offset?, limit?)` → names with one line each, 50 per page. It is how an agent answers "which guardrails in Baggage has nobody accepted".

Example, before and after. An agent is asked "can we reassign stand B14 during de-icing?"
- **Today at 2,000:** `summary` → 181,000 tokens; the session is over.
- **After:** `summary` (6,000 tokens) → `search("stand reassign")` → `describe("reassign_stand")` → `constraints_for("reassign_stand")` → `validate(…)`. About 12,000 tokens in all.

---

## Part 5 · The layers control (decision 10, 11)

**What is built.** The worktree has a first version (design system Part 9): a "Layers" button on the Overview and the Graph, a list of kinds of thing with a count and an "only" link each, "Show all", kept per space in this browser. Overview: links, areas, kinds, metrics, guardrails, actions, agents, sources. Graph: links, metrics, functions, guardrails, actions, sources, kinds.

**The full version.** One component, one state, three views (Overview, Graph, List).

```
┌ Layers ───────────────────────── 3 of 9 · 212 of 1,512 entities ┐
│ SHOW                                                             │
│ [x] Entities   1,512          [ ] Metrics     230   only         │
│ [x] Guardrails 2,240  only    [ ] Actions     470   only         │
│ [x] Links      5,020  only    [ ] Sources      18   only         │
│ [ ] Areas          9          [ ] Agents        3   only         │
│ [ ] Kinds        290  only                                       │
│ NARROW TO                                                        │
│ Area    [ Baggage ▾ ] [ + sub-area ]                             │
│ Kind of [ any ▾ ]        Owner [ any ▾ ]                         │
│ Grade   ( ) any (•) not yet accepted ( ) accepted ( ) checked    │
│ Light   [ ] needs a look   [ ] changed this week                 │
│ VIEWS   Guardrails to review ▾   [ Save this view ]  [ Copy link ]│
│                                            Show everything       │
└──────────────────────────────────────────────────────────────────┘
```

- **Show** is the kinds of thing. Entities can be unticked: then the drawing shows what is left *on* the entities, with the entity tiles as faint outlines so things keep their place. "Guardrails only" on the Overview: each tile shows its guardrail count and its worst severity, areas show totals, and Notes lists the guardrails of what you click. On the List it is simply the guardrails as rows.
- **Narrow to** filters whatever is shown. Area (and sub-area), kind-of (an entity and its kinds), owner (a person or team from People), grade, light. Each narrows the others' counts, so "Owner" shows only owners who have something in Baggage.
- **Hidden is dimmed or gone?** Filtered-out entities are gone from the List and faint on the drawings, so the map does not rearrange under you. A switch, "Hide instead of dim", removes them and refits.
- **Views.** Four ship: Everything; Guardrails only; Needs a look; Changed this week. "Save this view" keeps one under a name in this browser. "Copy link" puts the choice in the address (`#home?show=guardrails&area=Baggage&grade=proposed`), so Vinay can send it.
- **Always visible.** The button reads "Layers · 3 of 9"; when a filter is on, a line under the toolbar says what is hidden and offers "Show everything". Nothing is ever hidden without the page saying so.
- **Keys.** A key of its own opens it (to be chosen; `L` is the List); the first version's Esc and focus behaviour stay.

**How it scales.**
- The counts come from the server (`/api/list?count=1&…`), so the control never needs every row in the browser.
- With the plane sent by area (WP 5), a filter that names an area asks only for that area.
- The filters are the same parameters as MCP's `list` and the page's `/api/list`: one piece of server code (`Ontology::select(filter)` in `src/ontology.rs`), three users. A view a person saves is a question an agent can ask.

**Lands in:** `src/ui/index.html` (`LAYERS`, `layers()`, `layersCtl`, `drawPlane`, `drawGraph`, `planeList`), `src/ontology.rs` (`select`), `src/serve.rs` (`/api/list`).

**A note on the lines.** In the a500 screenshot the Overview at 73% is crossed by so many link lines that the areas are hard to read. Until decision 12 is built, the default in a space over 300 entities should be Links off when zoomed out, with the button saying so.

---

## Part 6 · Modelling additions (decisions 1, 6, 7, 8, 9)

These are format changes, so they go together as **format 3** (the format-2 rule: an older Lorefile refuses the space in words, never misreads it).

**Sub-areas.** In `ontology.yaml`:

```yaml
domains:
  - name: Airside
    areas:
      - name: Stands
      - name: Runways
```

An entity says `domain: Airside / Stands`. Two levels, no more. The colour belongs to the top area. Patch ops `add_area`, `move_to_area` and friends take the path. Lands in `schema::Domain`, `Ontology::domain_of`, `patch.rs` (area ops), `plane.rs` (areas), `people.rs` (owners by area), the Overview and the picker in `index.html`.

**Field sets.** A file under `fields/`:

```yaml
field_set: validity
description: When a record applies.
properties:
  valid_from: { type: datetime, description: "First moment it applies" }
  valid_to:   { type: datetime, description: "Last moment it applies; empty means still valid" }
```

An entity says `includes: [validity, location]`. The compiler copies the fields in, marks where each came from (as it does for inherited fields), and refuses a clash (the same field from two sets, or from a set and a parent: one new error code). `describe` says "from the field set validity". A set has no rows, no links and no guardrails: it is fields only, which is what keeps it from being a second kind of parent. The rule of three applies: the advice that suggests a parent (`validate::advise`) suggests a field set when the shared fields are not an "is a kind of".

**Objects.** No format change. The guide and the airport example use decision 1.

Before: someone writes `types/gate_a12.yaml`, `types/gate_a13.yaml`, … 180 files.
After: one `types/airside/stands/gate.yaml` with `source: { connection: reference_files, table: gates, key: gate_code }` and `data/gates.csv` with 180 rows. `instances Gate` lists them; `validate` checks a real gate against the guardrails.

**Layout** (convention, decision 9):

```
ontology.yaml                     areas and sub-areas, no types: lists
types/<area>/<sub-area>/<entity>.yaml
fields/<set>.yaml
constraints/<area>/<sub-area>.yaml        at most 50 guardrails a file
actions/<area>/<action>.yaml
logic/<area>.yaml                         metrics and functions
glossary/<area>.yaml
sources/<connection>.yaml
data/<table>.csv                          reference rows
```

`lorefile validate --layout` warns where a space of over 200 entities strays from it. Not on by default, never an error.

---

## Part 7 · Getting a large model in, and reviewing it (decisions 14, 15)

**Import from sheets.** `lorefile import --from-sheets <folder or .xlsx> --into <space>`

| Sheet | Columns |
|---|---|
| entities | name, area, kind_of, description, owner, source, table, key |
| fields | entity, field, type, description, values, unit, key |
| links | from, name, to, cardinality, description |
| guardrails | id, applies_to, name, rule, severity, rationale, owner |
| actions | name, description, inputs, checks, approval |
| words | term, means, description |

- `lorefile import --template sheets` writes an empty set with one example row each.
- `--from-schema <csv>` reads a database's column list (`table, column, type, is_key, comment`, which is what `information_schema.columns` exports) and makes one entity per table, bound to it. Nobody connects Lorefile to the database: the person exports the list.
- Everything imported is graded "proposed", as agent-written facts are.
- The importer writes the Part 6 layout and assigns guardrail ids when the column is empty.
- Like the other importers, nothing is written unless the result compiles; `--report` lists what was guessed.

**A bundle.** Today a proposal is one file (`propose_change`) or up to 40 ops (`propose_patch`). New: an import, and a new MCP tool `propose_bundle` (a list of files with their full text, up to 500 files and 5 MB, plus a reason), make **one** proposal. With branches (Part 8) it is one branch.

**Reviewing a bundle.** The review screen (`review.rs`, `renderReview`) opens on a summary, not on cards:

```
Import "airside draft 2" · from Claude Code · 28 Nov
+312 entities · +1,040 links · +505 guardrails · +96 actions      0 errors · 37 warnings
By area:   Stands 88   Runways 41   Taxiways 60   Pushback 37   …   [ open ]
Look at these first:  14 blocking guardrails with no rationale · 6 entities with no key · 3 names close to existing ones
[ Accept all 1,953 ]   [ Accept Stands (88 entities and what belongs to them) ]   [ Not now ]
```

- Checks run on the whole bundle once.
- **Accept by group** takes an area (or a kind of thing) out of the bundle and merges it, if what is taken compiles on its own with `main`. If Stands needs an entity that sits in Runways, the screen says which and offers to take both.
- Cards are paged, 50 at a time, grouped by entity.
- Accepting a group grades its definitions "accepted" by that person in one write, as "Vouch for all" does today.

---

## Part 8 · Branches and merging (decision 13)

**"How do I switch branch or merge?" Today, in Lorefile, you can't, and you don't need to.** A space has one line of history, `main`. There is nothing to switch to. The review step is the proposals inbox: an agent's change waits there, and your **Accept** is the merge. Your own edits on the page go straight to `main`. History (in the rail) lists every change and can undo one. If you run `git switch` yourself inside a space folder, Lorefile will serve whatever is checked out, and nothing on the page will tell you; don't.

**After the history-branches-identity spec** (written today, not yet approved; this spec asks you to approve it as decision 13 and does not repeat it):

| You want to | You do |
|---|---|
| Work without agents seeing half-finished edits | Just edit. Your edits go to your draft branch, `drafts/vinay`. The header says "Draft · 3 changes not published". |
| Merge your work | Press **Publish**. That merges your draft into `main`. |
| Merge an agent's work | Press **Accept** on its proposal. Each proposal is a branch, `proposals/<id>`. |
| See the branches | History → Branches: `main`, each draft, each proposal. |
| Switch branch | You never do. The folder always has `main` checked out; agents always read `main`. Your draft is laid over it for you only. |
| Throw work away | **Discard** on the draft, **Not now** on a proposal. |

**What this spec adds: an import is a branch.** `lorefile import … --into <space>` on a space that already has content, and `propose_bundle`, make `imports/<id>` from `main`'s tip. It shows under History → Branches with its size ("imports/airside-2 · 312 entities · by Claude Code · open"). Accept all merges it. Accept by group makes a merge commit of that group's files and leaves the rest on the branch, rebased onto the new `main`. An import into an empty or new folder writes directly, as today: there is nothing to protect yet.

Order inside that work stays as that spec's decision 9: the freeze fix, history on, names, proposal branches, drafts. Import branches come with proposal branches.

**If you meant the code repository** (the Ledger page sits uncommitted in a worktree on `claude/zen-robinson-0bd6f5`): commit it there, then from the main checkout `git merge claude/zen-robinson-0bd6f5`, or open a pull request with `gh pr create`. `git switch <branch>` changes branch, but a branch that is open in a worktree can't also be opened in the main checkout. Ask the session that owns the worktree to commit first.

---

## Part 9 · How examples ship (decisions 16–19)

**Today.** `build.rs` walks `examples/` and `packs/` and compiles every file into the binary (`include_bytes!`). At start `spaces::extract_builtin` writes them under `<dir>/.lorefile/examples/` and compares every file on every start. They are 217 files, 369 KB (about 100 KB compressed): 8 read-only spaces, the "plus 8 shipped examples" line. The crates.io package includes them. The picker loads and validates each one the first time the page opens.

**What an airport example would cost that way** (a2000's size: 2,797 files, 5.0 MB, 642 KB compressed):
- every clone gets 2,797 more files (the repository has 333 today). The download is small, well under 1 MB; the cost is the file count: search results, editors, and coding agents that grep the repository wade through 5 MB of YAML;
- every binary grows by 5 MB, for every user, whether or not they ever open it;
- every server start writes or compares 2,797 files in each spaces folder;
- the picker validates it on first open: 14 seconds today, about 1.5 after WP 2.

**The options**

| | Clone stays small | Binary stays small | Offline | In step with the binary | Cost |
|---|---|---|---|---|---|
| A. Main repo, in the binary (today) | no | no | yes | always (same commit) | none to build |
| B. Main repo, not in the binary (`gallery/`), release file | no | yes | after one download | always | small |
| **C. Separate repo, pinned, release file, list in the binary** | **yes** | **yes** | after one download | by the pin and the checksum | two repositories |
| D. npm or crates package per example | yes | yes | after install | by version number only | only helps people who use that installer; crates.io is for code |
| E. Generated from a seed | yes | yes | yes | always | it is not an example: nobody curated it, names mean nothing |
| F. Main repo, ask people to sparse-clone | only if they know to | yes | yes | always | a plain `git clone` still gets everything; most people use a plain clone |

**Recommendation: C**, with E for tests only.

- **`lorefile/examples`**, a new repository, one folder per large example. Its CI validates every example with the latest released `lorefile`.
- **The pin.** The main repository has `examples.lock`: the examples repository's commit, and per example its id, title, one-line description, counts, size, the lowest format it needs. Changing it is a normal pull request.
- **The release.** `release.yml` checks out the pinned commit, runs `lorefile validate` on each example with the binary it just built (the release fails if one does not load), packs each as `lorefile-example-<id>-<version>.tar.gz`, attaches it to the GitHub release next to the binaries, and writes each SHA-256 into the list that `build.rs` compiles in. So the binary knows exactly which bytes are its airport example.
- **In step.** An example you download with 0.3.0 is the one validated against 0.3.0. A later binary re-downloads its own; an older binary never sees a newer format.
- **The small ones stay inside** (decision 19): first run works with no network.

**Honest trade-offs of C.** Two repositories to keep; a format change means a pull request in each and a pin bump. A person who clones the main repository to read the airport example won't find it and needs the README line that says where it is. The first open of a large example needs the network once (or a file someone hands over). If you would rather keep one repository, B is the fallback: everything else in this part is the same, and only the clone grows.

**First run.**

On a terminal, in a folder with no space (decision 18):

```
$ lorefile
No space here yet. Start with:
  1  A blank space
  2  The starter (one small example entity with a guardrail and an action)
  3  An example to look at:
       ecommerce         12 entities   built in
       supply-chain      10 entities   built in
       airport        1,512 entities   download · 0.6 MB
> 3 airport
Downloading lorefile-example-airport-0.3.0.tar.gz (0.6 MB) … checked … unpacked.
Lorefile 0.3.0 · serving ./spaces · open http://127.0.0.1:7070/?token=…
```

The commands behind it:

```
lorefile examples                     the list: built in, downloaded, available
lorefile examples add airport         download, check, unpack as the read-only space @airport
lorefile examples add ./file.tar.gz   the same from a file (offline; still checked)
lorefile examples add airport --copy-to spaces/my-airport     a space of your own to change
lorefile examples remove airport
```

On the page, the New space screen and the picker list the same catalogue:

```
Start from   (•) Blank   ( ) Starter
             ( ) Copy of ecommerce        12 entities   built in
             ( ) Copy of airport       1,512 entities   [ Download · 0.6 MB ]
```

"Download" calls `POST /api/examples/add`; the row shows progress, then becomes a normal choice. On a server with no network it says so and shows the `lorefile examples add ./file` line.

**Details.**
- Downloads go to `~/.cache/lorefile/<version>/examples/` (the folder the npm launcher already uses) and are unpacked from there, so a second spaces folder does not download again.
- `LOREFILE_DOWNLOAD_URL` (already honoured by the npm launcher) points at a mirror.
- Fetching (decision 17): `curl -fsSL` with a 60-second limit, run **outside** the registry lock. No `curl`: the message gives the address and the `add ./file` command. The archive is unpacked only after its checksum matches, and paths that leave the folder are refused anyway.
- Docker: the image can bake one in with a build argument; by default it has the small ones.
- The catalogue's counts mean the picker never loads an example just to count it.

**Lands in:** `build.rs` (read `examples.lock`), `src/spaces.rs` (`extract_builtin`, a new `catalogue()`, `add_example`), `src/main.rs` (`Examples` command, bare `lorefile`), `src/serve.rs` (`/api/examples`), `index.html` (`newSpaceHtml`, `renderSpacePicker`), `.github/workflows/release.yml`, `Cargo.toml` (`flate2` and `tar`, both small), README "Install" and step 3.

---

## Part 10 · The airport example itself (decision 20)

Last, and its own spec when the foundation is in. What this spec fixes about it:

- **Size:** about 1,500 entities, 10,000 to 12,000 things, 8 to 12 areas with sub-areas.
- **Objects** are reference rows (terminals, gates, stands, runways, airlines), a few hundred, as CSV.
- **Built through the product:** drafted by agents as sheets or bundles, imported onto branches, accepted area by area on the page. Every rough edge found goes back into the backlog. This is the real acceptance test of work packages 1 to 11.
- **Honest grades:** what nobody checked stays "proposed". The example's README says how it was made and that it is not an operating airport's model.
- **Sources to draw on, not yet checked for licence or fit:** IATA AIDX (flight data messages), AIXM (aeronautical information), ACI ACRIS (airport community data model), NASA's ATM ontology (OWL, so `import --from-rdf` could bring part of it in). Each needs a licence check before anything is copied.

---

## Part 11 · Order of work

Sizes: S is up to a day of agent work with review, M is two to four, L is a week or more.

| # | Work package | What changes | Size | Done when | Needs |
|---|---|---|---|---|---|
| 1 | **Measure first** | The generator moves into the repository as test support (`tests/support/synth.rs`, seeded, same shape as Part 1; nothing it makes is committed). `tests/scale.rs`: the Part 2 budgets as `#[ignore]` tests for 500, 2,000 and 5,000. CI's release job runs them. | S | The tests exist, run in CI, and fail today on exactly the lines Part 1 names. | — |
| 2 | **Make it linear** | `validate::said_twice`; `plane` (`guards_of`, words, area lights); link and area maps in `Ontology::from_raw`; `summary_markdown`; `serve::overview`; `Registry::summaries` keeps what it loads; one load per save; grouped warnings. No behaviour change: same diagnostics, same JSON. | M | a2000: validate ≤ 1.5 s, `/api/plane` ≤ 0.5 s, one save ≤ 3 s. Every existing test passes. For each example, `validate --json` and `/api/plane` are byte-for-byte what they were. | 1 |
| 3 | **Nothing freezes the server** | Requests work on an `Arc` of the space, outside the registry lock. The git-freeze fix (option B of those notes). | M | The freeze notes' fake-git test passes. A test holds one request for 5 s and another answers in under 100 ms. | 2 |
| 4 | **MCP that fits** | `summary` as a map; the ceiling with "more"; `list`; filters and `offset` on `search`; `neighbors` and `describe` paged; nearest-name errors; `resources/list` cursor; server instructions. | M | No tool answers over 24 KB on a5000. The shipped examples' answers are unchanged. `tests/mcp_loop.rs` walks summary → area → list → describe on a2000 in under 15,000 tokens. | 2 |
| 5 | **The page loads in pages** | Small first answer; `/api/list`; `Ontology::select`; every list filtered, paged and sorted by the server; the plane by area; gzip. | L | a2000: first open to Overview ≤ 2 s; first answer ≤ 300 KB; no screen draws more than 2,000 nodes for a list; Playwright script from Part 1, kept as a check. | 2, 3 |
| 6 | **Layers, in full** | Part 5: show, narrow, views, link, the List. | M | "Guardrails only · Baggage · not yet accepted" gives the same set on Overview, Graph and List, and the same names as MCP `list` with those filters. Keyboard and screen-reader labels as the first version. | 5 |
| 7 | **Format 3: sub-areas, field sets, layout** | Part 6. `schema.rs`, `ontology.rs`, `validate.rs` (new codes, W122, `--layout`), `patch.rs` (ops), `plane.rs`, exports and imports (Ossie, RDF: a field set exports as its fields on each entity), the Overview's areas-first view and bundled lines, the Graph's hubs, the corner map. | L | A space using both loads; 0.2.0 refuses it in words; round trip through Ossie and RDF keeps the fields; the Overview of a2000 at "fit all" shows areas and bundles, no entity lines. | 5 |
| 8 | **Bulk in** | `import --from-sheets`, `--from-schema`, `--template`; `propose_bundle`; `add_action` op. | M | The template round-trips: export a2000 to sheets (a test helper), import, same space. A 500-file bundle becomes one proposal. | 7 |
| 9 | **History, branches, names** | `2026-11-28-history-branches-identity.md` as written, plus import branches (Part 8). | L | That spec's checks, plus: an import onto a non-empty space appears as `imports/<id>` and `main` is untouched until Accept. | 3 |
| 10 | **Review at size** | Part 7's summary, groups, accept by group, paged cards; History's big commits; ghosts only for small proposals. | M | A 1,953-change bundle opens in under 2 s; accepting one area leaves the rest waiting and `main` valid. | 8, 9 |
| 11 | **Examples on request** | Part 9: the examples repository, `examples.lock`, release files, `lorefile examples`, bare `lorefile`, the page. | M | A fresh machine: `lorefile` → pick an example → page open. With no network, `add ./file` works and a tampered file is refused. `git clone` of the main repository is no bigger than today. | 2 |
| 12 | **Docs** | "Modelling a large space" in the README (objects, areas, kinds, field sets, layout, import, review, layers); ARCHITECTURE: the measured table and the budgets; the tutorial rebuild. | S | A person who has only read it can import the template and find a guardrail with Layers. | 4–11 |
| 13 | **The airport example** | Part 10, in `lorefile/examples`. | L | It loads inside the budgets, every area was accepted by a person through the page, and the three-review ritual passes before the tag. | all |

Packages 4, 5 and 11 can run side by side once 2 is in. 9 can start as soon as 3 is in.

**A stop along the way.** After packages 1 to 4 (about two weeks), the product is safe to *start drafting* the airport in: validation, agents and saves work at size, even though the page's lists and the review are not there yet. Drafting can begin then, in sheets, while 5 to 11 are built.

---

## Checks for the build

- `cargo test` passes; `cargo test --release -- --ignored` runs the scale tests and they meet Part 2.
- Each shipped example's `validate --json`, `/api/summary`, `/api/plane` and MCP `summary` are unchanged after work package 2 (same bytes), and after 4 (same text, since they are under 150 entities).
- On a5000 no MCP tool answers more than 24 KB, and each cut answer names the call for the rest.
- With one request made slow on purpose, others answer in under 100 ms.
- The Part 1 Playwright run on a2000 with realistic guardrails: Overview in 2 s, Graph and List under 0.3 s.
- Layers: the same filter gives the same names in the three views and over MCP.
- A format-3 space is refused by 0.2.0 with the upgrade sentence; a format-2 space loads unchanged.
- `lorefile examples add` with a wrong checksum writes nothing; with no network and a file it works.
- The folder of a space never leaves `main` during an import, a draft or an accept.
- README, ARCHITECTURE and the MCP server instructions say what changed; the Part 1 table is updated with the "after" numbers, on the same machine if possible.

## Later

- Re-parsing only the files that changed (if the budgets hold without it, never).
- 10,000 entities and up.
- Layers views saved in the space's files and shared by a team.
- Splitting `trust.yaml` by area, if branches make it conflict.
- More large examples (a hospital, a bank) in the examples repository.
- An "all spaces" search across several large spaces.
- A built-in HTTP client, if `curl` turns out not to be there for real users.
- Several parents for kinds: still no.
