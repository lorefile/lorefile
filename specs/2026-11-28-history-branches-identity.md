# Spec: history, branches and names on by default

Written 2026-11-28 for Vinay to decide on. **Status: not built.**

It comes from his feedback on the live Ledger page: "we should always work in a branch right? where can i seee the branching and so on, it should be by default too. and we should also always identity user using the app, right now i don't have any profile set and its still working. we should default to a state which helps the agent the best."

Three changes, one idea: **what agents read is always reviewed, and every change says who made it and when.**

**Base:** `main` at `643eb56`, plus the Ledger page on branch `claude/zen-robinson-0bd6f5` (uncommitted).

**Standing rules, unchanged:**
- The ontology never executes and never holds credentials. The server still never stores a git token or key.
- Agents and builders write; people review.
- Honest status.

**This spec reverses one earlier decision**, on purpose: the local-git spec ([2026-11-26-yaml-view-hosting-local-git.md](2026-11-26-yaml-view-hosting-local-git.md), "What not to build") left branches out of the app, because nobody should have to answer "which branch is the MCP reading?". The answer below keeps that question from ever coming up: agents always read `main`.

---

## What is true today

| Question | Today |
|---|---|
| Do we always work in a branch? | **No.** There is one line of history, `main`. Agents can't write to it: `propose_change` and `propose_patch` queue a proposal, and a person's Accept writes it. So the inbox already works like a pull request, and Accept like a merge, but neither shows up in git as a branch. A person's own edits on the page go straight to `main`. |
| Why does branda say "History is off"? | branda was made before spaces started their history at creation (`init_space`, `src/edit.rs:1459`). Its folder is ignored by the repository around it (`/spaces/` in the dev repo's `.gitignore`), so Lorefile counts it as "no repository yet" (`Home::Nowhere`, `src/edit.rs:1388`), and nothing starts one until someone presses **Start history**. New spaces already get history. |
| Who made a change? | From the page: the name typed on the sign-in page or in Settings, else **"person via page"** (`page_by`, `src/serve.rs:507`). Accept and the other decisions already wait for a name. Browsing and editing don't. From agents: a connection signed in with OAuth is named ("agent via MCP (connection "ChatGPT", approved 27 Nov)"); one using the login token is just **"agent via MCP"**. |

---

## Before and after

**Today.** Vinay opens branda through ngrok with no name set. He edits a field sentence; it is saved to the files at once, and agents read it seconds later, as "person via page". ChatGPT proposes a new metric; it waits in the inbox. History says "History is off for this space", so neither change can be undone or traced.

**After.** The page won't open until he says who he is: one box, "Your name", kept in this browser, then a chip in the header, "Vinay · change". branda's history started by itself the first time the server saw it. His sentence edit goes on his draft branch, `drafts/vinay`; the header says "Draft · 1 change not published · Publish". Agents still read `main`, so they don't see it yet. ChatGPT's proposal is its own branch, `proposals/p-20261128-1`. The History page shows `main` with both branches beside it. He presses Publish, then Accept: two merge commits on `main`, each with his name, and agents read the result.

---

## Decisions

All for Vinay. The recommendation is in bold.

| # | Question | Options | Recommendation |
|---|---|---|---|
| 1 | History for spaces that have none | **A. Start it by itself: when the server starts, each space with "no repository yet" gets `git init -b main` and a first commit, "Start history", with one line printed. `--no-history` turns this off.** B. Keep the Start history button | **A.** Spaces inside *another project's* repository stay as today (uncommitted, explained on the History page), because a repository inside someone's project breaks theirs. |
| 2 | Where people's edits go | **A. On a draft branch per person (`drafts/<name>`); "Publish" merges it into `main`.** B. Straight to `main` with the person's name, as today | **A**, as you asked: agents never read half-finished work. The cost is one more click (Publish) for every batch of edits; the header always says how many are waiting. B is the realistic alternative if Publish feels like friction for one-person spaces. |
| 3 | Where agents' proposals go | **A. Each proposal is its own branch, `proposals/<id>`; Accept merges it, Not now leaves it set aside.** B. Keep proposals as files in `.lorefile/proposals/` only | **A.** Same review screen, but now History shows every proposal as a branch with its outcome. |
| 4 | What agents read | **A. Always `main`. Drafts and proposal branches are never served to an agent.** B. Let an agent pick a branch | **A.** That is the answer to "which branch is the MCP reading?", for good. |
| 5 | Saying who you are on the page | **A. Required before the page opens: the sign-in page's name box becomes required; with `--no-auth` the page asks once, on first open. Kept in this browser; shown as a chip in the header.** B. Optional, as today | **A.** |
| 6 | Page writes with no name | **A. Refused with "Say who you are first" (from the page, which always sends one after decision 5). Scripts can still write with an explicit `by`.** B. Keep "person via page" | **A** for the page; "person via page" stays readable in old history. |
| 7 | Naming agents that use the login token | **A. Use the name the agent gives when it connects (MCP `initialize` sends `clientInfo.name`, e.g. "Claude Code"): "agent via MCP (Claude Code)".** B. Keep "agent via MCP" | **A.** Free, and it tells Codex from Claude Code in History. |
| 8 | Real accounts (each person their own login) | A. Now. **B. Later, for the hosted service.** | **B.** With one shared login token, a typed name is a promise, not a proof. The page says so in Settings ("names are typed, not checked"). Per-person logins are the step that makes them proof. |
| 9 | Order of work | **A. The git-freeze fix first (option B in the freeze session: time limit, 30 s back-off, an honest "unknown"), then history (1), names (5–7), proposal branches (3–4), drafts (2).** B. Any order | **A.** History by default and branches add git calls on every request path; today one hung git call freezes the whole server. |

---

## Part 1 · History on by default (decision 1)

- At start-up, after the spaces are found and before serving, for each space whose `home()` is `Home::Nowhere` and when git is available: `init_space(dir, root, "Lorefile")`. Print one line per space: "History started for branda (its own repository in spaces/branda)".
- `--no-history` skips it, and the History page then keeps today's Start history button.
- `Home::Other` (a space inside another project's repository) is unchanged: uncommitted, and the History page explains in one sentence why, with "Give it a folder of its own" as the way out.
- `Home::NoGit`: unchanged ("History is off: git was not found on this computer").
- The History page never says just "off". It says why and what to do.

## Part 2 · Branches (decisions 2, 3, 4)

**The rule:** the space's folder always has `main` checked out, and that is what agents and the MCP read. Branches are made and merged **without changing the checked-out files** until a merge into `main`, using git's own plumbing (a temporary index, `commit-tree`, `update-ref`). Nothing ever switches the folder to another branch, so an agent can't catch it mid-switch.

**Proposal branches.** When an agent's `propose_change` or `propose_patch` passes the checks, the proposal is also committed on a new branch `proposals/<id>`, from `main`'s tip, by the agent's name (decision 7). The proposal file in `.lorefile/proposals/` stays the source of truth for the review screen.
- **Accept:** merge `proposals/<id>` into `main` (a merge commit, "Accept <headline>", by the person), write the files, reload. If `main` moved and the change no longer applies, the review screen says so in words, as it does today.
- **Not now:** the branch stays, marked set aside. **Take it back:** a revert commit on `main`.

**Draft branches.** A person's edits on the page (sentences, fields, links, guardrails, YAML) are committed to `drafts/<name>`, made from `main` when the first edit comes in.
- The person sees their draft: the page loads `main` with the draft's files laid over it (`loader::load_with`'s overrides already do this for checks). Everyone else, agents included, sees `main`.
- The header shows "Draft · 3 changes not published · **Publish** · Discard".
- **Publish:** the same checks as any save, then merge `drafts/<name>` into `main` ("Publish 3 changes", by the person), write the files, delete the branch.
- Two people editing the same file: the second Publish that no longer applies is refused with the file named, and "Update my draft" rebases it onto `main`.
- With decision 2 = B, this part is not built and page edits commit to `main` as today, with the person's name.

**Seeing them.** The History page gains a **Branches** section above the list of changes: `main`, then each open draft (whose, how many changes, since when) and each open proposal branch (which agent, its headline, open or set aside), each opening its review or diff. Merged branches show in the list of changes as "Merged proposals/p-… · Accepted by Vinay". Settings shows the same counts.

**What doesn't change:** push and pull still move `main` only. Drafts and proposals are local until merged.

## Part 3 · Names (decisions 5, 6, 7, 8)

- **Sign-in page:** "Your name" becomes required, with a short "why": "So every change says who made it. Kept in this browser."
- **No login (`--no-auth`):** on first open the page shows only that box ("Who's using Lorefile on this computer?"), then the page.
- **Header:** a chip with the name and "Change", in the slot left of "⋯". Changing it doesn't rewrite anything already recorded.
- **Server:** a write that carries the page's `X-Lorefile: write` header and no `by` is refused with 400 "Say who you are first". Scripts and older clients that don't send that header keep today's behaviour.
- **Agents:** `initialize`'s `clientInfo.name` is kept per MCP session and used in `via`: "agent via MCP (Claude Code)". OAuth-named connections keep their longer name.
- **Honest note** in Settings: "Names are typed, not checked: anyone with the login can type any name. Per-person logins come with the hosted service."

## Part 4 · The defaults that help agents most

| Default | Why it helps an agent |
|---|---|
| History on for every space | It can answer "who decided this and when" and cite it; every change can be undone. |
| Agents read `main` only | It never reads a half-finished draft or an unreviewed proposal. |
| Every change has a name | It can say "accepted by Vinay on 28 Nov" instead of "a person". |
| Proposals are branches | It can see that its proposal is still open, set aside or merged, from History. |
| Grades stay visible (unchanged) | It says "unverified" before relying on an unreviewed definition. |

---

## Checks for the build

- A space with no repository gets history at start-up; `--no-history` leaves it; a space inside another project's repository is untouched.
- An agent's proposal appears as `proposals/<id>` in `git branch` and on the History page; Accept merges it with the person's name; the folder never leaves `main`, checked with `git rev-parse --abbrev-ref HEAD` while proposals and drafts are being made.
- Two browsers with two names: each sees its own draft, the MCP sees neither until Publish.
- A page write with no name is refused; a script write without the page header still works.
- Agent writes over the login token carry `clientInfo.name`.
- With the freeze session's hanging fake git: no request waits more than about 5 s, and no draft or proposal branch is left half made.
- `cargo test` passes; README (the History and Settings paragraphs), `ARCHITECTURE.md` (§13) and the MCP server instructions say agents read `main`.

## Later

- Per-person logins (decision 8).
- Choosing a branch to push, and pulling someone else's branch.
- A draft shared by two people.
