# Spec: people instead of typed names, folding panels, areas you can make, and a Graph view

Written 2026-11-27 for Vinay to decide on. **Status: not built.**

Vinay tried the new page (Home, Map, kinds) on the shipped `@b2b-sales` example and on branda. He said, in short:

> this should be users instead of just text shouldn't it? … each sidebar should be made collapsable, how do i create more areas like core? … map doesn't have to be a sidebar item because home already has a tab for it … should it be a graph instead? … i think the 3d is fine and looks good, just the next part if graph can make it more easier to view the links and logic actions guardrails etc

This spec covers five things and the seven problems in his screenshots:
1. **People.** The owner box picks a person from a People list that fills itself. Names only, no logins.
2. **Folding panels.** The left menu, the list column and the right panel fold.
3. **Areas.** Add, rename, describe, reorder and delete areas, and move entities between them, from the page.
4. **Home becomes Overview | Graph | List.** "Map" leaves the sidebar, and the flat Map goes.
5. **A Graph tab** centred on one entity, with every line labelled and logic drawn as its own nodes. Click a neighbour to move there.

**Base:** branch `claude/learn-file-6a954e` at `5bd72ba`. It holds OAuth, learn_file, the review flow, the UI fixes (`218f983`), the home plane (`f47f7bd`) and kinds (`5bd72ba`). 209 tests pass. Nothing is pushed.

"Touches" says where this spec changes the earlier specs:
- [2026-11-27-ui-fixes.md](2026-11-27-ui-fixes.md): decision 14 (grades when an owner is set) is refined
- [2026-11-27-home-plane.md](2026-11-27-home-plane.md): the Map, its drag and its hand-off step are removed
- [2026-11-27-sub-kinds.md](2026-11-27-sub-kinds.md): `format: 2` gains two keys, and a kind follows its parent's area
- [2026-11-27-review-flow.md](2026-11-27-review-flow.md): "Show on the map" becomes "Show on the graph"

**Standing rules, unchanged:**
- The ontology never executes and never holds credentials. Rows never enter the ontology.
- Agents and builders write the YAML; people review it.
- Light theme only. One embedded page, no build step, no CDN.
- Honest status.
- The format refuses unknown keys (`deny_unknown_fields`).

**Read first:**
- `src/ui/index.html`:
  - names and saving: `ME`, `nameOk`, `nameBtn`, `post`
  - owners: `owner()`, `ownerLine`, `saveOwner`, `inlineFix`
  - routing and layout: `renderNav`, `render`, `onHash`
  - Home and Map: `renderPlaneScreen`, `drawFlat`, `toggleNode`, `wirePlane`, `selectItem`, `planeAside`, `planeLayout`, `domainColor`
  - new entities: `newTypeForm`
  - the `header` CSS
- `src/trust.rs`: `things`, `recipe`, `restamp`, `who_said`, `by_words`
- `src/patch.rs`: `add_type`, `set_metric`, `edit_type_file`, `plan_many_over`
- `src/serve.rs`: `overview()`; the route that sends `make_parent` through `plan_many` and `apply_many`
- `src/loader.rs`: `classify` (every `.yaml` in the space must be a known file)
- `src/ontology.rs`:
  - `domain_of`, and around line 522, where a kind takes its parent's area
  - `resolve_iris` (`Key::Domain`), `describe`, `summary_markdown`
- `src/plane.rs`: `diag_reason` (which warnings turn a tile amber), the `areas` list
- `src/rdf.rs` (around line 470: one `skos:ConceptScheme` per area) and `src/iri.rs` (`<ns>domain.<slug>`)
- `src/validate.rs`: E010
- BACKLOG.md:
  - "Accounts and single sign-on"
  - "Named, signed-in people"
  - "`iri:` … on domains"

---

## Before and after

### Naming an owner

**Before.** On the Logic card, Promo sale revenue says "Owner:" with a box already holding "Vinay", because the page puts the kept name in it. It looks saved. It isn't, so live branda still has no owner. Elsewhere you type free text, so "Vinay", "vinay" and "Vinay Karode" become three owners.

**After.**

```
Owner   no one named yet      [ Name an owner ▾ ]
```

Clicking opens a list:

```
┌──────────────────────────────────────────────┐
│ Find a person or team…                       │
│ You                                          │
│   Vinay Karode (you)                         │
│ People here                                  │
│   Priya            owns 2 metrics            │
│ Teams here                                   │
│   Growth team      owns 5 guardrails         │
│ From your other spaces                       │
│   Sam                                        │
│ + Someone new…                               │
└──────────────────────────────────────────────┘
```

Picking someone shows the change before anything is written:

```
Make Vinay Karode the owner of Promo sale revenue?
Saving also accepts this metric as it is now, in your name.
[ Save as Vinay ]  [ Cancel ]
```

After Save, the line reads `Owner  Vinay Karode · Change`.

A new **People** screen lists everyone the space mentions: who owns what and who decided what. Spellings that differ only in capitals or accents count as one person. It offers "Same person?" when one name looks like another ("Vinay" and "Vinay Karode").

### Making an area

**Before.** Areas live only in `ontology.yaml` (`domains:`). The new-entity form offers the existing areas as chips and nothing else.

**After.**
- **The Overview bar** has **+ New area**.
- **Clicking an area's name** on Overview opens its card: rename, describe, the entities in it with **Move here…**, and **Delete** (only once it is empty).
- **An entity's card and page** say `Area: Pipeline · Move…`.
- **The new-entity form's chips** end with **+ New area**.

### Seeing links, logic and guardrails

**Before.** Home draws every entity; the Map draws every entity flat. Link labels pile up, and on the Map "Show on the map" lands with Quote off-screen and everything else faded.

**After.** Home has three tabs: **Overview** (the 3D drawing, unchanged in spirit) | **Graph** | **List**. The Graph for Quote in `@b2b-sales` looks like this:

```
                         LOGIC
 ┌⛉ QT-001 over $250k┐ ┌⛉ QT-002 discount┐ ┌⛉ QT-003 floor┐ ┌⛉ QT-005 draft┐ ┌ϟ Submit quote ⛉1┐
 └─────────┬─────────┘ └────────┬────────┘ └──────┬───────┘ └──────┬───────┘ └────────┬────────┘
           └────────────────────┴───────┬─────────┴────────────────┴──────────────────┘
 LINKS IN                     ┌─────────┴─────────┐                         LINKS OUT
                              │ ▣ Quote       ●   │   for opportunity          ┌ Opportunity ┐
  Nothing links to Quote.     │ 8 fields · key    │ ─────────────────────────▶ │  Pipeline   │
                              │ ★ quote_id        │   many quotes → one opp.   └─────────────┘
                              │   amount_usd      │
                              │   discount_pct    │
                              │   …               │
                              └─────────┬─────────┘
 ROWS AND FAMILY                        │
                     ┌⛁ cpq_snowflake: agents fetch rows ┐
```

Every line has its words in its own row, so no two labels can overlap. Clicking Opportunity makes it the centre, and the browser's Back button returns to Quote.

### Panels

The left menu folds to a rail of icons (`[`), and the right panel folds to a thin strip (`]`). The list column on Entities, Files and Actions folds too. Each stays the way you left it, in this browser.

---

## Decisions

Each row gives the options, with the recommended one in bold.

| # | Question | Options | Recommendation |
|---|---|---|---|
| 1 | What a person is | **A. A name, nothing more.** The list is worked out from what the space already says, plus `people.yaml` for what can't be. B. Accounts with sign-in now. C. Free text, as today | **A.** B is the BACKLOG item "Accounts and single sign-on" (large). A's screens stay when B comes: B only swaps "typed name" for "signed-in person". |
| 2 | Where names come from | **A. Owners (entities, metrics, functions, guardrails), `trust.yaml` (`by`, `distinct[].by`), the activity log, and git authors (name only, never the email) when history is on.** B. Owners only | **A.** "Managed automatically" means nobody types a list. |
| 3 | Where the list lives | **A. Worked out on every load. `people.yaml` at the space root holds only what can't be worked out: other spellings (`also`), team or person, "left", and pairs that are not the same person. Written by Lorefile only and committed with the space, like `trust.yaml`.** B. `.lorefile/people.json` (local, not in git). C. New keys in `ontology.yaml` | **A.** A teammate's copy shows the same merges. A file of its own keeps diffs readable. `.lorefile/` is hidden from git, so B's merges would stay on one laptop. |
| 4 | Per space or for the folder | **A. Per space. The page's picker also suggests names from the other spaces this server serves ("From your other spaces"), never shipped examples, and never to agents.** B. One list for the whole folder | **A.** Spaces can be different clients, and an OAuth connection scoped to one space must not learn another's people. |
| 5 | What a person record holds | **A. A name, other spellings, team or person, and "left" (a date). No email, login or role.** B. Add email | **A.** Names only, as Vinay asked. Team members are left for later. |
| 6 | Agents | **A. Never listed as people.** Anything said "via MCP", by a connection, or containing "agent" stays under Agents. | **A**, the same rule `who_said` and `Who::of` use today. |
| 7 | The owner box | **A. A picker.** "You" comes first, then people here, teams, other spaces, and "Someone new…". It is empty until you pick. Picking shows a Save line saying what saving also does. Never pre-filled. B. Save on pick, with Undo. C. A typed box with suggestions | **A.** It fixes screenshot problem 7. B is quicker, but the side effect (accepting the metric) must be read first. |
| 8 | Where owners can be set | **A. Everywhere an owner exists**, with one new op, `set_owner {kind, name, owner}`: <br>- metrics: Logic card, Home card, review question <br>- guardrails: Guardrails screen <br>- entities: the entity page, a new Owner line <br>- functions: Logic card <br>B. Metrics only | **A.** Action approvers (`requires_approval_from`) are not owners: in every shipped example they are sentences with conditions ("Deal Desk above $250k; VP Sales above 20% discount"), so they stay free text and are never read as people. |
| 9 | Grades when only the owner changes (refines ui-fixes decision 14 as built) | **A.** A checked or accepted grade stays exactly as it was: same who, when and note, with a fresh fingerprint. An unverified, changed-since or ungraded one becomes accepted by the person saving, and the Save line says so first. B. As built: an accepted grade is accepted again in the setter's name | **A.** Who stands behind a definition shouldn't change because someone else named its owner. As built, the page warns for unverified and changed-since, but an ungraded metric also becomes accepted, silently (`restamp` gives `fresh(Accepted)` to any changed thing with no grade). |
| 10 | Rename and merge | **A. One action on the People screen.** Every owner field that says the old name, or one of its spellings, is rewritten. The old spellings go into `also`. `trust.yaml`, history and the activity log stay as written; the page shows them under the new name ("recorded as Vinay"). Renaming to an existing name merges. B. Rewrite `trust.yaml`'s `by` too | **A.** History is never rewritten (the signing rule: "only replaces `person via …`, never renames"). |
| 11 | Grades on rename | **A. Every grade whose item changed only in its owner keeps its record, with a fresh fingerprint, whatever the grade: unverified stays unverified. No new grade is made. A changed-since grade stays changed-since.** B. Treat it as an ordinary person's save | **A.** A rename is a spelling fix, not a review. B would accept every ungraded metric the person owns, in the renamer's name. |
| 12 | Spellings | **A. Names differing only in capitals, spaces or accents are one person (shown in the most used spelling), with nothing written. "Same person?" is offered when one name is the first word(s) of another, when two names match once spaces and punctuation are removed ("vinaykarode", a machine's git name, and "Vinay Karode"), or when two names of 5+ letters differ by one letter. Never merged without a click; "No, two people" is remembered.** B. Merge automatically | **A** |
| 13 | Teams and addresses | **A.** A name is a team when `people.yaml` says so, when it holds the word "team", or when it is an address (`finance@northstar.example` reads "Finance team", as today). Teams own things like people do; a team can't be "Your name". Existing addresses keep working. "Someone new…" takes names only, no `@`. `people.yaml` can say an address is a person. B. No teams | **A.** Every shipped example's owners are team addresses. |
| 14 | "Left" | **A. Hides a person from the picker. What they own reads "Sam (left)", and "What to do next" says "3 metrics are owned by someone who left". History is unchanged; "Back" undoes it.** B. Delete the person | **A.** A name in history can't be deleted. |
| 15 | What agents see | **A.** `describe` keeps `owner` as written and adds `owner_kind` (`person` or `team`), plus `owner_is` when the written name is another spelling. `summary` gets one "Owners" line (up to 12 names). An agent's owner with another spelling gets W119; a name nobody here has gets a "new name" line on the review screen. Agents can't edit `people.yaml`. B. Nothing | **A** |
| 16 | "Your name" | **A. Still typed and kept in this browser. The sign-in page and Settings suggest this space's people. Typing another spelling of someone offers the main one.** B. A picker | **A.** Your name must work before a space has any people. |
| 17 | What folds | **A. The left menu (to a 56 px icon rail), the right panel (to a 40 px strip), and the list column on Entities, Files and Actions (to a strip naming the open item).** Each is kept per browser, and phones keep today's layout. B. The left and right panels only | **A.** Vinay asked for "each sidebar". |
| 18 | A folded right panel and a selection | **A. The panel stays folded. The strip shows what is selected (its glyph and name) and how many "What to do next" items there are; clicking opens it.** B. It opens itself on every selection | **A.** It respects the choice. B is the realistic alternative if A feels hidden in use. |
| 19 | Keys | **A. `[` folds or opens the left menu, `]` the right panel**, when not typing and with no modifier key. The list column has its button only. | **A.** Neither key is used today. |
| 20 | The header (problem 6) | **A. One line, always.** Below 1280 px the counts hide; below 1100 px the meter hides; below 1000 px Reload and Summary.md move into a "⋯" menu. The space name shortens with "…". | **A** |
| 21 | Where areas are managed | **A. On Home and the entity page.** <br>- The Overview bar: **+ New area** <br>- An area's name on Overview: opens its card <br>- The entity card and page: "Area: … · Move…" <br>- The new-entity form: **+ New area** among its chips <br>B. A Settings › Areas screen | **A.** Areas are seen on Home, so they are made there. |
| 22 | Area ops | **A. `add_area`, `rename_area`, `describe_area`, `move_to_area`, `order_areas`, and `delete_area` (only when empty).** All are available to agents through `propose_patch`, reviewed as usual. B. Page-only endpoints | **A.** An agent that learns a file may want an area; a person still accepts it. |
| 23 | How a move is written | **A. Where the entity's area is said today: its file's `domain:`, its area's `types:` list, or both. An entity in no area gets `domain:` in its file, as `add_type` does. Any other area's list naming it is cleaned in the same save.** B. Always `domain:` in the entity's file | **A.** It keeps each space in its own style: the shipped examples use lists, page-made entities use `domain:`. |
| 24 | Kinds | **A. A kind with no area of its own follows its parent, as compiled today. Moving the parent moves it, and the confirm names it. Moving a kind on its own writes its own area, and it stops following.** | **A** |
| 25 | Rename and IRIs | **A. The area's derived IRI follows the new name (`<ns>domain.sales`). The confirm says so when the space sets its own `namespace:`. Entity IRIs never include the area, so they don't change. `iri:` on areas stays in BACKLOG.** B. Pin the old IRI with a new `iri:` key | **A.** Lorefile's own RDF import reads areas by name, so a round trip still works. |
| 26 | Colours | **A. By position, as today. An op that would recolour an area (deleting one that isn't last, or reordering) writes `color:` on each area whose colour would change, from eight named colours.** The 5th palette colour (`#b45309`, the same as the "look at it" amber) becomes slate `#475569`. B. Always write colours. C. A hash of the name | **A.** Most spaces never need the key. C collides about one time in three with three areas. |
| 27 | Compatibility | **A. `color:` on areas and `people.yaml` join `format: 2`, which Lorefile writes in the same save (0.2.0 isn't released, so format 2 can still grow). W118 covers a hand edit without it. If this ships after a 0.2.0 tag, they become format 3.** | **A.** It is the same mechanism as sub-kinds decision 15. Plain area ops (add, rename, move) write no new key, so 0.1.0 can still read the space. |
| 28 | Empty areas | **A. Drawn on Overview as a dashed frame: "Sales · no entities yet · Move entities here".** B. Hidden, as today | **A.** Otherwise "+ New area" appears to do nothing. |
| 29 | "Other" | **A. Reserved:** refused as a new area's name (any case). Entities in no area keep showing under Other. An existing area already called Other keeps working. | **A** |
| 30 | Areas that don't add up | **A. W120, a warning:** an entity names an area `ontology.yaml` doesn't list, is listed under one area but says another, or an area's `color:` isn't one of the eight. | **A.** Today a typo in `domain:` silently makes a new area. |
| 31 | Map in the sidebar | **A. It goes.** Home has three tabs: Overview \| Graph \| List. `#map` opens the Graph. The review screen's map opens the Graph with the proposal drawn over it. | **A**, as Vinay asked. |
| 32 | The flat Map | **A. Removed: its drawing, drag, "M" move key and saved positions (`lorefile-layout:<space>` is deleted on first load).** The Graph shows one entity opened up; the List keeps the whole space as rows. B. Keep it as a fourth tab | **A.** Two whole-space drawings is one too many, and four of the seven problems go with it. |
| 33 | One selection | **A.** Overview, Graph and List share the selection (`pl.sel`). Selecting an entity on Overview, then opening Graph, centres it. On Overview, Esc or empty space deselects. In the Graph, they go back to the centre. | **A** |
| 34 | The Graph's shape | **A. Centred on one entity:** <br>- left column: links in <br>- right column: links out <br>- band above: logic (metrics, functions, guardrails, actions) <br>- band below: rows and family (source, parent, kinds) <br>Each label has its own row. B. A whole-space force graph. C. Radial | **A.** Labels can't collide by construction, and the same space always draws the same way. B is a hairball past about 100 entities. |
| 35 | Labels | **A. Every line labelled: the link's name in words, and its cardinality in words ("many quotes → one opportunity").** The sentence shows on hover or focus and in the card. "No sentence" is shown in amber. | **A** |
| 36 | Clicking | **A.** Clicking an entity moves the centre to it (250 ms; instant with reduced motion) and selects it. Clicking anything else selects it, and the card shows it. Each centre is an address (`#graph/Quote`), so Back and Forward work, and breadcrumbs show the last 6 centres. | **A** |
| 37 | Many neighbours | **A. Up to 12 per column. More are grouped by link name ("+14 via contains"), and a group opens as a list in the card. "Show all" draws up to 60 per column.** | **A** |
| 38 | One step further | **A. "+ one more step" draws each neighbour's own neighbours: up to 6 each and 60 in all, smaller, labelled on hover. Off by default; remembered in this browser.** | **A** |
| 39 | Phone and screen readers | **A. Below 760 px the Graph is the same content as rows: the centre, then Links out, Links in, Logic, and Rows and family. Each row moves the centre.** The drawing has the same rows as its accessible twin. | **A** |

---

## Part 1 · People

### Where names come from

`src/people.rs` (new) works out the list for one space on every `/api/people` call. The result is cached per space version plus `.git/HEAD`.

| Source | What is read | Notes |
|---|---|---|
| Owners | `owner:` on entities, metrics, functions and guardrails | As written. Never filtered by the "agent" rule: an owner is an owner. |
| `trust.yaml` | `by` on every record, and `distinct[].by` | Through `trust::who_said`. "a person" (anonymous) and "an agent" are skipped. |
| Activity | `.lorefile/activity.jsonl` `who` | Only on the server that holds it. Used for "last seen". |
| History | `git log -500 --format=%an` when history is on | Through `who_said` ("Vinay via page" → Vinay). Never `%ae`. "Lorefile" (the author on a machine with no git identity) is skipped. |
| This browser | "Your name" | On the page only. It shows as "You", and only to you. |

**One person.** Two names are one person when they match after trimming, folding spaces, lower-casing and removing accents. The name shown is the spelling used most often; ties go to an owner field's spelling, then alphabetical order.

**Team or person.** In order:
1. `people.yaml`'s `team:` wins.
2. A name containing `@` is a team, shown with today's `owner()` rule: "Finance team".
3. A name containing the whole word "team" is a team.
4. Anything else is a person.

**Answer** of `GET /api/people` (the page only; agents get `describe` and `summary`):

```json
{
  "people": [
    { "name": "Vinay Karode", "kind": "person", "also": ["Vinay"], "left": null,
      "owns": { "metric": ["promo_sale_revenue"], "guardrail": [], "entity": ["PromoSale"], "function": [] },
      "decided": 4, "last_seen": "2026-11-27", "seen_in": ["owner", "trust", "history"] }
  ],
  "unowned": { "metric": ["refund_rate"], "guardrail": ["BR-002"] },
  "same_person": [["Priya", "Priya Shah"]],
  "other_spaces": ["Sam"],
  "editable": true
}
```

- `decided` counts grade records in `trust.yaml` and `distinct:` lines.
- `other_spaces` holds names from the other spaces this server serves, up to 200. It never draws on shipped examples (their owners are made-up team addresses).

### `people.yaml`

It sits at the space root and is written by Lorefile only, from the People screen. It exists only once there is something to keep.

```yaml
# people.yaml: who is who in this space. Written by Lorefile from the People screen.
# Names only: no emails, no logins.
people:
  - name: Vinay Karode
    also: [Vinay, vinay k]         # other spellings found in owners and history
  - name: Growth
    team: true
  - name: Sam
    left: 2026-11-27               # not offered as an owner any more; history keeps the name
not_same:
  - [Priya, Priya S]               # "No, two people": the suggestion doesn't come back
```

**Loader.** A file whose top key is `people:` is the people file. Anywhere else, or under another name, it is refused, with the same sentence `trust.yaml` gets. `types/people.yaml` in `@b2b-sales` holds `types:` and is unaffected. The structs are `PeopleFile { people, not_same }` and `Person { name, also, team, left }`, all with `deny_unknown_fields`, so an `email:` key is an error.

**W121** (a warning about the file itself):
- one spelling under two people
- an `also` that is another entry's `name`
- a `left` that isn't a date
- a `not_same` pair naming one person twice

### The People screen (`#people`, in the sidebar under Use, after Agents)

```
People                                                   [ Find a person… ]
Who owns and decides things in branda. Names only: Lorefile never asks for emails
or logins. This list fills itself from owners, grades and history.

  Vinay Karode   person  owns 1 metric · 1 entity        decided 4 · last 27 Nov   [⋯]
                 also written: Vinay
  Growth team    team    owns 5 guardrails                                         [⋯]
  Finance team   team    owns 2 metrics   (written finance@northstar.example)      [⋯]
  Priya          person  decided 2 · last 3 Nov · owns nothing                     [⋯]
  Sam (left)     person  owns 3 metrics                                            [⋯]

  Same person?  "Priya" and "Priya Shah"   [ Yes, merge ]  [ No, two people ]

  Nobody owns 2 metrics and 4 guardrails.  [ Give them owners ]
```

**Each row's menu (⋯):**
- **Rename or merge…** (the confirm counts what changes)
- **This is a team / This is a person**
- **Left / Back**
- **See what they own**, which lists each item with a link

**Give them owners** opens a list of unowned items, each with a picker, and saves them as one change (up to 40, the batch limit).

**On a read-only or shipped space** the screen shows the list with no menus. Teams, persons and "left" are still marked from `people.yaml`.

### The owner picker

It is one component used everywhere an owner is shown: the Logic card, the Home card, the review question, the Guardrails screen, the entity page and function cards.

- **Groups**, in order:
  1. **You.** Your kept name. When it is another spelling of someone, the main name, marked "(you)".
  2. **People here.** Persons who haven't left, ordered by how much they own, then by name.
  3. **Teams here.**
  4. **From your other spaces.** Shown on the page only.
  5. **+ Someone new…**, a box with the owner name rule.
- **Searching.** A Find box filters the list, and arrow keys and Enter move through it. A name typed into Find that matches nobody becomes "+ Add "Priyanka"".
- **Owner name rule.** It is the page's name rule plus `&` and `/` ("R&D", "Sales/Ops"): up to 60 characters, never "agent" or "via", and no `@`. The server checks the same rule in `set_owner`.
- **After a pick**, an inline Save line:
  - The question: "Make Vinay Karode the owner of Promo sale revenue?"
  - What saving does, when the grade will change (decision 9): "Saving also accepts this metric as it is now, in your name." It appears for unverified, changed-since and ungraded. It is never shown for entities and functions, which carry no grade.
  - The Save button is a `nameBtn`: "Save as Vinay", or "Type your name to save" when no name is kept. It needs a name because saving can accept the metric in your name. Cancel closes it.
- **With an owner set:** "Owner: Vinay Karode · Change".
  - A name written as another spelling reads as the main name, with "written as Vinay" on hover.
  - Someone who left reads "Sam (left)".
  - **Remove owner** sits behind Change, with a confirm. Grades follow decision 9.
- **On read-only or shipped spaces:** text only.
- **No prefilled boxes.** The three places that fill an owner box with your name stop doing so: `ownerLine`, `inlineFix` for owners, and the review screen's `prefilled` owner fix.
- **No refresh mid-pick.** While a picker is open or has focus, Home's 30-second refresh redraws the drawing but not the card.

### Grades when an owner changes (decision 9)

This goes in `trust::restamp`, for `Who::Person` only.

**When the change is owner-only:** before and after are equal once `owner` is taken out of both, and the record matched before. Then:
- checked or accepted: keep the record; replace only `of`
- unverified, changed-since, or no record: `fresh(Accepted)`, as today. The page has said so before Save.

**Agents: unchanged.** Any change gives Proposed, and `would_overwrite_graded` still holds an agent's change to an accepted or checked item for a person.

**Rename mode** (`restamp_keep`, used only by rename and merge):
- each item whose change is owner-only and whose record matched before keeps that record, with a fresh `of`, whatever its grade
- items with no record get none
- a record that was already changed-since is left untouched, so it stays changed-since and the earlier change isn't laundered

### Rename and merge

`POST /api/people {op: "rename", from, to, by}`:

1. `from` is a person as shown (their main name). `to` must pass the owner name rule. If `to` is already someone, this is a merge, and the confirm says "merge".
2. **Fields.** Every owner field equal to `from` or one of its spellings is set to `to`. The edit changes only that line (`owner: X` within the item's block), which keeps comments and order. A flow-style item such as `{ id: X, owner: Y }` falls back to the mapping rewrite, which loses comments in that file. The confirm names those files first.
3. **`people.yaml`.** `to`'s entry gains `from` and `from`'s spellings in `also`; `from`'s entry goes. `format: 2` is written if this is the first `people.yaml`.
4. **`trust.yaml`.** Rename mode, as above.
5. **Saving.** One save through `apply_many`, which makes one commit when history is on: "Vinay renamed Vinay to Vinay Karode: 1 metric, 1 entity". One activity line. Undo in History reverts all of it.
6. **Refusals:**
   - a file to be edited doesn't parse (the file is named)
   - `to` equals `from`
   - `to` breaks the rule
   - a shipped space

The other menu items (team, person, left, back, not the same) write only `people.yaml`.

### What agents see (decision 15)

- **`describe`** of an entity, metric, function or guardrail: `owner` as written, plus `owner_kind`, plus `owner_is: "Vinay Karode"` when `owner` is another spelling.
- **`summary`:** one line, `Owners: Vinay Karode (1 metric, 1 entity), Growth team (5 guardrails), …`, up to 12 names, then "and N more".
- **W119**, on the item: "metric promo_sale_revenue: owner `Vinay` is another spelling of Vinay Karode (people.yaml)". The fix op sets the main name. It turns nothing amber: a spelling is tidiness, not a risk to answers. It shows in Problems and on the People screen.
- **The review screen** says "New name: nobody here is called Priyanka yet" when a proposal names an owner the space has never mentioned. That is a note, not a question.

---

## Part 2 · Folding panels, and the header

**The left menu** (`nav`, the 240 px grid column):
- **Folded:** the column is 56 px. Each step shows a glyph with a tooltip and an `aria-label`. Warn dots and the pending-count badge become small corner badges. Headings and the bottom hint hide. The Find box becomes a search button, which opens the menu and focuses the box.
- **New glyphs** in `GLYPHS`: `home`, `gear` and `clock`. The others are reused:
  - Sources: cylinder
  - Entities: cube
  - Links: link
  - Logic: chart
  - Guardrails: shield
  - Actions: bolt
  - How to use it: chat
  - Agents: plug
  - People: people
  - History: clock
  - Data & SQL: sheet
  - Files: document
  - Settings: gear
- **The fold button** sits at the foot of the menu: "« Fold", or "»" when folded, with `aria-expanded`.
- **The default**, before any choice is kept: folded below 1100 px wide, open above.

**The right panel** (`.aside`: Home's card, and the Guardrails side panel):
- **Folded:** a 40 px strip with "«" and a vertical label: the selected item's glyph and name, or "What to do next · 3".
- On Home, folding or opening refits the drawing only if the camera hasn't been moved, the same rule as a window resize.

**The list column** (`.list`, on Entities, Files and Actions): it folds to a 40 px strip showing the open item's name vertically.

**Shared rules:**
- **Kept** in `localStorage` `lorefile-panels` = `{nav, list, aside}`, inside try/catch. Where nothing can be kept, everything opens.
- **Motion:** width changes take 150 ms, none with reduced motion.
- **Below 760 px** the fold buttons are hidden and today's phone layout stands (a menu row, a bottom sheet).

**The header** (problem 6). Today it has `flex-wrap: wrap` in a 52 px grid row, so Reload and Summary.md wrap below the bar and get cut ("eload"). Instead:
- `flex-wrap: nowrap`
- the name and space picker shorten with "…" (the picker keeps at least 140 px)
- the counts hide below 1280 px, and the meter below 1100 px
- below 1000 px, Reload and Summary.md move into a "⋯" button: `aria-expanded`, items as buttons, Esc closes
- the mode pill shortens to "edit" or "read-only" below 1000 px

---

## Part 3 · Areas

### The ops

| Op | Fields | Writes | Refused when |
|---|---|---|---|
| `add_area` | `name`, `description` | appends `{name, description, types: []}` to `domains:` | <br>- the name breaks the area name rule <br>- the name is taken (any case) <br>- the name is "Other" <br>- the name gives the same IRI as an existing area ("Sales Ops" and "Sales & Ops" both make `domain.sales-ops`) |
| `rename_area` | `area`, `to` | the `domains:` entry, and every `domain: <old>` line in entity files (line edit) | the same rules as `add_area`. A change of capitals only is allowed. |
| `describe_area` | `area`, `text` | the entry's `description` | |
| `move_to_area` | `type`, `area` | see "How a move is written" | <br>- no such area or entity <br>- a shipped space |
| `order_areas` | `order: [names]` | reorders `domains:`, and writes `color:` where needed (decision 26) | `order` must name every area exactly once |
| `delete_area` | `area` | removes the entry, and writes `color:` on later areas where needed | anything is in it: a `types:` entry, an entity whose `domain:` says it, or a kind that follows a parent in it. The message names them: "Pipeline still holds Quote, Stage and 3 more". |

- **Area name rule:** 1 to 60 characters, made of letters, digits, spaces and `& - ' . / ( ) ,`; trimmed.
- **Several files.** `rename_area`, `delete_area` and many moves touch several files, so they go through `plan_many` and `apply_many`, like `make_parent`. The serve route that special-cases `make_parent` becomes "any op whose plan touches more than one file".
- **Many entities at once.** "Move here…" on an area card sends one batch of `move_to_area` ops, up to 40 (the batch limit).

### How a move is written (decision 23)

Moving an entity to area B:
- **If its file says `domain: A`**, that line becomes `domain: B`, a line edit that keeps comments.
- **If area A's `types:` lists it**, it leaves A's list and joins B's.
- **If both are true, both change**, which also clears a W120.
- **If neither is true** (it was under "Other"), `domain: B` is added to its file, as `add_type` does.
- **Any other area's list naming it** is cleaned in the same save.
- **Kinds:**
  - A kind of it with no area statement of its own moves with it. The confirm names it: "PromoSale follows Sale and moves too".
  - A kind that says the same area A explicitly is offered: "Also move WholesaleSale (also in Pipeline)?".
  - Moving a kind on its own writes the kind's own area, and the confirm says "PromoSale stops following Sale's area".

**Deleting an entity** (the page's Delete on a types file) also removes its name from any area's `types:` list, in the same save. Today that list keeps the name, which makes E010 ("domain lists unknown type"). The save gate then probably refuses the delete; check this first with a test on a copied example.

### Colours (decision 26)

`domainColor` reads the area's `color:` first, then falls back to its position, as today.

**The eight names**, in palette order:

| Name | Colour |
|---|---|
| `blue` | `#2b5cff` |
| `teal` | `#0f766e` |
| `violet` | `#7c3aed` |
| `pink` | `#db2777` |
| `slate` | `#475569` (was `#b45309`) |
| `sky` | `#0369a1` |
| `green` | `#4d7c0f` |
| `rose` | `#9f1239` |

- **Which ops write `color:`.** Only `order_areas` and `delete_area`, and only on areas whose position colour would change. `add_area` appends, so nothing shifts. `rename_area` keeps the position.
- **Where it shows.** `overview()` and `/api/plane` send `color` with each area.

### On the page

- **The area card on Overview.** Clicking an area's name selects `area:<name>`, flies to it as today, and opens the card:
  - the name and description, each editable inline
  - its colour
  - the entities in it: up to 20, then "+N more"; each opens its own card
  - **Move here…**, a multi-select of entities from other areas
  - ↑ ↓ to reorder
  - **Delete area**, enabled only when the area is empty, with the reason otherwise
  - W120 lines for this area, if any
- **The entity's card and page.** "Area: Pipeline · Move…" opens a list of areas plus "+ New area…". A kind that follows its parent reads "Area: Pipeline (follows Sale)".
- **The new-entity form.** "+ New area" is the last chip. It asks for a name inline, and `add_area` and `add_type` go as one batch.
- **Empty areas** (decision 28). `planeLayout` keeps areas with no entities as a 2×1 dashed frame at the end of the grid, with "Move entities here" when editable.
- **Read-only and shipped spaces.** The card shows, with no buttons.

### Review words, and what agents are told

`review::words` gains one sentence per op:
- "Adds the area Sales."
- "Renames the area Core to Sales (its IRI becomes …/domain.sales)."
- "Moves Quote from Pipeline to Sales."
- "Deletes the empty area Old."
- "Reorders the areas."

`review::compare` diffs `domains:`, which today only diffs a type's own `domain`.

**W120** shows in Problems and on the area card and entity card. It turns nothing amber, because it is tidiness that agents don't misread. `propose_patch`'s description lists the six ops.

---

## Part 4 · Home: Overview | Graph | List

### Tabs and addresses

| Address | Shows |
|---|---|
| `#home` | Overview: today's isometric drawing |
| `#graph` | The Graph, centred on the selected entity, else the last centre, else the most important entity (`importance`, ties by name) |
| `#graph/Quote` | The Graph centred on Quote |
| `#list` | The List: today's rows, now a tab of its own. `L` switches to it and back. |
| `#map`, `#map/…` | Aliases for `#graph` (old links and bookmarks) |
| `#review/<id>/map` | Alias for `#review/<id>/graph` |

- **The sidebar's Home** is active on all three tabs.
- **The tab bar** reads `Overview | Graph | List`, with a hint line beneath:
  - Overview: "Everything at once. Click something to see it in the card."
  - Graph: "One entity in the middle, everything it touches around it. Click a neighbour to move there."
- **After creating a space**, the page opens `#home` (`createSpace` opens `#map` today).
- **On the Overview card**, "Show on the map" becomes **Open in graph**, which goes to `#graph/<name>`.
- **On the review screen**, "Show on the map" becomes **Show on the graph**.

### What leaves with the Map

These are removed from `index.html`:
- `drawFlat`, `flatBounds` and `NODE_W`'s Map use
- `toggleNode` and the Map's open nodes
- drag (`dropNode`, the pointer-capture path), the M move key and "Reset layout"
- the Show checkboxes and the Area select
- the `.node`, `.edge`, `.elabel` and `.mn` styles that only the Map used. The Graph reuses `.mn` for its centre box.

**Saved positions.** `planeLayout` stops reading `pl.layout`, and the page deletes `lorefile-layout:<space>` keys on first load. Pins (`lorefile-pins:<space>`) stay; they belong to Overview.

### Overview changes

- **Area names** select the area card (Part 3), and empty areas are drawn.
- **Link labels** (problem 3, the 3D part). Today they show only for the selected entity, at each line's midpoint, with nothing to stop overlap. Instead:
  - each label tries the midpoint, then 35% and 65% along its line
  - a label whose box would overlap one already placed is left out (its line keeps a `<title>`)
  - with more than 6 links, the first 6 by importance are labelled, and the card says "+N more links: Open in graph"

---

## Part 5 · The Graph

### Layout

The layout is the same on every draw: no physics, no randomness. Coordinates are in drawing units, and the camera pans and zooms as on Overview.

- **Centre.** A box 260 wide at the middle. It holds:
  - the glyph, label and light
  - "N fields · key"
  - the fields: the first 12, plus every field a metric uses
  - "+N more fields" (fold and unfold)
  - a Σ in each row a metric uses
  - a "Not described" box in each undescribed row, when editable
  - its top border in the area colour
- **Links out** (right column) and **links in** (left column). Neighbour boxes are 200 × 52, rows 64 apart, centred on the centre box. Each line leaves the centre's side at its own point, spread evenly along the edge. It runs level through a 300-unit label lane, then curves into the neighbour.
- **Label lane.** The label sits above the level part, in the neighbour's row:
  - line 1, the link's name in words ("for opportunity")
  - line 2, muted: the cardinality in words
  - each line cut at 32 characters with "…"
  - a ⛉ count when guardrails apply to that link

  **No label can overlap another, because each row owns its lane.**
- **Logic** (the band above). Chips are 180 × 44, centred, 6 to a row. Lines run to the centre's top edge.

  | Order | Chip | Glyph | What it says |
  |---|---|---|---|
  | 1 | metrics counting it | Σ | label, and the sentence on line 2 |
  | 2 | functions taking it as an input | ƒ | |
  | 3 | guardrails on it or its ancestors | ⛉ | id and name; "from Sale" when inherited |
  | 4 | actions involving it | ϟ | a ⛉ count for guardrails on the action |

- **Rows and family** (the band below):
  - **the source:** ⛁ name and words
    - for a `where` kind: "Sale's rows where kind is promo"
    - for inline rows: "inline rows (3)"
    - for none: "No data yet · Bind a table"
  - **the parent:** a dotted line, "a kind of"
  - **the kinds:** dotted lines, "kinds"
- **Lights.** Every entity, metric, guardrail and source node carries its light badge from `/api/plane`.
- **Fit.** The camera fits the drawn bounding box (`getBBox` of the camera group) with 40 px padding, scale 0.4 to 1.2. `0` refits.

**Words for cardinality**, for a link from X to Y:

| Cardinality | Words |
|---|---|
| many-to-one | many X → one Y |
| one-to-many | one X → many Y |
| one-to-one | one X ↔ one Y |
| many-to-many | many X ↔ many Y |

X and Y are the entities' labels in lower case, plural where it reads "many".

### Links that need care

| Case | Drawn as |
|---|---|
| A self-link (`Rep.manager → Rep`) | A loop on the centre's right edge, with its label in the first row of the out column ("→ itself: manager") |
| Two links between the same pair | One neighbour box; its row holds both labels, and the row grows 16 per extra label |
| A link each way between the same pair | One box in the out column, with → and ← labels |
| A link from an ancestor (`Sale.placed_by` on PromoSale) | Drawn, with "· from Sale" after the label |
| A link that ends at an ancestor (accepts any kind) | In the in column, with "· ends at Sale" |
| A link to an entity that doesn't exist | A red dashed box, "Partner (missing)", with the label in red |
| A waiting proposal's new link or entity | Dashed green, as on Overview's ghosts; up to 5 |

### Clicking, the card, back and forward

- **Clicking:**
  - **An entity box** moves the centre to it. Boxes that stay on screen glide to their new places (FLIP, 250 ms; instant with reduced motion), and the address changes to `#graph/<name>` with `location.hash`, so Back and Forward work.
  - **Anything else** (a metric, guardrail, action, function, the source, or a label) selects it, and the card shows it. A label's card shows the link's sentence, cardinality words, the guardrails on that link ("When linking Quote to Opportunity"), and "Describe this link" when it has no sentence.
  - **Empty space** puts the selection back on the centre.
- **No drag here.** Clicks select on `click`. The pointer capture that broke Map selection (Part 6, problem 2) is never used here.
- **Breadcrumbs** above the drawing: "Opportunity › Quote › Account", the last 6 centres, each clickable.
- **The bar:**
  - Find (`/`): picks an entity and moves the centre to it
  - the light filters, as on Overview
  - "+ one more step"
  - Show: Links · Logic · Rows and family (checkboxes, remembered)
  - the zoom buttons (`+`, `−`, `0`, as on Overview)
- **No links.** An entity with no links shows "Nothing links to Quote." in the empty column and, when editable, **Add a link**, which opens Links with "from Quote" filled in.

### Many neighbours, and one step further

- **More than 12 in a column.** Neighbours are grouped by link name, largest group first. The 12 rows are filled with single neighbours from the smallest groups first; the rest become group rows: "+14 via contains", with a count and the link's label. Clicking a group opens its list in the card, and each item moves the centre.
- **Show all.** At the column's foot, it draws up to 60 rows. Past 60 the column says "60 of 214 shown · Find to go to one".
- **+ one more step** adds an outer column on each side. Each neighbour's own neighbours sit in its band: up to 6 each, 60 in all, boxes 160 × 36, lines unlabelled with a `<title>`. A neighbour already drawn isn't drawn twice: its line goes to the existing box.

### Logic

- **A metric chip** shows the label and, on line 2, its sentence ("Add up revenue, leaving out lines where refunded is true."). Selecting it highlights the fields it uses in the centre (accent background), and the card shows the metric's card from Overview. A metric over several entities says "also counts PromoSale".
- **A guardrail chip** shows its id, its name and a severity stop (block, warn or note). An inherited one says "from Sale".
- **An action chip** shows the action's words ("Submit quote"), with ⛉1 when guardrails apply to the action (QT-004 on `submit_quote`).
- **A function chip** says where it runs (mcp, http, or manual).
- **More than 12 chips** of one kind are grouped: "+8 guardrails", opening a list in the card.

### The proposal overlay

`#review/<id>/graph`:
1. **The data** comes from `/api/graph?proposal=<id>`, as today.
2. **The centre** is the first entity the proposal adds or changes. Chips above the drawing name the others ("This proposal touches: PromoSale · Sale").
3. **Status styles.**

   | Status | Drawn as |
   |---|---|
   | new | dashed green |
   | changed | dashed amber |
   | removed | struck red |
   | same | plain |

4. **It is read-only:** no inline fixes, and a "Back to the review" bar, as today.
5. **A new entity** shows "new · N fields" in place of its field list, because it isn't written yet, so there is nothing to describe.

### Phone, keyboard, screen readers

- **Phone.** Below 760 px the Graph is rows:
  1. the centre's card
  2. "Links out (1)", each row "for opportunity → Opportunity · many quotes → one opportunity"
  3. "Links in (0)"
  4. "Logic (5)"
  5. "Rows and family"

  Each entity row moves the centre. Breadcrumbs stay, and Back works.
- **Keyboard.**
  - Tab lands on the centre.
  - ← and → move between the columns and the centre; ↑ and ↓ move within a column, or from the centre into the bands.
  - Enter moves the centre to an entity, or selects anything else.
  - Shift+Enter opens the item's page.
  - `E` toggles one more step. (`+` and `−` zoom, as on Overview.)
  - Esc puts the selection back on the centre.
  - The browser's Alt+← is Back.
- **Screen readers.** The drawing is a `role="group"` labelled "Quote and what it touches: 1 link out, none in, 4 guardrails, 1 action, rows from cpq_snowflake". Each node's `aria-label` is a sentence: "Opportunity. Quote links to it as for opportunity: many quotes to one opportunity." Moving the centre is announced in the polite live region. The rows above are the accessible twin, linked by "Skip to rows".

### Data: no new endpoint

The Graph is built in the page from:
- `/api/plane` (`PL`): labels, icons, lights, areas, links, `kind_links`, metrics with `entities` and `uses_how`, sources, and waiting proposals
- `/api/summary` (`O`):
  - constraints: guardrails on links, via `rulesFor`
  - actions, for their words
  - functions and their inputs
  - relationships with their sentences
- `/api/describe?name=<centre>` (`DC`): fields, links out and in (inherited ones marked), guardrails with `from`, and action names

**Fetching the centre** (problem 5). `describe` of the centre is fetched whenever it is missing from `DC`, on every draw. `loadSummary` empties `DC` after each write, so the next draw fetches again. A failed fetch shows "Couldn't load the fields · Try again" in place of a stuck "Loading fields…".

**Polling** every 30 s works as on Overview, keeping the centre. If the centre disappears, the Graph says "Quote is no longer in this space" and moves to the most important entity.

---

## Part 6 · The seven problems in the screenshots

| # | Problem | Probable cause (read in the code, not reproduced) | Fixed by |
|---|---|---|---|
| 1 | "Show on the map" from the card on Quote opens the Map faded, with Quote off-screen | The button sets `pl.open` and goes to `#map` with `pl.sel` still on Quote. Everything not linked is dimmed to 35%. `drawPlane(true)` keeps Overview's camera when it has moved (`pl.moved`), but the flat projection differs, so the camera points somewhere else. | The button becomes **Open in graph** (Part 4). The Graph draws only the neighbourhood, so nothing is dimmed, and it fits what it drew. |
| 2 | Clicking a box on the Map doesn't select it | `pointerdown` on an entity starts a possible drag and calls `svg.setPointerCapture`. The following `click` is then aimed at the `<svg>`, so `closest('.pl-item')` finds nothing and the click reads as empty space, which deselects. | The Map goes. The Graph never captures the pointer on a node; test "a click selects". |
| 3 | Link labels pile up | Labels sit at line midpoints with no collision check: on the Map for all links, in 3D for the selected entity's links | The Graph's label lanes (Part 5), and Overview's placement pass (Part 4) |
| 4 | A line runs off to nowhere; boxes cut off at the right | `flatBounds` adds fixed margins. The metrics lane sits at `maxX + NODE_W + 140` and is 270 wide, which reaches past the margin added. Open nodes grow below the bounds. | The Map goes. The Graph fits `getBBox()`. |
| 5 | Opening Account (▾) showed "Loading fields…" and stuck | `loadSummary` sets `DC = {}` after every write and on return to the tab, while `pl.open` keeps the node open, and nothing fetches again. "Show on the map" also calls `describe()` without redrawing when it returns. An error answer (no `properties`) is kept in `DC` and also shows "Loading fields…" for good. | The Graph fetches the centre's fields on every draw when missing, with a Try again (Part 5) |
| 6 | Reload and Summary.md drop below the top bar and are cut off | The header wraps (`flex-wrap: wrap`) inside a fixed 52 px grid row | Part 2's header |
| 7 | The owner box looks saved before Save | `ownerLine` and `inlineFix` fill the box with your kept name; the review screen marks it `prefilled` | The picker (Part 1) |

The owner Save itself works: the OAuth session checked it on a scratch copy of branda. The check was kept with a fresh fingerprint.

---

## Part 7 · New codes and format

| Code | Level | Says | Light |
|---|---|---|---|
| W119 | warning | "metric promo_sale_revenue: owner `Vinay` is another spelling of Vinay Karode (people.yaml)". Covers owner fields only. | none |
| W120 | warning | One of three messages: <br>- "Quote says area `Pipline`, which ontology.yaml doesn't list" <br>- "Quote is listed under Pipeline but says Sales" <br>- "area Pipeline: colour `purple` isn't one of blue, teal, violet, pink, slate, sky, green, rose" | none |
| W121 | warning | "people.yaml: Vinay is a spelling of both Vinay Karode and Vinay Kumar" (see Part 1 for the others) | none |

- **Lights.** `plane::diag_reason` returns `None` for W119 and W120, as it already does for W113 and W114, so they light nothing.
- **Format 2** now also covers `color:` on an area and a `people.yaml` file:
  - Lorefile writes `format: 2` in the same save.
  - W118's message is widened: "this space uses kinds, `distinct:`, area colours or people.yaml; add `format: 2` to ontology.yaml so older Lorefile says to upgrade".
  - **If 0.2.0 is tagged before this is built,** these become format 3, and `FORMAT` moves to 3.

---

## What this does not do

- No accounts, logins, passwords or emails. People are names (BACKLOG "Accounts and single sign-on").
- No team members, roles, permissions per person, or notifications to owners.
- No sub-areas. An area holds entities, not areas.
- No dragging entities between areas on Overview. Moves are explicit (the card, the entity page).
- No whole-space graph, and no graph across spaces.
- No editing links by drawing in the Graph. Describe a link inline, or Add a link on the Links screen.
- No `iri:` on areas (BACKLOG).
- No change to MCP tool names. `propose_patch` gains ops, and `describe` and `summary` gain fields.

---

## Edge cases

### People

| Case | What happens |
|---|---|
| Two different people named Priya | Lorefile can't tell. The People screen shows one Priya who owns both sets. Change one set's owner to a fuller name ("Priya S.") with the pickers. History stays as written. |
| A name written "Vinay", "vinay" and "Vinay " | One person, shown "Vinay"; nothing written |
| "Vinay" and "Vinay Karode" | "Same person?" offered. "No, two people" writes `not_same`. |
| Renaming to an existing person | A merge. The confirm says "Merge Vinay into Vinay Karode: 1 metric, 2 guardrails". |
| Renaming when a pending proposal sets the old spelling | The proposal is unchanged. On accept, W119 shows on its review screen, with the fix to the main name. |
| Renaming a team address (`finance@northstar.example` → "Finance") | Rewrites the fields. The address goes into `also`. |
| Owner fields in flow style (`{ id: BR-1, owner: x }`) | Mapping rewrite. The confirm lists the files that lose comments. |
| A file that doesn't parse holds an owner | Rename refused, naming the file. Fix it first. |
| An owner of "TBD", "none" or "?" | Listed as written, with "this isn't a person: name a real owner". W109 is unchanged. |
| An owner holding two names ("Vinay, Priya") | Listed as one name, with "looks like two names: owners are one person or one team" |
| An owner whose name contains "agent" | Listed (owners aren't filtered). The picker can't make a new one, because of the name rule. |
| A `by` of "person via page" | Skipped (anonymous). "Put your name on it" (ui-fixes decision 12) names it. |
| The same name as an agent's connection ("ChatGPT") in an owner field | Listed as an owner. Connections are still not people. |
| Someone who left owns metrics | "Sam (left)" wherever the owner shows, plus a nudge. The picker hides Sam unless searched ("Sam (left)" when typed). |
| `people.yaml` hand-edited with `email:` | Error: unknown field (E001 from the loader), like any file. Saves are refused until it's fixed. |
| `people.yaml` names someone nobody mentions | Kept and listed: "owns nothing". |
| No name kept in this browser | The picker has no "You" group. Save reads "Type your name to save". |
| A read-only server, or a shipped example | Owners show as text. People lists without menus. |
| An agent proposes `set_owner` on an accepted metric | It waits for a person, as today (`would_overwrite_graded`). The review screen shows the picker's Save line. |
| An agent proposes an owner nobody has | Review note "New name: nobody here is called Priyanka yet". Accepting adds them (they now own something). |
| A space with 500 owners | The People screen searches and pages by 100. The picker searches and shows 30 at a time. |
| Home's 30-second refresh while picking | The card isn't redrawn while a picker is open or focused. |
| Two people rename at once | The second save re-plans against the files as they are and reports its own counts. Nothing is merged silently. |
| History off | Rename is still one save. Undo is by hand (the toast says so, as for other edits). |
| Other spaces' names | On the page only, never in `/api/people` for an OAuth connection or in MCP. Shipped examples are never a source. |

### Owners and grades

| Item's grade before | Owner set by a person | Owner set by rename |
|---|---|---|
| checked | kept, same who and when, fresh `of` | kept, fresh `of` |
| accepted | kept, same who and when, fresh `of` (changed: as built, re-accepted by the setter) | kept, fresh `of` |
| unverified (proposed) | accepted by the setter; the Save line said so | stays unverified, fresh `of` |
| changed since | accepted by the setter; the Save line said so | stays changed since, `of` untouched |
| ungraded | accepted by the setter; the Save line now says so (as built it is silent) | stays ungraded, no record |
| an entity or function | no grade, no line | no grade |

### Panels and header

| Case | What happens |
|---|---|
| Folded menu with a pending proposal | A badge with the count on Agents' icon |
| The folded menu's search button | Opens the menu and focuses Find |
| `[` or `]` while typing | Ignored (inputs, textareas, selects, contenteditable) |
| Folding the right panel on Home | The drawing refits if the camera hasn't moved |
| A folded right panel and a new selection | The strip shows its glyph and name; it doesn't open (decision 18) |
| localStorage blocked | Everything opens; folding works for the tab only |
| The ⋯ menu open, then the window widens | The menu closes, and Reload and Summary.md return to the bar |
| Below 760 px | No fold buttons. Today's phone layout. |

### Areas

| Case | What happens |
|---|---|
| New area named "other" or "Other" | Refused: "Other is where entities with no area show" |
| "Sales Ops" when "Sales & Ops" exists | Refused: both would be `…domain.sales-ops` |
| Renaming "core" to "Core" | Allowed (capitals only) |
| An area and an entity with the same name | Allowed: their IRIs differ (`domain.sales` vs `Sales`) |
| Moving an entity into its own area | Nothing written: "Quote is already in Pipeline" |
| An entity listed under A while saying B (W120) | Moving it writes both places to the target, so W120 clears |
| Moving a parent whose kinds follow it | They move too; the confirm names them |
| Moving a kind | It gets its own `domain:`, and the confirm says it stops following |
| Deleting an area only a following kind is in | Not possible: its parent is in it too, so the area isn't empty |
| Deleting the last area | Allowed (it is empty). The new-entity form then shows only "+ New area". |
| Deleting a middle area | Later areas get `color:` so they keep their colours; `format: 2` is written |
| `order_areas` missing a name | Refused: "say where Channel goes" |
| A pending proposal's `add_type {domain: Core}` after Core was renamed | Its review shows W120, with chips "Put it in Sales" or "Create Core" |
| `add_type` from an agent with a new area name | W120 on review; the same chips |
| A hand edit deletes an area that entities still say | W120 on each. They show under their own area name, as today (`plane.rs` appends unknown names). |
| An area colour hand-written as `#ff0000` | W120 (not one of the eight); the position colour is used |
| 9 or more areas | Colours repeat, as today (8 names) |
| Rename where an RDF export was shared | The confirm says the IRI changes when the space has its own `namespace:`. Re-importing with Lorefile matches by name. |
| Area colours through export and import | Ossie carries `color` inside the `domains` it already exports, so it round-trips. RDF doesn't carry colours (they are presentation), so an RDF round trip falls back to position colours. |
| Shipped examples | The card shows, without buttons. "Copy it into a space of your own" as today. |

### Graph and selection

| Case | What happens |
|---|---|
| An empty space | "No entities yet" with **Add an entity** |
| `#graph/Nope` | "Nope isn't in this space", then centred on the most important entity |
| Switching space with `#graph/Quote` | The new space without Quote shows the same message |
| An entity with no links, rules or source | Centre only, with "Nothing links to it", "Nothing in its logic yet", "No data yet · Bind a table", and Add a link |
| A hub with 40 links out | 12 rows, the rest grouped by link name; Show all draws up to 60 |
| 500 neighbours (generated test space) | Groups, then "60 of 500 shown · Find to go to one"; draws in under 50 ms |
| Ring 2 reaching back to a drawn box | The line goes to the existing box, with no duplicate |
| Long names | Cut at 24 characters with "…"; the full name on hover and in the card |
| Two entities with the same label | The name shown in mono under the label, as Overview does |
| The centre deleted by an agent while watching | Next poll: "Quote is no longer in this space", then the most important |
| Back after many moves | Each move is one history entry; Back walks them in order |
| A proposal adding a linked entity | A dashed ghost row in the right column, with a Review button in its card |
| Reduced motion | No glide, no fly |
| A metric over the centre and others | One chip: "also counts PromoSale" |
| A metric tied to no entity | Not in any Graph. It shows in List and on Overview's loose shelf, as today. |
| The overlay of a proposal that removes the centre | The centre struck red; its card says "would be removed" |
| Phone | Rows, as Part 5 |
| Keyboard only | Every node reachable by arrow keys; Enter moves or selects |

---

## Build notes, file by file

- **`src/people.rs` (new):**
  - `derive(ont, dir, git) -> People`: sources, the one-person rule, teams, "left", same-person pairs
  - `rename_plan(ont, from, to)`: the files with owner lines edited, plus `people.yaml`
  - the owner name rule `owner_ok`, shared with `set_owner`
  - unit tests
- **`src/schema.rs`:**
  - `PeopleFile { people: Vec<Person>, not_same: Vec<[String; 2]> }`
  - `Person { name, also, team: Option<bool>, left: Option<String> }`
  - `Domain.color: Option<String>`
  - all with `deny_unknown_fields`
- **`src/loader.rs`:** classify `people:` (root `people.yaml` only; refuse elsewhere, like `trust:`), into `raw.people`.
- **`src/ontology.rs`:** carry `people`, and `person_of(name)` (main name, kind). In `describe`: `owner_kind` and `owner_is`. `summary_markdown`: the "Owners" line.
- **`src/validate.rs`:**
  - W119, W120, W121
  - widen W118 to area colours and `people.yaml`
  - set `subject` on W119 and W120 for the review screen and Problems
- **`src/trust.rs`:**
  - `restamp`: the owner-only rule for `Who::Person`
  - `restamp_keep` for rename
  - `owner_only(old, new)`, which compares with `owner` removed
- **`src/patch.rs`:**
  - `set_owner` (entity, metric, function, guardrail), as a line edit where possible
  - the six area ops, with line edits for `domain:` lines and mapping edits for `ontology.yaml`'s `domains:`
  - `format: 2` written with `color:`
  - the delete of a types file also cleans area lists
- **`src/serve.rs`:**
  - send any multi-file plan through `apply_many` (generalise the `make_parent` route)
  - `/api/patch` also takes `ops: [..]` (up to 40), planned with `plan_many` as `propose_patch` does, for the page's batches: "Give them owners", "Move here…", and "+ New area" in the new-entity form (`add_area` then `add_type`)
  - `GET /api/people` (with `other_spaces` from the registry, page only)
  - `POST /api/people` (rename, team, person, left, back, not_same), logged and committed like other writes; refused on shipped and read-only spaces
  - `overview()` sends `color`
- **`src/plane.rs`:** `areas[].color`; `diag_reason` returns `None` for W119 and W120.
- **`src/review.rs`:** words for the area ops and `set_owner`; `compare` diffs `domains:`; the "New name" note.
- **`src/mcp.rs`:** the `propose_patch` description lists `set_owner` and the area ops. Server instructions are unchanged.
- **`src/ui/index.html`:**
  - People screen
  - owner picker (one component)
  - Your name suggestions
  - panels and the header
  - tabs and addresses
  - remove the Map
  - `drawGraph` and the rows twin. A hash change from one `#graph/…` to another moves the centre in place (the FLIP glide), without `onHash` re-rendering the whole screen. `planeKeys` and the document keydown handler learn the Graph's Esc (back to the centre) and `E`.
  - area card, empty areas, the label placement pass
  - `domainColor` reads `color`, and the palette's 5th colour changes
- **Docs:**
  - README: People, Areas, the Graph; the Map paragraphs replaced
  - ARCHITECTURE: `people.rs`, `people.yaml`
  - BACKLOG: "Left for later" below
  - CHANGELOG: 0.2.0 notes
  - the "Words Lorefile uses" card: Person, Team, Area, Graph

### Hand-off plan

These go to learn-file-build on the same branch, one after another, because they share `patch.rs`, `serve.rs` and `index.html`:

1. **people-core** (Rust): `people.rs`, `people.yaml`, `set_owner`, the grade rules, `/api/people`, W119 and W121, `describe` and `summary`. Tests.
2. **areas-core** (Rust): the six ops, multi-file routing, colours, W120, cleaning lists on delete, review words. Tests.
3. **page-shell**: panels, the header, Overview | Graph | List and addresses, Map removed, the People screen, pickers everywhere, the area card, empty areas, Overview labels.
4. **graph**: `drawGraph`, groups, one more step, the overlay, the rows twin, keyboard.
5. **live**: restart 7171 on the new build, and check on branda:
   - name the owner of `promo_sale_revenue` with the picker; the check is kept
   - make an area and move PromoSale there
   - open the Graph from ChatGPT's review link
   - confirm on `@b2b-sales` that Quote's Graph matches the wireframe

---

## Tests

**Rust** (`tests/people.rs`, `tests/areas.rs`, unit tests):
- **Deriving people:**
  - every source counted
  - anonymous and agent `by` values skipped
  - capitals, spaces and accents make one person
  - teams by `people.yaml`, by `@`, and by the word "team"
  - "left" hides from the picker list
  - `other_spaces` excludes shipped spaces and is absent for OAuth and MCP callers
- **`people.yaml`:**
  - read at the root only; refused elsewhere and under another name
  - `email:` refused
  - W121 for each contradiction
  - the first write adds `format: 2`
  - hand-written without it gives W118
- **`set_owner` on each kind:**
  - the owner name rule (no `@`, no "agent", 60 characters)
  - grades: checked kept; accepted kept with the same who and when; unverified, changed-since and ungraded become accepted by the setter
  - an agent's `set_owner` on an accepted metric waits for a person
- **Approvers:** `requires_approval_from` is never listed as a person, and rename leaves it alone.
- **Rename:**
  - rewrites owners, including old spellings
  - adds `also`
  - comments kept in block style; flow-style fallback listed
  - merge into an existing person
  - grades in rename mode, for all five starting states
  - refused for a bad name, an unparsed file, a shipped space
  - one commit; Undo restores files, `people.yaml` and `trust.yaml`
- **W119:** an owner spelled another way; its fix; no light.
- **Areas:**
  - add (name rule, Other, taken, IRI collision)
  - rename (`domain:` lines, lists, derived IRI, colour kept)
  - describe
  - move: the `domain:` line, the list, both, neither; other lists cleaned
  - move with kinds that follow, and moving a kind alone
  - delete refused with names; delete when empty; `color:` written for a middle delete and a reorder
  - order needs every name
  - W120, all three messages, no light
  - deleting an entity cleans its area list (and the test that shows today's E010 refusal first)
  - review words; `propose_patch` accepts the ops
- **`describe` and `summary`:** `owner_kind`, `owner_is`, the Owners line, capped at 12.

**Page** (in-app browser, on scratch copies of branda, `@b2b-sales` copied, a blank space and the generated 2,000-entity space):
- **The header** at 1440, 1280, 1100, 1000 and 800 px: one line; the ⋯ menu by keyboard.
- **Folding:** the menu, the list column and the right panel; remembered after reload; `[` and `]`; nothing at 375 px.
- **People:**
  - the list, "Same person?", merge, "No, two people", team, left, back
  - Give them owners for 3 items
  - read-only on a shipped space
- **The picker:**
  - never pre-filled
  - "You" first
  - the Save line's wording for each grade
  - Save waits for a name
  - Change and Remove
  - not redrawn by Home's refresh
- **Areas:**
  - + New area; the empty area's frame
  - the area card: rename, describe, reorder, Move here
  - Delete refused, then allowed
  - Move… from the entity card, with a kind following its parent
  - the new-entity form's "+ New area"
- **The Graph:**
  - Quote on `@b2b-sales`: no links in, Opportunity out, 4 guardrails plus Submit quote with ⛉1, cpq_snowflake below
  - clicking Opportunity moves the centre; Back returns; breadcrumbs
  - a hub of 40 groups; Show all; one more step
  - a self-link, a pair with two links, a missing target, a kind family
  - a metric selected highlights its fields
  - inline describe of a field
  - the overlay from p-6's review screen
  - rows at 375 px; keyboard only; reduced motion
- **Addresses:** `#map` and `#review/<id>/map` land on the Graph; creating a space opens `#home`.
- **Overview:** labels never overlap for an entity with 8 links.
- **The console:** no errors from the page.

---

## Left for later

- Accounts and sign-in, so "who" is a signed-in person (BACKLOG). The People screen and pickers stay.
- Team members, roles, and "my things" (what I own, across spaces).
- Telling owners when something of theirs changes.
- `iri:` on areas, to keep an IRI through a rename.
- Sub-areas.
- Drag to move entities between areas on Overview.
- A whole-space Graph mode, and links between spaces.
- Adding a link by drawing it in the Graph.
