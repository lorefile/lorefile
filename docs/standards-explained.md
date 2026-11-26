# Ossie, OWL and friends, explained with one order

Written 2026-11-26 for Vinay, to explain Parts D and E of [the open-source release spec](../specs/2026-11-26-open-source-release.md), before any of it was built. **Updated the same day once both were built:** the commands below run, and the output is what the programs print, shortened. Where the build came out different from the first sketch, the text says so. The Lorefile files it starts from are real.

## The example we carry all the way through

Lumen Home, the fictional shop in `examples/ecommerce/`, already has everything we need. Here are the five pieces we follow, as they sit in the files today (shortened):

```yaml
# types/order.yaml
type: Order
description: One checkout. Status follows the parcel; payment_status follows the money.
properties:
  order_id:       { type: string, key: true, description: "Order number, e.g. ORD-1007" }
  status:         { type: enum, values: [placed, paid, packing, shipped, delivered, cancelled, returned] }
  payment_status: { type: enum, values: [pending, paid, refunded, partially_refunded] }
  total_usd:      { type: number, unit: USD, description: "What the customer was charged" }
relationships:
  placed_by: { to: Customer, cardinality: many-to-one,
               description: "An Order is placed by one Customer." }     # sentence added, see below
source: { connection: shop_files, table: orders, key: order_id }
owner: support@lumen.example

# glossary.yaml
terms:
  - term: basket
    means: Order
    description: The storefront calls an unpaid order a basket; the ERP calls it an order.

# logic/measurement.yaml
metrics:
  - metric: success
    description: The one number the business is run on, paid orders.
    source:
      connection: shop_bigquery
      table: orders
      filter: payment_status = 'paid' and status not in ('cancelled', 'returned')
      measure: count distinct order_id, by date(placed_on, 'Europe/London')
    grain: day
    basis: transactional        # counted from the shop's own records
    owner: growth@lumenhome

# constraints/orders/orders.yaml
constraints:
  - id: EC-001
    name: Only unshipped orders can be cancelled
    applies_to: Order
    rule: status in ["placed", "paid", "packing"]
    severity: block
    owner: support@lumen.example
```

Two notes. When this was first written the `placed_by` link had no sentence; the Ossie work added sentences to the shipped example's links, because both standards below want one for every link, so the file now says exactly this. And the guardrail's `rule` line is written in **CEL**, a small expression language, a bit like a spreadsheet formula, that Lorefile uses for every guardrail.

## 1. What is Ossie and why do we need it?

Every data tool keeps its own description of the business: Snowflake has its semantic views, dbt its semantic layer, Cube its data model, and catalogs such as Atlan, DataHub and Collibra their own. Moving a definition between two of them needs a converter for that pair, and there are dozens of pairs. **Apache Ossie** (formerly Open Semantic Interchange, started by Snowflake with dbt, Cube, AtScale, Atlan, DataHub and Collibra, now in the Apache incubator) replaces that with one shared file format. Each tool learns to read and write Ossie once, and then they can all swap definitions. NVIDIA's new Auto Ontology reads and writes it too. It is a draft (version 0.3.0.dev0), in YAML or JSON, Apache-2.0.

Ossie describes the data world. Its main file, the **semantic model**, lists **datasets** (a table, with a dotted address such as `sales.public.orders` and a key), **fields** (a column, with an SQL expression and a datatype such as String or Decimal), **metrics** (a number, written as SQL) and **relationships** (which columns join two datasets); almost anything can also carry an `ai_context`, a free-text note for AI tools. A second, smaller **ontology document** names the business concepts and gives each relationship a sentence, and a third, the **mapping document**, says which concept lives in which dataset.

What Ossie does not have: guardrails, actions, owners, caveats, a glossary, or data that lives behind an MCP instead of in a table. That is most of what makes Lorefile useful to an agent. So Lorefile uses Ossie's own escape hatch. Any level of an Ossie file may carry `nonportable_extensions`, a list of boxes shaped `{vendor_name, data}`, and every reader **must keep** these boxes even when it does not understand them. Lorefile puts everything that has no Ossie home into a box labelled `lorefile`, word for word. Other tools use the parts they understand; Lorefile gets everything back.

The Order entity and the `success` metric, before and after, as `lorefile export --ossie` writes them (shortened):

```
 LOREFILE (what you write)                   OSSIE SEMANTIC MODEL (what export --ossie writes)
 ──────────────────────────────────────────  ─────────────────────────────────────────────────────
 type: Order                                 datasets:
 description: One checkout…                    - name: Order
 source: shop_files · orders                     source: shop_files.orders
         key order_id                            primary_key: [order_id]
                                                 description: One checkout…
 glossary: basket means Order      ───────►      ai_context: { synonyms: [basket] }
 properties:                                     fields:
   order_id: string, key                           - name: order_id
                                                     expression: { sql: order_id }
                                                     datatype: String
   total_usd: number, USD                          - name: total_usd
                                                     expression: { sql: total_usd }
                                                     datatype: Decimal
 metric: success                             - name: shop_bigquery_orders     (a dataset for the table
   source: shop_bigquery · orders              source: shop_bigquery.orders      success reads)
   filter: paid, not cancelled/returned        metrics:
   measure: count distinct order_id              - name: success
                                                   expression:
                                                     sql: COUNT(DISTINCT order_id) FILTER (WHERE
                                                          payment_status = 'paid' and status not in
                                                          ('cancelled', 'returned'))
   grain: day, Europe/London                       nonportable_extensions: [lorefile: the recipe,
   owner, caveats, basis                             grain, owner, caveats, basis]
 owner of Order: support@…                   relationships:
 link placed_by → Customer                     - name: placed_by   from: Order   to: Customer
                                                 from_columns: [customer_id]   to_columns: [customer_id]
 guardrail EC-001                            nonportable_extensions:      (at the root)
 action cancel_order                           - vendor_name: lorefile
                                                 data: constraints: [EC-001, …]   actions: [cancel_order, …]
                                                       sources, functions, the glossary, file layout,
                                                       trust: the whole of trust.yaml, when there is one
```

Grades (who stands behind each definition, `trust.yaml`) travel the same way: the whole book rides in the root `lorefile` box. Ossie 0.3.0.dev0 has no field of its own for review status, so one more thing is added where any tool will see it: a metric or dataset whose definition an agent proposed and no person has reviewed gets `ai_context: { instructions: "Unverified: an agent proposed this definition and no person has reviewed it." }`, the note Ossie keeps for AI tools. The sentence is generated, so importing the file back drops it instead of turning it into a description.

Three details, each different from the first sketch. "basket" becomes an `ai_context` synonyms list, not a sentence. `success` reads the warehouse table (`shop_bigquery.orders`) while the Order entity points at the CSV sample, and Ossie wants a metric's SQL to name a dataset's fields, so the export declares a small dataset for that table and puts `success` on it, with the filter as `FILTER (WHERE …)`. And `placed_by` is a real Ossie join: Customer's key `customer_id` is also a field of Order, so the export joins on it (9 of the example's 13 links become joins this way); its sentence also goes into the ontology document as the pattern `An {Order} is placed by one {Customer}` (section 4). The `lorefile` box holds only what the portable parts cannot rebuild, plus whole objects Ossie has no word for, so an edit another tool makes to a description still wins on the way back.

## 2. What are RDF, IRI, Turtle, OWL, SKOS and SHACL?

These come from the W3C, the body behind HTML. They are older and broader than Ossie, and they describe the meaning world: what kinds of things exist, what they are called, and what must be true of them. Companies with a formal ontology speak them, and so do tools such as Protégé (an ontology editor) and GraphDB and Stardog (databases built for this kind of data).

One sentence each, in the order you need them:

- **RDF** is the grammar: every fact is a three-part sentence of subject, verb and object ("Order · is placed by · Customer"). Think of a spreadsheet that only ever has three columns.
- **An IRI** is a passport number for a concept, written like a web address (`https://schema.org/Order`). If Lorefile's Order and the Order in the company's old ontology carry the same IRI, every tool knows they are the same thing, whatever each one calls it. Nothing has to be served at that address.
- **Turtle** is a text format for writing RDF facts down compactly, the way CSV is a format for tables; **JSON-LD** is the same facts written as JSON.
- **OWL** describes kinds of things and their attributes: Order is a class, total_usd is a decimal attribute of Order, placed_by connects Order to Customer. Think of the category tree and attribute list behind a product feed.
- **SKOS** describes words: one preferred label and any number of alternative labels per concept. Think of a keyword list where "sofa" and "couch" land on the same product page.
- **SHACL** describes checks: "every Order's status must be one of these", with a severity (Violation, Warning or Info) and a message. Think of the ad review that rejects an ad for a missing field, or only warns.

Why several standards and not one? Each answers a different question, was written by a different working group at a different time, and has its own tools. They all share RDF and IRIs, so they mix freely in one file: like a passport, a driving licence and a CV, which are different documents about the same person, joined by name and number.

The same Order, its Customer link, the word "basket" and guardrail EC-001, as `lorefile export --rdf` writes them in Turtle (shortened; the `#` notes are mine, the program writes none):

```turtle
@prefix : <https://lorefile.dev/ns/lumen-ecommerce#> .        # the empty prefix: this space's IRIs
@prefix lorefile: <https://lorefile.dev/vocab#> .              # Lorefile's own words
@prefix owl:  <http://www.w3.org/2002/07/owl#> .               # short names for the standards themselves
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix sh:   <http://www.w3.org/ns/shacl#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .

:Order a owl:Class ;                                  # OWL: Order is a kind of thing
    rdfs:label "Order" ;
    skos:prefLabel "Order" ;                          # SKOS: its main name
    skos:altLabel "basket" ;                          # SKOS: the glossary, "basket" means Order
    rdfs:comment """One checkout. Status follows the parcel …""" ;   # its description
    lorefile:owner "support@lumen.example" ;          # who owns it (no standard home, so Lorefile's word)
    lorefile:file "types/order.yaml" .                # where it came from, so it goes back there

:Order.total_usd a owl:DatatypeProperty ;             # OWL: an attribute that holds a value
    rdfs:label "total_usd" ;
    rdfs:comment "What the customer was charged" ;
    rdfs:domain :Order ;                              # … which Orders have
    rdfs:range xsd:decimal .                          # … and which is a decimal number

:Order.placed_by a owl:ObjectProperty , owl:FunctionalProperty ;   # a link, at most one per Order
    rdfs:label "placed_by" ;
    rdfs:comment "An Order is placed by one Customer." ;           # the link's sentence
    rdfs:domain :Order ;                              # from Order
    rdfs:range :Customer .                            # to Customer

:EC-001 a sh:NodeShape ;                              # SHACL: a check
    lorefile:id "EC-001" ;
    sh:message "Only unshipped orders can be cancelled" ;   # the guardrail in words
    sh:severity sh:Violation ;                        # "block" in Lorefile is a Violation in SHACL
    sh:targetClass :Order ;                           # … about Orders
    sh:deactivated true ;                             # … switched off for SHACL tools (section 3)
    lorefile:action :cancel_order ;                   # … because it is checked before cancel_order
    sh:property [
        sh:path :Order.status ;                       # … looking at status
        sh:in ( "placed" "paid" "packing" )           # … which must be one of these three
    ] ;
    lorefile:cel 'status in ["placed", "paid", "packing"]' ;   # the original CEL rule, word for word
    lorefile:owner "support@lumen.example" .          # who owns the rule
```

Two differences from the first sketch. A field's IRI carries its entity (`:Order.total_usd`, not `:total_usd`), because two entities' `status` rarely mean the same thing and OWL would merge them; to say they do, give both the same `iri:`. And each entity also gets one SHACL shape saying how it uses its fields and links (name, order, datatype, required, allowed values, at most one), so that facts which belong to one entity are not stamped on a shared property.

When the space has grades, the graded thing says so and a small record says who, when and against what, in PROV-O, the W3C's vocabulary for provenance (here `success` after Priya checked it against the warehouse):

```turtle
@prefix prov: <http://www.w3.org/ns/prov#> .                  # provenance: who did what, when, using what

:success a lorefile:Metric ;
    lorefile:trust lorefile:Checked ;                         # the grade, on the thing itself
    lorefile:definition "{ … \"basis\":\"transactional\" … }"^^rdf:JSON .   # basis travels with the metric

:trust.metrics.success a lorefile:Grading , prov:Activity ;  # the record behind the grade
    lorefile:about :success ;
    lorefile:grade lorefile:Checked ;
    prov:wasAssociatedWith :agent.priya-via-page ;            # a prov:Person, labelled "Priya via page"
    prov:endedAtTime "2026-11-28T00:00:00Z"^^xsd:dateTime ;
    prov:used :success , :source.shop_bigquery ;              # what was run, and against which source
    rdfs:comment "412 paid orders for 16–22 Nov, the same as Shopify's own report" ;
    lorefile:of "3f9a1c2e70b4" .                              # the fingerprint of what was graded
```

A data binding's grade sits on the entity as `lorefile:bindingTrust`, with `lorefile:aspect "binding"` on its record; an agent's proposal is a `prov:SoftwareAgent` and the proposed thing also gets `prov:wasAttributedTo` it. Coming back in, grades start ungraded whatever the file says (a grade is this team's statement about this copy), the report counts what was claimed, and `--keep-grades` restores them for a team's own export where each still matches. PROV statements another tool wrote are counted and never read as grades.

EC-001 has a real SHACL form because "one of these values" is something SHACL can say natively. Section 3 shows a guardrail that has none, and why this one is switched off.

## 3. How export and import work, step by step

**Scenario A: a company brings what it already has.** Acme, a retailer, has an OWL file its data architects wrote years ago and a semantic model from Snowflake saved as Ossie. Agents cannot use either today. Lorefile's import reads each file, maps each piece using the spec's tables, compiles the result with the strict compiler, and writes nothing unless it compiles. Then it prints a report of what it guessed or dropped. The team works through the to-do list on the page, connects their agents, and the agents call `summary` that afternoon. For very large ontologies such as schema.org, `--only` names the classes to take and `--depth 1` adds the ones they link to.

```
 BEFORE                                  AFTER  (an illustration; the report lines are the real format)
 ┌────────────────────────────┐          $ lorefile import --from-rdf acme-business.owl --into spaces/acme
 │ acme-business.owl          │          $ lorefile import --from-ossie sales.semantic_model.yaml \
 │ sales.semantic_model.yaml  │              --into spaces/acme --report
 │ readable by Protégé and    │
 │ Snowflake; agents can't    │          spaces/acme/
 │ use them                   │            types/          14 entities with fields and descriptions
 └────────────────────────────┘            relationships/  11 links, sentence from the comment where there is one
 Lorefile: a blank space                   glossary.yaml   23 words, from alternative labels and ai_context
                                           constraints/     4 guardrails, from SHACL shapes
                                           sources/         1 source, kind sql (to set by hand)
                                           logic/           6 metrics, SQL kept as written

                                           report (--report)
                                             guessed: SALES_DB names no system Lorefile knows,
                                                      so the source is kind: sql; say which it is
                                             guessed: Order is a kind of Transaction
                                                      (rdfs:subClassOf): kept as the tag
                                                      subclass-of:Transaction
                                             kept:    2 SHACL constraints have no CEL form, so
                                                      they are listed here for a person
                                             dropped: 9 OWL statements used only for reasoning
                                             to do:   11 entities have no owner, 6 links no sentence
```

**Scenario B: a team takes its work out.** The Lumen team has built its space over months. It runs `lorefile export --ossie` for NVIDIA Auto Ontology or dbt, and `lorefile export --rdf --format turtle` for GraphDB or Protégé. Each tool reads what it understands and carries the rest. Importing either file back gives the same space, file for file: `tests/ossie.rs` and `tests/rdf.rs` prove it for every shipped example and pack by comparing the summary, the counts, `describe` of everything, and a second export. Python's rdflib reads the Turtle too, and pyshacl's check of the shapes themselves passes for every example.

```
 MONTHS OF WORK                 export --ossie                    export --rdf --format turtle
 spaces/lumen/                  ──────────────────────────────    ─────────────────────────────────
   12 entities, 13 links        lumen.semantic_model.yaml         lumen.ttl
   25 guardrails, 5 actions     lumen.ontology.yaml
   2 metrics, 10 words          lumen.mapping.yaml
   5 sources
                                NVIDIA Auto Ontology, dbt         GraphDB, Protégé
                                SEE     datasets, fields,         SEE     classes, attributes, links
                                        metrics as SQL, joins,            with sentences, labels and
                                        ai_context hints                  alternative labels, simple checks
                                IGNORE  the lorefile box          IGNORE  lorefile: lines (actions,
                                        (but must keep it)                metrics, sources, CEL rules)
                                                                          yet store them like any fact

             re-import either file  ──►  the same space: entities, fields, metrics, links, words, guardrails
```

What is lossy, said plainly:

- **Lorefile to Ossie.** Ossie has no glossary, so "basket" becomes a synonym in the dataset's `ai_context`: a hint another tool's AI may read, not a word it can look up. Guardrails, actions, owners, caveats, the metric's daily grain and metrics fetched from an MCP (such as `paid_performance`, spend from the Meta and Google Ads MCPs) ride in the `lorefile` box: carried, not used. An entity with no table becomes a concept with no dataset.
- **Lorefile to SHACL.** Only simple rules become real SHACL checks: allowed values, required, a number range, a fixed value. EC-003, "a first order above $2,000 needs fraud review" (`total_usd > 2000.0 implies is_first_order == false`), stays as a `lorefile:cel` note, marked `lorefile:celOnly`, that a SHACL tool will not run. In the ecommerce example 10 of the 25 guardrails get a native form.
- **One mismatch, settled.** A Lorefile guardrail is checked before an action: EC-001 runs when an agent wants to cancel an order. SHACL checks every row at any time, so a SHACL tool running EC-001 over all orders would flag every delivered one. So a guardrail that belongs to an action (it applies to the action, or the action lists it) is exported switched off for SHACL (`sh:deactivated true`), with `lorefile:action` naming the action. Tried with pyshacl on two sample orders: the delivered one passes, and the one with a status outside the field's allowed values is flagged by the entity's own shape.
- **Coming in.** An OWL hierarchy ("Order is a kind of Transaction") becomes the tag `subclass-of:Transaction` and a note, because Lorefile has no sub-kinds yet. Statements that exist only for reasoning engines are counted and dropped. SHACL property constraints (`sh:minCount`, `sh:in`, ranges, lengths, patterns) become guardrails in CEL that `validate` enforces; SHACL written as database queries (`sh:sparql`), or with `sh:or` and friends, is listed for a person. From Ossie, the source kind is guessed from the dotted prefix (bigquery, snowflake, postgres when the prefix says so, else `kind: sql` with a note), and the portable SQL is kept as written; a dialect form is used only when there is no portable one, with a note.
- **The round trip.** Lorefile to file to Lorefile is lossless, and the tests check it for every shipped folder in both formats. Through another tool it stays lossless only if that tool keeps what it was told to keep; if a tool throws the `lorefile` box (or the `lorefile:` statements) away, what comes back is only what that tool understood.

## 4. Why both, and how they line up

Ossie is for the data world: tables, columns and metrics as SQL, where Snowflake, dbt, Cube and NVIDIA work. OWL, SKOS and SHACL are for the meaning world: concepts, words and rules, where enterprise architects, Protégé and graph databases work. A larger company often has both, kept by different teams. Lorefile sits between them and holds what neither has: actions, owners, sources behind MCPs, and guardrails written for agents.

They join through the IRI. Ossie's ontology document already has an `iri` field on each concept (and ships an example that imports an OWL vocabulary), so Order's passport number appears in all three places. Built: an optional `iri:` key on every entity, field, link, word, guardrail and action, which defaults to the space's address plus the name (`namespace:` in `ontology.yaml` changes the address, `prefixes:` adds short names). Write `iri: schema:Order` on the Order entity instead, and both exports say "this is schema.org's Order"; importing either brings the `iri:` back. One IRI on two things is a compile error (E016), so two entities cannot both claim to be schema.org's Order.

```
                       one passport number: https://lorefile.dev/ns/lumen-ecommerce#Order
                                                   │
          ┌────────────────────────────────────────┼─────────────────────────────────────┐
          ▼                                        ▼                                     ▼
  DATA WORLD · Ossie                    LOREFILE · YAML in git                 MEANING WORLD · W3C
  semantic model                        (the source of truth)                  OWL    :Order, a class
    dataset Order, shop_files.orders      types/order.yaml                            placed_by → Customer
    metric success, as SQL                glossary: basket                     SKOS   altLabel "basket"
  ontology document                       logic: success                       SHACL  shape EC-001
    concept Order, iri ◄──────────────    constraints: EC-001   ──────────────► same IRI on the class
    placed_by: "An Order is placed        actions: cancel_order                (actions, metrics,
     by one Customer."                    sources, owners                       sources: lorefile: lines)
  mapping: Order ⇄ dataset Order
  read by Snowflake, dbt, Cube,                                                read by Protégé, GraphDB,
  NVIDIA Auto Ontology, Atlan,                                                 Stardog, TopQuadrant,
  DataHub, Collibra                                                            LinkML
```

The gap this section first pointed out is closed: the Ossie export writes each concept's `iri`, and it is the same resolved IRI the RDF export gives the OWL class and the page shows under the entity's description.

## 5. So what?

What you get at the end:

- **An open, plain-files home that any of these tools can feed or read.** A Snowflake model, a dbt project, an old OWL file or a slice of schema.org can each become a starting space, and the space can go back out to all of them.
- **No team starts from a blank page.** They import what exists, then add what agents need and those formats lack: the success metric, guardrails, actions, owners.
- **Nothing is locked in.** The YAML in git stays the source of truth, and an export to the two formats the industry is converging on is one command away.
- **A clear position.** NVIDIA, Databricks, Snowflake and Microsoft each shipped an agent ontology this autumn, tied to their own platform. Lorefile is the neutral keeper that speaks their file formats.

What this does not do:

- **No reasoning engine.** Lorefile will not work out new facts from OWL statements.
- **No graph database.** No SPARQL, no triple store; the Turtle file is just a file.
- **No change to how agents use Lorefile day to day.** They keep calling `summary`, `describe` and `validate` over MCP. Import and export are for people moving definitions in and out.
- **No live sync.** An export is a file written once, not a connection that keeps two tools in step.
- **No rows and no credentials.** An export holds table names and SQL text, never data or logins.

## 6. The build order and why

```
 1 rename (Part A)          2 login + MCP over HTTP (B) ─┐
   done                ──►                               ├──►  3 Ossie (D)  ──►  4 IRIs + RDF bridge (E)
   touched every file       2 npx, Docker, cargo (C)  ───┘       done              + the final README
                              both done,                                             and tutorial pass
                              no files shared                                        done
```

All four were built on 2026-11-26, in this order:

- **Rename first, alone,** because it touched nearly every file; anything built before it would have had to be renamed again.
- **Login and distribution in parallel,** because they share no files: one changes the server code, the other adds the npm launcher, the release workflow and the Docker image.
- **Ossie next,** because it edits the same server files as login (`main.rs`, `serve.rs`, the page) and so waited for login to finish rather than clash with it. It is smaller than the RDF work and needed no change to the YAML format.
- **The RDF bridge last,** because it changed the format itself (the new `namespace:`, `prefixes:` and `iri:` keys touch the compiler), added new libraries (Oxigraph's RDF crates), and followed the export and import pattern Ossie set.
- **The README once, at the end.** Sessions that run at the same time never edit the same files, and only the rename and the last session touch README.md, so the README is written once to describe what was actually built: install, login, Ossie and the standards.
