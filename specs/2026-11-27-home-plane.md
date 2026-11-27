# Spec: a home page that shows how things are, and a map you can open up

Written 2026-11-27 for Vinay to decide on. **Status: built 2026-11-27**, every decision as recommended (Vinay approved all 28). What differs from the text below is in "What changed during the build" at the end.

After the live test, Vinay asked for a home page people want to touch:
- the space's entities as icons on a 3D plane
- a traffic-light badge on anything that needs fixing
- with thousands of entities, the important ones first, more as you zoom, and the existing pages one click away

He also found the Map screen too closed: boxes with names only, and no metrics or sources. The findings are in the OAuth sessions' shared folder, `home-map-and-ui-findings.md`.

He shared another company's architecture diagram **as an idea only**: objects on an isometric plane, sources underneath, agents and apps above. Lorefile's version uses its own glyphs, its own style (the page's light theme, the logo's rounded strokes) and its own words. It copies no artwork, layout detail, icon or brand from that diagram, and it shows no product logos for agents either.

**Read with:** [2026-11-27-ui-fixes.md](2026-11-27-ui-fixes.md). Build that first. This spec uses its words (Entity, Field, Metric, Source, Guardrail, Also called), its grade lines and its name rule.

**Base:** branch `claude/learn-file-6a954e` at `8f8ba8e`, plus the UI-fixes build.

**Standing rules, unchanged:**
- The ontology never executes and never holds credentials. Rows never enter the ontology.
- Light theme only.
- Honest status.
- One embedded page (`src/ui/index.html` via `include_str!`): no build step, no CDN, works offline.

**Read first:**
- `src/ui/index.html`:
  - `renderMap`, `layout`, `facts`, `renderReviewMap`, `domainColor`, `PALETTE`
  - `renderNav`, `route` / `onHash`
- `src/serve.rs`:
  - `overview`, `trust_json`, `/api/graph`, `/api/oauth/connections`, `/api/activity`
- `src/review.rs`:
  - `graph`, `compare`, `words::*` (`label`, `metric_sentence`, `type_word`, `source_sentence`)
- `src/validate.rs`: `Diag`, and every `r.warn` / `r.error`
- `src/trust.rs`: `list`, `counts`
- `src/proposals.rs`: `list`, `activity`
- `src/oauth.rs`: the connection JSON (`name`, `space`, `last_used`, `writes`, `scopes`)
- `src/schema.rs`: `TypeDef`, which is `deny_unknown_fields`

The example carried through is branda now:
- PromoSale: 14 fields, key `line_id`
- source `user_files` (`kind: from_user`)
- `promo_sale_revenue`: checked, no owner
- two OAuth connections, ChatGPT and Codex

---

## Before and after

**Before.** The landing screen is Map: a dotted canvas of 220×70 boxes, four per row per domain, with names and tags only. A "What to do next" column sits on the right. Metrics, sources and agents are not drawn. Clicking a box selects it; "Open" goes to the entity page.

```
┌───────────────────────────────────────────────┬──────────────────────────────┐
│ · · · · · · · · · · · · · · · · · · · · · · · │ WHAT TO DO NEXT              │
│ · ┌──────────────────┐ · · · · · · · · · · · · │ Nothing waiting              │
│ · │ ● PromoSale      │ · · · · · · · · · · · · │ Every field has words, every │
│ · │                  │ · · · · · · · · · · · · │ link a sentence… [See agents]│
│ · └──────────────────┘ · · · · · · · · · · · · │                              │
│ · · · · · · · · · · · · · · · · · · · · · · · │                              │
│ (● Types)                                      │                              │
└───────────────────────────────────────────────┴──────────────────────────────┘
```

branda's "What to do next" says "Nothing waiting" although its metric has no owner. The UI-fixes spec adds that nudge.

**After: Home** (the new landing screen). The drawing is isometric; the boxes below stand in for tiles.

```
┌ Home ──────────────────────────────────────────────────────────────────┬──────────────────────────────┐
│ 1 entity · 1 metric · 1 source · 2 agents      ✗ 0   ! 1   ✓ 0        │ (nothing selected)           │
│                                                                        │ WHAT TO DO NEXT              │
│  AGENTS     (C) ChatGPT · used 2 h ago · waits for you   (C) Codex     │ ! Promo sale revenue has no  │
│                  ┆ saved Promo sale                                    │   owner                      │
│           ╱‾‾‾‾‾‾┆‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾╲                  │   [Who decides? ____] [Save] │
│  ENTITIES╱      ◇ (receipt)  !                       ╲                 │                              │
│         ╱       Promo sale                            ╲                │                              │
│         ╲       Σ1                    ◇ ┄┄ your next   ╱               │                              │
│          ╲________┆_____________________ entity ┄┄┄┄╱                  │                              │
│                   ┆ rows come from                                     │                              │
│  SOURCES   (clip) user_files · files people give agents                │                              │
│                                                                        │                              │
│ [Home] [Map] [List]   Find… (/)      ✗ wrong · ! look at it · ✓ a person stands behind it            │
└────────────────────────────────────────────────────────────────────────┴──────────────────────────────┘
```

Clicking Promo sale selects it, and the right column becomes its card (Part 6):

```
│ (receipt) Promo sale                    ! look at it │
│ Types · entity                                       │
│ One order line from a promo sales export.            │
│ 14 fields · key line_id · money in USD               │
│ Source      user_files: files people give agents     │
│             ✓ accepted by a person (no name given)   │
│ Metrics     Promo sale revenue: add up revenue,      │
│             leaving out lines where refunded is true │
│             ● checked                                │
│ Guardrails  none yet                                 │
│ Look at                                              │
│ ! Promo sale revenue has no owner                    │
│   Who decides what it means? [ Vinay   ] [Save]      │
│ [Open Promo sale]  [Show on the map]                 │
```

**After: Map**, the same component seen flat from above and opened up (Part 9):

```
 SOURCES               ENTITIES                                        METRICS
 ┌────────────────┐    ┌──────────────────────────────────────┐        ┌───────────────────────────────┐
 │ (clip)         │    │ (receipt) Promo sale           !  ▾   │        │ Σ Promo sale revenue    ● !    │
 │ user_files     │───▶│ One order line from a promo export.   │        │ Add up revenue, leaving out   │
 │ files people   │rows│ ★ line_id      text                   │        │ lines where refunded is true. │
 │ give agents    │    │   order_id     text                   │        │ Owner: no one yet             │
 └────────────────┘    │   order_date   date                   │        └───────────────────────────────┘
                       │ Σ revenue      money (USD)    ◀─────────────────────────┘ adds up
                       │ Σ refunded     yes / no       ◀─────────────────────────┘ leaves out
                       │   + 9 more fields                     │
                       └──────────────────────────────────────┘
 Show: [✓ Entities] [✓ Metrics] [✓ Sources] [✓ Links] [ Only what needs a look ]   Area: [all ▾]   [Reset layout]
```

---

## Decisions

All for Vinay. The recommendation is in bold.

| # | Question | Options | Recommendation |
|---|---|---|---|
| 1 | Where the plane lives | **A. A new Home screen (`#home`), the landing page; Map stays for detail.** B. Replace Map. C. A panel on Map | **A.** Home is the overview you look at; Map is where you open things up. |
| 2 | Home and Map: one component or two | **A. One component, two projections (isometric for Home, flat for Map), sharing data, layout, lights and the proposal overlay.** B. Home only, retire Map. C. Two components | **A.** Field lists can't be read on an isometric tile, and two components would drift apart. |
| 3 | How it is drawn | **A. One SVG with our own projection maths.** B. CSS 3D transforms on HTML boxes. C. Canvas 2D. D. A WebGL library embedded | **A.** Crisp at any zoom, real focusable elements for keyboards and screen readers, and fine at the 400-tile budget. B blurs text and is hard to cull. C needs its own hit-testing and gives screen readers nothing. D adds ~600 KB for a look we can draw. |
| 4 | Where the code lives | **A. In `index.html`, in one marked section, like every screen.** B. A second embedded file with its own route | **A.** No new route, and no question of how a script loads behind sign-in. Revisit if the page passes ~250 KB. |
| 5 | Where the data comes from | **A. One new `GET /api/plane` that works out lights, importance, which metric counts which entity, the fields each metric uses, agents and waiting proposals. Fields stay lazy (`/api/describe`).** B. The page works it out from `/api/summary` | **A.** The traffic-light rules live in Rust, tested once. Agents could use them later. |
| 6 | Icons | **A. Our own ~20 glyphs in the logo's stroke style (24×24, 2 px rounded strokes).** B. A subset of an open icon set, with its notice in NOTICE | **A.** Ours, small, and no licence admin. |
| 7 | Choosing an icon | **A. Guessed from the entity's name, then its source, else a cube. No new YAML key in v1.** B. An `icon:` key on types | **A.** `TypeDef` is `deny_unknown_fields`, so a space using `icon:` would fail to load in 0.1.0, which shipped today. Icons are presentation, not meaning. |
| 8 | Agents' look | **A. A monogram in a circle (first letter of the connection's name), never a product logo.** B. Logos | **A.** No trademarks, and a connection's name is chosen by the client, so it is not proof of who it is. |
| 9 | Layout | **A. Automatic and deterministic: same space, same picture. Dragging is only on Map, saved per browser, used by both views. A Reset button.** B. Saved to a file in the space. C. No dragging | **A.** A committed layout file would put every drag into git. `.lorefile/` is local and hidden from git anyway, so the browser is the honest place for a personal arrangement. |
| 10 | What "important" means | **A. A score from: counted by a metric, links, problems, waiting proposals, has data, changed this week. Plus pins, per browser.** B. Links only. C. A key in the YAML | **A.** It is explainable: the card says why each entity is shown. |
| 11 | Level of detail | **A. Four levels: areas, the top entities, everything in view, one entity close up. Under 60 entities, everything is shown at every zoom.** B. Always everything | **A.** Every shipped example and pack has 9 entities or fewer (meta-ads is the largest), so the levels only switch on for big spaces. |
| 12 | What a click and a zoom do | **A. Click selects (card); Enter or double-click opens the page; zooming in close shows an "Open Promo sale ›" pill. Zoom never navigates by itself.** B. Zooming past a point opens the page | **A.** A trackpad's momentum would throw people onto pages. |
| 13 | Traffic lights | **A. ✗ red, ! amber, ✓ green, and no badge. An entity shows the worst of itself, its source, its metrics and its guardrails.** B. Separate badges for each | **A**, with the reasons listed in the card. |
| 14 | Ungraded and fine | **A. No badge.** B. Amber | **A.** Otherwise every hand-written space, examples included, is wall-to-wall amber, and amber stops meaning anything. |
| 15 | No data behind an entity | **A. A grey "no data yet" chip, not amber.** B. Amber | **A.** It is a next step, not a fault. The "What to do next" list carries it, as today. |
| 16 | Agents above the plane | **A. OAuth connections that can reach this space, plus agents seen in this space's activity in the last 30 days. Lines run to entities each one saved in the last 7 days.** B. Connections only | **A.** It covers stdio agents and token-holders too. Lorefile does not log reads, so "used" means saved or proposed. |
| 17 | Logging reads to show "what agents use" | **A. Not in v1.** B. In-memory counts of entities named in tool calls | **A.** What agents ask about is a privacy choice of its own. Left for later. |
| 18 | Metrics on the plane | **A. Home: a Σ token with a count on the entity it counts. Map: their own nodes, with arrows to the fields they use.** B. Nodes in both | **A** |
| 19 | Guardrails, actions, functions | **A. Counts on the tile (shield, bolt) and lines in the card; no tier of their own in v1.** B. A logic tier below the plane | **A.** Keep the first version readable; a tier can come later. |
| 20 | Waiting proposals | **A. Ghosts on both views (dashed new tiles, amber dashed rings on changed ones, struck-through removed ones), up to 5 proposals. The review screen's "Show on the map" opens Map with that one overlaid.** B. Map only | **A.** One overlay, one place to see it, and `renderReviewMap` goes away. |
| 21 | Numbers on the plane | **A. Never row values, even when the rows are readable here.** B. Show a metric's value when readable | **A.** Lorefile keeps meaning. A number on the home page reads as a dashboard and goes stale silently. Data & SQL is where rows are read. |
| 22 | Fixing from the card | **A. Only the two questions that are one field each: a metric's owner (`set_metric`) and an entity's description (`set_type_description`). Everything else is "Open".** B. Every warning fixable inline | **A** |
| 23 | Keeping it fresh | **A. While Home or Map is visible, ask `/api/plane?version=` every 30 s, and on return to the tab. It answers `{same: true}` when nothing changed.** B. Manual Reload | **A.** Agents write while the page is open; that is the point of the agents tier. |
| 24 | Accessible twin | **A. A List view of the same items (kind, light in words, reasons, Open), one key away, and a "Skip to list" link.** B. aria labels only | **A**, plus roving focus and labels on the plane itself. |
| 25 | Phone width | **A. The plane at the top (top entities only), the list under it, and the card as a bottom sheet.** B. List only | **A** |
| 26 | Examples and many spaces | **A. Same view, read-only, no inline fixes. The space picker stays as it is.** B. An "all spaces" plane | **A.** All spaces is left for later. |
| 27 | Search | **A. In the page, over names, labels and also-called words. Enter flies to the entity and opens its card.** B. `/api/search` | **A.** The plane already holds every name, so there is no round trip. |
| 28 | Where problems are attached | **A. Add a `subject` (kind, name) to each diagnostic where it is made in `validate.rs`.** B. Parse names out of the messages, as `review::questions` does | **A.** A message is words for people. The subject is needed for lights, the card and the list, so it should be a field. |

---

## Part 1 · What Home shows

### Three tiers

| Tier | What sits there | Where the data comes from |
|---|---|---|
| **Agents** (top, a thin slab) | one chip per agent: monogram, name, "used 2 h ago", "waits for you" or "goes straight in", a ! badge with its waiting count | OAuth connections that can reach this space, plus `via` names in this space's activity for 30 days |
| **Entities** (the plane) | one tile per shown entity, grouped by area (domain); links as lines on the plane; tokens on each tile | the compiled space |
| **Sources** (bottom slab) | one chip per source: kind glyph, name, kind in words ("files people give agents", "readable here", "agents fetch rows") | `sources/` |

- **Lines between tiers** are dashed and vertical:
  - source → each entity bound to it, labelled "rows come from" on hover or focus
  - agent → each entity it saved or proposed in the last 7 days, labelled "saved" or "proposed"
- A selected tile highlights its lines and dims the rest to 35%, as the Map does today.

### A tile, close up

```
         (receipt)  !          ← glyph standing up, unskewed; light badge top right
        ◇──────────◇          ← rhombus top in white, 8 px sides in two greys; area colour as a 3 px top edge
        Promo sale             ← label below, unskewed, cut at 22 characters with "…"
        Σ1  ⛉0  ϟ0              ← metrics, guardrails, actions (a zero is hidden)
        no data yet            ← grey chip, only when nothing is bound
```

- **The badge** is a 16 px circle with a glyph: ✗ on red, ! on amber, ✓ on green. No light, no badge.
- **Ghost tiles** (a waiting proposal) are dashed with no fill, with "waiting" under the label and the agent's monogram on the badge.

### The bar above the plane

`1 entity · 1 metric · 1 source · 2 agents      ✗ 0   ! 1   ✓ 0`

- Each light count is a button that filters the List (and dims non-matching tiles on the plane) to that light.
- A zero is shown but not clickable.

### The legend, in words, always visible

`✗ wrong: agents may get wrong answers · ! look at it: a person should check · ✓ a person stands behind it · no badge: nothing to fix, nobody has graded it`

### The right column

- Nothing selected: "What to do next", the existing nudges from `renderMap`, worded per the UI-fixes spec.
- A tile, chip or ghost selected: its card (Part 6).
- Esc goes back to "What to do next".

---

## Part 2 · Traffic lights

Lights are worked out on the server (`src/plane.rs`), from signals that already exist. Each light comes with its reasons, in words, so the card and the list say why.

### The rules

| Kind | ✗ red | ! amber | ✓ green | no badge |
|---|---|---|---|---|
| **Entity** | any error whose subject is the entity (E-codes); a link to an entity that doesn't exist; its source names a connection that doesn't exist | W101 (no description) · fields without words · W103 (bound, no key) · W106 (its file isn't found) · W105 (one-of with no values) · its source is **proposed** (unverified) or **changed since** (W113) · a waiting proposal changes it | its source is accepted or checked, and nothing amber or red | nothing amber or red, and its source is ungraded or there is none |
| **Metric** | any error whose subject is the metric (e.g. its source connection doesn't exist) | W109 (no owner) · W101 (no description) · proposed · changed since (W113) · a waiting proposal changes it | accepted or checked, and nothing amber or red | ungraded, nothing amber or red |
| **Source** | any error whose subject is the source | W106 for a type bound to it (file not found) | (never: sources have no grade) | otherwise |
| **Agent** | (never) | it has waiting proposals in this space (the badge shows the count) | (never) | otherwise |
| **Guardrail** (counted on the entity) | an error on it | W102 (blocks, no reason) · no owner ("no sign-off", as the nav says today) · proposed · changed since | accepted | ungraded |
| **The space itself** | errors with no subject (a broken manifest) | W112 (two things with one IRI) and other subject-less warnings | – | – |

How a light rolls up and is shown:
- **An entity shows the worst light** of itself, its source, the metrics that count it and its guardrails. The card lists every reason, grouped by where it came from: "From Promo sale revenue: no owner".
- **An area (domain) shows the worst light of its entities**, and counts, e.g. "✗ 3 ! 12".
- **The space's own reasons** show as a badge on the bar's space name and as the first rows of the List.
- **Never colour alone**: every badge has a glyph, every card and list row has words, and every tile's `aria-label` ends with its light in words.

For branda now:
- PromoSale: **amber**.
  - "From Promo sale revenue: no owner. Who decides what it means?"
  - Its source is accepted, and its metric is checked.
- Promo sale revenue: **amber** (W109).
- user_files: no badge.
- ChatGPT: no badge (nothing waiting).

Once Vinay sets the owner, PromoSale and the metric turn **green**.

### Where a problem is attached (Decision 28)

`Diag` gains `subject: Option<Subject>`, where `Subject { kind: &'static str, name: String }`:
- `kind` is one of entity, field, link, metric, function, guardrail, action, source, term.

The rules for setting it:
- **Set where the diagnostic is made**: each `r.warn` / `r.error` in `validate.rs` that names one item passes it. A field's subject is its entity, carrying the field name in the reason. A link's subject is its `from` entity.
- **Loader errors with only a file**: map to the entity when the file defines exactly one (`type:`). A `types:` list file (`schema::TypesFile`) can hold several, and any other file maps to the space itself.
- **The JSON keeps `code`, `level`, `message` and `file`** and adds `subject`. Nothing that reads diagnostics today breaks.
- **`review::questions` switches to `subject`** and drops its message parsing. Same output, which its tests prove.

### The reasons, in words

These reuse `review::questions`' sentences where one exists:
- W109: "no owner. Who decides what it means?"
- W101: "no description: agents have only its name to go on"
- W103: "no field says which row is which"
- W106: "Lorefile can't find promo_sales.csv"
- W113: "changed since Vinay accepted it on 27 Nov"
- proposed: "unverified: ChatGPT proposed it on 27 Nov, no person has looked"
- waiting: "ChatGPT wants to change it (waiting for review)", with a Review button

Errors use their message, cleaned of backticks.

---

## Part 3 · What's important, and what zoom shows

### The score (server), shown in the card as "Why it's shown"

| Signal | Points |
|---|---|
| counted by at least one metric | +4 |
| each link in or out | +1, up to +6 |
| red | +6 |
| amber | +3 |
| a waiting proposal changes it | +4 |
| has data (bound or inline rows) | +1 |
| changed in the last 7 days (the git commit time with history on; else the newest trust `at` or accepted proposal for it) | +2 |
| pinned in this browser (page only) | always shown |

- Ties go by name, A to Z, so the order is stable.
- PromoSale scores 4 + 3 + 1 + 2 = **10**: a metric, amber, data, changed today.

### The four levels

Under 60 entities, nothing folds: every entity, link and token is shown at every zoom. Over 60, the levels switch on:

| Level | When | What is drawn |
|---|---|---|
| **Areas** | zoom < 0.6 | one big tile per area: its name, "212 entities", its light counts and its 3 top glyphs. Lines between areas carry a count ("4 links"). Clicking an area zooms to it. |
| **Top entities** (the fit view) | 0.6 to 1.6 | in each area the top *k* by score, where *k* = max(3, its share of a 120-tile budget). The rest fold into one "+209 more" tile per area. Links to folded entities end at that tile, bundled with a count. |
| **Everything in view** | ≥ 1.6 | every entity inside the viewport, up to 400 by score; past 400, the lowest fold into "+N more here". |
| **Close up** | ≥ 3, with one tile near the centre | that tile shows its first 6 field names on its top face, and an "Open Promo sale ›" pill floats under it. |

- Zoom out stops at Areas. The page never navigates on zoom alone.
- Pinned entities are always drawn, at every level.
- A selected entity stays drawn when the zoom would fold it.

### Search

- "Find…" (or `/`) searches the plane's names, labels and also-called words, case- and accent-insensitive.
- Results show as a list under the box.
- Enter (or a click) flies to the entity:
  - 300 ms ease, or instant with reduced motion
  - unfolds it if folded
  - opens its card
- A metric or source result flies to the entity it belongs to and opens the card on that line.

---

## Part 4 · Layout

All of it is computed in the page, deterministic, from `/api/plane` alone.

1. **Areas** go in the manifest's domain order, on a grid that is as square as it can be. Types with no area go in "Other", last. Empty areas are not drawn.
2. **Within an area**:
   - Start from the highest-scoring entity (ties by name).
   - Walk its links breadth-first, neighbours in name order, then the next unplaced entity by score.
   - Place each in that order on a square spiral of cells around the area's centre.
   - Linked entities land near each other, and the same space always draws the same.
3. **One cell** is 1 × 1 logical unit:
   - Flat: 260 × 120 px.
   - Isometric: a rhombus 160 px wide and 80 px tall (2:1), on a plane tilted the classic isometric way.
   - Areas are separated by two empty cells.
4. **Sources** sit on the bottom tier under the centre of the entities they serve; agents sit on the top tier, spaced evenly. With more than 12 of either, the tier scrolls sideways and shows "+N more".
5. **Changes stay local.**
   - Adding an entity appends it to its area's spiral, so the others don't move.
   - A score change reorders only when it changes the walk's starting entity, which happens rarely.
   - The layout is cached per `version`, so a refresh with nothing changed reuses it.

**Dragging (Map only)**:
- It moves a node's logical cell, kept in `localStorage` under `lorefile-layout:<space id>` as `{ "PromoSale": [u, v] }`. Home uses the same cells.
- Reset layout clears it.
- Names that no longer exist are pruned on load.
- Two nodes can't share a cell: a drop on a taken cell swaps them.
- With storage refused, dragging works for the visit and is forgotten.

---

## Part 5 · Icons

About 20 glyphs, drawn for Lorefile, kept as `<symbol>`s in one hidden `<svg>` in the page:
- 24 × 24, stroke `currentColor`, 2 px rounded caps and joins, like the logo.
- Each is under 300 bytes.

Each entity's glyph is guessed from its name, split into words (`PromoSale` → promo, sale; `order_line` → order, line):
- **First match wins, in this table's order.**
- A word matches whole, or with a plural "s".
- Otherwise the source decides: a `from_user` source gives document.
- Otherwise cube.

| Glyph | Name words |
|---|---|
| receipt | order, sale, sales, invoice, payment, transaction, purchase, checkout, refund, charge |
| person | customer, user, member, contact, person, lead, buyer, shopper, subscriber, patient, student |
| people | team, employee, staff, agent_group, owner_group |
| building | company, organization, organisation, org, account, vendor, supplier, carrier, partner, merchant, brand |
| box | product, item, sku, variant, inventory, stock, part, material |
| tag | promo, promotion, coupon, discount, offer, price, pricing, tier, plan |
| megaphone | campaign, ad, ads, creative, channel, audience |
| truck | shipment, parcel, delivery, package, fulfilment, fulfillment, route |
| pin | warehouse, location, store, region, site, address, country, city, zone |
| calendar | date, day, week, month, period, calendar, season, event_date |
| bolt | event, session, visit, click, pageview, impression, signal |
| repeat | subscription, renewal, contract, membership |
| chat | ticket, case, issue, conversation, message, review, feedback |
| chart | metric, kpi, score, forecast, budget, target |
| document | file, document, report, export, upload, sheet |
| link | relationship, mapping, assignment, association |
| cube | (anything else) |

Sources get a glyph by kind:

| Kind | Glyph |
|---|---|
| csv, parquet | sheet |
| duckdb, postgres, bigquery, snowflake | cylinder |
| s3 | bucket |
| mcp | plug |
| `from_user` | paperclip |
| http | globe |
| anything else | cylinder |

Rules:
- **Agents** get a monogram: the first letter or digit of the connection name, in a 28 px circle on the accent colour. A name with no letter gets "A".
- **Area colour** (`PALETTE`, `domainColor`) tints the tile's top edge and the glyph. Badges keep their own colours.
- **`/api/plane` sends `icon` and `icon_why`** ("name: sale"), so the guess is testable in Rust and the card can say "icon from the word *sale*".
- **The same table lives in `plane.rs`**; the page only draws `icon`.

---

## Part 6 · The card

One card for whatever is selected. The right column on desktop, a bottom sheet on a phone.

**Entity** (shown above for Promo sale), in this order:
- glyph, label, light in words
- area · entity
- description, or "No description yet. What is one Promo sale?" with the inline fix (Decision 22)
- "14 fields · key line_id · money in USD", the units summarised from the fields
- **Source**: name, kind in words, the grade line (UI-fixes Part 3), or "No data yet · [Bind a table]" → Entities › Data
- **Metrics**: each as label + sentence + grade, and an owner fix where W109
- **Guardrails**: count and the first three names; **Actions**: count; **Also called**: chips
- **Look at**: every reason with a fix or a link
- **Waiting**: each proposal that touches it, with [Review]
- **Why it's shown** (only when the levels are on): "a metric (+4), needs a look (+3), …"
- [Open Promo sale] (the entity page) · [Show on the map] · [Pin] (this browser)

**Metric** (from a token or the Map):
- label, sentence, its entities, the fields it uses
- grade line, owner line
- caveats
- [Open in Logic]

**Source**:
- name, kind in words, "readable here" or not
- the entities and metrics it serves
- its reasons
- [Open in Sources]

**Agent**:
- name, "connection" (or "seen in activity")
- approved when and with what, last used
- "its suggestions: wait for you to accept" or "go in straight away, marked unverified"
- waiting proposals with [Review], and the last 5 activity lines from it
- [Open in Settings › Connections] for an OAuth connection
- **Never a token.** The name is shown as text, escaped, cut at 40 characters, with "(connection)" after it, so a client that registers as "Vinay" can't pass for a person.

**Ghost** (a waiting entity):
- the proposal's headline, the agent
- what it would add (from `review::graph`)
- [Review]

What a card never shows:
- **Row values.** Not on a source that is readable here, not as an example, never.
- **A metric's value.** Data & SQL does that.

---

## Part 7 · The /api/plane JSON

`GET /s/<space>/api/plane` returns the whole picture for one space. `?version=<v>` answers `{ "same": true }` when nothing changed.

`version` is a hash of:
- the space fingerprint (`Loaded::fp`)
- the proposals folder's names and mtimes
- the activity file's length
- the OAuth connections' `last_used` values, rounded down to a minute

branda now (the agent `id` is the connection's id):

```json
{
  "version": "7e1c0a9b44d2",
  "space": { "id": "branda", "name": "branda", "builtin": false, "editable": true, "ok": true, "empty": false, "reasons": [] },
  "counts": { "entities": 1, "metrics": 1, "sources": 1, "agents": 2, "waiting": 0,
              "lights": { "red": 0, "amber": 1, "green": 0, "none": 0 } },
  "areas": [ { "name": "Types", "entities": 1, "light": "amber", "lights": { "amber": 1 } } ],
  "entities": [ {
    "name": "PromoSale", "label": "Promo sale", "area": "Types",
    "description": "One order line from a promo sales export.",
    "icon": "receipt", "icon_why": "name: sale",
    "fields": 14, "described": 14, "key": "line_id", "units": ["USD"],
    "source": { "name": "user_files", "kind": "from_user", "table": "promo_sales", "grade": "accepted", "by": "a person" },
    "metrics": ["promo_sale_revenue"], "guardrails": 0, "actions": 0, "also_called": [], "links": 0,
    "light": "amber",
    "reasons": [ { "light": "amber", "from": "metric promo_sale_revenue", "code": "W109",
                   "text": "Promo sale revenue has no owner. Who decides what it means?",
                   "fix": { "op": "set_metric", "metric": "promo_sale_revenue", "field": "owner" } } ],
    "importance": 10, "importance_why": ["counted by a metric (+4)", "needs a look (+3)", "has data (+1)", "changed this week (+2)"],
    "changed": "2026-11-27"
  } ],
  "links": [],
  "metrics": [ {
    "name": "promo_sale_revenue", "label": "Promo sale revenue",
    "sentence": "Add up revenue, leaving out lines where refunded is true",
    "entities": ["PromoSale"], "uses": ["revenue", "refunded"],
    "grade": "checked", "owner": null, "light": "amber", "reasons": [ { "light": "amber", "code": "W109", "text": "no owner. Who decides what it means?" } ]
  } ],
  "sources": [ { "name": "user_files", "kind": "from_user", "words": "files people give agents", "icon": "paperclip",
                 "entities": ["PromoSale"], "metrics": ["promo_sale_revenue"], "light": "none", "reasons": [] } ],
  "agents": [
    { "id": "…", "name": "ChatGPT", "monogram": "C", "seen": "connection", "last_used": "2026-11-27T16:40:00Z",
      "writes": "wait", "waiting": 0, "light": "none", "recent": [ { "entity": "PromoSale", "how": "saved" } ] },
    { "id": "…", "name": "Codex", "monogram": "C", "seen": "connection", "last_used": "2026-11-27T09:20:00Z",
      "writes": "wait", "waiting": 0, "light": "none", "recent": [] } ],
  "waiting": []
}
```

How each part is worked out:
- **Metric → entity.**
  - Match the metric's `source` (and each variant's) on `connection` and `table` against each entity's `source`.
  - No match, or SQL only: `entities: []`. The metric shows on the Map's "Metrics not tied to one entity" shelf and in the List. On Home, the shelf is a chip on the sources tier: "1 metric not tied to an entity".
- **`uses`.**
  - Take the identifiers in `measure` and `filter` (letters, digits, `_`).
  - Drop SQL words (`sum count avg min max distinct and or not is in like between null true false case when then else end`).
  - Keep those that are fields of the matched entity.
  - A field used by a metric gets a Σ on the Map's field row.
- **`recent`.**
  - The entities named by this agent's proposals in the last 7 days: `learned.entity`, plus types whose file is in `files[]`. "saved" when accepted or written straight in; "proposed" when waiting.
  - Direct writes come from the activity file, matched on `file`.
- **Agents seen only in activity** are `via` names that are not an OAuth connection, e.g. "agent via MCP (stdio)". They show as "an agent (stdio)", with `seen: "activity"` and no `last_used`.
  - A `via` that names a connection (`connection "ChatGPT"`, parsed as `agentOf` does today) is that connection, not a second chip.
  - People's lines ("Vinay via page") are never agents.
- **`waiting`.**
  - The 5 newest pending proposals (replaced and set-aside ones excluded), each `review::graph`'s nodes and edges plus `id`, `headline` and `agent`.
  - More than 5: `"waiting_more": N`.
  - Cached in memory by (proposal id, space fingerprint), so a refresh doesn't recompile them.
- **Fields are not in it.** Map's expand fetches `/api/describe?name=` (already cached per load as `DC`).

Size and speed: for 2,000 entities with 8 links each, under 600 KB and under 150 ms on the server, without the waiting overlays.

---

## Part 8 · Waiting proposals on the plane

How a waiting proposal is drawn:
- **New entities** are ghost tiles placed by the same layout, as if accepted. Changed ones get an amber dashed ring; removed ones are struck through with a red ✗ ghost badge. New links are dashed green, removed ones dashed red.
- **Each ghost carries its agent's monogram**, and its card has [Review] → `#review/<id>`.
- **With several waiting**, each proposal's ghosts are drawn. A "Waiting: ChatGPT (2) · Codex (1)" control on the bar shows them all, one, or none. It defaults to all, capped at 5 proposals.
- **The review screen's "Show on the map"** goes to `#review/<id>/map`, as today. That route now renders the Map component with only that proposal's overlay, a bar saying "What this proposal would change · read-only · Back to the review", and the legend in words. `renderReviewMap` is removed.
- **A proposal that no longer loads** (its files fail against today's space) shows no ghosts, plus a line on the bar: "1 waiting proposal can't be drawn: Review it".

---

## Part 9 · The Map, opened up

The same data and layout, drawn flat (Decision 2), with three lanes, so it reads left to right as "rows come from → entities ← metrics count":
- **Sources** in a lane on the left, each chip beside the first entity it serves. Arrows "rows come from" run into the entity's header.
- **Entities** in the middle, by area, at their layout cells (or dragged cells).
- **Metrics** in a lane on the right, each beside its entity. Arrows run to the field rows they use, labelled "adds up" (`measure`) or "leaves out" / "keeps" (`filter`), using `words::filter_words`' direction. A collapsed entity takes the arrows on its header.

### An entity node

- **Collapsed**:
  - glyph, label, light badge, "14 fields"
  - the source grade as a small chip
  - ▾ to expand
- **Expanded** (▾, Enter or double-click):
  - the description
  - then the fields from `/api/describe`: ★ for the key, the name, the type in words (`words::type_word`: "money (USD)", "yes / no", "date"), and Σ when a metric uses it
  - the first 12 fields, then "+ 2 more fields" to show all, scrolling inside the node past 360 px
  - fields without words show "Not described" in amber, with the same inline input as Entities › Fields when edit mode is on
- **Expanding grows the node downward over its neighbours**, raised, without pushing them. Collapsing puts it back.
- **Several can be open at once**, and which are open is remembered for the visit.

### Links

- Labelled with their name and cardinality ("places · many-to-one"). Without a sentence, the label is amber ("no sentence").
- A self-link is a loop. Two links between the same pair are one line with "2 links", split on hover or focus.

### Show and filter

- **Show**: Entities, Metrics, Sources, Links.
- **Only what needs a look**: hides green and no-badge nodes and says "9 hidden". Links to hidden nodes become short stubs.
- **Area**: all, or one.
- **Reset layout.**
- Zoom with the wheel, a pinch, +/−, or 0 to fit. Pan by dragging the background.
- The filters are remembered per space for the visit.

### Dragging

Any node can be dragged to a cell (Part 4), by mouse or touch. With the keyboard, select it and press M, then use the arrow keys and Enter.

---

## Part 10 · Drawing, and how fast

### Structure

```
<svg role="group" aria-label="branda: 1 entity, 1 metric, 1 source, 2 agents; 1 needs a look">
  <g class="camera" transform="translate(..) scale(..)">     ← pan and zoom touch only this
    <g class="tier sources">…</g>
    <g class="plane"> <g class="links">…</g> <g class="tiles">…</g> <g class="ghosts">…</g> </g>
    <g class="tier agents">…</g>
    <g class="between">…</g>                                  ← the dashed lines between tiers
  </g>
</svg>
```

### Projections

- Flat: `x = u·260, y = v·120`.
- Isometric: `x = (u − v)·80, y = (u + v)·40`.
- Tiers sit 160 px above and below the plane's bounding box.
- Glyphs, labels and badges are drawn unskewed at the projected point, so text stays crisp and readable.

### Budgets

| What | Budget |
|---|---|
| tiles in the document at once | ≤ 400 |
| link lines in the document at once | ≤ 800 |
| first paint with 2,000 entities (fetch excluded; layout, levels and drawing) | ≤ 300 ms on a 2020 MacBook Air class machine |
| pan and zoom | change only the camera `transform`, so 60 fps; re-cull and redraw for the new level 120 ms after movement stops |
| a refresh with `same: true` | redraws nothing |

### Look

- Light theme only: tiles white, sides `#ececE8` and `#e0e0db`, plane `#f3f3f0` with the page's dot grid, tiers `#fafaf8` with a hairline.
- Badge colours are the page's `--block`, `--warn` and `--ok`, each with a white glyph at 4.5 : 1 or better.
- Hover lifts a tile 3 px with a soft shadow. A selected tile gets the accent ring.

---

## Part 11 · Keyboard, screen readers, phone, motion

### Keyboard

- The plane is one tab stop, using roving focus.
- Arrow keys go to the nearest drawn item in that direction (tiles, ghosts, and the chips on both tiers).

| Key | Does |
|---|---|
| Enter | open its card |
| Shift+Enter | open its page |
| Esc | close the card, or clear search |
| `+` / `−` | zoom in / out |
| 0 | fit |
| `/` | search |
| L | toggle List |
| P | pin |
| M | move (Map only) |

### Screen readers

- **Every item** is `role="button"` with an `aria-label` that is a sentence: "Promo sale, entity in Types, 14 fields, counted by 1 metric. Look at it: Promo sale revenue has no owner."
- **A polite live region** says what was selected, and how many items appeared or folded after a zoom: "Showing 120 of 2,000 entities".
- **"Skip to list"** is the first focusable thing on Home.

### The List

- A table of everything on the plane: name, kind, area, light in words, reasons, waiting, Open. It is sortable, and the bar's light buttons filter it.
- It is the same data, never a second source of truth.

### Phone width (under 760 px)

- The plane sits at the top, 4 : 3, at the "Top entities" level with a budget of 12 tiles.
- Pinch zooms; one-finger drag pans. Tapping selects and opens the card as a bottom sheet (60% height, draggable, with Close).
- The List shows under the plane, open by default.
- The bar wraps. The legend becomes a "What the colours mean" toggle.
- Touch targets are at least 44 px. The camera never steals vertical page scroll: the plane only pans after a press-and-hold or with two fingers.

### Reduced motion

- `prefers-reduced-motion: reduce` turns off fly-to, hover lift, ghost pulsing and fades. Every jump is instant.

---

## Part 12 · Empty, tiny, huge, broken

| Space | Home shows |
|---|---|
| **0 entities** (a blank space) | The plane with one dashed tile, "Your first entity", and three buttons: [Add an entity] → Entities' new form · [Let an agent learn a file] (a sentence to copy into ChatGPT or Claude, e.g. "Save this file to Lorefile", and a link to Agents › Connect) · [Copy an example]. The tiers say "No source yet" and "No agent has connected yet · Connect". No error look: the picker says "empty" (UI-fixes). |
| **1 entity** (branda) | The tile centred at a larger scale, with a dashed "your next entity" cell beside it, linked by a dashed line labelled "links show here". The tiers as usual. |
| **2 to 60** | Everything, no levels. |
| **Over 60** | The levels (Part 3). |
| **Errors** | The plane draws what compiled. Red badges go on the subjects, and a banner reads "2 errors: agents may get wrong answers · [Problems]". If the space does not load at all, `renderNoSpace` as today. |
| **A link to a missing entity** | A short red stub with a ghost label "missing: Carrier", red ✗ on the linking entity. |
| **A shipped example** | Read-only banner (today's `roBanner`), no inline fixes. Pins and dragging still work, since they are kept in this browser, not in the space. |
| **A read-only server** | No inline fixes; everything else the same. |
| **OAuth off** | Agents tier from activity only, plus "Connect an agent" → Agents. |
| **No activity file** | Agents tier from connections only. |

---

## What this does not do

- **No row values or metric values** anywhere on Home or Map.
- **No new YAML key** (`icon:`, position or importance).
- **No reads logged**: "recent" means saved or proposed.
- **No WebGL or CDN.**
- **No dark theme.**
- **No "all spaces" plane.**
- **No editing beyond** the two one-field fixes and the existing field-description input on Map.
- **No change to MCP tools.** Agents don't see lights in v1.

## Edge cases

| Case | What happens |
|---|---|
| A name with markup in it (a description, an agent name, an also-called word) | Written with `textContent` or `esc`, never as HTML. Tested with `<img src=x onerror=alert(1)>` as a description and as an OAuth client name. |
| A client registers as "Lorefile admin" or "Vinay" | Shown as "Lorefile admin (connection)", monogram L; the card says "a connection's name is chosen by the app that connected". |
| Two entities whose labels read the same (`OrderLine` and `order_line` both read "Order line") | Names are unique, labels need not be. When two labels match, both tiles show the YAML name under the label, and so do the card and the List. |
| A label over 22 characters | Cut with "…" on the tile; whole in the card, the List and `aria-label`. |
| An entity in an area the manifest no longer lists | "Other", as `overview` does today. |
| 50 areas | The area grid; Areas level by default when over 60 entities. |
| An area with 1,500 entities and the rest tiny | Its share of the budget is capped at 60% so the others stay visible; "+1,440 more" on its fold tile. |
| A metric with variants on two sources | A Σ token on each matched entity; the card says "when channel is affiliate" per variant. |
| A metric whose `measure` names a field that doesn't exist | `uses` drops it; the metric's own diagnostic (if any) gives the light. |
| A source no entity uses | Its chip shows "not used yet", no badge. |
| An agent connection revoked | Gone from the tier on the next refresh; its past proposals still name it in activity and the card. |
| An agent with 40 waiting proposals | Badge "! 40"; ghosts for the 5 newest; "+35 more waiting" in its card with [Agents]. |
| A proposal from another space | Never shown here. |
| A dragged entity renamed | Its saved cell is pruned; it goes back to the automatic cell. |
| `localStorage` full or refused | Pins and drags last for the visit; no error. |
| The window resized | Refits only if the person hasn't moved the camera since the last fit. |
| The tab hidden | Polling stops; on return, one `version` check. |
| `/api/plane` fails | The last picture stays, with a quiet "couldn't refresh · Try again" on the bar. |
| `/api/plane` slow (a huge space) | The first paint waits with "Drawing 2,000 entities…"; refreshes keep the old picture until the new one is ready. |
| A space changes between version check and fetch | The fetch returns the newer version; the page draws what it got. |
| Zoomed in on an entity that a refresh folds | The selected entity stays drawn (Part 3). |
| A selected entity removed by a refresh | The card says "Promo sale is no longer in this space" with [Close]. |
| Keyboard focus on a tile that folds | Focus moves to its "+N more" tile, announced. |
| Two people drag on two browsers | Each sees their own layout; nothing shared, nothing to conflict. |
| Browser zoom at 200% | The SVG scales; hit areas and text grow with it. |
| Print | Not designed for; the List prints. |
| Very old browsers | Not supported beyond what the page needs today (ES2020, SVG 1.1). |

## Build notes, file by file

- **`src/validate.rs`**: `Diag.subject` (Decision 28), set at each `r.warn` / `r.error` that names one item, plus the loader's type-file mapping. Serialized as `{ "kind", "name" }` or absent.
- **`src/review.rs`**: `questions` reads `subject` instead of parsing messages. `graph` is reused as is for `waiting`.
- **`src/plane.rs`** (new):
  - `plane(st, space, proposals, oauth, activity) -> Json`, as in Part 7, built from small pure functions, each unit-tested:
    - `light_of`, `reasons_of`, `importance`, `icon_for`, `metric_entities`, `metric_uses`, `agents`, `version`
  - The ghost cache lives in the server state, keyed by (proposal id, fingerprint), dropped when the proposal leaves pending.
  - "Changed this week" with history on is **one** `git log --since=7.days --name-only --format=%ct` per version, mapped from file to items. Never one `git log` per file.
- **`src/serve.rs`**:
  - `GET /api/plane` with `?version=`.
  - Agents come from `oauth.connections_json` filtered to this space (`space` is this id or null) and `proposals::activity`.
  - No write routes are added: inline fixes use `/api/patch`, as the UI-fixes spec does.
- **`src/ui/index.html`**:
  - **A `// ---------- plane ----------` section**: data load and poll, layout, levels, the two projections, drawing, camera, hit and focus, search, the card, the List, ghosts, glyph `<symbol>`s.
  - **Routes**: `#home` (the default when the hash is empty), `#map`, `#review/<id>/map` now on the plane. The nav gets "Home · how things are" above Build, and the header logo goes Home.
  - **Removed**: `layout()`, the old `renderMap` body and `renderReviewMap`. "What to do next" moves to a function both Home and Map use.
- **README.md**: the home page in one paragraph and one screenshot. DEPLOY: nothing.
- **BACKLOG.md**: the "Left for later" items.

### Hand-off plan

1. **plane-data**:
   - `Diag.subject`, `src/plane.rs` and `/api/plane`, with its Rust tests.
   - Nothing in the page.
2. **plane-draw**: the page section, Home and the List, on a 2,000-entity generated fixture and on branda.
3. **map-open**: Map on the same component: expandable nodes, lanes, arrows to fields, filters, dragging, the review overlay.
4. **polish**: phone, keyboard, screen-reader labels, reduced motion, empty and tiny spaces, the README.
5. **live**: Vinay on branda through the tunnel, desktop and phone.

Steps 2 and 3 both edit `index.html`, so they run one after the other. Step 1 can start while the UI-fixes build is in review, since it touches no page code.

## Tests

Rust (`src/plane.rs` unit tests and `tests/plane.rs`):

**Lights**, one test per row of Part 2's table:
- red from an error with a subject
- amber from each of W101, W103, W105, W106, W109, W113, proposed and waiting
- green only when graded and clean
- no badge for ungraded and clean
- a source never green; an agent amber only with waiting proposals
- roll-up: worst of entity, source, metrics and guardrails; areas the worst of their entities; subject-less diagnostics on the space

**branda**:
- PromoSale amber with exactly one reason (W109 via the metric); the metric amber; user_files none.
- After `set_metric` owner: both green.

**Importance**: PromoSale scores 10 with the four reasons. Ties sort by name. The changed-this-week rule uses the commit time with history on, and trust `at` without it.

**Icons**:
- `PromoSale` → receipt ("name: sale"); `Customer` → person; `Warehouse` → pin; `Carrier` → building; `Thing` → cube.
- A `from_user` entity with no matching word → document.
- The first match in table order wins: `CampaignOrder` → receipt.

**Metric links**:
- `promo_sale_revenue` → entities [PromoSale], uses [revenue, refunded].
- A variant metric → both entities. A SQL-only metric → no entities.
- `count(distinct order_id)` → uses [order_id].

**Agents**:
- Two connections with `space` null or branda are shown; one bound to another space is not.
- A stdio `via` from activity shows as `seen: activity`.
- `recent` holds PromoSale as "saved" for ChatGPT.

**Version**:
- Unchanged between two calls; changes after a patch, a new proposal, an activity line, or a connection's use in a new minute.
- `?version=` with the current one gives `{same: true}`.

**Waiting**:
- At most 5, with `waiting_more`.
- A proposal whose files no longer load appears as "can't be drawn".
- The cache is hit on the second call.

**Never rows**: on the readable `supply-chain` example, the `/api/plane` body contains none of the CSV's cell values. Pick three distinctive values from the fixture and assert they're absent.

**Diag.subject**: every warning code in `validate.rs` that names an item sets it. A test runs the shipped examples and a crafted broken space, and asserts every W1xx diagnostic except W112 has a subject. `review::questions` output is unchanged for p-6.

**Size**: a generated 2,000-entity space (8 links each, 4 areas, 50 metrics) gives a `/api/plane` under 600 KB in under 150 ms in release mode. Release only, marked `#[ignore]` in debug.

By hand, in the in-app browser:
- branda, desktop:
  - Home shows the one amber tile, the two agent chips and the source chip.
  - Click the tile, set the owner in the card: green, and the bar reads "✓ 1".
  - "Show on the map" opens Map with the node expanded and arrows from the metric to `revenue` and `refunded`.
- The generated 2,000-entity fixture:
  - Areas at zoom-out, the top-120 fit, everything in view at 1.6, close-up with the Open pill.
  - Search "promo" flies to it.
  - First paint and panning measured in the Performance panel against the budgets.
- A waiting proposal (ask ChatGPT to learn the December file with a new column, or use a fixture proposal): ghosts on Home and Map, Review from the ghost's card, and the review screen's "Show on the map".
- Keyboard only: Tab in, arrows, Enter, Shift+Enter, `/`, Esc, L. VoiceOver reads the tile sentences and the live region.
- 375 px: plane on top, list below, bottom-sheet card, pinch and pan, page scroll not stolen.
- Reduced motion on: no fly-to.
- A shipped example: read-only banner, no fixes. A read-only server: no fixes.
- `<img onerror>` as a description and as an OAuth client name: shown as text.

## Left for later

- **An `icon:` key** (or `tags: [icon:…]`) once the YAML format can add keys without breaking older binaries. A format-version decision.
- **A shared layout file** for teams who want one arrangement in git.
- **What agents read**: in-memory counts of entities named in tool calls, for a "used by agents" signal and lines from agents to what they read. It needs its own privacy decision.
- **Lights for agents over MCP**: `summary` could carry "needs a look" so agents warn before using an amber metric. Today they already say "unverified" for proposed ones.
- **A logic tier** for functions, actions and guardrails.
- **An "all spaces" plane** above the space picker.
- **Metric values** on Home for readable sources. Decision 21 says no; revisit only with a "last computed at" line.

## What changed during the build

Built on `claude/learn-file-6a954e` on top of the UI fixes (`218f983`). All 28 decisions as recommended. Where the build differs from the text above:

1. **The layout walk is best-first, not breadth-first by name.** From the highest-scoring entity, the next placed is always the best-scoring neighbour of what is already placed (ties by name), then the next by score. Linked entities still sit together, and the important ones land in the middle of their area.
2. **Over 60 entities, the fit view has its own compact layout.** In the full layout an area of 500 entities is a spiral some 23 cells wide, so the "top" entities of four areas sat thousands of pixels apart and no zoom at or above 60% could show them together. The "top entities" and "areas" levels therefore draw each area's first-placed entities (the top *k* by the walk, which is near the top by score) close together, with the "+N more" tile after them. At 160% the full layout takes over, and the camera keeps the entity under the centre of the view where it was, so the switch does not jump.
3. **Size: about 1.4 MB, not under 600 KB.** A generated space of 2,000 entities with 8 links each (16,000 links) answered 3.6 MB as first written. Links now go as rows (`[from, to, name, cardinality, has a sentence]`, where from and to are places in `entities`, and to is the missing name for a link to nothing), and empty or zero fields are left out (the page fills them back). That gives about 1.4 MB, 124 KB gzipped. No compression crate is in the build, so gzip is in BACKLOG.
4. **Speed: about 0.30 s in release, not under 150 ms.** As first written it took 0.39–0.49 s. Three scans were quadratic, and each is now done once: each entity's link count scanned all 16,000 links; its area walked every manifest list; each source scanned every diagnostic per entity. For comparison, the existing `/api/summary` takes 0.21 s for the same space. A `?version=` check that finds nothing changed answers in 6 ms, so the 30-second poll costs almost nothing. The test `a_space_of_two_thousand_entities_is_drawn_quickly` is `#[ignore]` and asserts under 400 ms in release. In the browser, layout, levels and drawing for 2,000 entities took 18–37 ms (budget 300 ms), measured with `performance.now()`, not the Performance panel.
5. **A description is cut at 160 characters** in `/api/plane`. The card shows that; the entity page has all of it.
6. **Agents seen only in activity are named by channel**: "an agent (MCP)" and "the page's tool runner". MCP over stdio and over HTTP with the token both write `agent via MCP`, so "an agent (stdio)" could not be told apart.
7. **Rolled-up reasons** read "Promo sale revenue: no owner. Who decides what it means?". The card groups them under "From Promo sale revenue" and drops the repeated name.
8. **Close up (≥ 300%)**: the selected tile shows "14 fields · key line_id" on its face and the "Open Promo sale ›" pill. It does not show the first 6 field names: that would need a describe call per tile while zooming.
9. **Map nodes are HTML inside the SVG** (`foreignObject`), so field lists wrap and the field-description input works. An open node lists its first 12 fields plus every field a metric uses, wherever that field sits, so the metric's arrows always land on a row.
10. **A waiting proposal "can't be drawn"** when it is stale (`review::prepare`) or its files would add errors to today's space. The ghost cache is keyed by space, proposal and the space's fingerprint.
11. **Changed this week**, with history on, is one `git log --since=7.days --name-only --format=%ct`. With history off, it is the binding's trust `at` or an accepted proposal naming the entity.
12. **Search** also covers metrics and sources (each flies to the entity it belongs to).
13. **On a phone the bottom sheet is not draggable.** It opens at 60% height with Close. The plane pans after a press-and-hold or with two fingers, and the List is open under it.
14. **README** has the paragraph, not a screenshot.
15. **Removed**: `layout()`, the old `renderMap` and `renderReviewMap`. "What to do next" is one function (`nextSteps`) for Home and Map. The review screen's "Show on the map" opens the Map with that proposal drawn over it, read-only.
16. **A file-only error** on a `types/*.yaml` file names its entity only when the file defines exactly one (corrected while this was being built: a `types:` list file holds several).
17. **Also fixed on the way**: "What to do next" listed every entity with no data by name. Past four, it now gives the count.

Tests added: `tests/plane.rs` (branda amber then green, each warning's light, proposed, changed since and waiting, the version, at most five waiting with one that can't be drawn, never a row value on the readable `supply-chain` example, every warning naming an item carrying a subject, and the release-only size test), and unit tests in `src/plane.rs` (icons, identifiers, importance, monograms, the worst light, variant and SQL metrics, agents from connections and activity).

Checked by hand in the in-app browser:
- branda (a scratch copy): the amber tile, the card, setting the owner from the card (green, "✓ 1"), and Map with the node open and arrows to `revenue` and `refunded`
- a generated 2,000-entity space: areas when zoomed out, the top 120 at fit, everything in view at 160%, search "promo" (an also-called word) flying to the entity
- a waiting proposal: the dashed ghost with the agent's monogram, amber rings on what it changes, the new link dashed green, and the review's "Show on the map"
- keyboard: one tab stop, arrows, Enter, Esc, `/`, L
- 375 px: plane on top at 4:3, list below, the card as a bottom sheet, no sideways scroll
- reduced motion: fly-to is instant
- a shipped example (read-only, no fixes), a blank space ("Your first entity", no error look), and `<img onerror>` in a description (shown as text)

Not checked: VoiceOver (no screen reader here; the labels and the live region are in place), the agents tier with real OAuth connections (unit-tested only), and branda live through the tunnel.
