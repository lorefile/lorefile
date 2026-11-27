# Changelog

## 0.2.0 (not released yet)

> **Kinds of entities.** An entity can now be a kind of another (`extends: Sale`), with `where:` to pick the parent's rows. A space that uses them says `format: 2` in ontology.yaml, and Lorefile 0.1.0 refuses to load it ("unknown field `format`"). Upgrade every copy that reads the space first.

Also new since 0.1.0, each tested end to end and not yet tried live:

- **Kinds of entities** ([README](README.md#kinds-of-entities)): fields, links and guardrails come from the parent; `where` kinds read the parent's rows; `validate` takes `kinds`; RDF, SHACL and Ossie bring class hierarchies in as kinds (`--no-parents` keeps tags). New codes E019 to E024 and W118.
- **The same thing said twice**: W115 (same metric recipe), W116 (same rows, most fields shared, overlapping `where`), W117 (one word, two meanings). Saying a pair differs on purpose is kept under `distinct:` in `trust.yaml`. Three entities sharing a core get the rule-of-three advice, and `make_parent` acts on it.
- **OAuth sign-in** for ChatGPT's and Claude's connectors through any HTTPS tunnel.
- **`learn_file`**: a file a user gives an agent is learned, not stored.
- **The review screen**: what an agent proposes, by meaning, with a link from the chat.
- **Page fixes**: one set of words across the page and what agents say; decisions carry a name; Put your name on it.
- **Home and Map**: one picture of the space, with lights, reasons and waiting proposals.

## 0.1.0 (2026-11-27)

The first open-source release, under Apache-2.0: the format and its compiler, the web page, the MCP server over stdio and HTTP, spaces, metrics and functions, evidence grades, git history, Apache Ossie and OWL / SKOS / SHACL import and export.
