# Google Ads ontology pack

A Google Ads MCP server runs GAQL and returns rows: `metrics.conversions: 41.3`, `campaign.bidding_strategy_type: MANUAL_CPV`, `segments.geo_target_region: California`. It does not say that 41.3 counts only primary conversion actions, each under its own attribution model (data-driven by default, hence the fraction), booked on the click date and still rising; that a CPV campaign cannot be judged on CPA; or that "California" is the location criterion that matched, which under presence-or-interest targeting includes people who were not in California. This pack does.

## What it adds on top of the data MCP

- **Meaning for every object.** Account (customer), Campaign, AdGroup, Keyword, Ad, ConversionAction, BiddingStrategy, Audience (user list), GeoTarget, plus Readout (one metric value as a GAQL row returns it) and the two change records an agent writes before validate (BudgetChange, TargetChange).
- **The platform's own semantics, written down.** Campaign types and what each can report. Bid strategies and which KPI each one earns. Conversion action categories, counting rules (one per click for leads, every conversion for purchases), primary versus secondary, lookback windows, data-driven attribution as the default. `conversions` versus `all_conversions` versus `conversions_by_conversion_date`. Geo target versus location of interest, and which GAQL resource reports physical location.
- **16 rules.** Blocks: compare the same metric; live client account only; campaigns in a running experiment (ours, or a Google campaign experiment arm) are frozen; shared budgets are changed on the budget; a target only exists on the matching Smart Bidding strategy. Warns: conversion metrics younger than 72 hours, CPA on CPM/CPV/impression-share strategies, Smart Bidding still learning, enabled but not serving, counting rule that does not fit the event, budget steps over 20%. Infos: geo segments are matched location, presence-or-interest targeting, secondary actions, last-click actions in a data-driven account.
- **Six actions.** Three writes (`pause_campaign`, `change_budget`, `change_bid_target`) with checks and approvers, and three read-out guards (`read_performance`, `compare_readouts`, `audit_conversion_action`) that change nothing.
- **A glossary**: tCPA, tROAS, PMax, DMA, location of interest, DDA, gclid, conversion lag.

The pack holds no data and never calls Google. `where_is("Campaign")` answers with the tool, the `customer_id` parameter and a ready GAQL query. The query is the stable part; if your MCP server's query tool is not called `search`, change `tool:` in each `types/*.yaml`. Lines marked `# TODO verify` flag details the author was not certain of.

## Load both in Claude Desktop

```json
{
  "mcpServers": {
    "google-ads": {
      "command": "uvx",
      "args": ["google-ads-mcp"],
      "env": { "GOOGLE_ADS_CREDENTIALS_PATH": "...", "GOOGLE_ADS_DEVELOPER_TOKEN": "...", "GOOGLE_ADS_LOGIN_CUSTOMER_ID": "..." }
    },
    "google-ads-ontology": {
      "command": "/absolute/path/to/stella-ontology",
      "args": ["mcp", "--dir", "/absolute/path/to/packs/google-ads"]
    }
  }
}
```

If your Google Ads server has another name, set `server:` in `sources/google-ads.yaml` to match.

## Three questions, and what the agent does

**"How many conversions did the Brand campaign get last week, and can I trust the number?"**

```
google-ads-ontology  summary
google-ads-ontology  where_is("Readout")   → search(customer_id, "SELECT campaign.id, segments.date, metrics.conversions, ... FROM campaign WHERE ...")
google-ads           search(customer_id="1234567890", query="SELECT ... metrics.conversions, metrics.conversions_by_conversion_date FROM campaign WHERE campaign.id = 7 AND segments.date BETWEEN ...")
google-ads           search(customer_id, "SELECT conversion_action.name, conversion_action.counting_type, conversion_action.include_in_conversions_metric, ... FROM conversion_action")
google-ads-ontology  validate("audit_conversion_action", {conversion_action: {...}})   → GA-009 if a purchase counts one per click
google-ads-ontology  validate("read_performance", {account: {...}, campaign: {...}, readout: {metric: "conversions", hours_since_date_stop: 20, ...}})
                     → warn GA-002: provisional; the agent quotes conversions_by_conversion_date as the settled figure
```

**"Did Texas or Florida convert better?"**

```
google-ads           search(customer_id, "SELECT geographic_view.location_type, segments.geo_target_region, metrics.conversions, metrics.cost_micros FROM geographic_view WHERE segments.date DURING LAST_30_DAYS")
google-ads-ontology  validate("compare_readouts", {a: {segment: "geo_target_region", ...}, b: {...}})
                     → info GA-003: these are matched locations; filter location_type = LOCATION_OF_PRESENCE for residents
google-ads-ontology  describe("GeoTarget")   → the campaign's positive_geo_target_type decides whether outsiders are counted
```

The agent answers with the presence-only split and adds that neither number is incremental.

**"Raise the tROAS on the Shopping campaign to 5."**

```
google-ads           search(customer_id, "SELECT campaign.bidding_strategy_type, campaign.bidding_strategy_system_status, campaign.target_roas.target_roas, campaign.experiment_type ... FROM campaign WHERE campaign.id = 9")
google-ads-ontology  validate("change_bid_target", {campaign: {bidding_strategy_type: "TARGET_ROAS", bidding_strategy_system_status: "LEARNING_BUDGET_CHANGE", ...}, change: {target_type: "target_roas", current_target: 3.5, new_target: 5.0}})
                     → warn GA-005: still learning from last week's budget change; GA-016 rationale: step by about 20%
```

## Validate

```bash
./target/debug/stella-ontology validate --dir packs/google-ads
```
