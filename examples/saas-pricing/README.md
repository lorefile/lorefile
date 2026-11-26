# Northstar Analytics: a subscription software company

Northstar sells a product-analytics platform on four plans (Free, Starter, Growth, Enterprise) plus one legacy plan nobody can move to any more. Seats, discounts, contracts and invoices each carry rules that billing, sales and support all need to agree on.

**Before.** A sales rep asked Claude: *"Marlow Studio wants Enterprise, can you upgrade SUB-3011?"* Claude did not know that Enterprise is only sold annually, needs 25 seats and a signed contract, or that Marlow has 8 seats on monthly billing and no paper. It would have said yes.

**After.** Claude calls `validate(upgrade_plan, {subscription: SUB-3011, from: PLAN-STARTER, to: PLAN-ENT})`, gets three blocking rules with their rationale, and the one plan that would pass today (Growth). The same ontology tells it a 40% partner discount is allowed but needs the CFO, that a trial account cannot be invoiced for extra seats, and that the invoice it is about to credit lives in the finance warehouse and has to be fetched first.

## What is in the folder

| | Count | Notes |
|---|---|---|
| Types | 10 | Account, Plan, Subscription, Seat, Invoice, Discount, Feature, Entitlement, Usage, Contract |
| Rules | 25 | 20 block, 3 warn, 2 info, in `constraints/plans` and `constraints/billing` |
| Actions | 5 | upgrade_plan, apply_discount, issue_credit, cancel_subscription, add_seats |
| Data | `data/*.csv` | 10 to 25 rows per bound type; Plan and Feature are inline reference lists |
| Sources | 3 | `billing_files` (csv, readable here), `stripe_bigquery` and `product_snowflake` (declared for agents) |

Invoice is bound to `stripe_bigquery`, which the binary cannot read, so `where_is` points the agent at the table and `validate` accepts the row inline. One format note: empty CSV cells load as `null`, so "the subscription has a contract" is written `contract_id != null && contract_id != ""`.

## Validate

```bash
./target/debug/lorefile validate --dir examples/saas-pricing
```

```
✓ valid · 10 types · 11 relationships · 25 rules · 5 actions · 3 sources · 0 errors · 0 warnings
```

## Five checks

### 1. Blocked, with the plan that would pass

SUB-3011 is 8 seats on Starter, billed monthly, with no contract. Enterprise fails three rules; Growth is the only other active plan that fits.

```bash
./target/debug/lorefile check --dir examples/saas-pricing upgrade_plan subscription=SUB-3011 from=PLAN-STARTER to=PLAN-ENT
```

```
✗ upgrade_plan is blocked by SP-001 (block), SP-003 (block), SP-008 (block).
 FAIL  SP-001   Enterprise plans need annual billing — violated for subscription=SUB-3011, to=PLAN-ENT
         why: Enterprise pricing assumes a one-year commitment; monthly enterprise is not a product we sell.
 FAIL  SP-003   Plan minimum seats — violated for subscription=SUB-3011, to=PLAN-ENT
         why: The minimum is part of the plan's price; fewer seats means a different plan.
 FAIL  SP-008   Enterprise plans need a signed contract — violated for subscription=SUB-3011, from=PLAN-STARTER, to=PLAN-ENT
         why: Enterprise terms (SLA, DPA, liability) only exist on signed paper; billing cannot invent them.
  ok   SP-002   Seats cannot exceed the plan cap — holds for subscription=SUB-3011, to=PLAN-ENT
  ok   SP-004   Legacy plans take no new subscriptions — holds for from=PLAN-STARTER
  ok   SP-004   Legacy plans take no new subscriptions — holds for to=PLAN-ENT
  ok   SP-005   Changing to the same plan is a no-op — holds for subscription=SUB-3011, from=PLAN-STARTER, to=PLAN-ENT
  ok   SP-007   Subscription must be live — holds for subscription=SUB-3011
  ok   SP-006   Downgrades take effect at period end — holds for subscription=SUB-3011, from=PLAN-STARTER, to=PLAN-ENT
suggestions:
  to → Growth (PLAN-GROWTH)
approval: Nobody for self-serve plans; Deal Desk for Enterprise
```

Exit code 2.

### 2. Allowed with a warning

A 40% strategic-partner discount on Atlas Freight's annual Growth subscription. Nothing blocks it, but Deal Desk's authority stops at 30%.

```bash
./target/debug/lorefile check --dir examples/saas-pricing apply_discount subscription=SUB-3004 discount=DSC-PARTNER40
```

```
⚠ apply_discount may proceed with warnings: DS-001 (warn).
  ok   DS-002   Only stackable discounts combine — holds for subscription=SUB-3004, discount=DSC-PARTNER40
  ok   DS-003   Combined discount capped at 50% — holds for subscription=SUB-3004, discount=DSC-PARTNER40
  ok   DS-004   Discount must be active — holds for discount=DSC-PARTNER40
  ok   DS-005   Discount must match the billing period — holds for subscription=SUB-3004, discount=DSC-PARTNER40
  ok   SP-007   Subscription must be live — holds for subscription=SUB-3004
 warn  DS-001   Discounts above 30% need the CFO — violated for subscription=SUB-3004, discount=DSC-PARTNER40
         why: Deal Desk may approve up to 30%; anything deeper is a CFO decision under the pricing policy.
approval: CFO above 30% off; nobody below
```

Exit code 0.

### 3. Pass

Copperline's contract expired in January, so cancelling at the end of the current period is a plain billing change. CN-003 is informational: cancellation only turns auto-renew off.

```bash
./target/debug/lorefile check --dir examples/saas-pricing cancel_subscription subscription=SUB-3007 contract=CT-507
```

```
✓ cancel_subscription may proceed: all 5 rules hold.
  ok   CN-001   No cancellation inside an active contract term — holds for subscription=SUB-3007, contract=CT-507
  ok   CN-002   Contract must govern the subscription — holds for subscription=SUB-3007, contract=CT-507
  ok   SP-007   Subscription must be live — holds for subscription=SUB-3007
  ok   CN-005   Large churn goes to the save desk first — holds for subscription=SUB-3007
  ok   CN-003   Cancellation takes effect at period end — holds for subscription=SUB-3007
approval: CSM save desk above $5,000 MRR
```

Exit code 0.

### 4. A record the binary cannot read

Crediting invoice INV-9021 needs the invoice. It lives in the finance warehouse, so the first attempt says what to fetch:

```bash
./target/debug/lorefile check --dir examples/saas-pricing issue_credit invoice=INV-9021 account=ACC-1004
```

```
✗ issue_credit is blocked by IN-002 (block), IN-003 (block), IN-004 (block), IN-005 (warn).
 FAIL  IN-002   Credits apply to open or paid invoices only — could not evaluate for invoice=INV-9021
         error: Undeclared reference to 'status'
         why: Draft invoices are edited instead; void and uncollectible invoices are already written off.
 FAIL  IN-003   Credit cannot exceed the invoice — could not evaluate for invoice=INV-9021
         error: Undeclared reference to 'credit_usd'
         why: Anything above the invoice is a refund, which goes through the payment provider with different approvals.
 FAIL  IN-004   Invoice must belong to the account — could not evaluate for invoice=INV-9021, account=ACC-1004
         error: No such key: account_id
         why: Prevents crediting the wrong customer when two accounts share a company name.
 warn  IN-005   Large credits need Finance — could not evaluate for invoice=INV-9021
         error: Undeclared reference to 'credit_usd'
         why: Support Leads may credit up to $5,000; above that Finance reviews the cause.
approval: Finance above $5,000; Support Lead below
note: `invoice` = INV-9021 could not be loaded (Invoice data is not readable here); fetch the row with your own connector and pass it inline, e.g. {"status": "draft", "amount_usd": 1.0}
```

Exit code 2.

```bash
./target/debug/lorefile where --dir examples/saas-pricing Invoice
```

```
{
  "type": "Invoice",
  "key": "invoice_id",
  "connection": "stripe_bigquery",
  "kind": "bigquery",
  "table": "finance.invoices",
  "key_column": "invoice_number",
  "columns": {
    "invoice_id": "invoice_number",
    "account_id": "customer_ref",
    "subscription_id": "subscription_ref",
    "amount_usd": "total_usd",
    "status": "invoice_status",
    "issued_on": "created_date",
    "due_on": "due_date",
    "credit_usd": "credit_note_usd"
  },
  "readable_here": false,
  "config": {
    "project": "${GCP_PROJECT}",
    "dataset": "finance",
    "auth": "env"
  },
  "hint": "Query finance.invoices with your own bigquery connector; the ontology only holds the binding."
}
```

With the row fetched and passed inline, the $600 credit is allowed at Support Lead level:

```bash
./target/debug/lorefile check --dir examples/saas-pricing issue_credit account=ACC-1004 \
  invoice='{"invoice_id":"INV-9021","account_id":"ACC-1004","subscription_id":"SUB-3004","amount_usd":3980.0,"status":"paid","issued_on":"2026-03-18","due_on":"2026-04-17","credit_usd":600.0}'
```

```
✓ issue_credit may proceed: all 4 rules hold.
  ok   IN-002   Credits apply to open or paid invoices only — holds for invoice=INV-9021
  ok   IN-003   Credit cannot exceed the invoice — holds for invoice=INV-9021
  ok   IN-004   Invoice must belong to the account — holds for invoice=INV-9021, account=ACC-1004
  ok   IN-005   Large credits need Finance — holds for invoice=INV-9021
approval: Finance above $5,000; Support Lead below
```

Exit code 0.

### 5. Blocked

Quill & Co is still on trial and already at Starter's 10-seat cap. Adding seats would bill an account with no payment method.

```bash
./target/debug/lorefile check --dir examples/saas-pricing add_seats subscription=SUB-3002 plan=PLAN-STARTER account=ACC-1002
```

```
✗ add_seats is blocked by AS-003 (block), IN-001 (block).
 FAIL  AS-003   Seat cap not yet reached — violated for subscription=SUB-3002, plan=PLAN-STARTER, account=ACC-1002
         why: Seats cannot exceed the plan cap; upgrade the plan first, then add seats.
 FAIL  IN-001   Trial accounts cannot be invoiced — violated for account=ACC-1002
         why: Trial accounts have no payment method on file; adding seats or anything else that bills needs the trial converted first.
  ok   AS-001   Subscription must belong to the account — holds for subscription=SUB-3002, plan=PLAN-STARTER, account=ACC-1002
  ok   AS-002   Plan must be the subscription's current plan — holds for subscription=SUB-3002, plan=PLAN-STARTER, account=ACC-1002
  ok   SP-007   Subscription must be live — holds for subscription=SUB-3002
approval: Nobody; Sales is notified when an account crosses 80% of its cap
```

Exit code 2.

## Browse it

```bash
./target/debug/lorefile serve --dir examples/saas-pricing --port 7074
```
