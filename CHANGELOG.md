# Changelog

## 0.2.0 (not released yet)

> **Kinds of entities, people and area colours need format 2.** An entity can now be a kind of another (`extends: Sale`), with `where:` to pick the parent's rows; a space can now keep `people.yaml` and area colours. A space that uses any of these says `format: 2` in ontology.yaml, and Lorefile 0.1.0 refuses to load it ("unknown field `format`"). Upgrade every copy that reads the space first. Plain area changes (add, rename, move) write no new key.

Also new since 0.1.0, each tested end to end and not yet tried live:

- **Kinds of entities** ([README](README.md#kinds-of-entities)): fields, links and guardrails come from the parent; `where` kinds read the parent's rows; `validate` takes `kinds`; RDF, SHACL and Ossie bring class hierarchies in as kinds (`--no-parents` keeps tags). New codes E019 to E024 and W118.
- **The same thing said twice**: W115 (same metric recipe), W116 (same rows, most fields shared, overlapping `where`), W117 (one word, two meanings). Saying a pair differs on purpose is kept under `distinct:` in `trust.yaml`. Three entities sharing a core get the rule-of-three advice, and `make_parent` acts on it.
- **OAuth sign-in** for ChatGPT's and Claude's connectors through any HTTPS tunnel.
- **`learn_file`**: a file a user gives an agent is learned, not stored.
- **The review screen**: what an agent proposes, by meaning, with a link from the chat.
- **Page fixes**: one set of words across the page and what agents say; decisions carry a name; Put your name on it.
- **Home**: one picture of the space, with lights, reasons and waiting proposals, in three tabs: Overview, Graph and List. The flat Map is gone; `#map` links open the Graph.
- **The Graph** ([README](README.md#the-web-page)): one entity in the middle, links in and out each in their own labelled row, its logic above, its rows and family below; click a neighbour to move there.
- **People** ([README](README.md#people-and-areas)): owners are picked from a list that fills itself from owners, grades and history (names only). `set_owner` keeps a checked or accepted grade; rename and merge rewrite every spelling in one change and keep every grade. `people.yaml` for spellings, teams and who left. New codes W119 and W121.
- **Areas you can make**: add, rename, describe, reorder, move entities between and delete empty areas, from Home and from agents (`add_area`, `rename_area`, `describe_area`, `move_to_area`, `order_areas`, `delete_area`). Deleting an entity takes it off its area's list. New code W120.
- **Panels that fold** (the menu, the list column, the right panel; `[` and `]`) and a header that stays on one line.

## 0.1.0 (2026-11-27)

The first open-source release, under Apache-2.0: the format and its compiler, the web page, the MCP server over stdio and HTTP, spaces, metrics and functions, evidence grades, git history, Apache Ossie and OWL / SKOS / SHACL import and export.
