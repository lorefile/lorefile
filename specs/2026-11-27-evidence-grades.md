# Spec: evidence grades, so an agent can tell a checked definition from a guess

Written 2026-11-26 for Vinay to decide on. **Status: built the same day** (decision 2 by Vinay, the rest as recommended), then reviewed; what the review changed is listed at the end, and the text below is corrected where it did. It follows a YouTube discussion he shared, whose point was: anything a model produces should enter at the lowest grade and must not promote itself until it lines up with something measured. Lorefile covers half of that today. A proposal needs a person's Accept, and history records whether an agent or a person wrote a change. But nothing an agent reads says how well a fact is backed, so an agent states a definition another agent guessed last Tuesday in the same voice as one Finance checked against the ledger.

Standing rules, unchanged ([BACKLOG.md](../BACKLOG.md)): the ontology never executes and never holds credentials; agents and builders write the YAML, people review; light theme only; honest status. Read first: `src/edit.rs` (the save gate, `Note`, commits), `src/proposals.rs`, `src/mcp.rs` (`propose`, `propose_many`, `instructions`), `src/serve.rs` (`/api/proposals/accept`), `src/ontology.rs` (`summary_markdown`, `describe`), `src/rules.rs` (`check_action`), [ARCHITECTURE.md](../ARCHITECTURE.md) §17 and §18, [docs/standards-explained.md](../docs/standards-explained.md).

The example carried all the way through is Lumen Home, the shop in `examples/ecommerce/`, with its `success` metric (paid orders in `shop_bigquery.orders`).

---

## The gap, in one week

**Before (today).** Tuesday, an analyst asks Claude for return on ad spend. Claude's Lorefile runs with `--allow-edit` (the Claude Desktop config in the README does), so when it settles on "Meta's reported conversion value over spend" it saves `blended_roas` with `propose_patch` and the metric is written at once. Nobody opens the page. Wednesday a colleague asks "what was ROAS last week?". Claude reads `summary`, finds `blended_roas` next to `success`, and answers "ROAS was 4.2" as the company's definition. One of those two metrics was agreed by the growth team and matched against Shopify's own report. The other is a model's guess from yesterday. In `summary` and `describe` they look the same. The only trace is the author of a git commit, which an agent never sees, which is lost in an export, and which does not exist where git is off.

**After.** The same Wednesday:

```
Claude: ROAS last week was 4.2. That uses a definition an agent proposed on Tuesday and
        nobody has reviewed yet: Meta's attributed conversion value over spend. Paid orders
        (success) were 412; that definition was checked against shop_bigquery by Priya on
        28 Nov. Do you want to confirm the ROAS definition, or change it?
```

```
 TODAY                                          WITH GRADES
 summary                                        summary
   success        paid orders …                   success        transactional · checked 28 Nov
   blended_roas   Meta value over spend           blended_roas   attributed · UNVERIFIED (an agent
                                                                 proposed it, nobody has reviewed it)
 the agent says both as fact                    the agent says which one a person stands behind
```

---

## Decisions

One line each, with what I recommend. The parts below give the options and the reasons. Approve, change or strike each line.

| # | Decision | Recommendation |
|---|---|---|
| 1 | The grades | Three, lowest first: **proposed** (an agent wrote it, no person has looked), **accepted** (a person stands behind the words), **checked** (a person ran it against real rows and it was right: who, when, which source, what they compared). No grade at all is **ungraded**. No scores, no percentages. |
| 2 | Where the grade is stored | **Decided 2026-11-26 (Vinay): `trust.yaml`.** One file per space, in git, written only by Lorefile. Not a key on each fact. This differs from the first sketch (a `status:` or `verified:` key on the fact); Part 1 says why. |
| 3 | What gets a grade in the first version | Metrics, entity data bindings, glossary words, guardrails. A word tops out at accepted: there is no data to check a word against. Entities, fields, links, functions, actions and sources come later with the same mechanism. |
| 4 | Who sets a grade | The save gate, from who is saving. An agent's direct write: proposed. A person's edit in the page, or a person's Accept of a proposal: accepted. The "Mark as checked" form: checked. An agent can never write `trust.yaml`, and a grade key typed into a metric, entity, glossary or guardrail file is already a compile error (the compiler refuses unknown keys). |
| 5 | An agent changing something a person accepted or checked | Never written directly, even with `--allow-edit`: it is queued for a person, like any proposal when edit mode is off. Agents write new facts at the lowest grade; they do not overwrite graded ones. |
| 6 | A fact that changes behind the grade's back (a hand edit, a git merge, Claude Code editing the file) | Every grade carries a fingerprint of what it graded. When the fact no longer matches, the grade stops counting, the page says "changed since it was accepted", and `validate` warns (W113) until a person accepts again. |
| 7 | Agents reporting a check | Allowed as a report only: `propose_patch` with `op: record_check` is always queued, never written, and the grade becomes checked only when a person clicks "Yes, I saw it match". The report carries the fingerprint of the definition the agent compared; if the definition changes before the click, the click is refused. Strike this line for the strict version, where only the page's form can check. |
| 8 | Everything that exists today | Starts ungraded: existing spaces, the shipped examples and packs, copies of a space, and imports. Agents do not flag ungraded facts; they flag proposed ones. The page offers "Vouch for all" so a team can grade a space it already trusts in one click; it covers only what nobody has ever graded, never what an agent proposed or what changed since a person graded it. |
| 9 | `basis:` on metrics | A separate key, part of the metric's definition: `transactional`, `attributed`, `modelled` or `experimental`. It says what kind of number this is; the grade says who stands behind the definition. Optional, no warning when absent. `attribution:` and the rest of "Metric semantics" stay in the backlog. |
| 10 | What agents are told | `summary` gets one Trust line and marks proposed and checked things; `describe` returns the grade with a ready sentence; the server's instructions tell agents to say "unverified" for proposed definitions and never to add numbers of different basis. |
| 11 | `validate` | Verdicts do not change. An unreviewed guardrail still blocks (failing safe), but the result says it is unreviewed, and a pass that read rows through an unreviewed binding says so. |
| 12 | The page | A chip on every graded thing, "Looks right" on proposed ones, "Mark as checked…" on accepted ones, a "Written by agents, not reviewed" list on Agents, a line in "What to do next", and a "Your name" box in Settings. |
| 13 | Export | RDF: the grade as a `lorefile:` statement plus a PROV-O record of who, when and against what. Ossie: the grades ride in the `lorefile` box, and a proposed definition also carries one plain sentence in `ai_context`, where other tools' AI will read it. |
| 14 | Import | Imported facts start ungraded, always. What the file claimed is listed in the report. `--keep-grades` restores them for a team re-importing its own export. |
| 15 | What it does not do | Part 5. In short: it grades definitions, never answers or numbers; "checked" records a person's statement, Lorefile runs nothing; it is as tamper-proof as the repository's review and no more. |
| 16 | Codes | E018 (a grade that makes no sense), W113 (changed since graded), W114 (a grade for something that no longer exists). |

---

## Part 1 · A grade on what the space holds

### The ladder

```
 ungraded ─────────── nobody has said. Files from before grades, hand-written YAML, imports, copies.
 proposed ─────────── an agent wrote it and no person has looked.           by: agent via MCP · when
 accepted ─────────── a person stands behind the words.                     by: Priya via page · when
 checked  ─────────── a person ran it against real rows and it was right.   by · when · against which
                                                                            source · what was compared
```

A thing moves up only by a person's click. It moves down by itself the moment its definition changes.

| What happens | Grade afterwards |
|---|---|
| An agent saves a new metric, binding, word or guardrail with edit mode on | proposed |
| An agent's proposal waits in the inbox | nothing yet: it is not in the files agents read |
| A person presses Accept on a proposal | accepted (the agent stays the author in history, as today) |
| A person adds or edits it in the page | accepted |
| A person presses "Looks right" on a proposed thing | accepted |
| A person fills in "Mark as checked…" | checked |
| A person edits only the words of a checked thing (description, rationale, caveats, owner, tags) | stays checked |
| A person edits the recipe of a checked thing (source, filter, measure, the rule) | accepted: the check was of the old recipe |
| Anyone changes the file outside Lorefile | "changed since it was …", counts as ungraded, W113 |
| An agent tries to change an accepted or checked thing | nothing changes; queued for a person (decision 5) |

### Where the grade lives: three options

| | A · `trust.yaml`, one file per space (decided) | B · a `trust:` key on each fact | C · nothing stored, read it from git history |
|---|---|---|---|
| What it looks like | One file at the root of the space, one line per graded thing | One more line inside every metric, binding, word and guardrail | The commit's author and committer |
| "Never set by the agent" | A path rule: agents cannot propose `trust.yaml`. Their YAML never contains a grade, and a grade key in a fact file is an unknown key, which is already an error | The gate must strip grades from every agent-written file and put the right ones back, by editing the agent's text | True by construction |
| Hand-written files and their comments | Never touched | A line is inserted into each. Type files are rewritten through the YAML library today, which drops comments (backlog item "type-file edits keep the file's shape"); the packs carry 13 `# TODO verify` comments | Never touched |
| Waiting proposals | Unaffected | Every grade change rewrites the fact's file, so every other proposal waiting on that file turns stale (Accept answers 409 when the file changed) | Unaffected |
| Checked against data | Yes: who, when, source, note | Yes | No. Git knows who wrote and who accepted, not that anyone compared it with rows |
| Survives export, a nested repository with uncommitted changes, a server without git | Yes | Yes | No |
| Reading the YAML or a pull request | Two files change: the fact, and one line in `trust.yaml` | The grade sits next to the fact | Nothing to read |
| Finding everything unverified | Open one file | Search every file | `git log` per fact |

**Decided 2026-11-26: A.** The first sketch was B, and B reads nicely in a diff. But agents write these YAML files, and the one thing an agent must not write is its own grade; keeping the grade out of every file an agent can write makes that rule structural instead of something a filter has to get right every time. It also leaves hand-written files and their comments alone, and it does not turn waiting proposals stale. People read grades on the page and agents read them from `describe`, where both options look the same. B and C stay in the table as the record of what was weighed; nothing is built for them.

```yaml
# trust.yaml · who stands behind what in this space.
# Written by Lorefile when a person accepts or checks something. Not edited by hand; agents cannot change it.
trust:
  metrics:
    success:      { grade: checked,  by: Priya via page,  at: 2026-11-28, of: 3f9a1c2e70b4,
                    against: shop_bigquery,
                    note: "412 paid orders for 16–22 Nov, the same as Shopify's own report" }
    blended_roas: { grade: proposed, by: agent via MCP,   at: 2026-11-24, of: 9d02b7c15a11 }
  bindings:
    Order:        { grade: accepted, by: person via page, at: 2026-11-26, of: 77b0e1aa0c3d }
  terms:
    basket:       { grade: accepted, by: person via page, at: 2026-11-26, of: 0c1d2e3f8a90 }
  guardrails:
    EC-001:       { grade: accepted, by: person via page, at: 2026-11-26, of: 51aa90ce2b17 }
```

- **`by`** is whoever gave the current grade. Today that is `agent via MCP`, `agent via API`, `person via page` or `person via CLI`, the same words history uses, with the name from Settings in front when one is set (`Priya via page`). Lorefile has one shared login token and no accounts, so the name is what the person typed; accounts are in the backlog and would make it a signed-in identity.
- **`at`** is the UTC date. The exact time, and everything that happened before the current grade (who proposed, who accepted earlier), is in git history, where it already is.
- **`against`** must be a source under `sources/`: the one whose rows were used. What they were compared with (Shopify's report, the finance ledger) goes in `note`. Both are required for `checked`.
- **`of`** is the fingerprint: a short stable hash of the thing as compiled (its values with keys sorted, so comments, spacing and key order do not matter). For a metric, word or guardrail it covers the whole item. For a binding it covers the entity's name, its key field and its `source:` block, so adding a field description does not disturb the binding's grade. A metric's and a binding's also cover the definition of each source they read (without its description): the same recipe over another project or dataset is another number, so re-pointing a source stops every grade that depends on it. A word written twice, in two files or with two meanings, is graded as one thing with every meaning counted.

The file is one mapping per kind, keyed by name, so two people grading different things merge cleanly in git, and the existing Undo (which merges YAML key by key) handles it without new code.

### Who is asked, and when

**Scenario: an agent with edit mode on.** `propose_patch add_metric blended_roas` is written at once, as today, and in the same commit `trust.yaml` gains `blended_roas: proposed`. The tool result says so: *"written to logic/metrics.yaml and saved to history as a41c9e2. Grade: proposed. Tell the user a person still has to accept it (Agents › Written by agents, not reviewed)."*

**Scenario: an agent with edit mode off.** Nothing changes from today until Accept. Accept writes the fact and, in the same commit, `accepted` by the person.

**Scenario: an agent wants to change `success`, which is checked.** With edit mode on, today's server would overwrite it. With grades, the change is queued and the tool result says: *"success is checked (Priya, 28 Nov), so this change waits for a person: queued as p-20261129-101502-1."* On Accept the grade becomes accepted, because the recipe is new and nobody has compared it with rows yet.

**Scenario: the check.** Priya opens Logic, sees `success` is accepted, runs last week's number with her own BigQuery login, and compares it with Shopify's report: 412 both.

```
┌ Mark success as checked ──────────────────────────────────────────────────────┐
│ You are saying: I ran this definition against real rows and the result was     │
│ right. Lorefile records your word; it does not run anything.                   │
│                                                                                │
│ Against which source   [ shop_bigquery ▾ ]                                     │
│ What did you compare   [ 412 paid orders for 16–22 Nov, the same as Shopify's  │
│                          own report                                          ] │
│ Your name              [ Priya ]                                               │
│                                                [ Cancel ]  [ Mark as checked ] │
└────────────────────────────────────────────────────────────────────────────────┘
```

History gets one line: *"Checked metric success against shop_bigquery"*, by a person.

**Scenario: the agent did the comparing (decision 7).** In chat, Claude computed 412 through the BigQuery MCP and the user said "yes, Shopify says 412 too". Claude offers to record it and calls `propose_patch` with `op: record_check, name: success, against: shop_bigquery, note: …`. This op is queued whatever the edit mode. The inbox shows a different card:

```
│ An agent reports a check: success gave 412 paid orders for 16–22 Nov in        │
│ shop_bigquery, and the user said Shopify's report shows the same.              │
│ agent via MCP · 28 Nov 10:02                                                   │
│ Did you see this match yourself?        [ Yes, mark as checked ]  [ Not now ]  │
```

The grade names the person who clicked, and the note keeps "reported by agent via MCP". An agent saying "I checked" is itself a model's output, so it enters as a report and a person promotes it. Without this op the loop still works: the agent says "you can mark it as checked on the Logic page".

**Scenario: a file changed by hand.** Someone edits the filter of `success` in their editor and commits. Nothing stamped the change, so the fingerprint no longer matches. The page shows "changed since it was checked on 28 Nov", `describe` says the same, and `lorefile validate` prints `warning W113 trust.yaml: metric success changed since it was checked (Priya, 2026-11-28); a person has to accept it again`. One click on "Looks right", or `lorefile trust accept success`, grades it accepted.

### What existing spaces see

Nothing changes until someone grades something. A space with no `trust.yaml` compiles exactly as before, with no new warnings, so the shipped examples and packs stay clean. The page's "What to do next" gains one line when a space has ungraded metrics, bindings, words or guardrails: *"41 definitions are ungraded. If you know them, vouch for them."* The button lists them, asks for a name, and writes `accepted` for all in one commit, *"Vouched for 41 definitions"*.

---

## Part 2 · `basis:` on metrics

**Before.** The e-commerce example's caveat says, in prose: *"Platform-reported conversions (Meta, Google) are attributed and modelled, not the same thing. Compare them to this number; never add them to it."* The compiler cannot see a sentence, and an agent in a hurry skips caveats. So a report says "932 conversions last week", which is 412 paid orders plus 520 that Meta gave itself credit for.

**After.** One key says what kind of number each metric is:

```yaml
metrics:
  - metric: success
    basis: transactional          # counted from the business's own records
    …
  - metric: meta_conversions
    basis: attributed             # credit a platform assigns by its own rule
    …
```

| `basis` | Means | Example |
|---|---|---|
| `transactional` | Counted from the business's own records. What happened. | paid orders, refunds, sign-ups |
| `attributed` | Credit a platform or an attribution rule assigns to a channel. A share-out of what happened. | Meta's 7-day-click conversions, last-click revenue |
| `modelled` | An estimate from a statistical model. | a marketing mix model's contribution, a forecast, platform-modelled conversions |
| `experimental` | Read from a controlled test. Causal, for that test's cells and dates only. | a geo holdout, a conversion lift study |

### Same mechanism as the grade, or separate?

| | A · separate key on the metric (recommended) | B · one mechanism: `basis` as extra grades | C · leave it to the "Metric semantics" backlog item |
|---|---|---|---|
| What it says | Two answers to two questions: what kind of number is this, and who stands behind the definition | One ladder, e.g. modelled < attributed < transactional < checked | Nothing yet |
| Fits the facts | A transactional metric can be an agent's unreviewed guess; a modelled one can be carefully checked | They are not one ladder: an experiment answers "what caused it", a ledger answers "what happened" | |
| Who writes it | Whoever writes the metric, agents included; it is part of the definition, so it is graded with it | Would have to be split between agents and people | |
| Cost | One optional enum on the metric and on each variant, shown in `summary` and `describe` | A confused ladder | The Trust line cannot say "attributed", and the never-add rule waits |

**Recommendation: A.** `basis` is a fact about the business, like `grain` or `owner`. The grade is a fact about the review. They meet only in what the agent says: *"412 paid orders (transactional, checked 28 Nov); Meta reports 520 conversions (attributed, definition not reviewed)"*.

Details: `basis:` is optional on a metric, and on a variant when variants differ (store orders are transactional, Meta's count is attributed); a variant's value wins over the metric's. An unknown value is an error, like any unknown enum. There is no warning when it is absent, so every shipped folder keeps compiling clean; the two metrics in `examples/ecommerce/logic/measurement.yaml` get a `basis:` as content. `add_metric` in `propose_patch` accepts it. `attribution:` (which window, which model), the entity anchor, `time:` and the rest of ontologist F2, F3 and F7 stay in the backlog; when the packs' ROAS and CPA move from glossary prose into `logic/`, they arrive with a `basis:`.

The discussion's point shows here too. A `modelled` number is a model's output however good its definition's grade is, so the agent says "modelled" every time. Tying a modelled metric to the experiment that calibrated it (`calibrated_against:`) is the natural next step and is left for later.

---

## Part 3 · What agents and people see

### `summary`

One line near the top, and a short mark on the lines that need one. Accepted and ungraded lines carry no mark, so the summary stays the size it is.

```
# lumen-ecommerce — ontology summary
…
Trust: 3 checked against data · 14 accepted by a person · 1 proposed by an agent and not
reviewed · 41 ungraded. Say "unverified" when you use anything marked UNVERIFIED below.

## Metrics (what counts, and where it comes from)
- **success** (per day) [transactional · checked 2026-11-28 against shop_bigquery]: The one number …
- **blended_roas** (per week) [attributed · UNVERIFIED: an agent proposed it, nobody has reviewed it]: …

## Rules
- **EC-031** (Order) [UNVERIFIED]: Orders above $5,000 need a manager — `total_usd <= 5000.0`
```

### `describe`

Every graded kind gains a `trust` object; `say` is a sentence the agent can use as it is.

```json
{ "kind": "metric", "name": "blended_roas", "basis": "attributed", "…": "…",
  "trust": { "grade": "proposed", "by": "agent via MCP", "at": "2026-11-24",
             "say": "Unverified: an agent proposed this definition on 24 Nov and no person has reviewed it." } }

{ "kind": "metric", "name": "success", "basis": "transactional", "…": "…",
  "trust": { "grade": "checked", "by": "Priya via page", "at": "2026-11-28", "against": "shop_bigquery",
             "note": "412 paid orders for 16–22 Nov, the same as Shopify's own report",
             "say": "Checked against shop_bigquery by Priya on 28 Nov." } }
```

An entity's `source` gains the binding's `trust`; a guardrail's brief (in `describe`, `constraints_for` and `validate`) gains `grade`; `describe("basket")` shows the word's. Ungraded things answer `"trust": { "grade": "ungraded" }` with no `say`. A changed one answers `"grade": "ungraded", "changed_since": "checked 2026-11-28"`.

### The server's instructions

Three sentences added to the text every agent gets at `initialize`:

> Definitions carry a trust grade. When you rely on one marked proposed or UNVERIFIED, say so before the number or the action: an agent wrote it and no person has reviewed it. Checked means a person compared it with real rows; say who and when if the user asks how sure this is. You cannot set a grade: a person accepts or checks in the web page. Numbers with a different `basis` (transactional, attributed, modelled, experimental) are shown side by side, never added.

### How loudly should an agent speak about ungraded facts?

| | A · flag only proposed (recommended) | B · flag ungraded too | C · flag nothing, show it only in `describe` |
|---|---|---|---|
| Day one, existing space | Nothing changes in how agents talk | Every answer starts with a warning about every definition | Nothing changes |
| An agent's guess | Named as a guess | Named, among forty other warnings | Stated as fact unless the agent happened to call `describe` |
| Risk | An imported or hand-pasted file from a model is not flagged until someone looks | People stop reading the warning | The gap stays open |

**Recommendation: A.** A warning on everything is a warning on nothing. The Trust line still counts the ungraded ones, the page nudges a person to vouch for them, and an agent asked "how sure are you?" has the answer.

### `validate`

**Before.** An agent with edit mode on adds EC-031 (a made-up rule for this scenario), "orders above $5,000 need a manager", as a blocking rule nobody asked for. The next day `validate(cancel_order, ORD-1040)` says blocked, and the agent tells a customer-service lead that company policy forbids it.

**After.** The same verdict, with the truth about the rule:

```
✗ cancel_order is blocked by EC-031 (unreviewed: an agent proposed this rule and no person has accepted it).
 FAIL  EC-031   Orders above $5,000 need a manager — violated for order=ORD-1040     [proposed]
 note: EC-001 read Order rows through a data binding nobody has reviewed (proposed by an agent, 24 Nov).
approval: support-lead@lumen.example
```

| | A · verdict unchanged, say what is unreviewed (recommended) | B · an unreviewed block counts as a warning | C · unreviewed rules are skipped |
|---|---|---|---|
| A wrong rule an agent invented | Blocks; the agent says "blocked by a rule nobody has reviewed", and a person accepts or removes it in one click | Does not stop work | Does not stop work |
| A right rule an agent wrote down | Protects from the first minute | Does not protect until someone reviews | Does not protect |
| Failing safe | Yes: a false block costs a question, a false pass costs an order | No | No |

**Recommendation: A.** Each rule result carries its `grade`; the report gains `unverified: [...]` naming unreviewed rules and bindings it relied on; the summary sentence names them. `pass`, `warn`, `fail` and `unknown` mean what they mean today. Rules that can read a grade themselves ("a budget change needs a checked success metric") are left for later, with metric references in rules.

### The page

A chip on every metric card, binding, word and guardrail. Grey: ungraded. Amber: proposed by an agent. Blue: accepted. Green: checked, with the date and source. Amber with a pencil: changed since it was graded.

```
┌ Logic · metrics ─────────────────────────────────────────────────────────────────┐
│ success         per day · transactional                    ● checked 28 Nov       │
│ Paid orders. Not sessions, not platform conversions.         Priya · shop_bigquery │
│ shop_bigquery · orders · count distinct order_id                                   │
│ "412 paid orders for 16–22 Nov, the same as Shopify's own report"                  │
│                                                                                    │
│ blended_roas    per week · attributed                      ▲ proposed by an agent  │
│ Meta's reported conversion value over spend.                 24 Nov · not reviewed │
│ meta_ads · get_insights                        [ Looks right ]  [ Edit ]  [ Remove ]│
└────────────────────────────────────────────────────────────────────────────────────┘

┌ Agents ────────────────────────────────────────────────────────────────────────────┐
│ WAITING FOR YOUR REVIEW                                                             │
│   An agent learned: added word "cart" means Order            [ Accept ] [ Not now ] │
│   An agent reports a check: success gave 412 …   [ Yes, mark as checked ] [ Not now ]│
│ WRITTEN BY AGENTS, NOT REVIEWED                                                     │
│   metric blended_roas · 24 Nov · agent via MCP            [ Looks right ] [ Show ]  │
│   guardrail EC-031 · 25 Nov · agent via MCP               [ Looks right ] [ Remove ]│
└─────────────────────────────────────────────────────────────────────────────────────┘
```

Also: Accept on a proposal says under the button what it will record ("saved as accepted by Priya"); "What to do next" on the Map lists unreviewed and changed things before anything else; Files shows `trust.yaml` to read, never to edit, with the note "Who stands behind what. Written by Lorefile."; Settings gets "Your name, shown on what you accept and check", kept in the browser. A read-only space (a shipped example, or a server without `--allow-edit`) shows the chips and no buttons.

---

## Part 4 · Grades in `export --rdf` and `export --ossie`, and back in

**Scenario.** Lumen exports its space for the data team's GraphDB and for dbt. The architects ask which of these definitions anyone has verified. Today the files cannot say. A month later a consultant sends back an "improved" Ossie file in which every metric is marked verified; nobody at Lumen checked any of it.

### Going out

| | A · `lorefile:` statements only | B · `lorefile:` grade plus a PROV-O record (recommended) | C · grades stay home |
|---|---|---|---|
| Lorefile reads it back exactly | Yes | Yes | Nothing to read |
| Another tool understands who, when and against what | No, they are Lorefile's own words | Yes: PROV-O is the W3C's vocabulary for exactly that, and provenance tools read it | No |
| Cost | Small | One more built-in prefix (`prov`), one small node per graded thing | None |

**Recommendation: B.** The grade itself has no standard word, so it is `lorefile:trust`, one of `lorefile:Proposed`, `lorefile:Accepted`, `lorefile:Checked`. The evidence is a PROV activity. A sketch, to be checked against the PROV-O recommendation before a line is written:

```turtle
@prefix prov: <http://www.w3.org/ns/prov#> .

:success a lorefile:Metric ;
    lorefile:trust lorefile:Checked ;                         # the grade, on the thing itself
    lorefile:definition "{ … \"basis\": \"transactional\" … }"^^rdf:JSON .   # basis travels inside, as every metric key does

:trust.metric.success a lorefile:Grading , prov:Activity ;   # the record behind the grade
    lorefile:about :success ;
    lorefile:grade lorefile:Checked ;
    prov:wasAssociatedWith [ a prov:Person ; rdfs:label "Priya via page" ] ;
    prov:endedAtTime "2026-11-28T00:00:00Z"^^xsd:dateTime ;
    prov:used :success , :source.shop_bigquery ;              # what was run, and against which source
    rdfs:comment "412 paid orders for 16–22 Nov, the same as Shopify's own report" ;
    lorefile:of "3f9a1c2e70b4" ;
    lorefile:file "trust.yaml" .

:blended_roas lorefile:trust lorefile:Proposed ;
    prov:wasAttributedTo [ a prov:SoftwareAgent ; rdfs:label "agent via MCP" ] .
```

A binding's grade sits on the entity with `lorefile:aspect "binding"` on its record, since the binding is not a node of its own. Ungraded things get no statement.

**Ossie.** The whole of `trust.yaml` rides in the root `nonportable_extensions` box under `vendor_name: lorefile`, next to the glossary and the file layout, and `basis` rides in each metric's box with `grain` and `owner`; neither needs a new mechanism. One addition makes the grade useful to tools that ignore the box: a metric or dataset whose definition is **proposed** also gets one sentence in its `ai_context`, the note Ossie reserves for AI tools: *"Unverified: an agent proposed this definition and no person has reviewed it."* The export writes `ai_context: {synonyms: […]}` today, so the build session reads the current draft for the right free-text key, and checks whether the draft has gained a field of its own for review status; if it has, map `checked` and `accepted` to it as well. The sentence is generated, so import recognises it and does not turn it into a description.

### Coming in

| | A · imported facts start ungraded; claims go in the report; `--keep-grades` for your own export (recommended) | B · import whatever grades the file states | C · drop them silently |
|---|---|---|---|
| The consultant's file | 9 definitions arrive ungraded; the report says "9 arrived with a grade (9 checked); they start ungraded here" | 9 definitions arrive checked, by people who never saw them | Arrive ungraded, and nobody knows a claim was made |
| A team re-importing its own export | Adds `--keep-grades`; grades come back where the fingerprint still matches | Works | Loses its grades |
| The rule "a person here stood behind this here" | Kept | Broken | Kept |

**Recommendation: A.** A grade is a statement by this team about this copy. A file from outside can say anything, so on the way in it is a claim, listed for a person. `--keep-grades` is that person deciding, at the command line, that the file is their own; even then a grade whose fingerprint does not match the imported fact is dropped and listed. Grades found in another tool's own words (PROV statements in someone's OWL file, a review status in someone's Ossie model) are never turned into grades; the report counts them. The round-trip tests add `--keep-grades` and stay lossless, and a new test proves the default drops the grades.

`basis` is part of the metric and comes back with it, like `grain`.

---

## Part 5 · What this does not do

- **It does not grade answers.** Lorefile grades the definitions it holds. It never sees an agent's free-text reply or the number in it, and it does not score them. An agent using a checked definition can still do the arithmetic wrong.
- **It does not run the check.** "Checked" records a person's statement: who, when, against which source, what they compared. Lorefile fetches no rows and calls no tool to confirm it, not through the DuckDB door either. The ontology still never executes and never holds a credential.
- **It is not tamper-proof.** Someone with the repository can edit `trust.yaml` by hand, as they can edit a guardrail. The grade is as strong as the team's review of its own repository (ARCHITECTURE §17.5, "the one copy"), and history shows who wrote each line. With one shared login token, "Priya" is a typed name, not a signed-in person, until accounts exist.
- **It does not stop an agent using an unverified definition.** It tells the agent to say so. Blocking on a grade would need rules that can read grades, which is left for later.
- **It does not grade data quality.** A checked binding means the entity points at the right table, not that the table's rows are clean.
- **No scores, no decay.** Three words, not a confidence percentage. A check from a year ago still reads "checked", with its date; nothing expires on its own.
- **Not everything is graded yet.** Entities, fields, links, functions, actions and sources have no grade in the first version, and the Trust line says which kinds it counts.

---

## Build notes, file by file

Notes for every session: `cargo build --offline` works, run cargo with the sandbox off; `cargo test` must stay green (74 today); every folder under `examples/` and `packs/` must compile with no errors and no warnings (`tests/cli.rs`), which is why an absent `trust.yaml` and an absent `basis:` produce nothing. Commit nothing; Vinay commits.

| File | What changes |
|---|---|
| `src/schema.rs` | `TrustFile { trust: TrustBook }`, `TrustBook { metrics, bindings, terms, guardrails }` (each an `IndexMap<String, TrustRecord>`), `TrustRecord { grade, by, at, of, against: Option, note }`, `Grade { Proposed, Accepted, Checked }`, all `deny_unknown_fields`. `Basis { Transactional, Attributed, Modelled, Experimental }` as `basis: Option<Basis>` on `MetricDef` and `MetricVariant`, skipped when absent so existing files serialise as before. |
| `src/loader.rs` | `classify`: one more branch, a file whose top-level key is `trust` is the trust file (the kinds sit one level down, so it is never mistaken for a metrics or glossary file). `Raw.trust`. A second trust file is an error, like a second manifest. |
| new `src/trust.rs` | The fingerprint: the thing's compiled value as JSON with keys sorted at every level, loader-set fields (`file`) left out, hashed with a stable hash written in the crate (FNV-1a) or `sha2`, never `DefaultHasher`, which may change between Rust versions; first 12 hex characters. `resolve(ont) → kind, name → { grade, record, changed }`. `restamp(before, after, who) → new trust.yaml text`: compare fingerprints of every graded thing before and after a change; new or changed ones get the grade for `who`, deleted ones lose their entry, a person's words-only change to a checked thing keeps the check with a fresh `of`. `would_overwrite_graded(before, after)` for decision 5. The sentences (`say`) live here, used by MCP, CLI and page alike. |
| `src/ontology.rs` | `Ontology.trust` (resolved once at load). `summary_markdown`: the Trust line and the marks, `basis` in the metric line. `describe` and `constraint_brief`: `trust` and `basis`. `describe_markdown`: one "Trust:" line for metrics. |
| `src/validate.rs` | E018: a `checked` record without `against`, `by` or `at`; `against` that is no source; `checked` on a word; an entry with an empty `of`. W113: fingerprint mismatch. W114: an entry naming something that does not exist. Nothing when the file is absent. |
| `src/edit.rs` | `Note` gains `who: Who { Agent, Person }` and an optional name. `apply`, `apply_many` and `apply_files` call `trust::restamp` and write `trust.yaml` in the same commit as the facts, **before** the fact files, so a crash leaves a harmless orphan (W114) and never an unflagged fact. Read `trust.yaml` again inside the write lock and change only the entries this write touches; `mcp` on stdio is a second process, so guard the read-modify-write with a lock file under `.lorefile/`. `safe_rel_path` callers refuse `trust.yaml` for agents and for the page's raw editor, with the sentence "trust.yaml is written by Lorefile when a person accepts or checks something". New `grade(root, cfg, kind, name, to, record)` for Looks right, Mark as checked and Vouch for all; commit messages "Accepted metric success", "Checked metric success against shop_bigquery", "Vouched for 41 definitions". Undo does **not** restamp: `trust.yaml` is one of the files in the commit being undone, so it goes back with the facts through the existing key-by-key YAML merge, and the fingerprint check catches anything left inconsistent. |
| `src/proposals.rs` | `Proposal.check: Option<CheckClaim { kind, name, against, note }>` for `record_check`; such a proposal has no file content. |
| `src/patch.rs` | `add_metric` takes `basis`. `record_check` is planned here (the thing exists, the source exists, the thing is at least accepted) and returns a claim, not a file. |
| `src/mcp.rs` | `propose` and `propose_many`: when `would_overwrite_graded`, queue instead of writing, and say why. `record_check` always queues. `propose_change_result` adds the grade sentence. `instructions()` gains the three sentences. The `propose_patch` schema lists `record_check` and `basis`. |
| `src/rules.rs` | `RuleResult.grade`; `CheckReport.unverified: Vec<String>` (unreviewed rules that were evaluated; unreviewed or changed bindings of the entities whose rows were read); the summary sentence names them. `verdict_of` is untouched. |
| `src/serve.rs` | `/api/proposals/accept`: `who: Person` on the note, and the `check` kind writes the grade. `POST /api/trust` `{op: accept \| check \| vouch_all, kind, name, against, note, by}` behind the same write gate (`--allow-edit`, the header, the Origin check; 403 for a shipped example). `GET /api/trust` lists every graded kind with its resolved grade. `/api/summary` gains the counts. The write paths pass the name from the page. |
| `src/ui/index.html` | Chips on Logic cards, the entity's Data box, Words and Guardrails; "Looks right"; the "Mark as checked…" form; the two Agents sections and the check card; the Map line and "Vouch for all"; Files note; the Settings name box (browser storage, sent with every write). Light theme only. |
| `src/main.rs` | `lorefile trust` (list, grouped by grade), `lorefile trust accept <name> [--all] [--by NAME]`, `lorefile trust check <name> --against <source> --note "…" [--by NAME]`, both as `person via CLI`. `import … --keep-grades`. |
| `src/rdf.rs`, `src/iri.rs` | `prov` joins the built-in prefixes. Export: `lorefile:trust` on the thing and one `lorefile:Grading` record per entry, as sketched. Import: with `--keep-grades`, records Lorefile wrote are restored where `of` matches; otherwise counted in the report. Foreign PROV statements are counted, never read as grades. |
| `src/ossie.rs` | The trust book in the root extension; the generated `ai_context` sentence for proposed metrics and datasets, recognised and dropped on import; `--keep-grades`; the report lines. |
| `src/spaces.rs` | Copying a space leaves `trust.yaml` behind: a copy is a new place, and nobody has stood behind it there. |
| `examples/ecommerce/logic/measurement.yaml` | `basis: transactional` on `success`. `paid_performance` is platform spend next to success: give it `transactional` if Vinay agrees that what a platform billed is a record, otherwise leave it without one. No `trust.yaml` is shipped. |
| Docs, last | README (step 6 "Accept what it learned", the page tour, "What an agent can ask", the file format, the codes, the CLI list), ARCHITECTURE (a short §19 and one row in §17.1: "recording who stands behind each definition"), BACKLOG (move `basis` out of item 1; add the left-for-later list below), `docs/standards-explained.md` (the Turtle and Ossie samples gain the grade), then `python3 scripts/build-tutorial.py`. |

**Left for later**, to go into BACKLOG.md: grades on entities, fields, links, functions, actions and sources; rules that read a grade; `calibrated_against:` on modelled metrics; a check that expires or is asked for again after N days; `lorefile trust --audit`, which compares each line of `trust.yaml` with who committed it and flags one an agent's commit wrote; named, signed-in people once accounts exist; a machine check of a binding for local files (the table exists, the key is unique), if the frozen DuckDB door is ever reopened.

### Hand-off plan

| Order | Session | Scope (files) |
|---|---|---|
| 1 | **grades-core** | `schema.rs`, `loader.rs`, `trust.rs`, `ontology.rs`, `validate.rs`, `edit.rs`, `proposals.rs`, `patch.rs`, `mcp.rs`, `rules.rs`, `main.rs` (the `trust` commands), unit tests, `tests/mcp_loop.rs`, `tests/cli.rs` |
| 2 | **grades-page** | `serve.rs`, `src/ui/index.html`, `tests/http.rs`; after 1 |
| 3 | **grades-export** | `rdf.rs`, `iri.rs`, `ossie.rs`, `main.rs` (`--keep-grades`), `spaces.rs`, `tests/rdf.rs`, `tests/ossie.rs`; after 1, alongside 2 (no files shared except `main.rs`, where 3 touches only the import arguments) |
| 4 | **grades-docs** | the example's `basis`, README, ARCHITECTURE, BACKLOG, standards-explained, the tutorial; last |

---

## Test ideas

**The ladder (unit, `trust.rs` and `edit.rs`)**
- An agent's `add_metric` with edit mode on writes the metric and a `proposed` entry in one commit; the author is the agent.
- Accept on a queued proposal writes `accepted` by the person; the agent is still the commit's author.
- A person's page edit of a proposed metric makes it accepted.
- "Mark as checked" without a source or a note is refused; with both it writes `checked`, and history has "Checked metric success against shop_bigquery".
- A person changing the description of a checked metric keeps `checked` with a new `of`; changing its filter makes it accepted.
- Deleting a metric removes its entry; renaming one leaves no orphan.
- A batch of five ops stamps five entries in one commit.
- A space with no `trust.yaml` compiles with the same diagnostics as before, byte for byte in `validate --json`.

**Agents cannot promote themselves (`tests/mcp_loop.rs`, `tests/mcp_http.rs`)**
- `propose_change` with file `trust.yaml` is refused, with edit mode on and off, and nothing is written or queued.
- A metric, type, glossary or guardrail file containing a `trust:` or `grade:` key fails to compile (the unknown-key error). Source files take any key, and sources are not graded.
- With edit mode on, an agent's `set_source` on an accepted binding is queued, not written, and the result says why; on an ungraded binding it is written as proposed.
- `record_check` is queued with edit mode on; accepting it writes `checked` by the person with "reported by agent" in the note; `record_check` on a proposed metric is refused ("accept it first").
- The `initialize` instructions contain the unverified sentence; `summary` shows the Trust line and marks only proposed and checked lines; `describe` returns `trust.say`.

**Changes behind the grade's back**
- Editing the YAML on disk after a grade gives W113, `describe` answers ungraded with `changed_since`, and `lorefile trust accept` clears it.
- Reordering keys or adding a comment in the fact file does not change the fingerprint.
- An entry for a metric that does not exist gives W114; `against` naming no source gives E018; `checked` on a word gives E018.
- Undo of the commit that added a metric removes its entry too; undo of a check puts the grade back to accepted.
- Two stamps from two processes at once both land (the lock file).

**`validate`**
- A proposed blocking rule still gives `fail`, with `grade: proposed` on the result and the rule named in `unverified`.
- A pass over rows read through a proposed binding is still `pass`, with the binding in `unverified`.
- With no `trust.yaml`, the report equals today's apart from the empty list.

**`basis`**
- `basis: attributed` round-trips through `describe`, Ossie and Turtle; `basis: guessed` is an error naming the four values; a variant's basis wins over the metric's; `add_metric` with `basis` writes it.

**Export and import (`tests/rdf.rs`, `tests/ossie.rs`)**
- Every shipped folder still round-trips unchanged (they have no grades).
- A graded copy of the e-commerce example exported and imported with `--keep-grades` gives the same `trust.yaml`; without the flag there is none, and the report counts what was claimed.
- With `--keep-grades`, a grade whose fact was edited in the exported file is dropped and listed.
- rdflib parses the Turtle; the PROV statements are well-formed (`prov:Activity` with `prov:wasAssociatedWith`, `prov:used`, `prov:endedAtTime`).
- Ossie's own validator still passes the three documents; a proposed metric's `ai_context` carries the sentence, and importing it back does not add the sentence to the description.

**The page (`tests/http.rs`, then by hand in the browser)**
- `POST /api/trust` without the write header is refused; on a shipped example it is 403; `vouch_all` writes one commit.
- Chips match `GET /api/trust`; a read-only server shows chips and no buttons.

---

## What the review changed (2026-11-26, after the build)

The build was reviewed the same day: the core read line by line, a scratch space probed the way an agent would, and two independent reviewers on the exports and on the page and docs. The decisions stand. These were wrong, in the build or in this spec, and are fixed. Each has a test, except where a row says otherwise and for these: the `trust.yml` load error, `grades` appearing only on a written change, and everything on the page (it has no automated tests; those rows were checked by hand in a browser). `cargo test` runs 95 tests, clippy is clean and both slim builds compile. Still not run by anyone: the rdflib parse and Ossie's own validator, which need downloads.

| Found | Fix |
|---|---|
| An agent with edit mode on could point `shop_bigquery` at another dataset; the change was written at once and `success` still read "accepted". This spec's fingerprint covered the recipe and not the source it reads. | The fingerprint of a metric and of a binding now carries the definition of every source it reads. An agent's change to such a source is queued; a hand edit gives W113. |
| A crafted import file could get a grade in without `--keep-grades`: an object carrying `lorefile:file "trust.yaml"` (RDF), or a "source" with `file: trust.yaml` in the Ossie box, was written as the grades file. | No imported object may name the grades file as its path, in either importer; tests import a forged file with and without the flag. |
| An agent's reported check was not tied to what it compared: a person could edit the rule, then click "Yes", and the new rule became checked on the old comparison. | The report carries the fingerprint; the click is refused when it no longer matches. |
| The page built click handlers as `'${esc(name)}'`. The browser undoes HTML escaping before running the script, so a glossary word such as `y');…//`, which an agent can write with `propose_change`, ran script on "Looks right"; a word with an apostrophe gave a dead button. The pattern was page-wide and older than this feature. | All 45 handlers write names as JSON strings, then escape (`jq`). Checked in a browser with both words: no script ran, both buttons worked, and all 421 handlers on a tour of the pages parse. No automated test: the page has none. |
| "Vouch for all" also accepted definitions that had changed since a person graded them, without anyone looking, and its count disagreed with the Map's. | It grades only what nobody has ever graded; the confirm text and the count say the same. |
| A hand-edited definition was listed under "Written by agents, not reviewed". | Agents has two lists: written by agents, and changed since a person graded them. |
| The page promised a name the server silently replaced (an email address, a name containing "agent"). | The name box and the check form refuse such a name and say the rule; every write's reply carries the `by` that was used. |
| `where_is`, the tool an agent uses to find an entity's rows, showed no grade. | It returns the binding's `trust`. |
| A hand-made `trust.yml` was read as the grades file but saves wrote `trust.yaml` beside it, which broke the space. `Trust.yaml` and the like queued a junk proposal. | Only `trust.yaml` is the grades file; every spelling is refused for writes, and a `trust.yml` is a load error that says to rename it. |
| Grade claims in an import that did not parse vanished without a line; with Ossie one bad line dropped all of them. | Claims are read one by one; unreadable ones are counted in the report. |
| `--keep-grades` could restore a line the compiler then rejected (a check against a source that is not in the space). | Such a line is dropped and listed. |
| Our "Unverified: …" sentence leaked into a description when another tool appended to it or flattened `ai_context` to a string. | It is stripped wherever it leads the text, in both shapes. |
| Two people whose names reduce to the same slug (any two names in a non-Latin script) shared one PROV agent node. | One node per distinct name. |
| A duplicate glossary word was graded by its last entry while the RDF export marked its first. | A word written twice is one graded thing covering every meaning, so adding a second meaning is a change. |
| A non-object `lorefile:definition` on a metric made the RDF import panic. | It is dropped with a note. Not covered by a test. |
| A check's note had no length limit and a control character made the save fail with no reason shown. | Notes are flattened to one line and kept to 500 characters; the form shows the reason for any refusal. |
| `POST /api/trust` ignored an unknown `kind` and gave a wrong reason for a wrong one; a queued change reported grades as if they had been set. | Both are errors with the right sentence; `grades` appears only on a change that was written. |
| Docs said every rule result carries `grade` (only once the space has a `trust.yaml`) and that the import report lists the claims (it counts them); the README's status line did not mention grades. | Corrected. |

Left as found, and why: an import with `--keep-grades` into a space with a different namespace restores few grades, because the imported facts gain explicit IRIs and so differ from what was graded; `prov` is now a reserved namespace on import, so classes in the PROV namespace are skipped; a failed network request during a grade click shows nothing (the page's `api()` helper predates this work); the entity Data tab fetches sample rows without the space prefix (also older).

