# Spec: `learn_file`, so next month's file is read the same way

Written 2026-11-27 for Vinay to decide on. **Status: built 2026-11-27**, every decision as recommended, on branch `claude/learn-file-6a954e` (cut from the OAuth branch). Covered by tests through the real binary, and tried live in ChatGPT on the web on 27 Nov 2026: it learned the November file and gave $1,275.20 (README §6). What changed during the build is at the end.

This comes from the live test of OAuth connectors that day. The note is `oauth-live-test-and-learn-file-findings.md` in the OAuth sessions' shared folder. ChatGPT web connected to Lorefile over OAuth and worked. Then Vinay uploaded a promo-sales CSV, typed "@Lorefile save this file", and got a proposal that could never read a row.

This spec has two parts:
- The tool that fixes that: a read-only `match_file` and a write `learn_file`.
- Part 7: the four small OAuth fixes the same test found, for the OAuth branch before it is merged.

**Standing rules, unchanged** ([BACKLOG.md](../BACKLOG.md)):
- The ontology keeps meaning, not data. It never executes and never holds credentials.
- Agents and builders write the YAML; people review it.
- Light theme only.
- Honest status.

**For this feature, the keeper rules mean:**
- Example values are used only to guess types, then dropped.
- Rows never enter the ontology.
- No outbound requests.
- Lorefile never computes the number. The agent does, with its own tools, from the recipe Lorefile keeps.

**Read first:**
- `src/patch.rs` (`plan_many`, `add_type`, `set_source`, `add_metric`)
- `src/mcp.rs` (`tool_definitions`, `instructions`, `propose_many`, and on the OAuth branch `PROPOSE_TOOLS`)
- `src/data.rs` (`where_is`, `file_path`)
- `src/validate.rs` (E012, W106)
- `src/schema.rs` (`Property`, `TypeSource`, `MetricSource`)
- `src/proposals.rs`
- `specs/2026-11-27-evidence-grades.md` (proposed, accepted, checked)

**The example carried all the way through** is Branda's promo-sales export:
- `spaces/branda/data/promo_sales.csv`: November, 40 lines.
- `spaces/branda/data/promo_sales_2026-12.csv`: December, 24 lines.
- Both have the same 14 columns: `line_id, order_id, order_date, promo_code, campaign, channel, region, sku, product, units, unit_price, discount, revenue, refunded`.

---

## The gap, in one month

**Before (today, proposal `p-20261127-062831-4`).** Vinay uploads `promo_sales.csv` to ChatGPT and types "@Lorefile save this file".

1. ChatGPT first reads that as "store the CSV" and says there is no upload tool.
2. After "no, save the ontology", it sends `propose_patch` with 6 `add_type` and 5 `add_relationship` ops. It splits the file into PromoSale, Order, Product, Promotion, Channel and Region.
3. `add_type` can only make a generated key, so it makes `promo_sales_line_id`, `product_id` and `promotion_id`. The file says `line_id`, `sku` and `promo_code`.
4. None of the 14 columns became a field. `revenue`, `units`, `discount` and `refunded` appear only inside a description sentence.
5. There is no source, no binding and no metric. Nobody was asked which number matters.

In December a different agent, or the same one on a different day, has nothing to go on. One person gets $1,231.20 (refunds counted) and another gets $1,209.60.

**After.**

```
You:      [promo_sales.csv]  save this to Lorefile
ChatGPT:  I'll save it as "Promo sales": one row per order line, 14 columns.
          Two questions so next month's file is read the same way:
          1. Which number is the result you care about: revenue?
          2. Do refunded rows count?
You:      revenue, and no refunds
ChatGPT:  Saved for review: Promo sales (14 columns, each described) and
          promo revenue = revenue without refunded lines. Accept it in Lorefile: <link>
── next month, a new chat ──
You:      [promo_sales_2026-12.csv]  how did we do?
ChatGPT:  $1,209.60 in promo revenue, refunded lines left out, as you set on 27 Nov.
```

What happened underneath:

```
 November                                          December
 match_file  → "new": a draft and two questions    match_file  → "same": this is PromoSale;
 (read-only, ChatGPT does not ask to confirm)      the result is sum(revenue) where refunded = false,
 learn_file  → one proposal: the user_files source,   accepted by Vinay 27 Nov
               PromoSale with 14 typed, described  The agent computes it from the file with its own
               fields, key line_id, the binding,   tools. Lorefile never sees a row.
               and the metric
```

---

## Options

| | A · Instructions only, plus `add_type` taking columns | **B · `match_file` + `learn_file` (recommended)** | C · Nothing |
|---|---|---|---|
| Who turns a file into an entity | Each agent, from a recipe in the instructions | Lorefile, by fixed rules, from the column names and a few examples | Each agent, as today |
| Same file, two agents | Two different drafts: names, types and keys differ | The same draft every time | Two different drafts |
| Keys and column names match the file | If the agent is careful | Always: names come from the headers | No (the baseline) |
| Next month | The agent must notice it has seen this file before | `match_file` says so, from the headers | Nothing |
| Questions asked | Whatever the agent thinks of | The two that matter, worded the same way | None |
| Cost | Small | A new module, two tools, two ops, one source kind | None |

**Recommendation: B.** The product sentence is that the same file is read the same way by any agent. A recipe in the instructions still depends on each agent following it. The two small fixes from A (`add_type` with properties, `add_property`) are part of B anyway (Part 6), so agents writing by hand also get them.

---

## Decisions

One line each, with what I recommend. The parts below give the reasons. Approve, change or strike each line.

| # | Decision | Recommendation |
|---|---|---|
| 1 | The tools | Two. `match_file` is **read-only** (`readOnlyHint: true`): "do you know this file, and how is it read?". `learn_file` writes: it saves one proposal. Splitting them means ChatGPT asks the user to confirm only the save, not the look-up next month. |
| 2 | What the agent sends | The file's name, the column names in order, up to 5 example values per column, and if it can, the row count and each column's distinct and empty counts. For `learn_file` it also sends one short description per column and the answers. Rows are never sent. |
| 3 | What happens to example values | Used in memory to guess types and the key, then dropped. Never written to YAML, proposals, the activity log, history, errors or logs. A description that quotes one is left out, and the reply says which. |
| 4 | What gets drafted | One entity. Every column becomes a field with the header's exact name. A header that is not a plain name gets a clean field name plus a `columns:` mapping to the header. Types come from fixed rules (Part 2). Plus the key, a binding to the space's `user_files` source, and the metric from the answers. All as **one** proposal. |
| 5 | Who writes the meaning | The agent: it has the file and the conversation. Lorefile fixes the structure: names, types, key, binding, and the recipe from the answers. Lorefile never invents a description. A column the agent did not describe is saved with none, and the page shows "not described". |
| 6 | Where the rows live | A new source kind, **`from_user`** (first drafted as `upload`, renamed on Vinay's review: nothing is uploaded to Lorefile; the rows come from a person). There is one `user_files` source per space, made on first use. Its rows are never readable here. `where_is` says "this arrives as a file the user gives you; read it with your own tools". Part 3 weighs this against a `csv` source on `data/`. |
| 7 | Recognising a file again | By its column names (trimmed, lower-case, `_`/space/`-` treated alike), never by its file name. There are four answers: **same**, **changed** (most columns shared), **waiting** (matches a proposal nobody has accepted yet) and **new**. It matches every bound entity, not only from_user ones: an orders export matches `Order`. |
| 8 | A file whose columns changed | New columns are proposed as fields. Missing columns are never removed; the reply names them. A missing column that a saved metric uses is flagged loudly. A rename is mapped with `renamed`, which writes the binding's `columns:` so metrics keep working. |
| 9 | Questions | At most three, only when needed, in this order. (a) Which number is the result, with a suggestion. (b) Which rows do not count, asked as "Do refunded rows count?" when there is a flag column. (c) The currency of the money columns, only when nobody has said. (d) The key, only when none can be found. |
| 10 | How the answers come back | A second `learn_file` call with `answers`. Without `answers` it saves nothing and returns the questions. With `answers: {}` it saves the shape only, and only when `skip_because` carries the user's own words for leaving the number out (review-flow decision 23 refined this; without them the questions come back marked "ask the user now"). A newer `learn_file` for a shape that is still waiting replaces that proposal instead of queueing a second one. |
| 11 | What does not count | Goes in the metric: its `filter` (`refunded = false`) and a `caveats` sentence in the user's words. **Not a guardrail.** A guardrail is a rule every row or action must pass, and a refunded line is a valid row that just does not count (Part 5). |
| 12 | The metric's name and basis | The name defaults to `<entity>_<column>` (`promo_sale_revenue`). It is called `success` only when the user says this is the one number the business runs on and the space has no `success` yet. `basis` is set only when the agent passes it. |
| 13 | Glossary words | Only the words the user actually uses for the file (`also_called`), each pointing at the entity. No words are made up from headers. |
| 14 | One entity or several | Always one. A flat export is one thing per row. Splitting out Product or Promotion is a person's later call with `propose_patch`. |
| 15 | Fixes to `propose_patch` | `add_type` takes `properties` and `key` (and then makes no `<name>_id`). A new `add_property` op adds a field, with an optional `column` when the header differs. |
| 16 | The instructions | One recipe added to the text every agent gets at `initialize`: a file from the user means `match_file`; "save this file" means save its shape and meaning, not its rows (Part 6). |
| 17 | Strict schemas | Both tools get complete JSON schemas with `additionalProperties: false`, so strict clients keep every field (backlog item 6). |
| 18 | Read-only connections and shipped examples | `learn_file` joins `propose_patch` and `propose_change`: it is hidden from read-only OAuth connections and refused on shipped examples. `match_file` works everywhere. |
| 19 | The four OAuth fixes | Part 7: `/.well-known/*` paths Lorefile does not serve answer 404; list `offline_access`; mark read tools `readOnlyHint: true`; pin the `server/discover` reply with a test. They land on the OAuth branch before it is merged. |

---

## Part 1 · The two tools

### `match_file` (read-only)

**Input**

```json
{
  "file_name": "promo_sales_2026-12.csv",
  "row_count": 24,
  "columns": [
    { "name": "line_id",  "examples": ["L-2007", "L-2017", "L-2002"], "distinct": 24, "empty": 0 },
    { "name": "order_id", "examples": ["BR-6006", "BR-6013", "BR-6002"], "distinct": 17, "empty": 0 },
    …
    { "name": "revenue",  "examples": ["21.60", "54.40", "54.40"] },
    { "name": "refunded", "examples": ["true", "false", "false"] }
  ]
}
```

Limits:
- At most 200 columns and 5 examples per column.
- Each example is cut to 100 characters.
- `file_name` is up to 200 characters. It is a hint for naming, never for matching.
- A workbook is sent one sheet per call, named like `report.xlsx › Sales`.
- No `examples` is allowed; every column is then guessed as text.

**Answer: same.** This is the December case.

```json
{
  "match": "same",
  "entity": "PromoSale",
  "columns": { "in_file": 14, "known": 14 },
  "source": "user_files (the file is usually called promo_sales…)",
  "trust": { "grade": "accepted", "by": "Vinay via page", "at": "2026-11-27" },
  "read_it": [
    { "metric": "promo_sale_revenue", "measure": "sum(revenue)", "filter": "refunded = false",
      "caveats": "Refunded lines never count.",
      "trust": { "grade": "accepted", "by": "Vinay via page", "at": "2026-11-27" } }
  ],
  "say": "This is the promo-sales export (PromoSale, one row per order line). Its result is promo_sale_revenue: the sum of revenue over lines where refunded = false (accepted by Vinay on 27 Nov). Compute it from the file with your own tools; Lorefile does not have the rows."
}
```

`read_it` lists every metric whose source is this entity's binding: the same connection and table. Each carries its grade from `trust.yaml`. A proposed one carries the evidence-grades sentence ("Unverified: an agent proposed this definition…"). Guardrails that apply to the entity are listed too, by id and name.

**Answer: new.** This is the November case, on a blank space.

```json
{
  "match": "new",
  "draft": {
    "entity": "PromoSale",
    "grain": "one row per line (key line_id)",
    "fields": [
      { "name": "line_id", "type": "string", "key": true },
      { "name": "order_id", "type": "string" },
      { "name": "order_date", "type": "date" },
      …
      { "name": "units", "type": "integer" },
      { "name": "unit_price", "type": "number", "money": true },
      { "name": "discount", "type": "number", "money": true },
      { "name": "revenue", "type": "number", "money": true },
      { "name": "refunded", "type": "boolean" }
    ],
    "source": "user_files (made on first use), file usually called promo_sales"
  },
  "questions": [
    { "id": "success", "ask": "Which number in this file is the result you care about: revenue?",
      "suggested": { "column": "revenue", "how": "sum" } },
    { "id": "exclude", "ask": "Do refunded rows count?",
      "suggested": [{ "column": "refunded", "equals": true }] },
    { "id": "currency", "ask": "Which currency are unit_price, discount and revenue in?" }
  ],
  "next": "Ask the user these questions in your own words, then call learn_file with the same columns, one short description per column, and `answers`."
}
```

The currency question disappears when the examples carry an unambiguous symbol (€, £) or the agent sends a `unit`. A `$` only fills in the suggestion "USD?", because Canadian and Australian dollars share it.

**Answer: changed** (Part 4). It adds `added`, `missing`, `possibly_renamed`, and `metrics_at_risk` when a saved metric reads a missing column.

**Answer: waiting.** The shape matches a `learn_file` proposal nobody has accepted yet. It returns that proposal's id and date, how it says the file is read, and the sentence "saved for review on 27 Nov and not accepted yet: accept it in Lorefile (Agents)".

### `learn_file` (writes one proposal)

**Input.** The same `file_name`, `row_count` and `columns` as `match_file`, plus:

```json
{
  "entity": "PromoSale",
  "description": "One order line sold with a promo code, from the monthly promo-sales export.",
  "columns": [
    { "name": "line_id", "examples": […], "distinct": 40, "empty": 0, "description": "Identifier of one order line." },
    { "name": "revenue", "examples": […], "unit": "USD",
      "description": "What the customer paid for the line: units × unit_price − discount." },
    …
  ],
  "key": "line_id",
  "answers": {
    "success": { "column": "revenue", "how": "sum" },
    "exclude": [{ "column": "refunded", "equals": true }],
    "exclude_words": "Refunded lines never count.",
    "currency": "USD",
    "metric": "promo_sale_revenue",
    "basis": "transactional",
    "also_called": ["promo report"],
    "renamed": {}
  },
  "reason": "The user uploaded promo_sales.csv and said: save this; revenue counts, refunds do not."
}
```

**Optional fields:**
- `entity`, `description`, each column's `type` and `unit`, and `key` are optional. Lorefile fills them from its rules; anything the agent sends is checked against the examples.
- In `answers`, every field is optional.
- `how` is `sum`, `count_distinct` or `count`.
- An `exclude` item is `{column, equals}`. Several items are joined with `and`. The value must fit the column's type.

**Result**

- **It writes nothing** when `answers` is missing. The result is `saved: false`, with the draft and the questions, as `match_file` gives them.
- **It refuses** a file that is `same` ("already known as PromoSale; nothing to save"). It also refuses a name that belongs to an entity with other columns ("PromoSale exists with other columns; pass another `entity`").
- **Otherwise it plans the ops below with `patch::plan_many` and saves them with `propose_many`:** compiled and checked first, then written when edit mode is on, otherwise queued as one proposal. The grades follow from that: proposed when written by an agent, accepted when a person accepts.
  1. `add_source` user_files, only when the space has no `from_user` source
  2. `add_type` with `properties`
  3. `set_source`
  4. `add_metric`, when `success` was answered
  5. `add_term`, one per `also_called`
- **What the reply carries:**
  - the usual written or queued sentence
  - the proposal id
  - a link to Agents, when the server knows its public address
  - one line: *"learned promo_sales.csv: PromoSale (14 fields, key line_id) from user_files; metric promo_sale_revenue = sum(revenue) where refunded = false"*
  - which descriptions were left out, and why

---

## Part 2 · What gets drafted

### November, on Branda's blank space

`sources/user_files.yaml`, made only once per space:

```yaml
connection: user_files
kind: from_user
description: Files people hand to an agent (a chat upload, an export, an attachment). The agent reads the rows itself; Lorefile keeps what the columns mean and which number counts.
```

`types/promo_sale.yaml` (shown in flow style to save room; the real file is block style, as `serde_yaml` writes it):

```yaml
type: PromoSale
domain: Core
description: One order line sold with a promo code, from the monthly promo-sales export.
properties:
  line_id:    { type: string,  key: true, description: "Identifier of one order line." }
  order_id:   { type: string,  description: "The order this line belongs to; an order can have several lines." }
  order_date: { type: date,    description: "The day the order was placed." }
  promo_code: { type: string,  description: "The promo code used on the order." }
  campaign:   { type: string,  description: "The campaign the promo code belongs to." }
  channel:    { type: string,  description: "The marketing channel the order came from." }
  region:     { type: string,  description: "The customer's sales region." }
  sku:        { type: string,  description: "The product's stock-keeping unit." }
  product:    { type: string,  description: "The product's name." }
  units:      { type: integer, description: "How many units were on the line." }
  unit_price: { type: number,  unit: USD, description: "Price of one unit before the discount." }
  discount:   { type: number,  unit: USD, description: "Money taken off the line by the promo code." }
  revenue:    { type: number,  unit: USD, description: "What the customer paid for the line: units × unit_price − discount." }
  refunded:   { type: boolean, description: "True when the line was refunded." }
source:
  connection: user_files
  table: promo_sales
  key: line_id
```

`logic/metrics.yaml`, or the file the space's metrics already live in:

```yaml
metrics:
  - metric: promo_sale_revenue
    description: Sum of revenue in PromoSale, leaving out rows where refunded is true.
    source:
      connection: user_files
      table: promo_sales
      filter: refunded = false
      measure: sum(revenue)
    caveats: Refunded lines never count.
    basis: transactional
```

**Notes on the draft:**
- The entity's description and every field's description are the agent's words.
- The metric's description is Lorefile's fixed sentence, built from the answers, so it is the same whichever agent saved it. `caveats` is the user's words via `exclude_words`.
- `table: promo_sales` is the file name without its extension and its date part. It satisfies E012 (a binding names a table) and tells a person what the file is usually called.
- The filter uses the same SQL-like form as the e-commerce example's `success`.

### The rules, in order

**Entity name.**
1. The agent's `entity`.
2. Otherwise, from the file name:
   - Drop the extension.
   - Drop dates and periods (`_2026-12`, `-2026-11-30`, `_nov`, `_q4`) and copy marks (`(1)`, ` copy`, `_final`, `_v2`).
   - Make the last word singular with a short fixed list (`sales`→`sale`, `orders`→`order`, `categories`→`category`, `addresses`→`address`). Words ending in `ss`, `us` or `is` are left alone.
   - Write it in PascalCase.
3. If that name is taken by an entity with other columns, refuse and ask the agent for one.

**Field names.**
- A header that is already a plain name (letters, digits, `_`, starting with a letter) is the field name.
- Any other header (`Unit Price ($)`) gets `snake` of its words (`unit_price`) and a `columns:` entry in the binding mapping the field to the exact header.
- Two headers that give the same field name get `_2`, and the reply says so.

**Types.** Lorefile guesses from the examples, ignoring empty ones. The first rule that fits every example wins:

| Rule | Type | Notes |
|---|---|---|
| No non-empty example | `string` | |
| All are `true`/`false`/`yes`/`no` (any case) | `boolean` | `0`/`1` only when the name reads like a flag (`is_`, `has_`, ends in `ed`, `flag`, `active`); otherwise `integer` |
| All are ISO dates (`2026-11-20`) | `date` | `11/20/2026` and the like stay `string`, and the reply says "looks like a date, saved as text" |
| All are ISO date-times | `datetime` | |
| All are whole numbers with no leading zero | `integer` | `00123` (zip codes, SKUs) stays `string` |
| All are decimal numbers, optionally with `€ £ $` | `number` | Money when the name reads like money (`price, revenue, amount, cost, discount, total, spend, value, fee, tax, sales, gmv`) or a symbol is present |
| Anything else | `string` | **Never `enum`:** five examples cannot list every value, and next month's `HOLIDAY15` would be refused |

An agent's `type` wins when the examples allow it. "integer" for `18.00` is corrected to `number`, and the reply says so. Money fields get `unit` from, in order: the column's `unit`, `answers.currency`, or an unambiguous symbol. Otherwise they have no unit, and the currency question stays.

**Key.**
1. The agent's `key`, which must be a column.
2. Otherwise, the first column whose counts say `distinct == row_count` and `empty == 0`, preferring id-like names (`id`, `*_id`, `*_key`, `*_number`, `*_no`, `*_code`).
3. Otherwise, the first id-like column whose examples are all different.
4. Otherwise, no key and question (d).

In the promo file, rule 2 picks `line_id`, because `order_id` repeats. Rule 3 picks it too, since it comes first.

**Domain.** The space's only domain when it has exactly one (Branda's `Core`); otherwise none.

---

## Part 3 · Where the rows live

**Scenario.** The file is in ChatGPT's sandbox. Lorefile never gets it, and a hosted Lorefile has no folder for the user to drop it in. What should the binding say?

| | A · `csv` source on `data/` | **B · a new `from_user` kind (recommended)** | C · both: `from_user`, readable here when a matching file sits in a folder |
|---|---|---|---|
| ChatGPT, Claude, a hosted server | `where_is` says readable, and `instances` fails: the file is not there | `where_is`: "arrives as a file the user gives you; read it with your own tools" | As B |
| Claude Code on a local folder | `query` works, if the user saves each month under the one fixed name the binding names (there are no globs for csv) | The agent reads the file with its own tools, as in a chat | `query` over every matching file through DuckDB |
| Next month's file name | Must be renamed to `promo_sales.csv` | Does not matter: recognition is by columns | Does not matter |
| Fits the keeper rule | Lorefile reads rows | Lorefile never touches rows | Lorefile reads rows, through the frozen DuckDB door |
| Cost | None new | One kind: a `where_is` branch, a page sentence, no validation change | B plus globbing in the frozen door |

**Recommendation: B, with C left for later.**
- The case that broke was a chat upload, and there the rows are with the agent.
- `kind` is already a free string. `validate` checks only that a non-MCP binding names a `table` (E012), and W106 (file not found) looks only at csv, parquet and duckdb. So `kind: from_user` compiles today.
- What changes is what agents and the page are told.

`where_is("PromoSale")` gains its own branch:

```json
{ "type": "PromoSale", "key": "line_id", "connection": "user_files", "kind": "from_user",
  "table": "promo_sales", "readable_here": false,
  "columns": ["line_id", "order_id", "…", "refunded"],
  "hint": "Rows of PromoSale arrive as a file a person gives you (usually named promo_sales…). Read that file with your own tools; this ontology says what its columns mean and which number counts. Lorefile does not keep the rows." }
```

**Other surfaces:**
- `instances` and `query` on a from_user entity answer with the same sentence instead of today's "use where_is".
- The page's entity Data tab shows "Rows come from a file a person gives the agent. Lorefile keeps the meaning, not the rows." instead of "Lives in… fetch it with your own upload connector".
- Sources shows the from_user source with that sentence.
- `summary` adds `[file from the user]` to the entity's line, so an agent reading only `summary` still knows where rows come from.

---

## Part 4 · Next month

**Scenario: same shape** (December). `match_file` answers `same` with `read_it`, so the agent has the recipe without guessing. The answer's grade tells it what to say:
- A metric written with edit mode on and never reviewed is `proposed`. The agent says "unverified" before the number.
- After Vinay's Accept it is `accepted`.

`learn_file` on the same shape saves nothing and says so.

**Scenario: changed shape.** Next quarter the export drops `region` and adds `gift_wrap`.

```json
{ "match": "changed", "entity": "PromoSale", "shared": 13,
  "added": [{ "name": "gift_wrap", "type": "boolean" }],
  "missing": ["region"],
  "possibly_renamed": [],
  "metrics_at_risk": [],
  "say": "This is PromoSale with one new column (gift_wrap) and one gone (region). Nothing saved reads region. To save gift_wrap, call learn_file; region stays in Lorefile unless a person removes it." }
```

**The rules for a changed shape:**
- **When it counts as changed:** at least 3 columns are shared, and the shared ones are at least 60% of the larger of the two column sets. Below that the file is `new`. When two entities qualify, the one with more shared columns wins. A tie lists both and asks.
- **What `learn_file` proposes:** one `add_property` per added column, typed and described like a new file.
- **What happens to missing columns:** never removed. A month can leave a column out, and deleting meaning is a person's call on the page.
- **When a missing column is used:** if a saved metric's filter or measure names it (the file dropped `refunded`), it goes in `metrics_at_risk`. The `say` sentence leads with it: *"The saved result promo_sale_revenue leaves out refunded lines, and this file has no refunded column. Ask the user how refunds show now before giving a number."*
- **Renames:** one added and one missing column of the same guessed type are listed in `possibly_renamed` (`promo_code` → `promo`). The agent asks the user. If they confirm, `answers.renamed: {"promo_code": "promo"}` writes `columns: {promo_code: promo}` in the binding, so the field and every metric that uses it keep their name.
  - This changes the binding, so under evidence grades an accepted binding is queued for a person rather than written.

**Scenario: saved but not accepted.** Branda runs with edit mode off, and nobody opened the page in November. In December `match_file` finds no PromoSale in the files. It does find the waiting proposal: proposals made by `learn_file` record the entity name and the normalized headers (Part 8, `proposals.rs`; no values). The answer is `waiting`, with how the waiting proposal reads the file, so December's number still follows November's answers. The agent says "this is how it was described on 27 Nov; it has not been accepted yet".

When `learn_file` runs again for that shape:
- **Identical draft:** no new proposal ("already waiting as p-…").
- **Different draft** (answers added, a column changed): the new proposal replaces the old one, which is marked dismissed with "replaced by p-…".

So there is never a pile of duplicates waiting for one person.

---

## Part 5 · The questions, and where the answers go

### How the answers come back

| | **A · a second `learn_file` call (recommended)** | B · `learn_file` saves the shape; the agent adds the metric with `propose_patch` | C · `learn_file` saves; a second call edits the waiting proposal |
|---|---|---|---|
| What the person reviews | One proposal: source, entity, binding, metric | Two. With edit mode off, the metric's must be accepted after the entity's, or it fails to compile (seen in the MCP trial) | One |
| If the agent forgets to ask | It saves nothing, and the result tells it to ask | The shape is saved without a number | As B, until the second call |
| ChatGPT confirmations | One (the save) | Two | Two |
| New machinery | None: it is the batch path | None | Editing a waiting proposal, a new state for the page |

**Recommendation: A.** Both calls send the same columns. A call without `answers` is a dry run. `answers: {}` is the explicit "save the shape, no number", for the user who says "just save it". One small exception borrows from C: a newer `learn_file` for a still-waiting shape replaces it (Part 4). That is a whole new proposal, not an edit.

### Why "refunds don't count" is not a guardrail

The note asked for a guardrail, "refunded rows never count". A guardrail in Lorefile is a CEL rule that every record of an entity, or every action, must pass. `validate` fails what breaks it.

| | Put it in the metric's `filter` and `caveats` (recommended) | A guardrail on PromoSale: `rule: "!refunded"` |
|---|---|---|
| What it says | The result leaves out refunded lines | Every PromoSale must be unrefunded |
| A refunded line | A valid row that does not count | A violation: every refunded line "fails" |
| What an agent computing the number reads | The recipe: `sum(revenue)` where `refunded = false` | A rule about rows, with nothing saying which number it affects |

A rule that the business really wants enforced is still a guardrail. An example is "a promo discount never exceeds 30% of the unit price". The agent can offer it with `propose_patch add_constraint` when the user says it. `learn_file` does not guess rules.

### The grain trap the test files hide

In November, refunds are marked **per line**: three orders have one refunded line and other lines that count. "Refunded lines don't count" and "orders with a refund don't count" give different numbers, and the files show it:

| November | December | What it means |
|---|---|---|
| **1,275.20** | **1,209.60** | Right: lines where `refunded = false` |
| 1,435.40 | 1,231.20 | The saved rule was ignored |
| 1,142.40 | 1,117.80 | Whole orders with any refunded line dropped: the wrong grain |

This is why the key matters (one row per line), and why the agent's entity description should say what one row is. When the flag column's grain is not the key's, the exclude question says which: "Do refunded **lines** count?". This is the case when the key is `line_id` and the agent's description of `refunded` says "line".

---

## Part 6 · The small fixes that come with it

**`add_type` takes `properties` and `key`.** Before, every `add_type` makes `<name>_id` and nothing else. That is why the baseline had `promo_sales_line_id`. After:

```json
{ "op": "add_type", "name": "PromoSale", "description": "…",
  "properties": { "line_id": { "type": "string", "key": true, "description": "…" },
                  "units": "integer",
                  "revenue": { "type": "number", "unit": "USD", "description": "…" } } }
```

- `properties` uses the YAML's own shapes (shorthand or full).
- `key` names the key field when no property says `key: true`.
- With `properties` given, no `<name>_id` is generated.
- Unknown keys inside a property are an error, as in a file.

**`add_property`.** It adds a field to an existing entity: `{op: add_property, type, property, datatype, description, unit, key, column}`.
- It refuses a field that already exists.
- `column`, when it differs from `property`, is written to the binding's `columns:`.
- `datatype` is used because `type` already names the entity in the other ops.
- `learn_file` uses it for a changed shape, and agents can use it by hand.

**The instructions.** One recipe added to the text agents get at `initialize`:

> When the user gives you a file (a CSV, a spreadsheet, an export), call `match_file` with its name, its column names and up to five example values per column: it tells you whether Lorefile already knows this kind of file and how to read it (which number is the result, which rows do not count). If they ask you to save the file, they mean its shape and meaning, not its rows: ask the questions `match_file` returns, then call `learn_file` with the answers. Lorefile never stores rows or example values.

**The tool descriptions**, as the build should write them:

- `match_file`: *"Read-only. Tell Lorefile about a file the user gave you (CSV, spreadsheet sheet, export) without sending its rows: its name, its column names in order, up to five example values per column (used only to guess types, never stored), and if you can, the row count and each column's distinct and empty counts. Answers whether this kind of file is already known (same), known with columns added or missing (changed), saved and waiting for a person (waiting), or new; for a known file, how to read it: the number that counts and the rows that do not, with who stands behind each definition; for a new or changed file, what Lorefile would save and the questions to ask the user first. Call it whenever the user gives you a file, before answering questions about it."*
- `learn_file`: *"Save what a file means, so the next file of the same shape is read the same way by any agent: one entity with every column as a field (the file's own names, types, the key), where its rows come from (a file the user gives the agent), and from the user's answers the number that counts and the rows that do not. Saved as one change a person accepts. Send the same columns as to match_file, one short description per column (what it is and its unit; never quote values from the file), and `answers`. Without `answers` nothing is saved and you get the questions to ask; with `answers: {}` only the shape is saved. Lorefile never stores rows or example values."*

**Annotations.**
- `match_file` gets `readOnlyHint: true`.
- `learn_file` gets `readOnlyHint: false, destructiveHint: false, openWorldHint: false`. It only adds and describes; a person accepts.
- The same marking for every existing tool is in Part 7.

---

## Part 7 · The four OAuth fixes from the live test

These go on branch `claude/oauth-connectors-956d8a`, in its worktree, before Vinay merges it. That branch holds `d41e19a`, `2f6fb5b` and `4459e9e`; it is not pushed. Each fix has a test in `tests/oauth_http.rs` or `tests/mcp_http.rs`.

| # | Found live | Fix | Test |
|---|---|---|---|
| 1 | ChatGPT asks for `GET /.well-known/openid-configuration` three times and gets **401**: an unknown route behind the login | Any `/.well-known/…` path Lorefile does not serve answers **404**, before the login check, with no `WWW-Authenticate`. Lorefile is not an identity provider; a 401 on a discovery address tells a client to sign in to find out how to sign in. | `openid-configuration` and `/.well-known/anything` → 404 without a token; the two OAuth metadata documents unchanged |
| 2 | OpenAI's help article says the provider's metadata should list `offline_access`, or users "may have to re-authenticate". The OAuth spec left it out, following the MCP spec's advice | **List it** in `scopes_supported` in both metadata documents. Nothing else changes: unknown scopes are already ignored and refresh tokens are always issued, so asking for it gives the same two Lorefile scopes plus a refresh token. Update the OAuth spec's line that says it is not listed. | Both documents list it; an authorize request with `scope=lorefile:read lorefile:propose offline_access` succeeds; the token carries the two Lorefile scopes and a refresh token |
| 3 | ChatGPT asked the user to confirm every call, reads included: a tool with no `readOnlyHint` counts as a write. This was the OAuth spec's Part 5 item 4 | Every read tool (`spaces, search, describe, neighbors, path, constraints_for, validate, where_is, instances, summary, query, tables`, and later `match_file`) gets `annotations: {readOnlyHint: true, openWorldHint: false}`. `propose_patch` gets `{readOnlyHint: false, destructiveHint: false, openWorldHint: false}`. `propose_change` gets `{readOnlyHint: false, openWorldHint: false}`, keeping the default `destructiveHint: true`, since with edit mode on it replaces a whole file. | `tools/list` over stdio and HTTP: every tool has `annotations`, every read tool says read-only, neither propose tool does. Whether ChatGPT then stops asking is checked live. |
| 4 | `openai-mcp` sends `server/discover` first; it got a 200, and the body was not checked | **No code change expected.** `Hub::handle`'s last arm already answers any unknown method with a JSON-RPC error, `-32601 "method not found: server/discover"`, in an HTTP 200. That is correct for Streamable HTTP; as a notification (no `id`) it gets 202. Pin both with a test, so a future router cannot turn it into a 401 or 404. | POST `{"jsonrpc":"2.0","id":1,"method":"server/discover"}` → 200 with `error.code == -32601`; the same without `id` → 202 and an empty body |

**Two product facts from the test, for the README's and DEPLOY's ChatGPT section:**
- **The desktop app's MCPs are Codex only.** In the ChatGPT desktop app, Settings › Plugins › MCPs and the plugin marketplace are for Codex (local) only, and plain chats never see them.
- **Plain chats connect on the web.** They need chatgpt.com/plugins → + → "Add custom MCP server", with the URL and OAuth. Then type `@` in a chat to pick it.

Per OpenAI's help article as read on 27 Nov:
- On Business, only admins and owners can add a server.
- Pro gets read and fetch only.
- Plus is not stated.

Say "as of November 2026"; it is someone else's product and it moves.

---

## Part 8 · What this does not do

- **It does not read or keep rows.** The agent reads the file. Lorefile sees column names, five examples per column and counts. It keeps the names, the guessed types and the agent's descriptions. The examples are dropped when the call ends.
- **It does not compute the number.** `read_it` is a recipe. Whether the agent applies `refunded = false` correctly in its own sandbox is the agent's job, and the live test's job to check (the table in Part 5 lists the wrong answers to watch for).
- **It does not upload, fetch or call anything.** There are no outbound requests. The `from_user` kind is a statement about where rows come from, not a connector.
- **It does not decide meaning.** The user's answers set what counts. The agent's words describe columns. Lorefile's rules fix only names, types, key and structure, and a person accepts the result.
- **It does not notice a meaning that changes under the same headers.** If `revenue` starts including tax next quarter, the columns match and `match_file` says `same`. The caveat and the person's review are the guard; a check of totals is left for later.
- **It does not split a file into several entities**, guess guardrails, or turn values into enums.
- **It is not a file store.** "Save this file" never stores the file. The instructions and the tool descriptions say so, so an agent stops looking for an upload tool.

---

## Build notes, file by file

**Notes for every session:**
- Run cargo with the sandbox off; `cargo build --offline` and `cargo test --offline` work.
- `cargo test` must stay green, and clippy clean with `-D warnings`.
- Every folder under `examples/` and `packs/` must still compile with no errors and no warnings (`tests/cli.rs`).
- Commit nothing; Vinay commits.

| File | What changes |
|---|---|
| new `src/learn.rs` | **Pure functions, no I/O:** `normalize_header`, `entity_name_from_file`, `field_names` (with the `columns:` mapping), `guess_type`, `is_money`, `pick_key`, `questions`, `describe_metric`. **`match_shape(ont, pending, headers) → Same \| Changed{added, missing, renamed, at_risk} \| Waiting{proposal} \| New`**, over every bound entity and over pending `learn_file` proposals. **`plan(ont, input) → Vec<Json ops>`**, the ops in Part 1's order, fed to `patch::plan_many`. **`strip_quoted_examples(descriptions, examples)`**: a description containing an example value of 3 or more characters, verbatim, is dropped and reported. **The example values live only in the input struct**, which is dropped at the end of the call; nothing here formats a value into a string that leaves the module, error messages included. |
| `src/patch.rs` | `add_type` accepts `properties` (validated by deserializing each into `PropertySpec`, `deny_unknown_fields` intact) and `key`; with `properties` given, no `<name>_id`. New `add_property`, which edits the type file through `edit_type_file` and, when `column` differs, the binding's `columns`. `set_source` accepts `columns`. Both ops join the unknown-op message. |
| `src/mcp.rs` | `match_file` and `learn_file` in `tool_definitions`, with full schemas (`additionalProperties: false`) and the descriptions in Part 6. `call_tool`: `match_file` reads and answers; `learn_file` runs `learn::plan`, then `plan_many` and `propose_many`, with `reason` defaulting to "learned <file_name>". **The replace rule:** find a pending `learn_file` proposal with the same normalized headers; identical files → no new proposal; different → dismiss it with "replaced by …" after the new one is queued. **Access:** `learn_file` joins `PROPOSE_TOOLS` and the shipped-example refusal in `Hub::handle`. **Text:** the recipe in `instructions()`. **Annotations** on every tool (Part 7 item 3, if not already done on the branch). |
| `src/proposals.rs` | `Proposal.learned: Option<Learned { entity, headers: Vec<String>, file_name }>`, normalized headers only and `skip_serializing_if` none, so older proposal files read as before. A helper to mark a pending proposal dismissed with a reason. |
| `src/data.rs` | `where_is`: a `from_user` branch (Part 3) with `columns` (the headers each field reads). `instances` and the query views skip from_user bindings with the same sentence. |
| `src/ontology.rs` | `summary_markdown`: `[file from the user]` on an entity bound to a from_user source; the Data sources line names the kind. `describe` for a type: `source.kind: from_user` and the same hint. |
| `src/validate.rs` | Nothing required: `from_user` passes E012 when `table` is set, and W106 ignores it. Add a test that a space with a from_user binding compiles clean. |
| `src/serve.rs`, `src/ui/index.html` | The entity Data tab and Sources show the from_user sentence instead of "fetch it with your own upload connector". A `learn_file` proposal's card title is its one-line summary (`summary` already carries it). A dismissed-because-replaced proposal shows "replaced by p-…". Light theme only. |
| `tests/fixtures/` | `promo_sales_2026-11.csv` and `promo_sales_2026-12.csv`, copied from `spaces/branda/data/` (made-up data built for this test; `spaces/` is gitignored), plus two variants of December: one without `region` and with `gift_wrap`, one without `refunded`. |
| Docs, last | **README:** "Teach it a file" in the tour; `match_file` and `learn_file` in "What an agent can ask"; the `from_user` kind in the file format; the ChatGPT facts from Part 7. **ARCHITECTURE:** one paragraph, "a file is learned, not stored". **BACKLOG:** strike the `add_type` gap; add the left-for-later list. **Then** `python3 scripts/build-tutorial.py`. |

**Left for later**, to go into BACKLOG.md:
- A `from_user` source with `path:` and a glob, readable here through the frozen DuckDB door (option C in Part 3), so Claude Code users who drop each month's file in a folder can use `query`.
- A `format:` on date fields for `11/20/2026` and the like.
- A `set_property` op to change a field's type or unit.
- Splitting a flat entity into several, on request.
- Every sheet of a workbook in one call.
- A totals check against the last month ("revenue is 3× last month's") to catch a meaning that changed under the same headers.
- Grades per field.

### Hand-off plan

| Order | Session | Scope (files) |
|---|---|---|
| 1 | **oauth-fixes** | On the OAuth branch, in its worktree: `src/serve.rs` (the `/.well-known` 404), `src/oauth.rs` (`offline_access`), `src/mcp.rs` (annotations), `tests/oauth_http.rs`, `tests/mcp_http.rs`, the OAuth spec's line on `offline_access`, and the ChatGPT facts in README and DEPLOY. Then Vinay decides on merging the branch. |
| 2 | **learn-core** | After 1 is merged into `main` (both touch `src/mcp.rs`): `learn.rs`, `patch.rs`, `mcp.rs`, `proposals.rs`, `data.rs`, `ontology.rs`, the fixtures, unit tests and `tests/mcp_loop.rs`. If Vinay holds the OAuth branch back, build on `main`, and add `learn_file` to `PROPOSE_TOOLS` at the merge; the test "a read-only connection cannot see learn_file" catches a miss. |
| 3 | **learn-page-docs** | `serve.rs` and `index.html` (the sentences, the card, "replaced by"), then the docs and the tutorial; after 2. |
| 4 | **live test** | Vinay, with a session watching the server: ChatGPT web, Claude's custom connector and Claude Code over stdio, with the script below. |

---

## Test ideas

**The rules (unit, `learn.rs`)**
- **Entity names:**
  - `promo_sales.csv`, `promo_sales_2026-12.csv`, `Promo Sales (1).csv` and `promo_sales_final_v2.xlsx › Sheet1` all give `PromoSale` and table `promo_sales`.
  - `orders.csv` gives `Order`; `addresses.csv` gives `Address`; `status.csv` stays `Status`.
- **Types**, from the November file's first five rows: `line_id` string; `order_date` date; `units` integer; `unit_price`, `discount` and `revenue` number with money; `refunded` boolean; `campaign` string, never enum.
  - `00123` stays string; `0`/`1` is integer for `units` and boolean for `is_test`.
  - `11/20/2026` is string with a note; `€12,50` and `£3.00` give EUR and GBP; `$` gives no unit and the currency question.
- **Key:** with counts → `line_id` (`order_id` has 32 distinct values over 40 rows); without counts → `line_id`. A file with no id-like distinct column → no key, with question (d).
- **Headers:** `Unit Price ($)` → field `unit_price` with `columns: {unit_price: "Unit Price ($)"}`; two headers giving one name → `_2`.
- **Descriptions** quoting `WELCOME10` or `Market tote` are dropped and reported. `revenue`'s description mentioning "units × unit_price" is kept: column names are not values.
- **Matching:**
  - December against November's entity → same.
  - The `gift_wrap` variant → changed, with added `gift_wrap` and missing `region`.
  - The no-`refunded` variant → changed, with `promo_sale_revenue` in `metrics_at_risk`.
  - Shared columns below the threshold → new.
  - An orders export against the e-commerce example → same as `Order`.

**The draft (unit, `patch.rs` and `learn.rs` through `plan_many`)**
- **A clean draft:** November with the answers on a blank space gives four files (`sources/user_files.yaml`, `types/promo_sale.yaml`, `logic/metrics.yaml`, `glossary.yaml` when `also_called` is given). They compile clean together; the type has 14 fields named exactly as the headers and key `line_id`; the metric reads `filter: refunded = false` and `measure: sum(revenue)`.
- **Without answers:** no files. With `answers: {}`: no metric.
- **A second from_user entity** in the same space reuses `user_files`.
- **`add_type` with `properties`:** no `<name>_id`; a property with a misspelled key (`descripton`) is refused.
- **`add_property`:** adds a field; with `column` it adds the mapping; it refuses a field that exists.

**The recipe runs as written (`duckdb` feature only)**
- In a temp space, copy November's draft, point a `csv` source at the fixture, and run `select sum(revenue) from PromoSale where refunded = false` through `query`. Assert **1275.20**, then **1209.60** for December, and 1435.40 without the filter.
- This proves the saved recipe is real SQL over the real columns. It does not prove an agent will apply it; the live test does.

**Keeping out values (`tests/mcp_loop.rs`)**
- Call `match_file`, then `learn_file`, with edit mode off and on, using a sentinel example value (`ZZ-SENTINEL-41`) in a column.
- Afterwards, no file anywhere under the space contains it: YAML, `.lorefile/proposals/`, the activity log, git objects (`git grep` over all history).
- No tool reply contains it either.

**The MCP loop (`tests/mcp_loop.rs`, `tests/mcp_http.rs`)**
- **The tool list:** `tools/list` has both tools with full schemas and annotations; `match_file` says read-only.
- **The two calls:** `match_file` on a blank space → `new` with two or three questions; `learn_file` → one queued proposal carrying `learned`; `match_file` again → `waiting`; Accept over HTTP → `same`, with `read_it` showing `accepted`.
- **The replace rule:** `learn_file` twice, identical → one proposal; with an added answer → the old one is dismissed with "replaced by".
- **Refusals:** `learn_file` on a shipped example is refused; over OAuth, a read-only connection does not see `learn_file` and is refused if it calls it.
- **Edit mode on:** the files are written, `trust.yaml` grades the binding, metric and words `proposed`, and `match_file` passes on the "Unverified" sentence.
- **The instructions** contain the file recipe.

**The live test (by hand, recorded in the spec's status when done)**
1. Dismiss `p-20261127-062831-4`. In ChatGPT web, @Lorefile, upload `promo_sales.csv`: "save this to Lorefile".
   - Expected: `match_file` with no confirmation prompt, then two questions (three if currency is asked).
   - Answer "revenue, and no refunds".
   - Expected: one confirmation for `learn_file`, then one proposal with 14 described fields and the metric.
   - Accept it in the page.
2. In a new chat, upload `promo_sales_2026-12.csv`: "how did we do?". Right: **$1,209.60**. Wrong answers name their cause: $1,231.20 (the rule was ignored), $1,117.80 (whole orders dropped), $1,275.20 (November's number).
3. Upload the no-`refunded` variant: the agent must ask about refunds before giving a number.
4. Repeat 1 and 2 in Claude with the custom connector, and in Claude Code over stdio. All three save the same YAML, apart from the agent's descriptions, and give the same number.
5. Run `grep -r "L-1009" spaces/branda --exclude-dir=data`. It must find nothing.

---

## What changed during the build

Built 2026-11-27 in one session, in the spec's order: Part 7, Part 6, the `from_user` kind, the two tools and the recipe, the page, the docs. Every decision in the table was built as recommended. These are the places the build went further than the text, or read it one way where it could be read two:

**Behaviour**

1. **`learn_file` on a known file.** The spec says it refuses a `same` file. It does, as a normal answer (`saved: false`, with the recipe and "Already known as PromoSale: nothing to save"), not as a tool error, so an agent goes on to compute the number. One exception: a file saved shape-only (`answers: {}`) that comes back with `answers.success` gets its metric saved, since nothing said which number counts yet.
2. **A changed file learned again while its first proposal waits.** The first version matched it as `waiting` and then planned a brand-new entity, which was refused (PromoSale exists). Found while checking the page; now a waiting proposal about an entity that exists is planned against that entity, and replaces the waiting one. A unit test pins it.
3. **Money with whole numbers.** A money-named column whose examples are whole (`revenue: 100, 250`) is saved as `number` with money, not `integer`, unless the agent says integer. Otherwise December's `100.50` would land in an integer field.
4. **The question cap.** "At most three" with four possible (result, exclude, currency, key): when all four apply, the currency question waits, since a number can be saved without a unit and the key decides the grain.
5. **The exclude question** is asked only when the file has a true/false column (preferring names like refunded, cancelled, returned, test). Without one, no exclude question; the agent can still pass `exclude` on any column.
6. **Measures are real SQL**: `sum(revenue)`, `count(distinct line_id)`, `count(*)`. The DuckDB test runs the saved recipe as written.
7. **The exclude value is written.** For a text column, `exclude: [{column: status, equals: cancelled}]` writes `status <> 'cancelled'`. That value is the user's answer, not an example, so it is kept, and a description may name it.
8. **Workbook sheets.** A named sheet (`report.xlsx › Sales`) names the entity (`Sale`, table `report_sales`); a generic one (`Sheet1`) leaves the file name to do it.
9. **A file that matches an entity bound elsewhere** (an orders export against the e-commerce `Order`, which lives in a csv source) says where that entity's rows live and does not claim Lorefile lacks them.
10. **The link to Agents** (`accept_at`) is `<public address>/s/<space>/#agents` when the request came through a public HTTPS address, `http://127.0.0.1:<port>/…` for a request from this machine, and absent over stdio.
11. **The page's tool runner** (`/api/tool`) treats `learn_file` as a write, behind the same write header as the propose tools.
12. **`instances` and `query`** on a `from_user` entity answer with the from_user sentence; `query`'s "table not found" is replaced by it when the SQL names such an entity. `tables` still lists it under `not_readable`.
13. **`read_it` in the `changed` answer too**, so the agent has the recipe when nothing is at risk; the `same` answer adds `column_names` when a header differs from its field.

**Part 7**

14. **The `/.well-known` 404 applies on every address**, plain `http://127.0.0.1` and `--no-oauth` included, not only on OAuth addresses. Five existing assertions that pinned a 401 there (plain http, `--no-oauth`, a short token, a pinned server asked on another name, `openid-configuration`) now pin the 404, with no `WWW-Authenticate`.
15. **Annotations default to "write".** The read tools are an explicit list; a tool added later and not put on it is not marked read-only. `spaces` is on the list.

**Known and left as they are**

16. **A learned metric has no owner**, so the proposal compiles with no errors and one warning (W109). Lorefile does not invent who decides what a number means. In BACKLOG.
17. **`refunded = false` also leaves out rows where `refunded` is empty.** Right for these files. In BACKLOG.
18. **Accept needs `--allow-edit`.** Unchanged, but it matters for the live test: a server without it queues the agent's save and greys out Accept.

**Tests added**: 15 unit tests in `src/learn.rs` (names, types, money, key, headers, quoted values, matching, the clean draft, shape only, a known file, a changed file and a rename, errors without values, the waiting changed file, and the recipe in DuckDB: 1,275.20, 1,209.60, 1,435.40, and the wrong grain 1,142.40), two in `src/patch.rs`, one in `src/validate.rs`; five in `tests/mcp_loop.rs` (the loop to `waiting`, the replace rule, shipped examples, edit mode on, and the sentinel `ZZ-SENTINEL-41` found in no file and no commit, with edits off and on); two in `tests/mcp_http.rs` (`server/discover` and annotations over stdio and HTTP; learned over stdio, accepted on the page, read over stdio); two in `tests/oauth_http.rs` (`offline_access`; the whole flow over an OAuth connection, with a read-only connection refused). Fixtures in `tests/fixtures/learn/`: November and December as given, plus two variants made from December (without `region` and with `gift_wrap` true on every fourth line; without `refunded`).

**Not verified here**: the live test below (ChatGPT web, Claude's custom connector, Claude Code over stdio), including whether ChatGPT stops asking to confirm reads now that they carry `readOnlyHint`.
