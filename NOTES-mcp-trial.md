# MCP trial: a marketer's agent on Lorefile (then called Stella Ontology)

2026-11-25. I used the `lorefile` MCP server from `.mcp.json` (read-only, `--dir spaces`) from Claude Code, acting as Lumen Home's marketing agent getting ready for "give me last week's performance". Every call below was actually made. The edits-on runs and the two-space run used a scratch copy of `branda` driven over stdio, so the real `branda` only holds the three proposals from step 3. Points marked *(from code)* were read in `src/` and not run.

## Verdict in one paragraph

Reading works. `summary` → `describe` → `where_is` → `validate` gets an agent from nothing to a ready recipe and a guardrail answer in four calls, and the answers are short and plain. Teaching is where it breaks. In a blank space the agent cannot finish teaching in one sitting: each save waits on the one before it being accepted by a person. The very first word saved in a new space comes out as broken YAML. And @ecommerce, the example that is supposed to teach good habits, has a `success` recipe that would undercount, plus a forecast function whose arguments do not match the real tool on the measurement MCP.

## 1. Summary of @ecommerce, read for "last week's performance"

**What it told me:** what counts (`success` = paid orders, `shop_bigquery.orders`, `status in ('paid','fulfilled')`, count distinct `order_id`, per day, owner growth@). It said platform conversions are compared to that number, never added to it. It showed that spend comes from two MCPs (`paid_performance`, a Meta variant and a Google variant) and that there is a forecast (`forecast_success` on the measurement MCP). All of that fits on one screen, which is the product working.

**What it did not tell me, and I had to guess:**
- **Which date counts for "per day".** It could be `placed_on` or the payment date, and no time zone is given. "Last week" (Mon–Sun? which time zone?) is not defined anywhere.
- **What to do without BigQuery.** `where_is Order` answers `shop_files` csv, `readable_here: true`, `row_count: 18`. Nothing marks that file as an 18-row sample from Feb–Mar 2026. An agent with no BigQuery connector will `query` it and report numbers from it, or report zero rows for last week.
- **How to join Google spend.** `paid_performance` says "joined on campaign name", but the Google variant fetches only `cost_micros` and `date`, with no campaign name. Neither variant shows where the date range goes. `when: channel == "meta"` refers to a `channel` that is not defined anywhere.
- **Revenue.** Only an order count exists. "Performance" usually means revenue and ROAS too.

**Content bugs in the example** (an agent copies these habits):
- `success` filters `status in ('paid','fulfilled')`, but `Order.status` values are `placed, paid, packing, shipped, delivered, cancelled, returned`. `fulfilled` does not exist, and shipped or delivered paid orders are left out. On the sample: `status = 'paid'` gives 2 rows, while `payment_status = 'paid'` gives 11.
- `forecast_success` calls `mmm_run_budget_optimizer` with `{study_id, budget, horizon}` and `horizon_days`. The real tool on the measurement MCP takes `studyId`, `timePeriodWeeks`, `totalBudget`, and `constraints`. It runs for 2–5 minutes and must be polled with `mmm_refresh_study`, and it optimises revenue, not paid orders. The study id came from an environment variable the agent cannot read. Following the recipe word for word fails.
- `describe shop_bigquery` returns `used_by: []`, and its description says "nothing in this folder binds to it yet", while `success` reads from it. The page shows "Not used by anything yet" on `meta_ads` and `google_ads` too, though `paid_performance` uses both. Metrics and functions do not count as users.

## 2. The Monday story, call by call

| Call | What it let me do | What I still had to guess |
|---|---|---|
| `describe {name:"success", space:"@ecommerce"}` | Write the BigQuery SQL almost verbatim; quote the owner and the caveat | Date column, time zone, that `fulfilled` is wrong |
| `where_is {type:"Order", space:"@ecommerce"}` | Know the rows are readable here, with a ready `select * from Order` | That these are 18 sample rows, not the order history `success` points at |
| `constraints_for {name:"cancel_order", space:"@ecommerce"}` | See the one blocking rule (EC-001) with its rationale and owner | Nothing; clean |
| `validate {action:"cancel_order", inputs:{order:"ORD-1007"}}` | Tell the user "no, ORD-1007 is delivered; EC-001", with the full record, approval and side effects | Which alternative to offer (`suggestions: []`; a delivered order should point at `refund_order` or a return) |
| `validate {action:"cancel_order", inputs:{order:{status:"shipped", total:1450.0}}}` | Check an inline record I fetched myself | Who approves: `requires_approval_from` is the prose "Nobody below $1,000; Support Lead above", not worked out for $1,450 |
| `validate {action:"cancel_order", inputs:{order:"ORD-99999"}}` | **Misled me.** `verdict: "fail"`, `summary: "cancel_order is blocked by EC-001."` The order does not exist; the rule errored with "Undeclared reference to 'status'". An agent would tell the user the order has shipped. | |
| `describe {name:"forecast_success", space:"@ecommerce"}` | See the server, tool, args, who may call it, and the caveat "report the intervals" | Every argument name (see above), and how to fill the study-id environment variable |
| `describe {name:"cancel_order", space:"@ecommerce"}` | **Misled me.** It returns the *function* `cancel_order` (manual; "cancels … and refunds it"; outputs a `Return`), not the action. The action is hidden behind the name clash, and the function contradicts EC-001's "handled as a return, not a cancellation". | |

## 3. Teaching `branda` (read-only server, so everything queues)

| Call | Result |
|---|---|
| `propose_patch {op:"add_type", name:"Order", domain:"Core", space:"branda", …}` | Queued `p-20261125-181848-1`, summary "created Order" ✓ |
| `propose_patch {op:"add_term", term:"checkout", means:"Order", space:"branda"}` | Refused: "`Order` is not an entity here; add it first". It is, but only queued. |
| `propose_change {file:"sources/shop_warehouse.yaml", space:"branda", …}` | Queued `p-20261125-181901-2`. There is no `add_source` op, so I hand-wrote YAML; I guessed the shape from `describe shop_bigquery` and it happened to be right. **Summary is empty.** |
| `propose_patch {op:"add_metric", metric:"success", connection:"shop_warehouse", …}` | Refused: "no connection named `shop_warehouse`; add it under sources/ first". Same waiting problem. |
| `propose_change {file:"logic/measurement.yaml", …}` (same metric) | **Queued with an error** as `p-20261125-181930-3` (E008). In the same situation, `propose_patch` refuses while `propose_change` queues. **Summary is empty.** |

All three files are under `spaces/branda/.lorefile/proposals/` and show in http://localhost:7076/s/branda/#agents under "Waiting for your review". Only the type gets the plain-words line ("An agent learned: created Order"). The two `propose_change` cards show only a name.

On the scratch copy, with the type and the two sources accepted first:
- **Bug:** `propose_patch {op:"add_term", term:"checkout", means:"Order"}` queued with **E001 invalid YAML**. The blank-space `glossary.yaml` holds `terms: []`, and the patch appends `  - term: checkout` under it. The live `branda` has the same file, so the first word any agent saves in a new space can never be accepted.
- `add_metric success` queued clean ("added metric success (shop_warehouse.orders)"). A second `add_metric success` also queued clean, with no duplicate warning. `measure: "avg(total_price"` was accepted unchecked.
- `set_source {type:"Order", connection:"shop_warehouse", table:"orders"}` queued clean ("bound Order to shop_warehouse.orders").
- *(from code)* Four pending proposals each carry the full text of `logic/metrics.yaml` from the same base. Accept writes `content` verbatim (`src/serve.rs:233-236`), and `before` is only used for the diff, so accepting the second one silently drops the first.
- *(from code)* A proposal's `ok` is frozen when it is queued (`src/proposals.rs:71`), and the page disables Accept when `!ok`. So the live metric `p-…-3`, which failed only because its source was pending, stays un-acceptable after the source is accepted.

## 4. Trying to break it

| Call | Result |
|---|---|
| `propose_patch {op:"set_type_description", space:"@ecommerce", …}` | Clear refusal. But "Create a space of your own (or copy the example)" points at something no MCP tool can do. |
| `propose_patch {op:"add_term", means:"Widget", space:"branda"}` | Clear: "`Widget` is not an entity here; add it first…" ✓ |
| `propose_patch {op:"add_metric", connection:"meta_ads"}` with no tool (scratch, mcp source) | Clear: "`meta_ads` is an MCP source, so the metric needs the tool to call" ✓. Tool on a bigquery source: "the metric needs the table" ✓ |
| `describe {name:"success"}` with no space (one user space) | **Silent default to branda.** Returns "Nothing named 'success'. Closest:" followed by an empty list, with no hint that it looked in `branda` or that `success` exists in `@ecommerce`. Likewise `validate cancel_order` returns "unknown action `cancel_order`. Known actions:" (empty), and `where_is Order` returns "unknown type Order". |
| `summary {}` with two user spaces (scratch: branda, brandb) | Right answer: "say which space you mean (space: "branda"). Spaces here: …". The example value nudges the agent toward `branda` instead of asking. |

Also found along the way:
- **Blank `branda` is invalid** from birth: E000 "no types found". `spaces` shows `errors: 1` and the page footer shows "✗ invalid", but `summary` says nothing. No MCP tool shows an ontology's own errors, because the MCP `validate` means "check an action", unlike the CLI's `validate`. The page's BUILD list shows six green ticks for the empty space.
- *(from code)* The `space` enum in every tool schema is fixed when the server starts (`src/mcp.rs:39`), and `listChanged` is false. A space created in the page after Claude connects cannot be named until the MCP restarts.

## What confused me from the agent's side

- **Tool descriptions hide the headline feature.** `summary` says "types by domain, blocking rules, actions, data sources, glossary". `describe` lists "a type, a relationship, an action, a rule, or a data source". Neither mentions metrics or functions, though both return them. An agent picking tools from descriptions would not know `success` exists.
- **`validate` vs `constraints_for`.** `constraints_for` says "Call this before proposing any change", which reads like `propose_patch`, but it is about actions. `validate` means "may I take this action", while the page and CLI use "valid" to mean "the ontology compiles".
- **`propose_patch` puts its whole schema in prose.** It declares only `op`, `reason` and `space`, with `additionalProperties: true`, and every field (`connection`, `table | tool + params`, `variants`…) is described in one long sentence. Claude copes. Strict-schema clients (OpenAI structured tools) may drop undeclared fields *(not run)*.
- **`propose_change` asks for "the full new text of one YAML file"**, but no tool returns a file's current text. Editing an existing file means rebuilding it from `describe` output and hoping nothing is lost.
- **The instructions text** promises "both are compiled and checked first, and queued". That is true of each save alone, but not of a chain of saves (see step 3).

## What a Claude or ChatGPT user hits first

- **ChatGPT:** it cannot connect. The server is stdio only, and the page says HTTP is on the roadmap.
- **Claude Desktop:** the page's config snippet runs a bare `lorefile` with a relative `--dir spaces`. That needs the binary on PATH and a working directory Desktop does not set, so the server most likely fails to start. The snippet also lacks `--allow-edit`, while the Claude Code line on the same page has it, so saves are written from one client and queued from the other.
- **Claude Code (this repo):** it works out of the box, read-only. The first time the user says "save that", the agent queues a type and is then refused on the word and the metric until someone clicks Accept. If the word is the first save, it arrives broken (glossary bug).

## The three changes I would make

1. **Let one sitting finish teaching.** Accept a list of ops in one `propose_patch` (type + source + metric + word), compiled together and queued as one proposal. Add an `add_source` op. Re-check pending proposals when they are listed, and refuse or rebase an accept when the file changed since `before`. Fix the `terms: []` append. Fill `summary` for `propose_change`.
2. **Make wrong-space and missing-record answers say so.** On a miss in the default space, say "looked in branda (the default); `success` is in @ecommerce" and actually list the closest names. Make `validate` with an unresolved key return `verdict: "unknown"` ("ORD-99999 not found; fetch it and pass it inline"), not "blocked by EC-001". Add `metric` and `function` to the `summary`, `describe` and `search` descriptions, and give an MCP way to see the space's own errors.
3. **Fix the Monday recipe in @ecommerce**, since agents copy it.
   - `success` should use statuses that exist, and name its date column and time zone.
   - The Google variant should fetch the campaign name, and both variants should show where the date range goes.
   - Define `channel`.
   - Make `forecast_success` match the real measurement MCP tool (`studyId`, `timePeriodWeeks`, `totalBudget`, async with polling).
   - Rename or drop the `cancel_order` function so `describe cancel_order` returns the action.
   - Count metrics and functions in `used_by`.
   - Mark the CSV as a sample.
