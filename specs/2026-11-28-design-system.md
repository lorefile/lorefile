# Spec: the design system (Ledger, with a jump search in the header)

Written 2026-11-28 for Vinay to decide on. **Status: approved 2026-11-28; built 2026-11-28 (all five steps of Part 11, committed on branch `claude/zen-robinson-0bd6f5`, not merged). See "As built" at the end.** Vinay approved it as written, so every decision below stands as recommended. This is the rulebook every screen of the web page follows from now on: the named values ("tokens"), the shared patterns, and the component rules. The fixes in the companion spec, [2026-11-28-ui-audit.md](2026-11-28-ui-audit.md), are written in these terms.

**The direction, as Vinay chose it** (2026-11-28, in the session that started this work): "lets do mix of A ledger as main design and bring the search box from B workbench in the header and adapt to A ledger design."
- **Ledger** is the base: the page reads like the business's record book, and every statement carries a visible mark of who stands behind it.
- **From Workbench**, only the header search: one "Find or jump to…" box, Ctrl/Cmd-K from anywhere, a jump list that mixes entities, guardrails, actions and commands. Redrawn in Ledger's look.
- **Not taken** (decision 6 below): Review desk's before/after seam and "needs you" queue, which the audit had suggested borrowing.

The mockups are on the canvas "Lorefile layout directions" (https://claude.ai/artifact/BW7twMi9hfCNGxzgCybdhc): row A (Ledger) for the shell, the entity page and the Graph; boards A2 for the header search.

**Base:** `main` at `643eb56`. One file: `src/ui/index.html`.

**Standing rules, unchanged:**
- Light theme only.
- The page ships inside the binary and works offline: no CDN fonts or scripts at run time.
- Plain English; honest status.
- The page never executes anything and holds no credentials.

---

## Before and after, in one screen

**Today**, on Order's page: a 16 px-tall box holds the description, so you see the top of one line and drag it open. Field sentences sit in one-line boxes in the last column of a table, cut off at "Customer who placed it; see". Grades are small grey pills. The menu's "« Fold" is below the bottom of the window. Everything is system font and one stock blue.

**With this system:** Order's description reads as a paragraph in a serif, with a small pencil. Click it and it turns into a box exactly as tall as the text, with Save and Cancel. Each field is a line: `discount_pct` in monospace, "number" in grey, and its sentence underneath, whole. In the left margin of each line sits a stamp: a dashed "UNGRADED" outline, an amber "NOT DESCRIBED YET", or a blue "ACCEPTED · Priya · 28 Nov". Every panel has the same head: its name in small capitals and one fold button in the top corner. The header has one search box; typing "refund" lists the Refund order action, guardrail EC-004, the Order entity and the payment_status field, and Enter opens the first.

---

## Decisions

**Decided 2026-11-28: every one as recommended (in bold)**, when Vinay approved the spec as written.

| # | Question | Options | Recommendation |
|---|---|---|---|
| 1 | Fonts | **A. System fonts: the computer's serif for reading text, its sans for controls, its monospace for names (0 KB).** B. Embed Source Serif 4 (regular and semibold, Latin only, est. 50–70 KB in the binary, unmeasured), system sans and mono. C. Embed all three faces the mockups use (est. 150–200 KB) | **A** first. The mockups use Source Serif 4, Inter and JetBrains Mono from Google Fonts only so they look the same on every screen. On a Mac the system serif is New York, close to the mockup; on Windows it is Palatino Linotype or Georgia. If that look disappoints, B is one step later and changes nothing else. |
| 2 | Dark mode | **A. Light only, as today.** B. Follow the computer's setting, once every colour is a token | **A.** The token block below has one place for a dark set; nothing in this spec stops it later. |
| 3 | The amber number on menu rows (audit UI-29) | **A. The real count of things needing attention (12, 2).** B. A plain dot | **A.** It is what the Ledger boards show. |
| 4 | How sentences are edited | **A. Read as text; click the sentence or its pencil to edit in place (Ledger).** B. Always show boxes, as today | **A** for descriptions and field sentences, with the safeguards in Part 8 so nobody misses that they can edit. Forms that create something (+ New entity, a guardrail) stay as boxes. |
| 5 | Where grades show | **A. Stamps beside every graded statement: description, each field, each metric, each guardrail, the Notes card.** B. Only on the Notes card | **A.** That is the signature: you can scan a page and see what is and isn't verified. |
| 6 | Review desk's before/after seam and "needs you" queue | A. Add them now. **B. Leave them out; the review screen keeps today's change cards.** | **B**, because your pick named only the search. The seam would fit Ledger well later (Part 9 leaves room for it). |
| 7 | Field results in the jump list | **A. The server adds a `field` hit kind for the page only (`/api/search?fields=1`); MCP's `search` tool is unchanged.** B. Also for agents. C. No field results | **A.** Typing a column name is common, and today a field only raises its entity. Agents keep their current output until that is decided on its own. |
| 8 | Keys for the search | **A. Ctrl/Cmd-K and `/` both focus it.** B. Ctrl/Cmd-K only | **A.** `/` already focuses Find on Home today, so nobody loses a habit. |

---

## Part 1 · Five rules

1. **Read first, edit second.** A statement is shown as text people read. Editing is one click away, always in place, never in a second screen.
2. **Every statement says who stands behind it.** Graded things carry a stamp. Ungraded things say so, as a dashed outline, never as silence.
3. **One way to do one thing.** One fold pattern, one search, one button set, one tag, one tab style, one selected look. A new screen reuses them; it does not invent.
4. **Named values only.** Colours, sizes, spaces, corners and shadows come from the token block. A raw hex value or pixel size in a rule is a bug (Part 11 has a test for it).
5. **Nothing moves by accident.** Rows that hold controls do not wrap. When there is less room, things leave in a fixed order, into a fixed "⋯" menu.

---

## Part 2 · Tokens

One block at the top of the `<style>`, replacing today's `:root` (line 9). Everything else uses `var(--…)`.

```css
:root{
  /* ---- colour: ground and ink ---- */
  --paper:#FAF7F0;          /* page background */
  --card:#FFFDF8;           /* panels, inputs, the reading column's raised parts */
  --ink:#1F1B16;            /* text: 16.0:1 on paper */
  --ink-2:#5C544A;          /* second text, labels, hints: 6.95:1 on paper, 7.32:1 on card */
  --rule:#E4DCCB;           /* hairlines between things (decorative, not for controls) */
  --rule-strong:#8F8573;    /* borders of anything you type in or choose from: 3.40:1 on paper */
  --tint:#F3EEE3;           /* hover on rows */

  /* ---- colour: the one accent ---- */
  --accent:#8C2F1B;         /* oxblood: primary buttons, links, selection, focus. 7.73:1 on paper; white on it 8.28:1 */
  --accent-press:#6E2414;   /* pressed and hover of primary */
  --accent-soft:#F6E7E1;    /* "you are here" background. Accent text on it 6.87:1 */

  /* ---- colour: status (each also differs in lightness or carries a glyph, never hue alone) ---- */
  --ok:#2F6B3A;             /* checked, valid, a person stands behind it: 5.97:1 */
  --warn:#8A5200;           /* look at it, not described, needs words: 5.97:1; 5.40:1 on --warn-soft */
  --warn-soft:#F5EBD6;
  --stop:#A3231A;           /* wrong, error, a guardrail that stops: 6.98:1 */
  --stop-soft:#F8E4E0;
  --accepted:#2B4C7E;       /* the "accepted" stamp only: 8.05:1 */
  --agent:#5C544A;          /* the "agent wrote · not reviewed" stamp: pencil grey (= --ink-2) */

  /* ---- colour: areas (the drawing, area labels; never used for status) ---- */
  --area-1:#3F4A9E;  /* indigo   7.31:1 */
  --area-2:#1F6F68;  /* teal     5.56:1 */
  --area-3:#7B3B7E;  /* plum     7.09:1 */
  --area-4:#586B1F;  /* olive    5.55:1 */
  --area-5:#4A5563;  /* slate    7.08:1 */
  --area-6:#1F628F;  /* sky      6.13:1 */
  --area-7:#8E3A5E;  /* mulberry 6.73:1 */
  --area-8:#6B5434;  /* umber    6.67:1 */

  /* ---- colour: the drawing (Overview and Graph) ---- */
  --draw-ground:#F3EEE3; --draw-tile:#FFFDF8; --draw-edge:#8F8573; --draw-side-l:#ECE5D6; --draw-side-r:#E2D9C6;
  --draw-line:#B3A890; --draw-line-soft:#D6CCB6;

  /* ---- type ---- */
  --serif:ui-serif,"New York","Iowan Old Style","Palatino Linotype",Palatino,Georgia,serif;
  --sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Inter,Roboto,sans-serif;
  --mono:ui-monospace,SFMono-Regular,"JetBrains Mono",Menlo,Consolas,monospace;
  --fs-micro:11px;   /* small capitals: panel names, group labels, stamps */
  --fs-small:12px;   /* hints, tags, captions */
  --fs-ui:13px;      /* controls, menus, lists, tables */
  --fs-read:15px;    /* sentences people wrote: descriptions, field sentences, card text */
  --fs-title:20px;   /* card and section titles (serif) */
  --fs-h1:30px;      /* the page title (serif) */
  --lh-ui:1.4; --lh-read:1.6;
  --fw-normal:400; --fw-mid:500; --fw-strong:600;
  --caps-track:.08em;  /* letter-spacing of small capitals */

  /* ---- space: seven steps ---- */
  --sp-1:2px; --sp-2:4px; --sp-3:8px; --sp-4:12px; --sp-5:16px; --sp-6:24px; --sp-7:32px;
  --page-pad:24px;   /* 16px under 720 (see breakpoints) */

  /* ---- shape ---- */
  --r-stamp:3px; --r-control:4px; --r-card:6px; --r-round:999px;  /* round: count badges only */
  --hair:1px; --stamp-line:1.5px;

  /* ---- sizes ---- */
  --h-control:36px; --h-compact:30px; --h-touch:44px;
  --panel-head:36px; --rail:44px; --rail-menu:56px;
  --w-menu:220px; --w-index:200px; --w-notes:300px;
  --measure:720px;   /* the reading column never grows wider */

  /* ---- depth: two shadows ---- */
  --shadow-pop:0 8px 24px rgba(31,27,22,.12);    /* menus, the jump list, pickers, toasts */
  --shadow-modal:0 18px 50px rgba(31,27,22,.20);  /* dialogs, phone sheets */

  /* ---- layers ---- */
  --z-sticky:10; --z-pop:20; --z-drawer:30; --z-modal:40; --z-toast:50;

  /* ---- motion ---- */
  --ease:150ms ease;   /* none under prefers-reduced-motion */
}
```

**Breakpoints.** CSS can't put a variable inside `@media`, so these four numbers are written as constants, in one place in the CSS and once in JS (`TIERS` beside a `tier()` helper that replaces `phone()`):

| Tier | Width | Name in code |
|---|---|---|
| Phone | under 720 px | `phone` |
| Tablet | 720–1023 px | `tablet` |
| Laptop | 1024–1279 px | `laptop` |
| Desktop | 1280 px and up | `desktop` |

Today's 760, 1000, 1100 and 1280 go away. No other width may appear in a media query.

---

## Part 3 · Type

| Use | Family | Size / line | Weight |
|---|---|---|---|
| Page title ("Order") | serif | `--fs-h1` / 36px | 600 |
| Section and card titles ("About Order", "47 definitions are ungraded") | serif | `--fs-title` / 26px | 600 |
| Sentences people wrote: descriptions, field sentences, rule text, card body | serif | `--fs-read` / `--lh-read` | 400 |
| Controls, menus, lists, tables, labels | sans | `--fs-ui` / `--lh-ui` | 500 (labels 600) |
| Hints, captions, tags | sans | `--fs-small` | 400 / 600 |
| Small capitals: panel names, group labels, stamps | sans | `--fs-micro`, uppercase, `--caps-track` | 600 |
| Field names, ids, file paths, YAML | mono | `--fs-ui` (12px inside tables) | 400 / 600 |

Rules:
- Neighbouring levels differ by at least 2 px or by weight, never by half a pixel.
- Serif is for what people *wrote*; sans is for what the page *says*. A hint ("Every field needs plain words…") is sans; the field's sentence is serif.
- 800 is retired. Italic only for "Not described yet. What does placed_on mean?" and for "example" notes.
- No text below 11 px anywhere, the drawing included (UI-43).

---

## Part 4 · Colour

**One accent.** Oxblood is for: primary buttons, links, the selected item, focus rings, the active tab underline. Nothing else is oxblood. It is not an area colour and not a status colour.

**Status** has four colours and each always comes with a glyph or a word, so nobody needs to tell red from green:

| Meaning | Colour | Glyph | Words used |
|---|---|---|---|
| Wrong: agents may get wrong answers | `--stop` | ✕ in a circle (drawn) | "wrong", "stop" |
| Look at it: a person should check | `--warn` | ! in a circle | "look at it", "not described yet", "needs words" |
| A person stands behind it | `--ok` | check in a circle | "checked", "valid" |
| Nothing to fix, nobody graded it | `--ink-2`, dashed | empty dashed circle | "ungraded" |

**Areas** use `--area-1` … `--area-8` in order of creation (today's `AREA_COLORS`, line 414, becomes these tokens). An area colour appears only as a tile's top edge and outline, a small square swatch, and the area's small-capitals label. It never fills a tile, and never marks status.

**Hairlines over boxes.** Sections are separated by `--rule` lines and space, not by bordered cards. A card (border `--rule`, background `--card`, corner `--r-card`) is used only for something you can pick up or act on as one thing: a "What to do next" item, a result, a dialog.

Retired: `--faint` (3.25:1, UI-40), the second blue `--info`, `--line2`, and the 37 raw colours listed in the audit (B3).

---

## Part 5 · Layout and the four tiers

**The shell** (desktop): header 48 px · Contents (the menu, `--w-menu`) · Index (the list column, `--w-index`, on screens that have one) · the reading column (centred, at most `--measure` wide, `--page-pad` around it) · Notes (`--w-notes`) · footer 32 px.

| Tier | Contents | Index | Notes |
|---|---|---|---|
| Desktop ≥1280 | open | open | open, beside the reading column |
| Laptop 1024–1279 | rail (icons) | open | open; becomes a rail if the reading column would drop under 560 px |
| Tablet 720–1023 | rail | rail; opens **over** the page as a drawer | closed; opens over the page as a drawer from the right |
| Phone <720 | "Menu" button in the header opens it as a full-height drawer | its own screen: list → tap → item, with "‹ Entities" to go back | a fold-out "About this entity" under the title; on Home and the Graph, a bottom sheet |

- A **drawer** slides over the page instead of making it narrower. It closes with its own panel head button, Esc, or a tap outside.
- A person's choice for a panel is kept per panel, per browser (`lorefile-panels`, as today). Without a kept choice, each panel follows the tier. A choice made at one tier does not stop another panel adapting (fixes UI-06).
- **The page head** is one helper, `pageHead({eyebrow, title, desc, tools})`, used by every screen: eyebrow in small capitals ("CHAPTER 2 OF 6 · BUILD" on Build screens, the file path on item pages), serif title, one sans sentence, tools at the right. Same position on every screen (UI-26).
- **The reading column** is never wider than `--measure`. Tables that need more width (Data & SQL results, History) scroll inside their own box; the page itself never scrolls sideways (UI-14).
- **Phone list → item.** `#entities` shows the list; `#entities/Order` shows only Order. Same for Files, Actions, Guardrails and Logic (UI-17).

---

## Part 6 · The panel pattern (one way to fold)

Every panel that folds (Contents, Index, Notes) has the same head and the same folded rail. One helper builds it: `panelHead(key, name, keyHint)`, replacing `foldbtn` (line 599), `colbtn` (line 559) and `asideHead` (line 561).

**Open:**
- A head row `--panel-head` (36 px) tall, hairline under it.
- The panel's name in small capitals: **CONTENTS**, **ENTITIES** (or FILES, ACTIONS…, whatever the index lists), **NOTES**.
- One icon button, 30×30, in the head's corner **facing the reading column**: top right on Contents and Index, top left on Notes. The icon is a drawn panel-with-arrow glyph (new `g-panel` in `GLYPHS`), mirrored for Notes. Never «, » or text.
- Tooltip and `aria-label`: "Hide contents ( [ )", "Hide list ( \ )", "Hide notes ( ] )". `aria-expanded="true"`.

**Folded:**
- A rail, `--rail` (44 px) wide; Contents uses `--rail-menu` (56 px) because it keeps its icons.
- The **whole rail is one `<button>`** (Enter and Space both work), with the same icon at the same top position, and the panel's name written down it (vertical text). Contents' rail also shows each item's icon; a count or dot sits at the icon's top right.
- Tooltip: "Show contents ( [ )" and so on. `aria-expanded="false"`.

**Shared:**
- Keys: `[` Contents, `\` Index, `]` Notes, when not typing and with no modifier.
- Folding hides one wrapper, `.panel-body`, never the children one by one, so an inline style can't leak through (fixes UI-05).
- Width changes take `--ease`; none with reduced motion.
- On a tablet, the same head is the drawer's close button. On a phone, Notes' fold-out uses the same head and glyph turned 90°.

**Width** (added 2026-11-28, from Vinay's first look at the build: "make the sidebars resizable too so i can make more room to read"):
- Each open panel's inner edge (the side facing the reading column) is a drag handle: an 8 px strip with a 2 px `--accent` line on hover, focus and drag, cursor `col-resize`.
- Double-click the edge to put the default width back.
- The handle is a focusable `role="separator"`: ← and → change the width by 16 px (48 with Shift), Home and End go to the narrowest and widest, Enter puts the default back. It says its width in `aria-valuenow`.
- Limits: Contents 180–380 px, the Index 160–440 px, Notes 240–560 px.
- Widths are kept per panel, per browser (`lorefile-widths`, beside `lorefile-panels`).
- **The reading column never drops under 560 px** on a laptop or desktop. When the open panels would push it under, widened panels go back to their default widths first, then panels fold to rails in this order: Notes, Contents, the Index. Folding this way is never stored, so the panel comes back when there is room.
- No handles on a tablet or phone, where panels are drawers or screens of their own.

---

## Part 7 · The header and the jump search

### The header

One line, 48 px, never wraps, and the same four things at every width:

| Slot | Desktop 1280+ | Laptop 1024–1279 | Tablet 720–1023 | Phone |
|---|---|---|---|---|
| Left | logo + "Lorefile" wordmark (serif), space picker with name and version ("ecommerce · lumen-ecommerce v0.1.0") | the same | logo, picker with the name only | "Menu" button, "Lorefile" and the space name as the picker |
| Middle | the search box, starting right after the picker, 320–560 px | the search box, shrinking toward 320 px first | the search box, flexible | — |
| Right | mode pill ("edit mode · history off"), "⋯" | the same; the pill shortens to "edit" only after the search has reached 320 px and the picker has dropped its version | pill "edit", "⋯" | search icon button (44 px, boxed), "⋯" (the mode pill moves inside it) |

- Things shorten in a fixed order and never change position: the search box narrows, then the picker drops its version, then the pill shortens.
- "⋯" always holds Reload, Summary.md and "Connect an agent" (the MCP command with Copy), plus the mode on a phone. Nothing else moves in and out of it (UI-16).
- Counts and the "filled in" meter leave the header for good; they live in Home's caption.
- The picker, the pill and "⋯" are `--h-compact` tall, `--r-control` corners, `--rule-strong` borders.

### The search box

It replaces **both** of today's Find boxes: the menu's "Find anything…" (in `renderNav`) and Home's "Find… (/)". Neither stays.

**Closed:** a box with a magnifier glyph, placeholder "Find or jump to…" in sans, and a key hint at the right in a small bordered key cap ("⌘K" on a Mac, "Ctrl K" elsewhere). Background `--card`, border `--rule-strong`, corner `--r-control`, height `--h-compact`.

**Open** (focused, or Ctrl/Cmd-K, or `/`): the box gets a 2 px `--accent` ring, the page behind dims with a flat `--ink` scrim at 22%, and a jump list drops under the box: 640 px wide (or the box's width, if wider), `--shadow-pop`, background `--card`, hairline border. It is drawn like a book's index:
- **Groups** in small capitals with their count at the right, in this order when they have hits: ACTIONS, GUARDRAILS, ENTITIES, FIELDS, LINKS, METRICS, SOURCES, ALSO CALLED, COMMANDS. At most 4 rows per group, "+N more" under it.
- **A row:** the kind's glyph, the name (serif for entities, metrics and actions; mono for ids, fields and sources), the matched letters underlined in oxblood (not highlighted in yellow), a one-line snippet in `--ink-2`, and at the right a small stamp when the thing has a grade, or a tag for a guardrail's severity.
- **The selected row** has `--accent-soft` behind it and an "↵ open" key cap at its right.
- **A footer line:** the keys as small key caps ("↑↓ move · Enter open · Esc close"; "⌘↵ open in graph" when an entity is selected) at the left, the counts ("4 results · 2 commands") at the right.
- **Empty query:** the last 5 things opened in this space (kept per browser), then the commands.
- **No hits:** "Nothing called 'refnud'. Try fewer letters, or a field name." and the commands.

**Keys:** Ctrl/Cmd-K and `/` focus it from anywhere (decision 8), except while typing in another box. ↑↓ move, Enter opens, Ctrl/Cmd-Enter opens an entity in the Graph, Esc closes and returns focus where it was. The list is a `role="listbox"` with `aria-activedescendant`; the box says how many results there are to screen readers.

**What it searches:** `GET /api/search?q=…&limit=30` as today (entities, links, guardrails, actions, sources, metrics, functions, also-called words), plus field hits if decision 7 is A (`&fields=1`, a new `field` kind giving "Order · payment_status" with the field's sentence as snippet). Commands are matched in the page:
- Go to any screen ("Guardrails", "People", "Data & SQL" …)
- "+ New entity / guardrail / action / link / source" (edit mode only)
- "Centre the graph on <entity>" (for each entity hit)
- "Hide or show contents / list / notes"
- "Reload", "Summary.md", "Copy the MCP command"

The A2 board also shows "Search every YAML file for 'refund'". There is no full-text file search today, so that command is **not** in the first build; it is listed in "Later".

**Phone:** the search icon in the header opens the jump list as a full-height sheet: the box at the top with a clear (×) button inside it and Cancel beside it, the same groups below, two-line rows at least `--h-touch` tall (name, then the snippet), no selected row and no keys footer, and a last line "4 results · 2 commands. Tap a row to open it."

The boards A2 on the canvas show the open list on a desktop and a phone, and the closed phone header. The closed desktop box (placeholder "Find or jump to…") is not drawn.

---

## Part 8 · Forms and text fields

**Read mode, then edit in place** (decision 4) for descriptions and field sentences:
- Read: the sentence in serif, `--fs-read`. A pencil glyph sits at the end of the line, always visible (not only on hover, because hover does not exist on a phone). Hovering the sentence draws a dotted underline in `--rule-strong`.
- Click the sentence or the pencil, or press Enter on it: it becomes a box **exactly as tall as its text** (below), with Save (primary, compact) and Cancel (secondary, compact) under it, and "Not saved yet" beside them.
- Save, or Ctrl/Cmd-Enter: the box returns to text, and a note under it says "Saved" with a check glyph for 3 seconds, or "Not saved: <reason>" in `--stop` until the next edit (UI-10). Esc cancels.
- Read-only spaces and shipped examples: no pencil, and the sentence can be selected and copied.

**Every multi-line box grows with its text** (UI-08). One class, `.autogrow`, on every `<textarea>`:
```css
.autogrow{field-sizing:content;min-height:calc(2lh + 18px);max-height:40vh;resize:none;overflow:auto}
```
plus a fallback of a few lines of JS that sets `height = scrollHeight` on draw and on each input, for browsers without `field-sizing`. No box is ever drawn shorter than its text, and nothing inside a scrolling column is allowed to shrink: `.main > *, nav > *, .aside > *, .list > * { flex-shrink: 0 }` (UI-07).

**The field row** replaces the fields table (UI-09). One row per field:
- Left, 140 px slot: the field's stamp.
- Line 1: the field name in mono, its kind in `--ink-2` sans beside it ("number (USD)", "enum · pending / paid / refunded / partially_refunded"). An enum with more than 4 values reads "enum · 7 values", with the values in a tooltip and in YAML. Line 1 may wrap.
- Line 2: the sentence, in read mode as above. A field with no sentence shows "Not described yet. What does placed_on mean?" in italic `--warn`, with a dashed `--warn` line under it, and its stamp is NOT DESCRIBED YET.
- Under 720 px the stamp moves to the end of line 1.

**Forms that create things** (+ New entity, + New guardrail, the owner picker) stay as boxes:
- One label style, `.label`: `--fs-small`, 600, `--ink-2`, above the box. No grey label beside a box.
- Inputs, selects and buttons share `--h-control` (or `--h-compact` together), `--r-control` corners, `--rule-strong` border, `--card` background.
- A box that needs words has `--warn-soft` behind it and a `--warn` border.
- A button that can't be used yet keeps its real label and says why beside it: "Looks right" (disabled) with "Add your name to grade" as a quiet link that focuses the name box. Never a disabled button dressed as an input (UI-12).

---

## Part 9 · Components

**Buttons.** Three kinds, two sizes (UI-31):
- **Primary:** `--accent` fill, white text, 600. At most one per area of the screen, for the step the page is asking for (Save, Open Order, Accept).
- **Secondary:** `--card` fill, `--rule-strong` border, `--ink` text. "+ New …" is always a secondary button in the page head.
- **Quiet:** text only, `--accent`, underlined on hover, with padding so it is at least `--h-compact` tall ("Move…", "Change").
- Sizes: `--h-control` (36) and `--h-compact` (30). On a phone every control is `--h-touch` (44). Corners `--r-control`. A link that looks like a button is an `<a class="btn">`, never a button inside a link (UI-21).

**Stamps** (the signature; one helper, `stamp(grade, who, when)`):

| Grade | Look | Words |
|---|---|---|
| Ungraded | dashed `--rule-strong` outline, `--ink-2` text | UNGRADED |
| Proposed (an agent wrote it) | solid `--agent` outline, `--agent` text | AGENT WROTE · NOT REVIEWED |
| Changed since graded | solid `--warn` outline | CHANGED SINCE GRADED |
| Not described | dashed `--warn` outline, `--warn` text | NOT DESCRIBED YET |
| Accepted | `--accepted` double outline (1.5 px line, 2 px paper gap, 1 px line), `--accepted` text | ACCEPTED · Priya · 28 Nov |
| Checked | `--ok` double outline | CHECKED AGAINST ROWS · Priya · 28 Nov |

Small capitals, `--fs-micro`, `--r-stamp` corners, at most two lines. Turned by 2° only on the Notes card's large stamp; stamps in rows sit straight. A stamp is never the only place a grade is said: its `aria-label` carries the full sentence ("Accepted by Priya on 28 November: agents use it").

**Tags** replace the eight pill classes (UI-32): one `.tag` with a tone (`plain`, `ok`, `warn`, `stop`, `info`) and an optional `.tag.small`. Square-ish (`--r-control`), small capitals for severities ("STOP", "WARN"), normal case for words ("basket"). Only count badges are round.

**Tabs:** one `.tabs` (UI-33). Sans 600 `--fs-ui`, `--ink-2`; the active tab is `--ink` with a 2 px `--accent` underline. Never wraps; extra tabs go behind "More ▾".

**Selection** has two looks only (UI-28):
- **You are here** (menu rows, index rows, a selected node, a selected result): `--accent-soft` background, `--ink` text at 600. No bar on the edge.
- **Switched on** (filter chips, toggles): solid `--accent`, white text.

**Lists and rows:** row height `--h-control` on desktop, `--h-touch` on a phone, hover `--tint`. The Contents rows keep one 20 px leading slot so every name starts at the same place: Build rows show their chapter number there in serif (1–6), Home and the Use rows show their icon, and folded, every row shows its icon. Status sits at the row's right edge: a check glyph when complete, else the real count in a `--warn` tag (decision 3).

**Menus and popovers** (header "⋯", People "⋯", the owner picker): plain rows, not bordered buttons; `--card`, hairline border, `--shadow-pop`, `--r-card` (UI-22).

**Dialogs and sheets:** `--card`, `--shadow-modal`, `--r-card`; on a phone, sheets come up from the bottom with a grab bar and `--h-touch` actions at thumb height.

**Toasts** stay for things that happen far from where you clicked (an agent's change arriving, a reload). Saving a field shows its note in place instead.

**Empty, loading and failed** are three different patterns (UI-36):
- Empty: one sentence on what would be here, one example in serif, one secondary button.
- Loading: one quiet line with a moving dot, in the space the content will take, so nothing jumps.
- Failed: `--stop-soft` box, what failed in plain words, a Retry button.

**Icons:** the drawn `GLYPHS` set only (24×24, 1.6 px stroke, `currentColor`), through one helper `icon(name)`. New glyphs: `panel`, `more`, `check`, `close`, `chevron-down`, `warn`, `plus`, `minus`, `star`, `pencil`, `key-cmd`. Typed characters (« » ⋯ ✓ ✕ × ▾ ⚠ ★) leave the controls (UI-34). Every icon-only button has an `aria-label`.

**The footer:** one line, 32 px, never wraps: "✓ valid · 0 errors · 0 warnings" (the check drawn) at the left, a quiet "Connect an agent" at the right that opens the MCP command with Copy. The folder path is never printed in full (UI-20). A blank space (its only error is E000, no entities) reads "empty: add the first entity", not "invalid", as the space picker does. History is said in words and opens the History page: "History on · branch main", "History off · branch main" when the server doesn't commit, "History kept by the repository around it" for a nested space; never a bare "git main". Switching or merging branches is not in the app (that is the history-branches spec).

**The Overview and the Graph as plates.** The drawing sits in a ruled frame (`--rule-strong`, 1 px) on `--draw-ground`, with a caption under it in serif: "Figure: Order in the middle, and everything it touches." and the counts at the right in sans ("12 entities · 13 links · …"). The legend is a "KEY" box in the corner. Tiles are `--draw-tile` with an `--draw-edge` outline and their area's colour as the top edge. Link labels are serif italic ("placed by", "fulfils"). Auto-fit never goes below the zoom where labels are 11 px on screen; below that the drawing scrolls (UI-43, UI-19).
- **Words fit their box.** An entity's name and counts are written on its tile's white top face (176 × 88 in a 192 × 96 cell), the name where the face is widest, the counts under it, so the row in front can never cover them. Every word in the drawings is measured in its own font and cut with "…" to fit its face, box, chip or lane; the full words are in the tooltip and on the Notes card.
- **Readable first, details on zoom.** The opening zoom is the one where names (15 px) are 11 px on screen. Secondary words (counts, areas, cardinalities, field types: 12 px) show once the zoom makes them 11 px too.
- **The Graph fits its canvas.** The lane between the centre and each column narrows to fit the canvas at the opening zoom (170–300); a link's "many orders → one customer" falls back to "many → one" when the lane is narrow; the logic rows hold as many chips as the columns below are wide (2–6).
- **The KEY** sits in the caption line, closed by default (kept per browser); open, it rises over the drawing.

**Layers.** One show/hide list for both drawings, so the Overview and the Graph use the same component and the same choices.
- **The control:** a secondary button in the plate's toolbar, before the zoom: layers glyph, "Layers", and "all" or "3 of 8" in `--ink-2`. It opens a popover (`--card`, hairline, `--shadow-pop`, `--r-card`) headed "SHOW ON THE DRAWING" in small capitals. One row per layer: a checkbox with the layer's name, its count at the right in tabular figures, and a quiet "only" button that shows that layer alone and turns every other layer off on both drawings (so "guardrails only" is one click). The foot has a quiet "Show all" and "kept for this space, in this browser". Rows are `--h-compact`, `--h-touch` on a phone. While open, the button has the "you are here" look.
- **The layers:** the Overview offers Links, Areas, Kinds, Metrics, Guardrails, Actions, Agents and Sources (entities are always drawn; Metrics, Guardrails and Actions are the counts on the tiles). The Graph offers Links, Metrics, Functions, Guardrails, Actions, Sources and Kinds around the entity in the middle. A key means the same on both drawings.
- **Kept:** per space, per browser (`lorefile-layers:<space>`), only the layers someone changed. Until then everything shows, except Agents while no agent has been used and Sources while the space has none, so a new space doesn't show a band of connections that never change.
- **Behaviour:** a change redraws at once and the popover stays open on the box just changed; Esc or a click outside closes it. It is placed on the page under its button (the toolbar clips), and follows it on scroll.

**Room for later:** a before/after view for agent proposals (decision 6) would be a field row split into two halves at the sentence line, "before" struck through in `--ink-2`, "after" in serif with an AGENT WROTE stamp. Nothing above stands in its way.

---

## Part 10 · Accessibility rules

- **Focus:** `:focus-visible{outline:2px solid var(--accent);outline-offset:2px}` on every button, link, input, select, tab, row and anything with `tabindex`. On an `--accent` fill, a 2 px `--card` inner ring as well (UI-38).
- **Contrast:** text at least 4.5:1 (every pair in Part 2 is measured and passes); borders of things you type in or choose from at least 3:1 (`--rule-strong`, 3.40:1).
- **Targets:** nothing smaller than `--h-compact` (30 px) with a mouse; `--h-touch` (44 px) under 720 px (UI-39).
- **Real elements:** buttons are `<button>`, links are `<a>`, the logo is an `<a>`, fold rails are `<button>` (UI-42).
- **Status is never colour alone:** glyph plus word, as in Part 4.
- **Motion:** only `--ease` transitions on widths and the drawing's tile lift; all off with `prefers-reduced-motion`.

---

## Part 11 · Applying it to `index.html`

**How the CSS is organised:** one `<style>` with sections in this order: tokens · base elements · layout (shell, tiers) · patterns (panel head, page head, toolbar, field row, autogrow) · components (button, tag, stamp, tabs, menu, sheet, toast, states) · screens (only what is truly one-off) · the drawing. Today's per-screen blocks fold into these.

**Helpers in the JS,** so markup is never retyped: `icon`, `panelHead`, `pageHead`, `stamp`, `tag`, `fieldRow`, `autogrow`, `btn`, `emptyState`, `tier`. Inline `style="…"` is for values computed in JS (widths, positions in the drawing) only. Target: under 30 inline styles, from 323 (UI-25).

**A test that keeps it this way.** `tests/ui_tokens.rs` reads `src/ui/index.html` and fails when:
- a hex colour appears outside the `:root` block,
- a `font-size`, `border-radius` or `box-shadow` in the CSS is a raw value instead of a token,
- a media query uses a width other than 720, 1024 or 1280,
- the count of inline `style="` grows above a ceiling (set to today's count at the start, lowered as screens are converted, like a ratchet).
That turns "named values only" (rule 4) from a promise into a check that runs with `cargo test`.

**Order of work** (each step leaves the page working):
1. The audit's direction-proof fixes, in today's look: UI-07 (no shrinking), UI-08 (autogrow), UI-20 (footer), UI-05 (folding hides one wrapper).
2. The token block, the test (with ceilings set to today), and the four tiers (Part 5).
3. The panel pattern (Part 6) and the header with the jump search (Part 7).
4. The Order entity page in full Ledger (Parts 3, 4, 8, stamps): Vinay looks at it in the browser before it spreads.
5. The other screens, one at a time, lowering the test's ceilings as each lands: Home and the Graph (as plates), Guardrails, Logic, Sources, Links, Actions, People, History, Data & SQL, Files, Settings, the review screen.

**Checks for each step:** screenshots at 1440, 1024, 768 and 375 px of every screen touched; at 375 no page is wider than the window; every textarea's height equals its text height; every fold works by mouse, by keyboard (`[`, `\`, `]`, Enter, Space) and survives a reload; `cargo test` passes.

---

## Later (not in this spec's build)

- Full-text search across YAML files (the "Search every YAML file" command).
- The before/after view and a "needs you" queue (decision 6).
- Dark mode (decision 2).
- An embedded serif (decision 1, option B).

---

## As built (2026-11-28)

All five steps of Part 11 are in `src/ui/index.html`; `tests/ui_tokens.rs` guards rule 4. Inline `style="` attributes went from 321 to 8, all of them area colours computed in JS. Where the build differs from the text above:

- **Grades on sentences.** `trust.yaml` grades only metrics, an entity's source, also-called words and guardrails. A description, a field sentence or a link sentence has no grade, so its stamp can only say UNGRADED or NOT DESCRIBED YET; the Notes card says so in words. Making sentences gradable is server work and a decision of its own.
- **Auto-fit (Part 9, UI-43).** The Overview and the Graph never auto-fit below the zoom where labels are 11 px; the drawing pans instead. The toolbar's "Fit all" still shows everything on purpose, labels small.
- **The header.** The mode pill shortens and the picker drops its version at the tablet width, rather than by measuring the space left.
- **Logic** stays one reading page (metrics and functions as rows with stamps), without the phone's list → item steps; Guardrails, Entities, Actions and Files have them.
- **Notes** on an entity has no history section yet.
- **The review screen** keeps its change cards (decision 6), restyled with the tokens.
- **After Vinay's first look** (28 Nov): entity pages got a Graph tab; tabs switch in place (only the tab's body is replaced, the scroll stays); an ended sign-in shows "Your sign-in has ended · Sign in again" instead of reloading; side panels resize (Part 6, "Width"); the reading column keeps 560 px on laptop and desktop; long values in Notes wrap; the KEY moved into the caption line.
- **After his second look** (28 Nov): names and counts are written on the tile faces, which grew to 176 × 88; every word in the drawings is measured to fit its box; the Graph's lanes narrow to fit its canvas; the Layers control replaced the Graph's Show boxes and made the Overview's AGENTS and SOURCES bands optional; a blank space's footer says "empty: add the first entity"; the footer says "History on · branch main" and opens History (Part 9).
- **Checked in** headless Chromium only, at 1440, 1024, 768 and 375 px: not Safari, Firefox, real devices or a screen reader.
