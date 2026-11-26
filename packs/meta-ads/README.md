# Meta Ads ontology pack

A Meta Ads MCP server gives an agent rows: campaigns, ad sets, insights, a list of `actions` with `purchase: 143`. It does not tell the agent that 143 is a 7-day-click-plus-1-day-view count, booked on the day of the click, still rising for a week, partly modelled because of iOS, and not a count of sales the ads caused. This pack does.

## What it adds on top of the data MCP

- **Meaning for every object.** Account, Campaign, AdSet, Ad, Creative, Audience, Placement, plus the measurement objects a data MCP does not name: Insight (a delivery row), Conversion (one attributed count under one window), AttributionWindow (reference data, inline), LiftStudy (the only incremental number Meta has), BudgetChange (a proposed edit).
- **The platform's own semantics, written down.** Which objective buys which result and therefore which KPI is fair. Why `status` and `effective_status` differ. That ad-set budgets under Advantage campaign budget are floors and caps. That geo breakdowns stop at region and DMA and show where ads were delivered, not what they caused. That 28-day windows were retired in January 2021.
- **16 rules an agent can check before it speaks or acts.** Blocks: never compare counts under different windows or action types; never pause or re-budget a campaign that is a cell in a running lift study or geo experiment; lookalike seeds and ratios inside Meta's limits. Warns: counts younger than 72 hours or inside an open 7-day window, awareness campaigns judged on CPA, ad sets still learning, budget steps over 20%. Infos: CBO ad-set budgets are advisory, geo breakdowns are not incrementality.
- **Five actions.** Three writes (`pause_campaign`, `change_budget`, `create_lookalike`) with their checks and approvers, and two read-out guards (`read_performance`, `compare_conversions`) that an agent calls before quoting or comparing a number. The guards change nothing on Meta.
- **A glossary** so "ROAS", "frequency", "lift test" and "Advantage+" resolve to the right object.

The pack holds no data and never calls Meta. `where_is("Campaign")` answers with the tool to call on your Meta Ads MCP server and the parameters to pass. Bindings use the tool names of the open-source meta-ads-mcp server; change `tool:` in each `types/*.yaml` if yours differ. Lines marked `# TODO verify` are places where the author was not certain of a platform detail.

## Load both in Claude Desktop

Add two servers to `claude_desktop_config.json`: your Meta Ads MCP server under the name the pack expects (`meta-ads`), and this ontology.

```json
{
  "mcpServers": {
    "meta-ads": {
      "command": "uvx",
      "args": ["meta-ads-mcp"],
      "env": { "META_ACCESS_TOKEN": "..." }
    },
    "meta-ads-ontology": {
      "command": "/absolute/path/to/lorefile",
      "args": ["mcp", "--dir", "/absolute/path/to/packs/meta-ads"]
    }
  }
}
```

If your Meta server is registered under another name, set `server:` in `sources/meta-ads.yaml` to match so `where_is` names the right one. Then start a conversation with "load the meta-ads-ontology summary first".

## Three questions, and what the agent does

**"What was our Meta ROAS last week?"**

```
meta-ads-ontology  summary
meta-ads-ontology  describe("Conversion")          → ROAS = value / spend, one window, click-date booked
meta-ads-ontology  where_is("Insight")              → get_insights(object_id, time_range, level, action_attribution_windows)
meta-ads           get_insights(object_id="act_123", time_range={since, until}, level="account",
                                fields=[spend, actions, action_values], action_attribution_windows=["7d_click","1d_view"])
meta-ads-ontology  validate("read_performance", {campaign: {...}, conversion: {attribution_window: "7d_click_1d_view",
                                hours_since_date_stop: 30, ...}})
                   → warn MA-002, MA-003: the number is provisional; the agent says so
```

**"Which ad set has the best CPA, A or B?"**

```
meta-ads           get_adsets(account_id, campaign_id)   → A is LEARNING, B is SUCCESS
meta-ads           get_insights(...) for A and B under the same action_attribution_windows
meta-ads-ontology  validate("compare_conversions", {a: {...}, b: {...}})
                   → block MA-001 if the windows differ; otherwise ok
meta-ads-ontology  constraints_for("AdSet")             → MA-008: A's CPA is not stable yet
```

**"Pause the campaign in Texas, it's underperforming."**

```
meta-ads           get_campaigns(account_id)             → objective OUTCOME_AWARENESS, in_active_experiment true (from the measurement team)
meta-ads-ontology  validate("pause_campaign", {campaign: {...}})
                   → FAIL MA-010: the campaign is a geo-test cell; approval: measurement team
meta-ads-ontology  constraints_for("read_performance")  → MA-006: an awareness campaign was judged on CPA
```

The agent answers: the campaign is a test cell and cannot be paused until the test ends, and its "underperformance" is a CPA read on a reach campaign, which is not a fair KPI.

## Validate

```bash
./target/debug/lorefile validate --dir packs/meta-ads
```
