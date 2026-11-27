# Spec: reviewing what agents propose, in plain words

Written 2026-11-27 for Vinay to decide on. **Status: built 2026-11-27**, every decision as recommended (Vinay approved all 24, decision 23 included: "Yes, build all"), on branch `claude/learn-file-6a954e`. Tested end to end with the real binary and checked by hand in a browser; not yet tried live with ChatGPT. What differs from the text below is listed at the end, in "What changed during the build".

It comes from the live `learn_file` test. ChatGPT learned `promo_sales.csv` and replaced its first, shape-only proposal with `p-20261127-080758-6`. That proposal has 14 fields, money in USD, `promo_sale_revenue = sum(revenue) where refunded = false`, 0 errors and one warning. Then Vinay opened Agents to accept it and found a card he could not read quickly. The findings are in the OAuth sessions' shared folder, `review-flow-findings.md`.

**Base.** Build on branch `claude/learn-file-6a954e` (`fd7adee`, on top of the OAuth branch), where the page, the proposal store and `learn_file` now live.

**Standing rules, unchanged** ([BACKLOG.md](../BACKLOG.md)):
- The ontology never executes and never holds credentials.
- Agents and builders write the YAML; people review it.
- Light theme only.
- Honest status.

**Read first:**
- `src/serve.rs` (`/api/proposals/accept`, `/api/proposals/dismiss`, `page_by`, `LOGIN_HTML`)
- `src/proposals.rs`
- `src/mcp.rs` (`propose`, `propose_many_titled`, `link_base`, `accept_at`)
- `src/oauth.rs` (`approval_page`, connections)
- `src/edit.rs` (`EditConfig`, `apply_many`, `undo`, `graded_overwrites`)
- `src/patch.rs` (`BatchPlan`)
- `src/ui/index.html` (`renderAgents`, `card`, `diffHtml`, `renderMap`)
- `specs/2026-11-27-evidence-grades.md`
- `specs/2026-11-27-learn-file.md`

The example carried through is `p-20261127-080758-6`.

---

## The gap, in one review

**Before (today).** Agents › Waiting for your review shows:

```
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│ Learned promo_sales.csv: PromoSale (14 fields, key line_id) from user_files; metric        │
│ promo_sale_revenue = sum(revenue) where refunded = false   [a file]  sources/user_files.yaml│
│                         agent via MCP (connection "ChatGPT", approved 27 Nov) · 2026-11-27 08:07:58 │
│ Use the user-confirmed definition of promo sales revenue: sum revenue in USD, excluding…   │
│ ⚠ W109 metric `promo_sale_revenue` has no owner (who decides what it means)               │
│ Show what changes                                                    [Accept] [Not now]   │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

**What is wrong with it:**
- The title is a log line.
- The path names only the first of three files.
- The time is UTC.
- The warning is a code.
- "Show what changes" opens three line diffs of YAML.

**Things found while reading the code:**
1. **The file headings are wrong.** In "Show what changes", `logic/metrics.yaml` is headed "bound PromoSale to user_files.promo_sales". A batch stores one summary per *op* next to each *file* by position, and this batch has four ops in three files.
2. **Accept needs a second server.** Accept answers 403 unless the server runs `--allow-edit`. But with `--allow-edit`, ChatGPT's own saves are written straight away instead of waiting. The live test needed a second server just to press Accept.
3. **The link from chat can lose its way.** `learn_file`'s `accept_at` link ends in `#agents`. The browser never sends that part to the server, so a sign-in through the form returns to the Map, not Agents. It also opens the whole page, not the card.
4. **Who set it aside is not recorded.** Not now is logged as `person via page` whatever name the person set.
5. **The activity log names a file, not the change.** It says "accepted the change to sources/user_files.yaml".
6. **Proposals with errors still queue,** with a card that says to "fix the proposal". Nobody can: agents cannot edit a proposal, and the page has no way to.
7. **The note says Accept doesn't check `before`. It does, since the 2026-11-26 review:** a file that changed answers 409. But it refuses for *any* change to the file. A metric accepted in between makes every other waiting proposal that touches `logic/metrics.yaml` unacceptable, with "ask the agent to propose it again".
8. **There are two review lists.** "Waiting for your review" holds proposals. "Written by agents, not reviewed" holds what an agent wrote with edit mode on. They look different and use different buttons for the same decision.

**After.** One review screen, reached from the card or from a link in the chat:

```
┌ ChatGPT wants Lorefile to remember a file ───────────────────────────────────────────┐
│ promo_sales.csv · asked in a chat with ChatGPT · today 1:37 pm                          │
│ ChatGPT says: "Use the user-confirmed definition: sum revenue in USD, excluding          │
│ refunded order lines."                                                                    │
│                                                                                           │
│ ADDS   1 thing (Promo sale, 14 columns) · 1 number (Promo revenue)                        │
│        · 1 place rows come from (files people give an agent)                               │
│                                                                                           │
│ + Promo sale · one row is one order line · key line_id                                     │
│     revenue     money (USD)   what the customer paid for the line                          │
│     refunded    yes / no      true when the line was refunded                              │
│     units       whole number  how many units were on the line                              │
│     + 11 more columns                                                       [ show all ] │
│                                                                                           │
│ + Promo revenue = add up revenue, leaving out lines where refunded is true                 │
│     Lorefile can't preview this number: the rows stay with the agent.                      │
│                                                                                           │
│ ? Who decides what Promo revenue means?   [ Vinay              ]  (optional)               │
│                                                                                           │
│ You can take this back until something else changes (history is off for branda).          │
│ [ Accept as Vinay ]   [ Not now ]                         Show the files · Show on the map │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

Next month, a changed file shows only the difference:

```
┌ ChatGPT wants to update Promo sale ──────────────────────────────────────────────────┐
│ promo_sales_2027-01.csv · asked in a chat with ChatGPT · 3 Jan                          │
│ CHANGES  Promo sale: 1 column added                                                     │
│ ~ Promo sale                                                                             │
│     + gift_wrap   yes / no   true when the order was gift-wrapped                         │
│   region is not in this file. It stays in Lorefile; nothing that counts uses it.          │
│ [ Accept as Vinay ]   [ Not now ]                                                         │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## Vinay's four questions, answered

| Question | Answer |
|---|---|
| 1. A before/after review, like a pull request? | **Yes, but by meaning, not by lines.** The screen lists what is added (+), changed (~, with before → after) and removed (−): things, columns, numbers, words and rules, in plain words. The YAML diff stays one click away for whoever wants it. A line diff is the wrong unit here. Adding one column rewrites a whole type file, and an op on a commented file drops its comments. Two proposals can touch the same file for unrelated reasons. |
| 2. A graph view? | **Yes, on the Map, when it adds something.** "Show on the map" draws the space as it would be after Accept, with new things green, changed amber and removed red. It appears only when a proposal adds, changes or removes entities or links. For a learned file it would be one green box, which the words already say better. |
| 3. Approve in ChatGPT? | **No, and nothing is lost.** Accept means a person looked at it. Lorefile cannot tell a person's "yes" in chat from the model's own tool call: both arrive on the same OAuth token, and a file or web page can talk a model into calling a tool. The two real needs are covered differently. The chat gives a **link that opens on this proposal's review screen**, through sign-in. For people who want chat-only, a connection can be set to **write straight away, marked unverified**, as edit mode does today, and be reviewed later with one click (Part 4). |
| 4. Branches and merges, like GitHub? | **No branches.** A proposal already is the pull request, Accept is the merge, and History is the commit log when git is on. Branches would need agents that can run git (ChatGPT cannot). They would need history on (branda has it off). And they would make people read YAML diffs, the thing this spec removes. Teams that review on GitHub keep doing it there with the YAML in their repository; BACKLOG item 10 left git branches out on purpose. |

---

## Decisions

One line each, with what I recommend. The parts below give the reasons. Approve, change or strike each line.

| # | Decision | Recommendation |
|---|---|---|
| 1 | The unit of review | Meaning, not lines. Lorefile compiles the space with and without the proposal and compares the two: things, columns, links, numbers, words, rules, sources, functions, actions. |
| 2 | Where review happens | One **review screen** per proposal (`#review/<id>`). The Agents list keeps short cards with a Review button. Edit-mode writes that nobody has reviewed use the same screen, so "Waiting" and "In use, not reviewed" are one list in two parts. |
| 3 | The headline | Who wants what, in words, from the comparison: "ChatGPT wants Lorefile to remember a file", "Claude suggests a new rule for Order", "ChatGPT wants to change how Promo revenue is counted". The agent's own words follow, quoted and labelled as its claim. |
| 4 | Numbers | Each new or changed metric is written as a sentence ("add up revenue, leaving out lines where refunded is true"). A recipe too complex for a sentence is shown as written. No preview number in this version (Part 2). |
| 5 | Warnings | In words, as questions. Three get a fix on the screen: who owns a number (W109), what a thing or number is (W101), and, for a learned file with no number, which number counts. The answers ride along with Accept as the person's own changes. |
| 6 | Accept with answers | Accept takes an optional list of fixes. They are planned on top of the proposal and written in the same change, credited to the person. Only the fixes the screen offers are allowed; it is not a free editor. |
| 7 | A space that changed since | If only unrelated parts of the same files changed, a proposal made from ops (`propose_patch`, `learn_file`) is **re-applied to today's files**. The screen shows the result before Accept. A full-file `propose_change` cannot be re-applied, so the screen says what changed since and offers "ask the agent again". |
| 8 | Proposals with errors | **Not queued any more.** The agent gets the errors at once and can fix them. When the error names something another waiting proposal adds, the message says so ("PromoSale is waiting for review in p-…; send this together with it, or after it is accepted"). Old ones with errors show the errors in words and only Not now. |
| 9 | Replaced proposals | The new one says "replaces an earlier version from the same chat" and what differs ("adds Promo revenue; USD on 3 columns"). The old one sits in Recently decided as "replaced", with a link. |
| 10 | Not now | Asks an optional "why?". The proposal goes to Recently decided as "set aside by Vinay", and **Look again** brings it back. `match_file` tells an agent that a file shape was set aside, and why. |
| 11 | Taking back an Accept | With history on: the existing Undo, linked from the decided card. **With history off: "Take it back"**, which restores the files and `trust.yaml` as they were, but only while nothing has changed them since. The proposal then waits again. |
| 12 | Accept and edit mode | Untangled per connection. When the person approves an OAuth connection they choose **"wait for me to accept"** (the default) or **"go in straight away, marked unverified"**. The owner's own agents (stdio, the login token) keep today's behaviour. One server with `--allow-edit` then serves both ChatGPT, whose changes wait, and the page's Accept. |
| 13 | A server without `--allow-edit` | Review buttons are off, with one sentence saying why and how to change it. Not now, Look again and Take it back need `--allow-edit` too: a decision is a change of record. |
| 14 | Who is recorded | The name from the screen, asked at the first Accept and kept in the browser like Settings' "Your name". It is recorded on every decision: Accept, Not now, Look again, Take it back. It goes into History (as committer), `trust.yaml` (`by`) and the activity log, with the change in words. The agent stays the author. |
| 15 | The link from chat | `accept_at` becomes `<public address>/s/<space>/?review=<id>`. The query survives sign-in. The page opens the review screen and tidies the address. The screen handles a link to a proposal that was accepted, set aside, replaced or no longer exists. |
| 16 | Two people at once | Accept, Not now, Look again, Take it back and the replace rule take one lock per space. The second person is told what happened ("accepted by Priya a moment ago"), not "changed since this was proposed". |
| 17 | Grades on the screen | A change to something checked or accepted says so before Accept: "Promo revenue is checked by Priya. After you accept, it is accepted, not checked, until someone checks it again." |
| 18 | Removals and renames | A removal is red and needs a second click that names what goes. Anything else that mentions it is listed, including a metric's recipe text (the compiler does not read those). A thing removed and an identical one added under a new name in the same proposal is shown as "renamed". |
| 19 | Comments | When a proposal would drop `#` comments from a file (any op on a type file does today), the screen says how many and in which file. |
| 20 | Many waiting | Newest first. Grouped under the agent that sent them when there are more than five. No "accept all": each Accept is a person looking. "Set aside all replaced" is the only bulk action. |
| 21 | The batch headings bug | Fixed: each file gets the sentences of the ops that touched it. |
| 22 | The activity log | It says what was decided in words ("accepted ChatGPT's promo sales file: Promo sale and Promo revenue") and by whom. |
| 23 | `learn_file` asks first (approved by Vinay 2026-11-27) | `answers: {}` is refused unless the agent passes `skip_because` with the user's own words ("just save it"). Otherwise it returns the questions with "ask the user now". The review screen's "which number counts?" (decision 5) is the safety net either way. Strike this line to leave `learn_file` as it is. |
| 24 | The page | Light theme only. It works at phone width (the screen stacks, and the buttons stay at the bottom). Symbols plus words, never colour alone. Real buttons. Keyboard reachable. Local times. |

---

## Part 1 · The review screen

### What it is built from

A new `GET /api/proposals/<id>/review`. The server compiles the space as it is, then with the proposal's files laid over it, using the same `edit::check_many` the save gate uses. It compares the two compiled ontologies thing by thing. The page only renders the answer, so every rule below lives in Rust and is unit-tested.

```json
{
  "id": "p-20261127-080758-6", "status": "pending",
  "headline": "ChatGPT wants Lorefile to remember a file",
  "context": { "agent": "ChatGPT", "how": "asked in a chat", "at": "2026-11-27T08:07:58Z",
               "file_name": "promo_sales.csv",
               "says": "Use the user-confirmed definition of promo sales revenue: …" },
  "counts": { "adds": ["1 thing (Promo sale, 14 columns)", "1 number (Promo revenue)", "1 place rows come from"],
              "changes": [], "removes": [] },
  "changes": [
    { "kind": "entity", "name": "PromoSale", "label": "Promo sale", "op": "add",
      "grain": "one row is one order line", "key": "line_id",
      "fields": [{ "op": "add", "name": "revenue", "type": "money (USD)", "description": "…" }, … ] },
    { "kind": "metric", "name": "promo_sale_revenue", "label": "Promo revenue", "op": "add",
      "sentence": "add up revenue, leaving out lines where refunded is true",
      "preview": "Lorefile can't preview this number: the rows stay with the agent." },
    { "kind": "source", "name": "user_files", "op": "add", "sentence": "files people give an agent" }
  ],
  "questions": [{ "code": "W109", "ask": "Who decides what Promo revenue means?",
                  "fix": { "op": "set_metric", "metric": "promo_sale_revenue", "field": "owner" } }],
  "notes": [{ "kind": "undo", "text": "You can take this back until something else changes (history is off for branda)." }],
  "can": { "accept": true, "dismiss": true, "why_not": null },
  "files": [{ "file": "sources/user_files.yaml", "summaries": ["added the from_user source user_files"] }, … ]
}
```

### The rules for each kind

| Kind | Added | Changed | Removed |
|---|---|---|---|
| Entity | Name, what one row is (from its description), key, its columns in a short list (the first three, then "+ N more") | Each column added, removed or changed. A changed column shows before → after for type, unit and description. Also shows a changed binding ("rows now come from…") and changed links. | Red, with everything that uses it: links, rules, actions, words, metric recipes |
| Column | Name, type in words, description | Type, unit or description, before → after | Red, plus any metric whose filter or measure mentions it |
| Metric | Its sentence, plus grain, owner and basis if set | Its sentence before → after when the recipe changed. A words-only change says "described differently" with the two texts. | Red. Agents can no longer quote it. |
| Rule | Its name, what it applies to, its rule in words where simple (`refunded == false` becomes "refunded must be false"), its severity in words ("stops the action" / "warns" / "notes") | Before → after of the rule and severity | Red: "agents will no longer be stopped by EC-031" |
| Word | "people say *promo report* for Promo sale" | Before → after of what it means | Red |
| Source | Its sentence by kind ("files people give an agent", "a BigQuery dataset", "tools on the meta-ads MCP server") | Before → after, plus every grade that depends on it (evidence grades fold sources into binding and metric fingerprints) | Red, plus everything bound to it |
| Link, function, action | Name and a sentence | Before → after | Red, plus what uses it |

**Types in words:**
- `string`: text
- `integer`: whole number
- `number`: number, or money (USD) when it has a currency unit
- `boolean`: yes / no
- `date`: date
- `datetime`: date and time
- `enum`: one of (the values)
- `json`: structured data

**Metric sentences.**
- `sum(x)` → "add up x"
- `count distinct x` or `count(distinct x)` → "count the different x"
- `count(*)` and `count rows` → "count rows"
- `avg(x)` → "average x"
- A filter of simple comparisons joined by `and`:
  - `a = false` → "leaving out rows where a is true"
  - `a = 'x'` → "only rows where a is x"
  - `a != 'x'` → "leaving out rows where a is x"
  - `a in (…)` → "only rows where a is one of …"
- Anything else is shown as written in code type: "where `payment_status = 'paid' and …`".
- `variants` become one sentence each, under their `when`.
- A `sql:` metric says "counted by its own SQL" with the SQL behind a button.

The rows are called by the entity's grain when the key says it: "lines" for `line_id`, "orders" for `order_id`, otherwise "rows".

**Renames.** Lorefile looks for a thing removed and a thing added, of the same kind, in the same proposal:
- **Entities** with the same columns (names and types) and the same binding → "renamed Order → Purchase".
- **Columns** in the same entity, with the same type and description → "looks like a rename of promo_code → promo".
- **Metrics** with the same recipe → "renamed".

A rename that the rest of the space did not follow cannot compile, so it never reaches review (decision 8). When it compiles, the screen lists what was updated with it ("and 4 places that said Order").

**Comments.** Lorefile counts the full-line `#` comments in each touched file, before and after. When the count drops, the screen says: *"types/order.yaml would lose 3 comments (Lorefile rewrites type files and does not keep comments yet)."*

### Warnings in words

| Code | On the screen | Fix on the screen |
|---|---|---|
| W109 metric has no owner | Who decides what *Promo revenue* means? | A name box. It writes `owner` through the new `set_metric` op. |
| W101 no description | What is one *Promo sale*? / What does *Promo revenue* measure? | A text box: `set_type_description`, or `set_metric` `description` |
| learned file, no metric | Which number in *promo_sales.csv* is the result? | A pick from its number columns plus "leave out rows where [flag ▾] is true". This writes `add_metric` exactly as `learn_file` would. |
| W103 bound, no key | No column says which row is which, so agents cannot look one up. | None: Not now, and ask the agent |
| W102 blocking rule without a rationale | This rule stops actions but doesn't say why. | None |
| W105 enum with no values | *status* is "one of" but lists none. | None |
| W106 file not found | Lorefile can't find *data/orders.csv*, so rows can't be read here. | None |
| W107 action edge mismatch | *assign_owner* links the wrong way round for its inputs. | None |
| W108 word means nothing | *promo report* points at *PromoSales*, which doesn't exist. | None |
| W111 function doesn't say where it runs | Nobody can run *forecast*: it doesn't say where. | None |
| W113, W114 grades | Changed since Priya accepted it / a grade for something gone | None (these come from the grades, not the proposal) |
| E0xx (old proposals only) | Each error's own message, under "This can't be accepted as it is" | None: Not now |

Only warnings this proposal **adds** are shown. Warnings the space already had are counted in one line ("the space has 4 other warnings"), so a person is not asked to fix what the agent did not touch.

### What the card in the list shows

```
│ ChatGPT wants Lorefile to remember a file                     today 1:37 pm  │
│ Adds Promo sale (14 columns) and Promo revenue · 1 question     [ Review ]   │
```

When a proposal is stale, replaced, has errors or removes something, the card shows it as a one-word chip: changed since, replaced, can't accept, removes.

---

## Part 2 · What the screen deliberately does not show

- **No example values.** `learn_file` never stored any. The screen shows only what the YAML holds, plus counts.
- **No preview number, in this version.** For files a user gives an agent (`from_user`) Lorefile has no rows, so the screen says *"Lorefile can't preview this number: the rows stay with the agent."* For sources readable here (csv, duckdb), a preview would run the recipe through the frozen DuckDB door. A recipe's `measure` is words ("count distinct order_id"), not always SQL. The page already reads sample rows for those, so this is possible, but it is left for later (Part 9) rather than half-done.
- **No agent's number.** The agent may have told the user "$1,275.20". That is the agent's arithmetic. It belongs in a `record_check` report (evidence grades), not on the definition's review.
- **No free editing.** Accept with answers covers the fixes in the table above. Anything else is Not now plus a word to the agent, or Accept and then edit on the entity's page.

---

## Part 3 · Accept, and everything that can go wrong around it

### The order of checks when someone presses Accept

1. **Lock the space** (the per-space lock file `edit.rs` already uses for `trust.yaml`). It is held until step 7.
2. **Read the proposal again.** If it is not pending: "accepted by Priya at 1:42 pm" (or set aside, or replaced by p-…, with a link). Nothing is written.
3. **Fresh, or changed since?** Is every file's text still the proposal's `before`?
   - **Yes** → go on.
   - **No, and the proposal has a replay** (it came from ops: `propose_patch` or `learn_file`) → re-plan it on today's files. If that compiles clean, write the re-planned files. The screen already showed this result before the click, and the request carries the fingerprint of what was shown, so the person accepts exactly what they saw. If it no longer plans ("PromoSale already exists"), refuse in words.
   - **No, and no replay** (`propose_change`) → refuse: "orders.yaml changed since this was proposed (Vinay added a field at 1:40). Accepting would undo that. Set this aside and ask the agent again." This is today's 409 in words.
4. **Fixes.** If the request carries fixes, plan them on top. Only the fixes the screen offered are accepted.
5. **The save gate.** It compiles, checks graded overwrites (evidence grades) and writes all files and `trust.yaml` in one change: one commit when history is on, with the agent as author and the person as committer.
6. **Mark the proposal accepted,** with `accepted_by` and, when history is off, the snapshot for Take it back (below).
7. **Log in words, and unlock.**

### The replay

Proposals from ops store how to make them again, without values:
- **`propose_patch`:** the ops.
- **`learn_file`:** its input without `examples`: the column names, guessed types, units, descriptions, key and answers. Re-running `learn::plan` on today's files does the right thing when PromoSale was accepted in the meantime: it yields only the difference.

`propose_change` has no replay.

### Changed since, in pictures

```
│ ⟳ The space changed since ChatGPT proposed this                                  │
│   logic/metrics.yaml gained gross_margin (Vinay, 1:40 pm). Nothing in this        │
│   proposal touches it, so here it is applied to today's files: same result.       │
│ [ Accept as Vinay ]                                                               │
```

```
│ ✗ This can't be accepted as it is                                                 │
│   types/order.yaml changed since this was proposed (Vinay added gift_wrap at      │
│   1:40 pm), and this proposal replaces the whole file. Accepting would undo that. │
│   Copy for the agent: "The Order file changed; propose your change again from     │
│   the current file."                                    [ Copy ]  [ Not now ]     │
```

### Replaced

`learn_file` replaces its own waiting proposal for the same file shape (it already does). The new screen compares the two "afters":

```
│ Replaces an earlier version from the same chat (1:32 pm): this one adds           │
│ Promo revenue and sets USD on unit_price, discount and revenue.     [ see it ]    │
```

Recently decided shows the old one as *"replaced by the 1:37 pm version"*. **Race:** a person pressing Accept on the old one while the new one is being made is handled by the lock. Whichever comes second sees the first's result. If the old one won, the new one is changed-since and replays to just the difference (the number).

### Not now, Look again

- **Not now** opens a small "Why? (optional; the agent sees this)". The proposal becomes set aside with `decided_by`, `why` and the time.
- **Look again** on a set-aside proposal makes it pending again (and changed-since, if the space moved on).
- **A replaced proposal** cannot be looked at again: its replacement is the one to review.
- **`match_file`** on a shape whose `learn_file` proposal was set aside answers *"set aside by Vinay on 27 Nov: 'not until finance agrees on refunds'"*, so the agent does not propose the same thing blindly.

### Take it back

**With history on,** a decided card links to History's existing Undo for that commit.

**With history off** (branda), Accept saves a snapshot in the proposal: each file's text before and after, and `trust.yaml`'s text before and after. "Take it back" is offered while every one of those files still equals its after-text. It writes the before-texts back through the save gate and marks the proposal pending again ("taken back by Vinay"). Once anything else changes those files, the button goes, and the screen says why: *"Can't take back: logic/metrics.yaml has changed since (gross_margin added). Press Start history in Settings to be able to undo any change."* The button already exists.

### Proposals that depend on others

Waiting proposals never depend on each other by construction:
- **Ops are planned against the files**, so a `propose_patch` naming an entity that exists only in a waiting proposal is refused at once. The refusal now names the waiting proposal.
- **Full-file changes** that don't compile are not queued any more (decision 8).

What remains is ordering by staleness: accepting one may make another changed-since. That one replays when it can, and otherwise says what to do.

### Read-only and shipped spaces

- **A server without `--allow-edit`** shows the screen with the buttons off and *"This Lorefile server is read-only (started without --allow-edit), so nobody can accept here. Restart it with --allow-edit."* The review endpoint still answers, so a person can read it.
- **Shipped examples** never have proposals: writes to them are refused before anything is queued.

---

## Part 4 · Accept and edit mode, untangled

**Today.** One flag, `--allow-edit`, means three things:
- the page may change files
- Accept works
- every agent's save is written at once

The live test wanted ChatGPT's saves to wait while the page could accept, so it ran two servers on one folder: 7171 read-only behind the tunnel, 7172 with `--allow-edit` locally.

| | A · per connection (recommended) | B · a new server flag `--review` (agents always wait) | C · Accept works without `--allow-edit` |
|---|---|---|---|
| Vinay's setup | One server, `--allow-edit`. ChatGPT's connection was approved with "wait for me". | One server, `--allow-edit --review` | One read-only server. The token holder can still accept. |
| Claude Code on the same server, with the login token | Writes straight away, as today | Waits too | Waits too |
| What "read-only" means | Unchanged | Unchanged | Read-only stops meaning read-only |
| Where the choice is made | On the approval page, by the person approving. It is shown and changeable in Settings › Connections. | At the command line, for everyone | Nowhere |
| Fits the OAuth scope's words | Yes: the box already says "suggest changes" | Yes | |

**Recommendation: A.** The person approving a connection decides how much they trust it. The approval page's "suggest changes" line becomes:

```
☑ suggest changes
    ◉ wait for me to accept them
    ○ go in straight away, marked unverified until a person looks
```

**How it behaves:**
- **Stored** on the connection in `oauth.json`. Only the page can change it: agents cannot reach `.lorefile/`, and the OAuth endpoints never change it after approval.
- **Defaults.** Existing connections read as "wait". That is a change for connections approved on a server with `--allow-edit`; their saves wrote straight away until now, and the release notes say so.
- **Without `--allow-edit`,** everything waits, as today.
- **"Straight away"** is today's edit mode for that connection: written, graded proposed, listed under "In use, not reviewed", with "unverified" said by every agent that uses it. That is also the honest answer to "approve in ChatGPT": the chat can put a definition to use, but only a person in Lorefile can say they stand behind it.

---

## Part 5 · The link from chat

**Before.** `accept_at` is `https://…/s/branda/#agents`.
- **With a cookie already there:** a click from chatgpt.com arrives without the cookie, which is SameSite=Strict. The login page's script finds the cookie and reloads with path and hash, so it lands on Agents. That works, but not on the card.
- **Without a cookie:** the person signs in through the form. The form's `next` carries only what the server saw, and the server never sees `#agents`, so they land on the Map.

**After.**
- `accept_at` is `https://…/s/branda/?review=p-20261127-080758-6`. The server sees the query, so `next` keeps it through the form.
- The login script reloads with `location.pathname + location.search + location.hash`.
- The page, on load, turns `?review=<id>` into `#review/<id>` and removes the query with `history.replaceState`, so a reload does not repeat it.
- `learn_file`, `propose_patch` and `propose_change` results all carry `review_at` (the old `accept_at` name stays as an alias). It is present only when the server knows its public address, which it does over HTTP. Over stdio the result says "open Lorefile's page › Agents".

**What the screen says for each state of the linked proposal:**

| The linked proposal is… | The screen shows |
|---|---|
| pending | The review |
| accepted | "Accepted by Vinay at 1:42 pm", plus what it added, plus Take it back or Undo when possible |
| set aside | "Set aside by Vinay: '…'" and Look again |
| replaced | "Replaced by a newer version from the same chat", with a link that opens it |
| missing (wrong id, other space) | "There is no proposal p-… in branda", with a link to Agents |
| in a space this person cannot open | The sign-in page. A person with only an OAuth connection (a colleague in ChatGPT) cannot review. The page says "ask whoever runs this Lorefile". |

---

## Part 6 · Who is recorded

| Action | History (when on) | `trust.yaml` | Activity log | The proposal |
|---|---|---|---|---|
| Accept | Author: the agent (`agent via MCP (connection "ChatGPT", approved 27 Nov)`). Committer: the person. Message: the headline. | `accepted` by the person, for each graded thing it writes | "Vinay accepted ChatGPT's promo sales file: Promo sale and Promo revenue" | `accepted_by`, `resolved_at` |
| Accept with answers | The same commit. The fixes' sentences are added to the message ("owner set by Vinay"). | The same | One line, with the fixes | `fixes` listed |
| Not now | Nothing | Nothing | "Vinay set aside ChatGPT's …: 'why'" | `decided_by`, `why` |
| Look again | Nothing | Nothing | "Vinay looked again at …" | Back to pending |
| Take it back | A new commit when on (the Undo) | Restored from the snapshot | "Vinay took back …" | Back to pending, `taken_back_by` |

The name comes from the screen. It is asked the first time and kept in the browser with Settings' "Your name", and it follows the evidence-grades rules: no email addresses, nothing containing "agent". With no name, it is "person via page", as today. On a server run with `--no-auth`, anyone who can reach it can type any name, and the screen says so beside the box.

---

## Part 7 · The map view

`GET /api/graph?proposal=<id>` returns the space's entities and links as they would be after Accept. Each one carries a `status` of `new`, `changed`, `removed` or `same`, with removed ones kept so they can be drawn.

`renderMap` gains an overlay mode:
- **Colours and lines:** new is green with a solid outline, changed amber with a dashed one, removed red and struck through.
- **A legend in words**, so colour is never the only signal.
- **A bar:** "Showing ChatGPT's proposal · Back to the review".
- **Read-only:** the map can be explored but not edited.

**The button appears only when** the comparison adds, changes or removes an entity or a link. A changed entity counts only for structure (columns, key, binding), not for description alone.

---

## Part 8 · Small changes that come with it

- **`set_metric` op**, for words only: `{op: set_metric, metric, owner?, description?, caveats?, grain?, basis?}`. It refuses recipe fields, since those change what the number is. It serves the W109 and W101 fixes, and agents can use it to describe a metric without rewriting `logic/metrics.yaml`.
- **The batch headings fix.** `BatchPlan` records which file each op wrote (`plan_in` already returns `Patch.file`). Each `ProposedFile.summary` joins the sentences of its own ops: `types/promo_sale.yaml` gets "created PromoSale; bound PromoSale to user_files.promo_sales", and `logic/metrics.yaml` gets "added metric promo_sale_revenue".
- **Errors are not queued.** `propose` and `propose_many` return the errors and store nothing when the change does not compile. The "waiting in p-…" hint comes from looking up each unknown name in pending proposals' compiled names. One compile per pending proposal happens only on this error path; proposals are few.
- **`learn_file` asks first** (decision 23, if approved). `answers: {}` needs `skip_because` (the user's words, kept in the proposal's reason). Without it the result is `saved: false`, the questions again, and *"Ask the user these questions now. Save without a number only if they say so, and pass their words as skip_because."*
- **Times** shown in the browser's own time zone, "today 1:37 pm" style, never raw UTC.
- **The agent's reason** is shown as a quote, labelled *"ChatGPT says:"*, cut to 400 characters with "more". It is an agent's claim. A reason saying "Vinay already approved this" is just text, and the screen never treats it as anything else.

---

## Part 9 · What this does not do

- **It does not let a chat accept.** Accept is a person in Lorefile. A connection set to "straight away" puts things in use as unverified, which is a different, weaker promise, and agents say so.
- **It does not add git branches or merges** (question 4).
- **It does not preview numbers or show rows.** Previews for sources readable here are left for later. Example values are never on the screen because they were never stored.
- **It does not merge two full-file rewrites.** A changed-since `propose_change` is refused with words. Only op-made proposals replay.
- **It does not bulk-accept.**
- **It is not an editor.** Three fixes ride along with Accept; everything else is the entity's page after Accept, or the agent again.

**Left for later**, to go into BACKLOG.md:
- A preview number for metrics whose rows are readable here.
- Agents seeing what happened to all their proposals: a `my_proposals` tool, or a line in `summary`.
- Changing a guessed column type on the review screen (needs `set_property`).
- Review comments that go back to the agent.
- Per-person accounts, so "who accepted" is a signed-in person, not a typed name.
- Keeping comments when type files are rewritten.
- Notifications ("ChatGPT proposed something") by email or Slack.

---

## Build notes, file by file

**Notes for every session:**
- Branch from `claude/learn-file-6a954e`.
- Run cargo with the sandbox off; `cargo build --offline` and `cargo test --offline` work.
- `cargo test` must stay green, clippy clean.
- Every folder under `examples/` and `packs/` must compile with no errors and no warnings.
- Commit nothing; Vinay commits.

| File | What changes |
|---|---|
| new `src/review.rs` | `compare(before: &Ontology, after: &Ontology) -> Vec<Change>` for every kind, with renames. `words::type_word`, `words::metric_sentence`, `words::rule_sentence`, `words::source_sentence`, `words::code` (the warnings table, with the three fixes). `comments_lost(before_text, after_text)`. `headline(proposal, changes)`. `review(st, proposal) -> Json`: the endpoint's whole answer, including changed-since, replay result, replaced-diff, grade notes, undo note and `can`. Pure apart from reading the space; no I/O beyond `check_many`. |
| `src/proposals.rs` | New optional fields, each skipped when absent so old proposal files read as before: `replay` (ops, or learn input without examples), `accepted_by`, `decided_by`, `why`, `taken_back_by`, `fixes`, `snapshot` (files' and `trust.yaml`'s before and after texts, history-off only). New statuses: `set_aside` (reading old `dismissed` as the same), and pending again after Look again or Take it back. The decisions are written under the space's lock. |
| `src/patch.rs` | `set_metric`. `BatchPlan` keeps the file of each op so summaries can be grouped per file. |
| `src/mcp.rs` | Errors are not queued; the "waiting in p-…" hint. `replay` stored with every op-made proposal. `review_at` (alias `accept_at`) as `?review=`. `match_file` reports set-aside shapes with the reason. `learn_file` `skip_because` (decision 23, if approved). For an OAuth connection set to "wait", `propose`/`propose_many` use edit-off whatever the server flag says. |
| `src/oauth.rs` | A `writes: wait \| direct` field on connections, defaulting to `wait`. The approval page radio. Settings › Connections shows it and lets the owner change it (page-only endpoint, behind the write gate). |
| `src/serve.rs` | `GET /api/proposals/<id>/review`. `POST /api/proposals/accept` gains the lock, the order of checks (Part 3), `fixes`, the replay with the shown fingerprint, and words for every refusal. `POST /api/proposals/dismiss` takes `why`, records `page_by`, and needs `--allow-edit`. New `POST /api/proposals/reopen` and `POST /api/proposals/take_back`. `GET /api/graph?proposal=`. Activity lines in words. `LOGIN_HTML`'s script keeps `location.search`. |
| `src/ui/index.html` | The review screen (`#review/<id>`), the short cards, one "To review" list in two parts (Waiting, In use and not reviewed), Recently decided with Look again, Take it back and Undo, the name box at first Accept, the inline fixes, the Not now "why", the map overlay, `?review=` routing, local times. Phone width; light theme only; every name through `esc` and `jq` as the evidence-grades review fixed. |
| Docs, last | README: "Reviewing what agents propose" replaces the Accept paragraph, plus the connection choice in "Connect ChatGPT". BACKLOG: item 10 note, the left-for-later list. DEPLOY: one server, not two. Then `python3 scripts/build-tutorial.py`. |

### Hand-off plan

| Order | Session | Scope (files) |
|---|---|---|
| 1 | **review-core** | `review.rs`, `proposals.rs`, `patch.rs`, `mcp.rs`, `serve.rs` (endpoints and Accept), unit tests, `tests/http.rs`, `tests/mcp_loop.rs` |
| 2 | **connection-writes** | `oauth.rs`, the one `propose` switch in `mcp.rs`, Settings › Connections in `index.html`, `tests/oauth_http.rs`. After 1, since both touch `mcp.rs`. |
| 3 | **review-page** | `index.html` (the screen, the list, the map overlay, routing), the login script in `serve.rs`. After 1, alongside 2 if they agree on who edits `index.html` first. |
| 4 | **docs** | README, BACKLOG, DEPLOY, the tutorial; last |
| 5 | **live test** | Vinay, on one server with `--allow-edit`, using the script at the end |

---

## Test ideas

**The comparison (unit, `review.rs`)**
- **The p-6 shape** on a blank branda: one source, one entity with 14 fields, one metric. The headline is "… wants Lorefile to remember a file". The sentence is "add up revenue, leaving out lines where refunded is true", and it says "lines" because the key is `line_id`.
- **A `propose_change` rewrite** of a type file that adds one field, removes one and changes one description gives exactly three changes, plus a "loses 3 comments" note when the original had comments.
- **Renames:** an entity renamed with its links and words updated shows as one rename; a column with the same type and description shows "looks like a rename".
- **A removal** lists every use: links, rules, words, and a metric whose filter mentions a removed column.
- **Metric sentences** for `sum`, `count distinct`, `count(*)`, `avg`, `=` / `!=` / `in` filters, `and`-joined filters, a filter too complex (shown as written), variants and `sql:`.
- **Warnings:** only new ones are shown, and the space's own are counted. W109 gives the owner question with a `set_metric` fix. A learned file without a metric gives the "which number" question with an `add_metric` fix.

**Accept (`tests/http.rs`)**
- **Fresh:** a proposal accepts as today. History records the agent as author and the person as committer. The activity line is in words and names the person.
- **Fixes:** Accept with an owner fix gives one change, W109 is gone, and the fix is credited to the person. A fix the screen did not offer is refused.
- **Changed since, with a replay:** add `gross_margin` to `logic/metrics.yaml`, then accept p-6's equivalent. It replays, writes both metrics, and loses nothing.
- **Changed since, replay that fails:** PromoSale accepted from another proposal → the replay gives only the metric (learn input), or refuses in words (ops that add PromoSale).
- **Changed since, `propose_change`:** refused in words; nothing written.
- **A screen shown before a change:** the fingerprint no longer matches, so the request is refused with "the space changed while you were looking; review again".
- **Two Accepts at once** on one proposal: one writes, the other answers "accepted by … a moment ago" (not 409 stale).
- **Accept on a replaced proposal** → "replaced by …". The race between replacing and accepting, run as two threads, ends in one of the two orders, and both are consistent.
- **Without `--allow-edit`:** the review answers with `can.accept: false` and the sentence. Accept, Not now, Look again and Take it back all refuse.

**Decisions after Accept**
- **Not now with a why:** the proposal is set aside with `decided_by`. `match_file` on the same shape reports the why. Look again makes it pending. Look again on a replaced proposal is refused.
- **Take it back with history off** restores the files and `trust.yaml` byte for byte and makes the proposal pending again. After another change to one of the files, the button is gone and the reason is given.
- **With history on,** the decided card offers Undo, and the undo goes through `edit::undo`.

**MCP (`tests/mcp_loop.rs`, `tests/oauth_http.rs`)**
- **Errors:** a `propose_change` that does not compile is not stored, and the result lists the errors. A `propose_patch` naming PromoSale while it waits in p-… names that proposal.
- **Per-connection writes:** with `--allow-edit`, a "wait" connection's save is queued and a "direct" one is written and graded proposed. A connection without the field reads as "wait". The approval page shows the radio, and agents cannot change the setting.
- **`review_at`** is `?review=<id>` over HTTP and absent over stdio.
- **`learn_file` `answers: {}`** without `skip_because` saves nothing and repeats the questions; with it, it saves (decision 23).
- **Batch headings:** a learn-file batch's files carry their own ops' sentences.

**The page (by hand in a browser, at desktop and phone width)**
- **The link:** opening `?review=` signed out → sign in → the review screen, then a reload stays there. Signed in via the cookie retry → the same.
- **Linked states:** accepted, set aside, replaced and missing proposals each show their state.
- **Escaping:** a proposal whose entity is named `x');alert(1)//` and whose reason contains `<img src=x onerror=alert(1)>` renders as text, and every button works.
- **The map overlay** colours new, changed and removed with the legend. Back returns to the review.
- **Phone width:** no horizontal scroll except inside "Show the files". The buttons stay reachable.

**The live test (Vinay, recorded in the status when done)**
1. One server: `lorefile serve --dir spaces --allow-edit` behind the tunnel. Stop the extra page server on 7172.
2. In Settings › Connections, ChatGPT shows "wait for me".
3. Ask ChatGPT about `p-20261127-080758-6` (or learn the file again). Open its link: the review screen appears through sign-in.
4. Answer "who decides" with your name and Accept. The space has the source, the entity, the metric and its owner. `trust.yaml` says accepted by you. The activity line reads in words.
5. Take it back, then Accept again.
6. December: in a new chat, "how did we do?" → **$1,209.60**, with "accepted by Vinay".

---

## What changed during the build

Built 2026-11-27 on `claude/learn-file-6a954e`. Everything above is built as written, except:

1. **"In use, not reviewed" does not open the review screen.** An agent's write in edit mode (or from a connection set to "straight away") keeps no proposal, so there is nothing to compare. Those rows sit in the second part of "To review" with Looks right and Show, as before. Keeping a record of each direct write is in BACKLOG.
2. **The decision lock is its own file**, `.lorefile/decide.lock`, not the `trust.yaml` lock. The save gate takes `trust.lock` itself, so holding it from step 1 to step 7 would wait on itself. The decision lock is taken first and the save gate's lock inside it, in that order everywhere (Accept, Not now, Look again, Take it back, and `learn_file`'s replace rule).
3. **A replaced proposal gets the status `replaced`** at once (the replace rule sets it aside). So "Set aside all replaced" has nothing to do, and was not built. Old files that say `dismissed` read as `set_aside`, or `replaced` when `replaced_by` is set.
4. **The commit message is the headline** ("An agent suggests 2 changes: Widget, patron"), as Part 6 says. One old test pinned the old subject ("An agent learned: …") and now pins the headline.
5. **Take it back remembers the errors the space already had.** branda started blank, with "no types found" (E000). Putting the files back brings that error back, and the save gate refused it as new. Accept now records the space's errors; Take it back allows exactly those, and nothing else. History's own Undo has the same limit on a blank space; that is unchanged and listed in BACKLOG.
6. **The fingerprint is required for a replayed Accept**, and checked when sent for a fresh one. An Accept that sends only `id` still works when the files are as the agent saw them, so scripts and old tests keep working.
7. **The "which number counts" fix** is sent as `{how, column, leave_out}`, and the server builds the `add_metric` op, named `<entity>_<column>` as `learn_file` names it. A free `add_metric` from the page is never accepted.
8. **File headings for proposals stored before this build** come from the comparison ("adds the number promo_sale_revenue"), since their stored headings are off by position. `p-20261127-080758-6` reads right now. New proposals store the right headings.
9. **W101 on an action or a function** is said in words, without a fix. Only a thing and a number get the description box.
10. **`propose_change` and `propose_patch` results carry `queued`**, the proposal's id, as `learn_file`'s already did. They also carry `review_at` and `accept_at` over HTTP.
11. **Phone width needed a small rule for the whole page.** The page had none, so the nav becomes a strip across the top under 760px. The review screen stacks, and its buttons stay at the bottom.
12. **`set_metric` rewrites the metrics file through serde_yaml**, like the type-file ops, so it drops `#` comments in that file. The review screen says so when it happens.
13. **Times.** The page shows local times ("today 1:37 pm"). Words from the server use relative times ("accepted by Priya a moment ago", "set aside by Vinay on 27 Nov"), which need no time zone.
14. **A link's label is its name**: "Customer holds Voucher". A key that does not name its grain reads "one row per code (the key)".
15. **"Show on the map" appears for a learned file too**, since it adds an entity (Part 7's rule). The words above it say the same thing.
16. **A decided proposal's screen** shows what it changed from its own texts laid over today's space. The "replaces" note shows only while it waits.
17. **Straight away is offered only on a server with `--allow-edit`.** It is refused for a connection that may only read.
18. **Text links on the review screen are real buttons**, so they are reachable by keyboard.

**Tests added:** `tests/review.rs` (13 tests through the real binary: the p-6 shape by meaning, Accept with the owner fix, a fix not offered, the state words, the replay when `logic/metrics.yaml` moved on, a replay that no longer plans, a whole-file proposal refused in words, a learned file whose entity came another way replaying to only the metric, two Accepts at once, Not now with a why that `match_file` reports, Look again, Take it back with history off byte for byte and after another change, Take it back with history on through Undo, a replaced proposal, a read-only server, errors not queued with the "waiting in p-…" hint, `learn_file` asking first and the "which number" fix, the map, and a review link kept through sign-in). Also: one in `tests/oauth_http.rs` (wait by default with `--allow-edit`, Accept on the same server, straight away after the owner changes it, an agent cannot change it, an old connection reads as wait), unit tests in `src/review.rs` (metric sentences, words, comments, a whole-file rewrite as three column changes, renames, removals and what uses them, only new warnings and the fixes allowed), and `set_metric` in `src/patch.rs`. 174 tests in all.

**Not verified here:** the live test at the end of this spec. Also not checked by hand: Settings › Connections' wait / straight-away menu (it needs OAuth on an HTTPS address; the endpoint is tested), and the sign-in form in a browser (the link through sign-in is tested over HTTP).

