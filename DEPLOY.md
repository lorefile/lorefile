# Deploying Lorefile

Two ways to run it today, on a laptop and in your own cloud, a note on a hosted version later, and how data access and approvals work. Plain English, with the honest status of each path.

---

## 1. The mental model first

Two kinds of MCP servers sit next to an agent:

```
                 ┌─────────────────────────┐
   "What is a   │  Ontology MCP            │   knows the MEANING: types, rules, actions,
    supplier,   │  (this project)          │   who approves, and WHERE each table lives.
    what rules  │                          │   Never holds warehouse credentials in files.
    apply?"     └─────────────────────────┘
  Agent ──┤
   "Give me     ┌─────────────────────────┐
    the rows    │  Data MCPs               │   hold the DATA and the user's own permissions:
    for Q3"     │  BigQuery, Snowflake,    │   BigQuery MCP, Snowflake MCP, Meta Ads MCP,
                │  Meta Ads, Google Ads…   │   Google Ads MCP, Postgres MCP …
                └─────────────────────────┘
```

The ontology MCP tells the agent *what things mean and which table to look in*. The data MCP, with the user's own service account or OAuth login, *fetches the rows*. Permissions stay where they already are: in the warehouse or the ad platform. The ontology never widens them.

DuckDB inside the binary is a convenience for the cases where you want the ontology server itself to read rows, so `validate` can check a rule against a real record without a second MCP. It is a connector, not a database. CSV, Parquet, S3 and Postgres work today; it is optional and can be compiled out.

---

## 2. Ways to run it

### A. On a laptop (today, works)

```bash
lorefile serve --dir ./ontology          # web page on http://127.0.0.1:7070, one space
lorefile mcp --dir ./ontology            # MCP over stdio, for Claude Code / Claude Desktop / Cursor

lorefile serve --dir ./spaces --allow-edit   # a folder of spaces: one per brand, created from the page
lorefile mcp --dir ./spaces                  # agents pass space: "brand-two" in each tool call
```

The first `serve` makes a login token, saves it to `<dir>/.lorefile/auth.toml` and prints a link that signs the page in (`http://127.0.0.1:7070/?token=…`), plus the command that connects Claude Code over HTTP. Open the link once and the page remembers it. `--no-auth` turns the login off on a laptop. Details in [PRODUCTION.md](PRODUCTION.md#signing-in-and-mcp-over-http).

**A laptop behind a tunnel.** Any HTTPS tunnel in front of `lorefile serve` (ngrok, Cloudflare Tunnel, Tailscale Funnel, localtunnel, …) gives ChatGPT and Claude an address to reach, and Lorefile works the address out from each request, so nothing needs configuring. Keep the default `--host 127.0.0.1`: the tunnel connects locally. Give ChatGPT or Claude the address the tunnel printed, with `/mcp` on the end. Lorefile prints an address read from requests only after a connection was approved on it, since anyone can send a made-up name. A tunnel that changes its address on every start means adding the connector again; a fixed name (ngrok gives free accounts one, Cloudflare named tunnels have one) saves that. TCP-only forwarders give no HTTPS, so they work for Claude Code with the header but not for ChatGPT or Claude. Details in [PRODUCTION.md](PRODUCTION.md#connectors-with-oauth).

The folder is the deployment. Copy it, zip it, `git clone` it. Nothing else to install. The examples and packs ship inside the binary and show up in the page's picker as read-only spaces (`@ecommerce`, `@meta-ads`), so a new team can look at how it is done and copy one. Spaces, storage and the production checklist are in [PRODUCTION.md](PRODUCTION.md).

### B. In your own cloud (today, works; Docker image untested on our side)

The [Dockerfile](Dockerfile) builds a small image: one binary, running as a non-root user, with your spaces on a volume at `/spaces`.

```bash
docker build -t lorefile .
export LOREFILE_TOKEN=$(openssl rand -hex 32)      # or take it from your secret manager
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN="$LOREFILE_TOKEN" lorefile        # read-only page, API and MCP
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN="$LOREFILE_TOKEN" lorefile serve --dir /spaces --host 0.0.0.0 --allow-edit
```

Open `http://127.0.0.1:7070` and paste the token into the login screen, or connect Claude Code to the container:

```bash
claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer $LOREFILE_TOKEN"
```

Without `LOREFILE_TOKEN` the container makes a token on its first start, saves it in the volume (`/spaces/.lorefile/auth.toml`) and prints it in `docker logs`. `--no-auth` is refused on `--host 0.0.0.0` unless `--i-know` is added, for the case where a proxy in front already signs people in.

Runs anywhere a container runs: Cloud Run, Fly.io, ECS, Kubernetes, a $5 VPS (Fly.io config and commands are in §3). State is files, so it restarts in a second; run one replica per spaces folder. The folder comes from a volume, a `git clone` at boot, or a bucket mount (S3 or GCS through a FUSE mount, or synced with `rclone`); the trade-offs and the recommended pattern are in [PRODUCTION.md](PRODUCTION.md). Warehouse credentials come from environment variables or your secret manager, never from the folder.

For a real cloud deployment:

- **HTTPS in front.** The token travels in a header or a cookie, so anything past `127.0.0.1` goes over HTTPS: Caddy, or the platform's own proxy (Fly, Cloud Run and Render give you one).
- **MCP over HTTP.** Built: `POST /mcp` speaks MCP's Streamable HTTP transport behind the same token, so Claude Code, and any agent that can send a header, reaches the container.
- **ChatGPT and Claude connectors.** They connect from their vendor's cloud and sign in with OAuth, which Lorefile now runs itself. Behind the platform's HTTPS it is on with no flag: give them `https://<your address>/mcp` and choose OAuth. Add `--public-url https://<your address>` to answer sign-ins on that one name only. Behind your own nginx, pass the name and HTTPS (`proxy_set_header Host $host; proxy_set_header X-Forwarded-Proto $scheme;`) or use `--public-url`. The login token must be 32 characters or more (`openssl rand -hex 32` makes 64), or OAuth stays off. Details in [PRODUCTION.md](PRODUCTION.md#connectors-with-oauth). ChatGPT plain chats add a server on the web (chatgpt.com/plugins › + › *Add custom MCP server*); as of November 2026 the desktop app's MCP settings are for Codex only. Checks with Claude's connector are still to do (§8).
- **One login for everyone.** The token is shared. For a login per person, put an SSO proxy in front (Cloudflare Access, IAP, Tailscale); accounts and SSO in Lorefile itself are backlog.

### C. Hosted, later

A hosted version may come later. It would run the same open-source binary for you and add what is hard to do alone: one safe copy of each space that agents read through one URL, SSO, an audit log, and GitHub sync, with nothing of your data passing through it. It is not being built now; the earlier plan is in [BACKLOG.md](BACKLOG.md) under "Hosted version, later".

---

## 3. Vercel, Fly.io and friends

**Short answer: Vercel hosts the docs, not the app.** Vercel runs static files and short-lived functions: every request starts fresh, no disk survives, and a function lives for seconds or minutes. Lorefile is one long-running process that owns a folder: it watches the YAML, keeps DuckDB in memory, writes proposals and the activity log under `.lorefile/`, and runs git. It needs a machine with a disk that outlives a restart.

| Option | What it would mean | Verdict |
|---|---|---|
| **The app on Vercel** | Keep each space in a bucket, re-read it on every request, drop git and the inbox or rebuild them on a database, pay a cold start on each call. | **Bin.** A different product with a worse story. |
| **The page on Vercel, the API elsewhere** | Serve the page from Vercel and point it at the binary on another host. | **Bin.** The binary already serves the page; splitting them brings back cross-origin requests (removed on purpose) and a second login problem, for no gain. |
| **The docs on Vercel, the app on a host with a disk** | The tutorial, and later a landing page, as static files on Vercel. The app as the existing Docker image on Fly.io, Railway, Render or a VM, with the spaces folder on a persistent volume. | **Recommended.** Each host does what it is good at. |

```
   Vercel  ──  the docs: tutorial now, landing page later     static files, vercel.json
   Fly.io  ──  the app: page + JSON API, a volume at /spaces  one machine, fly.toml
                 └─ later: one machine and one volume per customer
```

**The docs on Vercel.** [vercel.json](vercel.json) publishes the `docs/` folder as it is: no framework, no install, no build. `docs/index.html` (the page at `/`) is a copy of `docs/tutorial.html`; `python3 scripts/build-tutorial.py` writes both from README.md, and the HTML is committed, so rebuild and commit after README changes. [.vercelignore](.vercelignore) uploads only `docs/` and `vercel.json`, and explains each line of vercel.json, since JSON has no comments.

```bash
python3 scripts/build-tutorial.py     # writes docs/tutorial.html and docs/index.html
vercel deploy                         # a preview URL; vercel deploy --prod for the live one
```

Connecting the repository in Vercel's dashboard works the same way: it reads vercel.json on every push.

**The app on Fly.io.** [fly.toml](fly.toml) builds the [Dockerfile](Dockerfile), runs `serve --dir /spaces --port 7070 --host 0.0.0.0` on one small machine, mounts a volume named `spaces` at `/spaces`, and lets the machine stop when idle and start on the next request (`min_machines_running = 0`). [.dockerignore](.dockerignore) keeps `target/`, `spaces/`, `docs/` and `.git/` out of the build, so the upload is under 1 MB.

```bash
fly launch --no-deploy --copy-config              # creates the app from fly.toml; pick a free app name
fly volumes create spaces --size 1 --region lhr   # a 1 GB disk, in the same region as primary_region
fly deploy --ha=false                             # builds on Fly (5-10 minutes the first time), one machine
fly ssh console -C "ls -ld /spaces"               # the volume should belong to uid 10001 (fly.toml says how)
```

Before pointing anyone at it:

- **It is public; the token is the lock.** `https://<app>.fly.dev` is on the internet as soon as it deploys. Set the token before the first deploy with `fly secrets set LOREFILE_TOKEN=$(openssl rand -hex 32)` (Fly passes secrets in as environment variables); without it the server makes one and prints it in `fly logs`. Anyone with the token can read, and write when `--allow-edit` is on. For a login per person, an SSO proxy (Cloudflare Access) in front, and note that the `fly.dev` address goes around a proxy set up on your own domain.
- **The health check.** Every route but `/healthz` now needs the token, so fly.toml's check must ask `/healthz`; it still asks `/api/spaces`, which answers 401 (see the status table).
- **MCP over HTTP.** Fly terminates HTTPS, so Claude Code connects with `claude mcp add --transport http lorefile https://<app>.fly.dev/mcp --header "Authorization: Bearer <token>"`.
- **ChatGPT and Claude.** Fly's proxy marks requests as HTTPS and passes the name, so OAuth works as fly.toml stands: give them `https://<app>.fly.dev/mcp` and choose OAuth. To answer sign-ins on one name only, add `--public-url https://<app>.fly.dev` to the command in fly.toml. The token from `openssl rand -hex 32` is long enough (OAuth needs 32 characters or more). Not yet tried on Fly.
- **One machine per volume.** Two servers must never share a spaces folder; `--ha=false` stops Fly starting a spare. Fly warns that a single volume has no standby; that is the trade-off for now, and git as the home ([PRODUCTION.md §2](PRODUCTION.md#2-where-the-files-live-disk-git-or-a-bucket), answer B) makes the disk disposable.

**Railway, Render, a VM.** The same image runs anywhere that gives a container a persistent disk: mount it at `/spaces`, expose port 7070, keep the command above. Cloud Run is the same image but stateless; its bucket mount is the FUSE case in [PRODUCTION.md §2](PRODUCTION.md#2-where-the-files-live-disk-git-or-a-bucket).

---

## 4. How a new business would start

Say you have just started an app business and want agents to understand it.

1. `lorefile init ./ontology` writes a starter folder with two types, two rules and one action.
2. Open the folder in Claude Code and say: *"This is a subscription app. Add Customer, Plan, Subscription, Invoice. A customer on a trial cannot be invoiced. Refunds over $500 need Finance."* Claude edits the YAML files directly, because they are just files. `lorefile validate` catches anything misspelled.
3. Point the data bindings at where the rows live: a Postgres `source:` for customers, a Stripe export in Parquet, a BigQuery table for events.
4. Register the MCP server with Claude Desktop or Claude Code (one command, in [README.md](README.md)). From now on, every conversation starts with the same understanding of the business.
5. With MCP over HTTP and an HTTPS address in front, the same folder gets a URL that ChatGPT, claude.ai and hosted agents can use too, signing in with OAuth. Everyone reads from one place.

What the ontology **ingests** is structure and meaning, not rows. Phase 3 adds `init --from postgres://…` to draft types from an existing database schema so step 2 starts from a filled-in list instead of a blank page.

---

## 5. How approvals work

There are two different approvals, and they should not be confused.

| Approval | What it protects | How it works |
|---|---|---|
| **Changing the ontology** (a new rule, a changed threshold) | The definitions themselves | A file edit, reviewed as a pull request. Each type, rule and action has an `owner:`; they are the natural reviewers. Merge, and the server reloads. The page's Agents inbox runs the same flow for agents' proposals: Accept is the merge. |
| **Taking an action** (switching a supplier, approving a contract) | The business | `validate` says pass / warn / fail and names `requires_approval_from`. The agent reports that to the human. The ontology records the policy; it does not sign off on behalf of anyone. |

Agents can read everything through MCP. Writing through MCP is `propose_change`: the change is compiled and checked; with `--allow-edit` and a clean result it is written (and committed, since a space keeps its own git history), otherwise it waits in the web page's Agents inbox for a person to accept or set aside. Shipped examples never accept changes.

---

## 6. Warehouse permissions, concretely

- **Credentials never live in the folder.** A `source:` file names an environment variable, nothing more. The repository is safe to share.
- **Read-only and fenced.** When the binary reads through DuckDB it reads local files only (csv, parquet, `.duckdb`), refuses anything but a single `SELECT`, and after building the views it locks DuckDB to the folders those views read from: no other file on the machine, no extension install, no network. Warehouse and S3 sources are declared, never read here. Use a read-only service account for the connectors agents use anyway.
- **Row-level security stays in the warehouse.** If BigQuery hides EU rows from a user, the ontology MCP does not un-hide them, because it never fetched them; the user's own BigQuery MCP did, under the user's own login.
- **A hosted version**, if one comes, would give each tenant its own isolated process and an audit log of every change, and would hold no warehouse credentials either.

---

## 7. Ontology packs for common MCPs

Meta Ads, Google Ads, Shopify, Stripe, HubSpot and similar MCP servers return data but no business meaning: what is a "conversion", which campaign objective maps to which KPI, what "state-level" reporting can and cannot tell you. A pack is a ready-made ontology folder for one of those sources, with the type descriptions, glossary, data bindings and the gotchas written down, so an agent that has both MCPs knows what it is looking at. Three ship inside the binary today (`@meta-ads`, `@google-ads`, `@shopify`), read-only in the picker; copy one into a space to adapt it. Stripe and HubSpot are next.

---

## 8. Status of each path

| Path | Status |
|---|---|
| Laptop: `serve`, `mcp` over stdio | Built, verified |
| Spaces: many ontologies per server, picker, `space:` in tool calls | Built, verified |
| Shipped examples and packs inside the binary | Built |
| DuckDB fenced to each space's data folders | Built, tested |
| Docker image | Dockerfile written (non-root, volume at `/spaces`), not yet built on our side (no Docker on the build machine) |
| `--host 0.0.0.0` for containers | Built |
| Docs on Vercel: `vercel.json`, `.vercelignore`, `docs/index.html` (§3) | Files written, deploy not run on our side |
| App on Fly.io: `fly.toml`, `.dockerignore`, one machine, a volume at `/spaces` (§3) | Files written, deploy not run on our side. fly.toml's health check still asks `/api/spaces`, which now needs the token: change its `path` to `/healthz` |
| App on Vercel | Binned (§3): no disk that survives, no long-running process |
| `propose_change` through MCP, with a review inbox | Built, verified |
| Clone-at-boot and push to a git remote | Not built; an entrypoint loop does it (PRODUCTION.md) |
| MCP over HTTP (`POST /mcp`, Streamable HTTP, bearer token) | Built, tested end to end with a scripted client |
| OAuth for ChatGPT's and Claude's connectors (address from each request, DCR, approval page, tokens, Settings › Connections) | Built, tested end to end with a scripted client and tunnel-style headers, no network; the approval page and Settings › Connections checked by hand in a browser on `127.0.0.1` |
| OAuth with a real ChatGPT account | Tried live on 27 Nov 2026 through ngrok. ChatGPT on the web (chatgpt.com/plugins › + › *Add custom MCP server*, URL and OAuth) registered as "ChatGPT" with auth `none` on the stable return address `https://chatgpt.com/connector_platform_oauth_redirect`, and called `summary` in a plain chat. Codex desktop connected too. As of November 2026 the desktop app's Settings › Plugins › MCPs are Codex only: plain chats need the server added on the web. Fixed after that test: `/.well-known/openid-configuration` (asked three times) now gets 404, not 401; read tools carry `readOnlyHint` (whether ChatGPT then stops asking to confirm each read is not yet checked live); `offline_access` is listed |
| `match_file` and `learn_file` (a file the user gives the agent) | Built, tested end to end over stdio and over HTTP with an OAuth connection, no network. Not yet tried live in ChatGPT or Claude |
| OAuth with real Claude (claude.ai, Desktop, mobile; a `/s/<id>/mcp` connection) | Untested |
| OAuth with Claude Code and no header (`/mcp` › Authenticate) | Untested |
| OAuth through ngrok, Cloudflare Tunnel (quick and named), Tailscale Funnel, localtunnel | Untested: which header each passes the name and HTTPS in is still to record |
| OAuth behind Caddy, and behind nginx with the two `proxy_set_header` lines | Untested |
| OAuth on Fly.io (`https://<app>.fly.dev/mcp`) | Untested |
| Login: one token for the page, the API and MCP (`LOREFILE_TOKEN`, or made on the first run) | Built, tested; accounts and SSO are backlog |
| `GET /healthz` for load balancers, no token needed | Built |
| `init --from <database>` | Not built (Phase 3) |

A hosted version is not on this list: it may come later ([BACKLOG.md](BACKLOG.md), "Hosted version, later").
