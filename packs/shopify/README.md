# Shopify ontology pack

A Shopify MCP server gives an agent orders, line items, customers and refunds. It does not say which of those orders are revenue, which day they count on, in which currency, that `source_name: pos` is a till and not a marketing channel, that `orders_count` is a lifetime number and cannot split new from returning customers in a period, or that a discount code on an order proves the code was typed and nothing more. This pack does, from the point of view of someone measuring marketing against store revenue.

## What it adds on top of the data MCP

- **Meaning for every object.** Shop, Order, LineItem, Product, Variant, Customer, Fulfillment, Refund, DiscountCode, each described the way Shopify actually behaves: money as strings in the shop currency, `processed_at` as the reporting date, `financial_status` for the money and `fulfillment_status` for the goods, `current_total_price` as what is left after refunds, personal data flagged.
- **The revenue rules Shopify's own reports apply.** Test orders are never revenue (block). Cancelled orders net to zero (block). Pending or authorized payments are booked, not collected (info). Multi-currency orders carry a shop-currency conversion (info). Back-dated or imported orders report at `processed_at` (info). Refunded orders are counted once, either net or with the refund on its own day (info). Gift card purchases are liabilities (warn).
- **Attribution semantics.** `source_name` is the sales channel that took the order, not the channel that earned it; the customer journey (first and last visit, UTMs, referrer) is where attribution lives; `customer_order_index` is how to tell a first order; a discount code is proof of use, not of channel, and unlimited codes leak within days.
- **15 rules and four actions.** Three writes (`cancel_order`, `refund_order`, `create_discount_code`) with Shopify's own preconditions (only unfulfilled orders cancel, only captured money refunds, within what is left on the order, percentages between 0 and 100) and approvers, and one read-out guard, `attribute_order`, to call before counting or crediting an order.
- **A glossary**: GMV, AOV, net sales, total sales, sales channel, new customer, SKU, return, coupon.

The pack holds no data and never calls Shopify. `where_is("Order")` names the tool and parameters on your Shopify MCP server and carries an equivalent Admin GraphQL query in `graphql`, for servers that expose a raw query tool. Tool names use the common `get_orders` shape; change `tool:` in each `types/*.yaml` if yours differ. Lines marked `# TODO verify` flag details the author was not certain of.

## Load both in Claude Desktop

```json
{
  "mcpServers": {
    "shopify": {
      "command": "npx",
      "args": ["-y", "shopify-mcp"],
      "env": { "SHOPIFY_ACCESS_TOKEN": "...", "MYSHOPIFY_DOMAIN": "your-store.myshopify.com" }
    },
    "shopify-ontology": {
      "command": "/absolute/path/to/lorefile",
      "args": ["mcp", "--dir", "/absolute/path/to/packs/shopify"]
    }
  }
}
```

If your Shopify server has another name, set `server:` in `sources/shopify.yaml` to match.

## Three questions, and what the agent does

**"What was revenue last week?"**

```
shopify-ontology  summary
shopify-ontology  describe("Order")        → revenue = non-test, non-cancelled orders at processed_at, shop currency, total_price
shopify-ontology  where_is("Order")        → get_orders(status=any, created_at_min=...), or the GraphQL in `graphql`
shopify           get_orders(status="any", created_at_min="2025-11-17T00:00:00Z", limit=250)
shopify-ontology  validate("attribute_order", {order: {...}}) for the doubtful ones
                  → SH-001 drops two test orders; SH-005 shows an imported order whose processed_at is in another week
```

**"How much did the ANNA10 creator code bring in?"**

```
shopify           get_discount_codes()                           → ANNA10: usage_limit null, usage_count 412
shopify-ontology  describe("DiscountCode")                        → usage_count includes cancelled orders; unlimited codes leak
shopify           get_orders(...) filtered to discount_codes containing ANNA10
shopify-ontology  validate("attribute_order", {order: {...}})     → SH-015 on each: proof of use, not channel; check last_visit_source
```

The agent reports orders carrying the code net of test and cancelled, and splits them by `last_visit_source` to show how many came through coupon sites rather than the creator's link.

**"Refund order #1042 in full and restock it."**

```
shopify           get_order(order_id="1042")                      → financial_status paid, fulfillment_status fulfilled, current_total_price 89.00
shopify-ontology  validate("refund_order", {order: {...}, refund: {amount: 89.0, restock: true, ...}})
                  → ok SH-009, SH-010; approval: Store manager
shopify-ontology  describe("Refund")                              → restock_type should be return, since the goods shipped
```

## Validate

```bash
./target/debug/lorefile validate --dir packs/shopify
```
