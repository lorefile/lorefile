# Spec: see the YAML in examples · where to host · git that just works locally

Written 2026-11-26 for hand-off to build sessions. Three asks from Vinay, each with the options, a recommendation, what to bin, what done looks like, and the tests. Read [BACKLOG.md](../BACKLOG.md) for the standing rules of this project (the ontology never executes and never holds credentials; YAML is written by agents and builders, people review; keep the page light-themed; tell the truth about what is not built).

Build notes for every session: the binary is Rust (`cargo build --offline` works, run cargo with the sandbox off, zsh wants `$?` not `PIPESTATUS`); `cargo test` runs 33 tests and must stay green; every folder under `examples/` and `packs/` must still compile with no errors and no warnings (`tests/cli.rs` checks); the page is one file, `src/ui/index.html`, light theme only; the preview server named `ontology-spaces` in `.claude/launch.json` runs the debug binary on port 7076 with `--dir spaces --allow-edit`, so rebuild then restart it, or start your own on another port. Commit nothing; Vinay commits when he asks.

---

## Part 1 · See the YAML in the shipped examples

### Before

The shipped examples and packs (`@ecommerce`, `@meta-ads`, …) are read-only, which is right. But the way the page says "read-only" is by greying out every **Edit YAML** button, so on the Sources, Logic and Actions pages of an example you cannot see the file behind a card at all. The Entities page has a YAML tab, and the Files page shows every file, so the content is reachable, just not from where you are looking.

```
 success   per day                               owned by Growth team
 The one number the business is run on: paid orders …
 shop_bigquery · orders · where payment_status = 'paid' …
 logic/measurement.yaml                              [ Edit YAML ]  ← greyed out, dead end
```

### After

Read-only never hides anything. Where you cannot change a file you can still open it, read it, copy it, and jump to it in Files.

```
 success   per day                               owned by Growth team
 …
 logic/measurement.yaml                              [ View YAML ]

 ── click ──────────────────────────────────────────────────────────
 success                    viewing logic/measurement.yaml
 ┌───────────────────────────────────────────────────────────────┐
 │ This example ships with Stella Ontology and stays read-only.  │
 │ Copy it into a space of your own to change it.                │
 └───────────────────────────────────────────────────────────────┘
 ┌ metrics:                                                      ┐
 │   - name: success                                             │
 │     description: The one number the business is run on …     │
 │     …                                                         │
 └───────────────────────────────────────────────────────────────┘
 [ Copy YAML ]  [ Open in Files ]  [ Copy this example into a space ]  [ Back ]
```

The same read-only view is used when the server was started without `--allow-edit` on a space of your own, with the banner saying `start with --allow-edit to change it` instead.

### What to build

All in `src/ui/index.html`; no server change is needed (`GET /api/yaml?file=` already serves any file of any space, read-only).

1. Every `Edit YAML` / `Edit this file` button that is currently rendered `disabled` when `ro()` instead renders as **View YAML**, enabled, calling the same `openEditor(file, name, kind)`.
2. `openEditor` no longer refuses when read-only. `editorHtml()` gets a read-only mode: the textarea is `readonly` (keep it a textarea so long files scroll and select the same way; or a `<pre>`; either is fine), no Save, no Delete, no ⌘S hint; instead **Copy YAML** (clipboard), **Open in Files** (`ui.file = file; go('files')`), and, when the space is a shipped one and the server can create spaces, **Copy this example into a space** (the existing `ui.ns={name:'',from:id};go('new-space')` flow). A banner above the text says why it is read-only: shipped example, or edit mode off.
3. The Files page keeps showing the file; its button becomes **View** when read-only (or is simply hidden, since the file is already on screen; pick one and keep it consistent).
4. Nothing else changes: builders, description boxes and the proposal inbox stay disabled in read-only mode as they are now; the 403 from the server for writes to a shipped space stays.

Bin: a separate "raw" route per card. Backlog: a "View YAML" link in the MCP `describe` output (agents already get `files` in `summary`; not needed).

### Done looks like

- On `@ecommerce` → Logic, every card has a working View YAML that shows the file with the banner and the four buttons; Copy YAML puts the text on the clipboard; Open in Files lands on that file selected.
- On a space of your own with `--allow-edit`, nothing changed: Edit YAML, Save, Delete work as before.
- On a space of your own without `--allow-edit`, View YAML works and the banner names the flag.
- `tests/http.rs`: one new test that `GET /api/yaml` on a shipped space returns the file (it already does; the test pins it) and that `POST /api/edit` on it still returns 403.
- Checked in the browser on both an example and a space of your own; README's page tour (the "Build" and "Files" lines under "The page, screen by screen") mentions View YAML in one clause. Do not rebuild the tutorial in this session (the git session rebuilds it last).

---

## Part 2 · Where to host it, and the honest answer on Vercel

### What Vercel is, and what this app is

Vercel runs static sites and short-lived serverless functions. Each request starts fresh, there is no persistent disk, a function lives for seconds to a few minutes, and native libraries have to fit the function bundle. Stella Ontology is one long-running process that owns a folder: it watches YAML files, keeps DuckDB in-process, writes proposals and an activity log under `.stella/`, runs git, and speaks MCP over stdio (and later over HTTPS with sessions). Those two shapes do not meet.

| Option | What it would mean | Verdict |
|---|---|---|
| **A. The app on Vercel** | Keep each space in a bucket, re-read it on every request, give up git and the inbox or re-implement them on a database, pay the cold start per call. | **Bin.** It can be made to answer requests, but it is a different product with a worse story. Not worth a week. |
| **B. The page on Vercel, the API elsewhere** | Serve `index.html` from Vercel's CDN and point it at an API on another host. | **Bin.** The binary already serves the page; splitting them re-opens CORS (removed on purpose) and adds a login problem. No benefit. |
| **C. Docs and marketing on Vercel, the app on a host with a disk** | `docs/tutorial.html` and a landing page on Vercel (static, free). The app runs as the existing Docker image on Fly.io (Machines + a volume per app), Railway or Render (persistent disks), or a VM. | **Recommended.** Vercel does what it is good at; the app keeps its folder. |

**Why Fly.io for the app.** One Docker image, a persistent volume mounted at `/spaces`, one machine per customer later (which is the per-tenant isolation the hosting plan wants anyway), scale-to-zero, about $5 a month for a small one. Cloud Run is the same image but stateless; its GCS volume mount is a FUSE mount with the caveats in [PRODUCTION.md §2](../PRODUCTION.md). Railway and Render are fine too. Nothing here needs code changes: the binary already takes `--host 0.0.0.0` and a `/spaces` volume.

```
   vercel  ──  ontology.stella.app          static: tutorial, docs, landing page
   fly.io  ──  app.ontology.stella.app      the binary, a volume at /spaces, behind SSO (Cloudflare Access) until login exists
                 └─ each customer later: its own machine + volume
```

### What to build

1. `vercel.json` at the repository root that publishes `docs/` as a static site (`docs/index.html` is the tutorial; keep `docs/tutorial.html` as it is and add an `index.html` that is a copy or a redirect). `scripts/build-tutorial.py` writes both. No framework, no build step on Vercel: the HTML is committed.
2. `fly.toml` for the app: the existing `Dockerfile`, internal port 7070, a `[mounts]` entry for `/spaces`, `--host 0.0.0.0` in the command, one machine, auto-stop allowed, `min_machines_running = 0`. Add a `.dockerignore` (target/, spaces/, docs/, .git/) so the image build is small.
3. `DEPLOY.md`: a short section "Vercel, Fly.io and friends" with the table above in plain words, the two files, and the exact commands (`vercel deploy`, `fly launch --no-deploy && fly volumes create spaces --size 1 && fly deploy`). Status column stays honest: files written, deploy not run on our side.
4. `PRODUCTION.md §2`: one sentence pointing at the new section.

Do not run a deploy; Vinay has the accounts. Do not touch `src/` or `index.html` (another session is editing them).

### Done looks like

- `vercel.json`, `fly.toml`, `.dockerignore` exist and are commented in plain English; `python3 scripts/build-tutorial.py` also writes `docs/index.html`.
- `DEPLOY.md` answers "how do we host this on Vercel" in one screen, with the bin reasons stated once.
- `cargo test` still green (nothing in `src/` changed).

---

## Part 3 · Git that works locally from day one, and only meets GitHub when you want it to

### The question

Is there a smaller git we can embed, so the hosted service and the open-source binary keep history without asking people to set up a GitHub repository first? And can everything work locally (init, commits, branches, merges, review) until the day someone wants to share, when they push to GitHub or GitLab?

### Short answer

Yes to local-first, and it needs no embedded git. The binary already shells out to the `git` on the machine (`src/edit.rs`, `git_*` functions), and that is the right engine: it is on every developer machine, it is in the Docker image already, and, most importantly, when a push needs a login it uses the person's own credential helper or SSH key, so Stella never holds a token. What is missing is the flow: history should start by itself when a space is created, every accepted change should become a commit without a flag, the page should show the history and let you undo a change, and "share it" should be a paste-a-URL button that works for GitHub, GitLab, Bitbucket, Gitea or a bare folder on a shared drive, because they are all just git remotes.

### The options for the engine

| Option | What it is | Cost | Verdict |
|---|---|---|---|
| **A. System git** (what we have) | Run `git` as a subprocess. | Needs `git` on the box: every dev machine has it, the Dockerfile installs it (about 10 MB). | **Recommended.** Zero build cost, the user's own auth, every host works. The binary keeps working without git; the page then says history is off. |
| **B. `git2` (libgit2) embedded** | A C library compiled into the binary. init, commit, log, diff, branch, merge, push all supported. | Vendored C build plus OpenSSL and libssh2 for HTTPS/SSH; longer builds; and push credentials become our problem: we would have to ask for and hold a token or key. | Backlog, for a hosted box with no git. Not now. |
| **C. `gix` (gitoxide) embedded** | Pure-Rust git. | Lighter than libgit2, but push was incomplete the last time we checked; verify before relying on it. | Dead end today. |
| **D. Our own mini-VCS** | Snapshots under `.stella/history`. | Easy to write; no diff tooling, no GitHub, no GitLab, no one else can read it. | **Bin.** |

### Before

```
 Settings
 ┌ Version control ─────────────────────────────────────────────┐
 │ Not a git repository.                              [ Start ]  │   ← starts a repo for the WHOLE spaces folder
 │ 3 uncommitted changes · [ message ……… ] [ Commit ]            │   ← only with --commit-edits do saves commit by themselves
 │ To back this up on GitHub: create an empty repository, then   │
 │ run  git -C … remote add origin git@github.com:you/x.git      │   ← a terminal step before the Push button appears
 └───────────────────────────────────────────────────────────────┘
```

A new space is just a folder; nothing records who changed what until someone presses Start, and even then saves pile up uncommitted unless the server was started with `--commit-edits`.

### After

```
 New space "Brand Two" ───────────────────────────────────────────
   created · history on (git)                                      ← automatic, silent; "history off: git not found" if no git

 every accepted change ──► one commit, the summary in words as the message,
                           author "Growth agent via MCP" or "Vinay via page"

 History                                                           ← new page under Use
 ┌───────────────────────────────────────────────────────────────┐
 │ today 14:02   agent via MCP   An agent learned: 5 changes …    │  [ Show ] [ Undo ]
 │ today 11:40   person via page Describe Order.basket            │  [ Show ] [ Undo ]
 │ yesterday     person via page Start the space                  │
 └───────────────────────────────────────────────────────────────┘
   Show  → the diff of that change, file by file (the inbox card's diff view, reused)
   Undo  → restores those files to how they were, compiled first like any save, as a new commit "Undo: …"

 Settings · Version control
 ┌───────────────────────────────────────────────────────────────┐
 │ History on · 14 changes · branch main · nothing shared yet     │
 │ Share this space:  [ git@github.com:you/brand-two.git     ]    │  [ Connect ]
 │   GitHub, GitLab, Bitbucket, Gitea or a folder on a drive:     │
 │   any git address works. Create an empty repository there      │
 │   first. Your own git login is used; Stella keeps no token.    │
 │ ── once connected ──                                           │
 │ Shared with github.com/you/brand-two · 2 changes not pushed    │  [ Push ] [ Pull ]
 └───────────────────────────────────────────────────────────────┘
```

Proposals stay the review step: the inbox is the local pull request, Accept is the merge. There are no branches to choose in v1 (see "What not to build").

### What to build

**Backend, `src/edit.rs`, `src/spaces.rs`, `src/serve.rs`, `src/main.rs`, `src/mcp.rs`.**

1. **History starts with the space.** `Registry::create` (and the CLI `init`) runs `git init -b main` in the new space folder, sets repo-local `user.name` and `user.email` (`Stella Ontology <ontology@stella.local>`, so commits never fail with "please tell me who you are" on a server with no global git config), and makes the first commit "Start the space". Skip, without failing, when `git_root(dir)` already finds a repository (the spaces folder, or the dev repo, is already one) or when `git` is not on the PATH; the create result and `GET /api/git` say `history: "on" | "off (git not found)" | "off (inside another repository)"`.
2. **Every accepted change commits, by default.** Today `EditConfig.commit` is false unless `--commit-edits`. New rule: commit when the repository belongs to the ontology, that is `git_root(space.dir)` is the space folder itself or the spaces folder (`reg.root`). When the space is nested deeper inside some other repository (the stella-ontology dev repo, say) keep today's behaviour: leave the files uncommitted unless `--commit-edits`. `--no-commit-edits` turns it off; `--commit-edits` stays accepted. The commit message is the summary in words that already exists for every write (the proposal summary, the patch summary, "Edited logic/measurement.yaml"); the author is the `via` string plus a fixed email (`<via>@stella.local`, lower-cased and dashed, so the History page can tell agent from person). Page edits that today go through `apply`/`apply_many` already have the commit hook (`commit_many`); make sure the Settings Commit button is still there for the nested case.
3. **`GET /api/git/log?limit=50&file=<optional>`**: the space's commits, newest first: short hash, author, relative and absolute time, message, files touched. `GET /api/git/show?hash=`: the unified diff for that commit limited to the space folder, as text. Both read-only, no write header needed, scoped to the space (`-- <space dir>`), and refused on shipped examples with the usual read-only message (they have no repository anyway).
4. **`POST /api/git/undo` `{ hash }`**: for each file that commit touched, take its content at `hash^` (`git show hash^:path`; deleted at the parent means delete), run the set through the same gate as a save (`edit::apply_many`, which compiles the whole space with the files in place and refuses on errors), and commit as `Undo: <original message>`. Only the newest commits are safe to undo blindly; for an older one the same gate applies and the compile decides. Write header and edit mode required, like every write.
5. **`POST /api/git/remote` `{ url }`**: `git remote add origin <url>` or `set-url` when one exists. Accept whatever git accepts (SSH, HTTPS, a local path to a bare repository). `POST /api/git/push` stays; `POST /api/git/pull` is new: `git pull --ff-only origin <branch>`, then the fingerprint reload picks the files up. Both keep `GIT_TERMINAL_PROMPT=0`, and when git cannot sign in the error shown is in words: "git could not sign in to github.com. Push once from a terminal to set up access, or use an SSH address." The server never stores a token or a key.
6. **`GET /api/git`** grows: `history` (as in 1), `commits` (count), `remote_host` (for the Share card), `behind` (commits on the remote not pulled), and keeps `uncommitted`, `ahead`, `branch`.
7. **MCP**: `summary` gets one line, "History: on, 14 changes, last: …" so an agent can say so. No new tools in this round (a `history` tool is backlog).
8. **Docker and hosting**: no change needed; the image has git, the volume holds the repositories. Note in `PRODUCTION.md §2` that option B ("git is the home") is now the default inside the app, that the per-minute push loop is replaced by the Push button (an automatic push on every commit when a remote is set is a one-line follow-up, flag `--push-edits`, backlog), and that Stella Cloud's backup is a push to an internal mirror, which needs no customer credentials, while GitHub or GitLab sync for a customer is a later hosted feature.

**Page, `src/ui/index.html`.**

9. **History** page under Use (nav line "History · what changed, who, when"): the list above, Show opens the diff inline under the row (reuse the diff rendering from the inbox card; the server's unified diff is already line-based, colour `+` and `-` lines), Undo asks once in words ("Put logic/measurement.yaml back as it was before 'Describe success'?") and shows the compile result the way saves do. Agent and person rows look different (a chip: `agent` / `person`).
10. **Settings · Version control** becomes the card above: the state line, the Share form with the URL box and the sentence about logins, Push and Pull with the counts, and the Commit box only when there are uncommitted changes (the nested-repository case). Remove the terminal instruction.
11. **New space** flow: the success toast says "history on" or why not.

**Docs.** README: the Settings paragraph under "The page, screen by screen" and step 9 of the ten steps (version control) rewritten to this flow; a History line in the page table; the status line at the top gains "local git history from day one". `BACKLOG.md`: item 5 (GitHub App sync) stays hosted; add "`--push-edits`", "branch per experiment", "`history` MCP tool", "embedded git for hosts without git" as backlog lines. Rebuild the tutorial last: `python3 scripts/build-tutorial.py` (and `--fragment` if present).

### What not to build (and why)

- **Branches, merges and pull requests inside the app.** The proposals inbox is already the review step, and Accept is the merge. A branch would raise a question nobody wants to answer in a chat: which branch is the MCP reading? When a team wants branch-based review they push to GitHub or GitLab and use that review there. Backlog: "a branch per experiment", with the MCP pinned to `main`.
- **Embedding git.** See the engine table. Revisit only if a hosted box cannot have git, which the Dockerfile already rules out.
- **Holding tokens for push.** Open source: the person's own git login. Hosted: Stella's backup is an internal mirror; customer GitHub or GitLab sync is the hosted GitHub App item in the backlog and lives in the hosted secret manager, never in a space.

### Done looks like

- Create a space on the page: it is a git repository with one commit, no flag given. Describe a field: a second commit with that sentence as the message. An agent's batch accepted from the inbox: one commit whose message is the "An agent learned: …" summary, author ending in `agent`. The History page shows the three, Show gives the diff, Undo on the second puts the sentence back and adds an "Undo: …" commit, and the page and MCP both see the reverted file.
- Paste a GitLab or GitHub SSH address into Share: Push works with the person's own key; with no key the message says what to do, and nothing hangs.
- A space nested in the dev repository (`serve --dir examples/ecommerce --allow-edit` from the project folder) behaves as today: uncommitted files listed, Commit button, no automatic commits.
- Tests: `src/edit.rs` unit tests for `init_space` (sets identity, first commit, skips when inside a repo, skips when git missing: fake the PATH), for the "repository belongs to the ontology" rule, and for undo through the gate (a bad undo that would not compile is refused and leaves the tree unchanged); `tests/http.rs` for `/api/git/log`, `/api/git/show`, `/api/git/undo`, `/api/git/remote` (a local bare repo as the remote, push and pull round-trip, no network); `tests/mcp_loop.rs` extends the blank-space test to assert the commits exist afterwards. All 33 existing tests still pass; every example and pack still validates clean.
- Checked in the browser: create a space, teach it over the API, History, Undo, Share to a local bare repository, Push, Pull.

---

## Hand-off plan

| Session | Scope (files) | Runs |
|---|---|---|
| **yaml-view** (Part 1) | `src/ui/index.html`, one test in `tests/http.rs`, one clause in README | now |
| **deploy-docs** (Part 2) | `vercel.json`, `fly.toml`, `.dockerignore`, `scripts/build-tutorial.py` (docs/index.html), `DEPLOY.md`, `PRODUCTION.md` | now, in parallel: no files shared with yaml-view |
| **local-git** (Part 3) | `src/edit.rs`, `src/spaces.rs`, `src/serve.rs`, `src/main.rs`, `src/mcp.rs`, `src/ui/index.html`, tests, README, BACKLOG, tutorial rebuild | after yaml-view ends: both edit `index.html` |

All three work in this checkout on `master` with today's uncommitted work in place; none commits, none switches branch. Vinay reviews and commits.
