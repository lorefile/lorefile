# RDF fixtures for `tests/rdf.rs`

| File | What it is | Licence |
|---|---|---|
| `schemaorg-slice.ttl` | schema.org release 30.1, cut down to Organization, Person, Order, Product, every property schema.org lists for them, and the DataType classes they range over. Each block is copied unchanged from <https://schema.org/version/latest/schemaorg-current-https.ttl> (fetched 2026-11-26). | Creative Commons Attribution-ShareAlike 3.0, by schema.org's sponsors (<https://schema.org/docs/terms.html>). The ShareAlike terms apply to this file, not to the rest of the repository. |
| `orders-shacl.ttl` | A small OWL + SHACL file written for these tests: a class with a key, three datatype properties and a link, a shape whose constraints become guardrails, and one SPARQL constraint that is listed for a person. | Apache-2.0, like the rest of the repository. |

Neither file is in the published crate (`Cargo.toml`'s `include` leaves `tests/` out).
