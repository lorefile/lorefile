# Meridian Systems: a long-lead B2B sales organisation

Meridian sells industrial-maintenance software through account executives and channel partners, with six-to-nine-month cycles. The sales process is a list of stages, each saying what a deal must have before it may enter; the rules read that list, so changing the process means editing `types/stage.yaml`, not the rules.

**Before.** A rep asked Claude: *"Move the Fairmont deal to Proposal, we're sending it Friday."* Claude did not know that Proposal requires a named economic buyer, that OPP-5004 has none, or that the deal is already forecast as commit at 35% probability. It would have moved it.

**After.** Claude calls `validate(advance_stage, {opportunity: OPP-5004, from: ST-03, to: ST-04})` and answers: blocked by ST-001 until an economic buyer is qualified, and the only stages the deal can move to today are backwards or Closed Lost. The same ontology tells it that a $310k quote needs Deal Desk before the customer sees it, that a 24% discount needs the VP Sales, and that the quote itself lives in the CPQ warehouse and has to be fetched first.

## What is in the folder

| | Count | Notes |
|---|---|---|
| Types | 11 | Account, Contact, Opportunity, Stage, Quote, Proposal, Territory, Rep, Quota, Partner, Activity |
| Rules | 25 | 20 block, 4 warn, 1 info, in `constraints/pipeline` and `constraints/people` |
| Actions | 5 | advance_stage, submit_quote, reassign_account, close_won, register_partner_deal |
| Data | `data/*.csv` | 10 to 20 rows per bound type; Stage, Territory and Partner are inline reference lists |
| Sources | 3 | `crm_files` (csv, readable here), `cpq_snowflake` and `crm_api` (declared for agents) |

Quote is bound to `cpq_snowflake`, which the binary cannot read, so `where_is` points the agent at the table and `validate` accepts the row inline. One format note: "no two reps own the same account in one territory" is a cross-row rule; RP-001 checks the row-level half (the new owner must be in the account's territory) and carries the uniqueness query in `sql:`, which is parsed but not evaluated yet (see the format gaps in ARCHITECTURE.md).

## Validate

```bash
./target/debug/lorefile validate --dir examples/b2b-sales
```

```
✓ valid · 11 types · 15 relationships · 25 rules · 5 actions · 3 sources · 0 errors · 0 warnings
```

## Five checks

### 1. Blocked, with the stages that would pass

OPP-5004 has no economic buyer, so it cannot enter Proposal. The suggestions are honest: without a buyer the deal can only go backwards or be lost.

```bash
./target/debug/lorefile check --dir examples/b2b-sales advance_stage opportunity=OPP-5004 from=ST-03 to=ST-04
```

```
✗ advance_stage is blocked by ST-001 (block), ST-003 (info).
 FAIL  ST-001   Proposal and later need a named economic buyer — violated for opportunity=OPP-5004, to=ST-04
         why: Deals without a budget owner stall after the proposal and distort the forecast. Naming the economic buyer is the exit criterion for Discovery.

  ok   ST-002   Negotiation and later need an approved quote — holds for opportunity=OPP-5004, to=ST-04
  ok   ST-004   Moving to the same stage is a no-op — holds for opportunity=OPP-5004, from=ST-03, to=ST-04
  ok   ST-007   The from stage must be the deal's current stage — holds for opportunity=OPP-5004, from=ST-03, to=ST-04
  ok   ST-005   One stage at a time — holds for opportunity=OPP-5004, from=ST-03, to=ST-04
 info  ST-003   Commit forecast belongs to late stages — violated for opportunity=OPP-5004, to=ST-04
         why: Informational; a committed deal in an early stage is reviewed in the weekly forecast call.
suggestions:
  to → Prospecting (ST-01)
  to → Qualification (ST-02)
  to → Closed Lost (ST-07)
approval: Nobody; the sales manager is notified when a stage is skipped
```

Exit code 2.

### 2. Allowed with a warning

Moving Cobalt Logistics from Marcus (Americas East Enterprise) to Sofia (Americas West Enterprise). Sofia is active and ramped, but the account would be owned outside its territory.

```bash
./target/debug/lorefile check --dir examples/b2b-sales reassign_account account=ACC-2003 from=REP-11 to=REP-14
```

```
⚠ reassign_account may proceed with warnings: RP-001 (warn).
  ok   RP-002   Only active reps own accounts — holds for from=REP-11
  ok   RP-002   Only active reps own accounts — holds for to=REP-14
  ok   RP-003   Strategic accounts need ramped reps — holds for account=ACC-2003, to=REP-14
  ok   RP-005   Reassigning to the same rep is a no-op — holds for account=ACC-2003, from=REP-11, to=REP-14
 warn  RP-001   One owner per account per territory — violated for account=ACC-2003, to=REP-14
         why: Ownership follows territory. An account owned by a rep outside its territory has two claims on it (the territory's rep and the owner), which is how two reps end up working the same account. The row-level rule checks the new owner's territory; the cross-row uniqueness check is the SQL below.

approval: Sales Ops; the VP Sales for strategic accounts
```

Exit code 0.

### 3. Pass

Halden's $180k rollout has an accepted, legally reviewed proposal, a named economic buyer and a signed order form.

```bash
./target/debug/lorefile check --dir examples/b2b-sales close_won opportunity=OPP-5001 proposal=PRP-901
```

```
✓ close_won may proceed: all 5 rules hold.
  ok   CW-001   Closed-won needs a signed order form — holds for opportunity=OPP-5001
  ok   CW-002   Closed-won needs an accepted proposal — holds for proposal=PRP-901
  ok   CW-003   Legal reviews deals above $100k — holds for opportunity=OPP-5001, proposal=PRP-901
  ok   CW-004   Proposal must belong to the opportunity — holds for opportunity=OPP-5001, proposal=PRP-901
  ok   CW-005   Closed-won needs a named economic buyer — holds for opportunity=OPP-5001
approval: Sales Ops; Finance above $500k
```

Exit code 0.

### 4. A record the binary cannot read, then blocked and warned

Quotes live in the CPQ system's Snowflake replica. The first attempt tells the agent what to fetch:

```bash
./target/debug/lorefile check --dir examples/b2b-sales submit_quote quote=Q-7781 opportunity=OPP-5003
```

```
✗ submit_quote is blocked by QT-001 (block), QT-003 (block), QT-004 (block), QT-005 (block), QT-002 (warn).
 FAIL  QT-001   Quotes over $250k need Deal Desk — could not evaluate for quote=Q-7781
         error: Undeclared reference to 'deal_desk_approved'
         why: Pricing policy PP-4; large quotes carry non-standard terms Deal Desk must review before the customer sees them.
 FAIL  QT-003   Discount floor at 35% — could not evaluate for quote=Q-7781
         error: Undeclared reference to 'discount_pct'
         why: Below 65% of list the deal is unprofitable over its term; the CFO has never approved an exception.
 FAIL  QT-004   Quote must belong to the opportunity — could not evaluate for quote=Q-7781, opportunity=OPP-5003
         error: No such key: opportunity_id
         why: Prevents sending a quote built for another deal.
 FAIL  QT-005   Only draft quotes can be submitted — could not evaluate for quote=Q-7781
         error: Undeclared reference to 'status'
         why: Submitted quotes are locked; changes go through a new version in CPQ.
 warn  QT-002   Discount above 20% needs VP Sales — could not evaluate for quote=Q-7781
         error: Undeclared reference to 'discount_pct'
         why: Reps may discount to 20% on their own authority; deeper discounts are a VP Sales decision.
approval: Deal Desk above $250k; VP Sales above 20% discount
note: `quote` = Q-7781 could not be loaded (Quote data is not readable here); fetch the row with your own connector and pass it inline, e.g. {"status": "draft", "payment_terms": "net30"}
```

Exit code 2.

```bash
./target/debug/lorefile where --dir examples/b2b-sales Quote
```

```
{
  "type": "Quote",
  "key": "quote_id",
  "connection": "cpq_snowflake",
  "kind": "snowflake",
  "table": "CPQ.QUOTES",
  "key_column": "QUOTE_NO",
  "columns": {
    "quote_id": "QUOTE_NO",
    "opportunity_id": "OPP_ID",
    "amount_usd": "TOTAL_USD",
    "discount_pct": "DISCOUNT_PCT",
    "status": "STATUS",
    "valid_until": "VALID_UNTIL",
    "deal_desk_approved": "DD_APPROVED",
    "payment_terms": "PAY_TERMS"
  },
  "readable_here": false,
  "config": {
    "account": "${SNOWFLAKE_ACCOUNT}",
    "database": "REVOPS",
    "schema": "CPQ",
    "warehouse": "SALES_WH",
    "auth": "env"
  },
  "hint": "Query CPQ.QUOTES with your own snowflake connector; the ontology only holds the binding."
}
```

With the quote passed inline, the verdict is real: $310k without Deal Desk is blocked, and the 24% discount needs the VP Sales on top.

```bash
./target/debug/lorefile check --dir examples/b2b-sales submit_quote opportunity=OPP-5003 \
  quote='{"quote_id":"Q-7781","opportunity_id":"OPP-5003","amount_usd":310000.0,"discount_pct":24.0,"status":"draft","valid_until":"2026-05-15","deal_desk_approved":false,"payment_terms":"net60"}'
```

```
✗ submit_quote is blocked by QT-001 (block), QT-002 (warn).
 FAIL  QT-001   Quotes over $250k need Deal Desk — violated for quote=Q-7781
         why: Pricing policy PP-4; large quotes carry non-standard terms Deal Desk must review before the customer sees them.
  ok   QT-003   Discount floor at 35% — holds for quote=Q-7781
  ok   QT-004   Quote must belong to the opportunity — holds for quote=Q-7781, opportunity=OPP-5003
  ok   QT-005   Only draft quotes can be submitted — holds for quote=Q-7781
 warn  QT-002   Discount above 20% needs VP Sales — violated for quote=Q-7781
         why: Reps may discount to 20% on their own authority; deeper discounts are a VP Sales decision.
approval: Deal Desk above $250k; VP Sales above 20% discount
```

Exit code 2.

### 5. Blocked, with partners that would pass

Atlas Field Services is suspended. The engine lists the active partners whose agreement covers EMEA, and notes that a $520k deal would normally go to a gold partner.

```bash
./target/debug/lorefile check --dir examples/b2b-sales register_partner_deal opportunity=OPP-5006 partner=PTR-03
```

```
✗ register_partner_deal is blocked by PD-001 (block), PD-003 (warn).
 FAIL  PD-001   Partner must be active — violated for partner=PTR-03
         why: Suspended partners are in a compliance or payment dispute; no new registrations until Channel clears them.
  ok   PD-002   Partner must cover the deal's region — holds for opportunity=OPP-5006, partner=PTR-03
  ok   PD-004   One partner per deal — holds for opportunity=OPP-5006
  ok   PD-005   Register before Proposal — holds for opportunity=OPP-5006
 warn  PD-003   Large deals prefer gold partners — violated for opportunity=OPP-5006, partner=PTR-03
         why: Deals above $250k need a partner with enterprise delivery capacity; Channel reviews exceptions.
suggestions:
  partner → Rhine Industrial IT (PTR-02)
  partner → Vantage Consulting (PTR-05)
approval: Channel manager
```

Exit code 2.

## Browse it

```bash
./target/debug/lorefile serve --dir examples/b2b-sales --port 7075
```
