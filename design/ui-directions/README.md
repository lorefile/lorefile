# UI directions

Three clickable prototypes of the web page for marketers, built on the Meta Ads pack data. The live canvas is at https://claude.ai/artifact/GVswjEFUEmns18Kgv8W5Uk (private until shared).

| File | Direction | One line |
|---|---|---|
| `A-guided-tasks.dc.html` | Guided tasks | Start from what you want to do; one question per screen; plain-English verdict and approver at the end |
| `B-ask.dc.html` | Ask | Type the question; answers as cards; the right panel shows what was checked, mirroring the MCP flow |
| `C-map.dc.html` | Map | The business as a picture; click a thing for its meaning, guardrails, and actions |
| `E-guided-workspace.dc.html` | Guided workspace (all features, simplified) | Every feature of today's page, reorganised: Build steps (Sources, Things, Links, Guardrails, Actions) and Use pages (Map, Agents, Data & SQL). Fields, links, rules, data, YAML, try-it, MCP tools, SQL and the agent inbox all kept. Built on the real e-commerce example. |
| `D-build-the-map.dc.html` | Build the map (revised after feedback) | No chat. Five steps: where the data lives, what is there, what each thing is, how things link, guardrails. The map fills in as you go. An inbox shows what agents proposed over MCP for review. |

Directions D and E follow the product idea; E keeps all of today's features and is the candidate for the real page: the chat lives in Claude or ChatGPT, the UI puts the ontology in place and shows it growing.

These are design-canvas components, not the served UI. Once a direction is chosen, it is rebuilt as `src/ui/index.html` against the real API.
