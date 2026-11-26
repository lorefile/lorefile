# Lumen Home: an online store

Lumen Home sells lighting, kitchen and outdoor goods online from three warehouses. Some of what it sells is hazardous in transport (lithium batteries, aerosols, camping gas), which is where most of the interesting rules come from.

**Before.** A support agent asked Claude: *"The customer wants her order shipped faster, can we move parcel SHP-3004 to the air carrier?"* Claude had no idea the parcel held a rechargeable lantern, that lithium batteries cannot fly, or which other carriers serve the east region. It either guessed or asked five questions.

**After.** Claude calls `validate(reroute_shipment, {shipment: SHP-3004, from: CAR-GRN, to: CAR-AIR})` and answers in one turn: blocked by SH-001 (hazardous goods cannot ship by air), with two ground carriers that would pass and the approval needed. The same ontology tells it that a $2,450 first order needs fraud review before a discount, that a refund goes back to the original card, and that the card itself lives in the payments warehouse and has to be fetched separately.

## What is in the folder

| | Count | Notes |
|---|---|---|
| Types | 12 | Customer, Order, OrderLine, Product, Variant, Inventory, Warehouse, Shipment, Carrier, Return, Promotion, PaymentMethod |
| Rules | 25 | 21 block, 3 warn, 1 info, in `constraints/orders` and `constraints/fulfilment` |
| Actions | 5 | refund_order, cancel_order, apply_promotion, reroute_shipment, restock |
| Data | `data/*.csv` | 10 to 22 rows per bound type; Warehouse and Carrier are inline reference lists |
| Sources | 3 | `shop_files` (csv, readable here), `payments_bigquery` and `shop_bigquery` (declared for agents) |

PaymentMethod is bound to `payments_bigquery`, which the binary cannot read, so `where_is` tells an agent where to fetch the row and `validate` accepts it inline. Two format notes: the refund action's return input is called `rma` because `return` is a reserved word in CEL, and the 30-day return window uses `timestamp()` and `duration()` on the ISO date strings.

## Validate

```bash
./target/debug/lorefile validate --dir examples/ecommerce
```

```
✓ valid · 12 types · 13 relationships · 25 rules · 5 actions · 3 sources · 0 errors · 0 warnings
```

## Five checks

### 1. Blocked, with alternatives that would pass

Parcel SHP-3004 contains a lithium-battery lantern. Moving it to SkyParcel (air) is blocked, and the engine lists the carriers that satisfy every rule for this parcel.

```bash
./target/debug/lorefile check --dir examples/ecommerce reroute_shipment shipment=SHP-3004 from=CAR-GRN to=CAR-AIR
```

```
✗ reroute_shipment is blocked by SH-001 (block).
 FAIL  SH-001   Hazardous goods cannot ship by air — violated for shipment=SHP-3004, to=CAR-AIR
         why: IATA dangerous-goods regulations and our carrier contracts forbid lithium batteries, aerosols and fuels on aircraft without a certified shipper, which we are not.

  ok   SH-002   Carrier must serve the destination — holds for shipment=SHP-3004, to=CAR-AIR
  ok   SH-003   Parcel within the carrier weight limit — holds for shipment=SHP-3004, to=CAR-AIR
  ok   SH-006   Carrier must be active — holds for from=CAR-GRN
  ok   SH-006   Carrier must be active — holds for to=CAR-AIR
  ok   SH-007   Only unshipped parcels can be rerouted — holds for shipment=SHP-3004
  ok   SH-008   Rerouting to the same carrier is a no-op — holds for shipment=SHP-3004, from=CAR-GRN, to=CAR-AIR
suggestions:
  to → SwiftPost (CAR-SWF)
  to → Regional Ground (CAR-RGL)
approval: Fulfilment Lead if the new carrier costs more than 25% extra
```

Exit code 2.

### 2. Allowed with a warning

ORD-1012 is a $2,450 first order. The promotion itself is fine; the order trips the fraud-review rule.

```bash
./target/debug/lorefile check --dir examples/ecommerce apply_promotion order=ORD-1012 promotion=PROMO-SPRING15
```

```
⚠ apply_promotion may proceed with warnings: EC-003 (warn).
  ok   PR-001   No promotion stacking above 40% off — holds for order=ORD-1012, promotion=PROMO-SPRING15
  ok   PR-002   Promotion must be live — holds for promotion=PROMO-SPRING15
  ok   PR-003   Promotion valid on the order date — holds for order=ORD-1012, promotion=PROMO-SPRING15
  ok   PR-005   First-order codes only on first orders — holds for order=ORD-1012, promotion=PROMO-SPRING15
  ok   PR-006   Promotions apply to unpaid orders only — holds for order=ORD-1012
 warn  EC-003   Large first orders need fraud review — violated for order=ORD-1012
         why: A first order above $2,000 is the classic card-testing and refund-fraud pattern. Risk reviews it before any discount or refund is applied.

  ok   PR-007   Promotion budget nearly used up — holds for promotion=PROMO-SPRING15
approval: Nobody; Marketing is notified when a code passes 90% of its budget
```

Exit code 0.

### 3. Pass

ORD-1003 has not shipped, so cancelling it needs nobody's approval.

```bash
./target/debug/lorefile check --dir examples/ecommerce cancel_order order=ORD-1003
```

```
✓ cancel_order may proceed: all 1 rules hold.
  ok   EC-001   Only unshipped orders can be cancelled — holds for order=ORD-1003
approval: Nobody below $1,000; Support Lead above
```

Exit code 0.

### 4. A record the binary cannot read

Refunding ORD-1007 needs the card it was paid with. PaymentMethod lives in the payments warehouse, so the first attempt tells the agent what to fetch:

```bash
./target/debug/lorefile check --dir examples/ecommerce refund_order order=ORD-1007 rma=RET-501 payment_method=PM-88213
```

```
✗ refund_order is blocked by PM-002 (block).
 FAIL  PM-002   Payment method must be active — could not evaluate for payment_method=PM-88213
         error: Undeclared reference to 'status'
         why: Refunds to expired or removed cards bounce; Support issues store credit or asks for a new card.
  ok   EC-004   Refunds need a paid order — holds for order=ORD-1007
  ok   PM-001   Refund to the original payment method — holds for order=ORD-1007, rma=RET-501, payment_method=PM-88213
  ok   RT-001   30-day return window — holds for rma=RET-501
  ok   RT-002   Return must be received before refunding — holds for rma=RET-501
  ok   RT-004   Return must belong to the order — holds for order=ORD-1007, rma=RET-501, payment_method=PM-88213
  ok   EC-003   Large first orders need fraud review — holds for order=ORD-1007
  ok   RT-003   Opened changed-mind returns get store credit — holds for rma=RET-501
approval: Support Lead above $500; Risk if the customer is fraud-flagged
note: `payment_method` = PM-88213 could not be loaded (PaymentMethod data is not readable here); fetch the row with your own connector and pass it inline, e.g. {"kind": "card", "status": "active"}
```

Exit code 2.

`where_is` says where that row is:

```bash
./target/debug/lorefile where --dir examples/ecommerce PaymentMethod
```

```
{
  "type": "PaymentMethod",
  "key": "payment_method_id",
  "connection": "payments_bigquery",
  "kind": "bigquery",
  "table": "payments.payment_methods",
  "key_column": "payment_method_token",
  "columns": {
    "payment_method_id": "payment_method_token",
    "customer_id": "customer_ref",
    "kind": "method_type",
    "brand": "card_brand",
    "last4": "card_last4",
    "expires": "card_exp_yyyymm",
    "status": "method_status"
  },
  "readable_here": false,
  "config": {
    "project": "${GCP_PROJECT}",
    "dataset": "payments",
    "auth": "env"
  },
  "hint": "Query payments.payment_methods with your own bigquery connector; the ontology only holds the binding."
}
```

The agent fetches the row with its own BigQuery connector and passes it inline:

```bash
./target/debug/lorefile check --dir examples/ecommerce refund_order order=ORD-1007 rma=RET-501 \
  payment_method='{"payment_method_id":"PM-88213","customer_id":"CUS-0107","kind":"card","brand":"visa","last4":"4242","expires":"2028-09","status":"active"}'
```

```
✓ refund_order may proceed: all 8 rules hold.
  ok   EC-004   Refunds need a paid order — holds for order=ORD-1007
  ok   PM-001   Refund to the original payment method — holds for order=ORD-1007, rma=RET-501, payment_method=PM-88213
  ok   PM-002   Payment method must be active — holds for payment_method=PM-88213
  ok   RT-001   30-day return window — holds for rma=RET-501
  ok   RT-002   Return must be received before refunding — holds for rma=RET-501
  ok   RT-004   Return must belong to the order — holds for order=ORD-1007, rma=RET-501, payment_method=PM-88213
  ok   EC-003   Large first orders need fraud review — holds for order=ORD-1007
  ok   RT-003   Opened changed-mind returns get store credit — holds for rma=RET-501
approval: Support Lead above $500; Risk if the customer is fraud-flagged
```

Exit code 0.

### 5. Blocked by the data itself

INV-2007 is a negative, hazardous stock record at a warehouse with no dangerous-goods licence. Nothing can be booked against it until the warehouse recounts.

```bash
./target/debug/lorefile check --dir examples/ecommerce restock inventory=INV-2007 warehouse=WH-SEA
```

```
✗ restock is blocked by INV-001 (block), INV-004 (block), INV-002 (warn).
 FAIL  INV-001  Inventory cannot go negative — violated for inventory=INV-2007
         why: A negative count means the system and the shelf disagree; the warehouse recounts before anything is booked against the record.
 FAIL  INV-004  Hazardous stock only in licensed warehouses — violated for inventory=INV-2007, warehouse=WH-SEA
         why: Fire code and the dangerous-goods storage licence; unlicensed sites may not hold lithium or aerosols.
  ok   WH-001   Warehouse must be active — holds for warehouse=WH-SEA
 warn  INV-002  Reserved cannot exceed on hand — violated for inventory=INV-2007
         why: We have promised more units than we hold (oversold); Support has to warn the affected customers.
approval: Nobody; Planning is notified when a record crosses its reorder point
```

Exit code 2.

## Browse it

```bash
./target/debug/lorefile serve --dir examples/ecommerce --port 7073
```
