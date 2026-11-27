# Running Lorefile in production

The questions this answers: how several brands share one server, where the files live (a disk, git, or an S3/GCS bucket), whether DuckDB is "installed per brand", what a production deployment still needs, what SOC2 asks of it, and which open-source licence to use. Plain English first, commands second, honest status at the end of each part.

---

## 1. One server, many spaces

A **space** is one ontology: one brand, one business unit, one client. The server holds many of them.

**Before** (one folder per brand):

```
lorefile serve --dir brand-one --port 7070
lorefile serve --dir brand-two --port 7071      # two servers, two ports, two MCP entries
```

**After** (one folder of spaces):

```
spaces/
  brand-one/        ← a space: ontology.yaml, types/, constraints/, actions/, data/ …
  brand-two/
  .lorefile/          ← the server's own state; it writes a .gitignore that hides itself
    examples/       ← the examples and packs that ship inside the binary, rewritten at start

lorefile serve --dir ./spaces --allow-edit    # one server, one port
lorefile mcp   --dir ./spaces                 # one MCP server for every space
```

The page shows one space at a time and has a picker at the top:

```
┌──────────────────────────────────────────────────────────────────┐
│ [brand-one · 12 entities ▾]  brand-one v0.1.0   12 entities · 13 links │
│   Your spaces                                                      │
│     brand-one · 12 entities                                          │
│     brand-two · 4 entities                                           │
│   Examples that ship with it                                       │
│     ecommerce · 12 entities        ← read-only, to see how it is done │
│     supply-chain · 10 entities                                        │
│   Packs for common MCPs                                             │
│     meta-ads · 12 entities                                            │
│   ＋ New space…                 ← blank, starter, or a copy of any   │
└──────────────────────────────────────────────────────────────────┘
```

Each space has its own URL (`/s/brand-two/`), its own API (`/s/brand-two/api/…`), its own DuckDB catalog, its own proposals inbox and activity log. Shipped examples have ids that start with `@` (`@ecommerce`, `@meta-ads`), are always read-only, and can be copied into a space of yours with one click. Leave them out with `--no-examples`.

Over MCP, agents say which space they mean. With one space of yours it is implied; with several, every tool takes `space:` and the tool list names them:

```
describe(space: "brand-two", name: "Campaign")
validate(space: "brand-one", action: "cancel_order", inputs: {…})
spaces()                                     → what each space holds
```

A server dedicated to one space is one flag away: `lorefile mcp --dir ./spaces --space brand-two`.

**What is shared and what is not.** Folders, catalogs, logs and proposals are per space. The process, its flags and its credentials are shared: `--allow-edit` applies to every space, and there is one login token for the whole server, not one per space. That is the right shape for brands inside one company. For separate customers, run one container per customer (part 4): a process boundary is what an auditor wants to see, and it is what a hosted version would do.

Status: built. `validate --dir ./spaces` checks every space, for CI.

---

## 2. Where the files live: disk, git, or a bucket

An ontology is a folder of small text files. The binary reads and writes a folder; it does not talk to buckets itself. So the question is how that folder gets onto the machine and how it survives a restart. Three answers.

| | How it works | Good for | Watch out for |
|---|---|---|---|
| **A. A persistent disk** | Docker volume, EBS, a GCE persistent disk, mounted at `/spaces`. | The simplest start. One container, one disk. | One replica only (two servers on one folder would race). Back the disk up yourself. |
| **B. Git is the home** (recommended for teams; the default inside the app) | Every accepted change is already a commit: a new space starts its own repository, and a `/spaces` that is one repository (cloned at boot) gets one commit per change, no flag. Push from Settings, or a small loop. The disk becomes disposable. | Review, history, rollback, and the change-management evidence SOC2 wants, for free. | Pushing from a container needs a deploy key in its secret store. Pushing by itself on every commit is a backlog flag (`--push-edits`), not built. |
| **C. A bucket is the home** (S3, GCS) | A bucket is not a filesystem, so either mount it (`gcsfuse`, AWS `mountpoint-s3`, `s3fs`) at `/spaces`, or sync a disk to it (`rclone sync` on a timer). | Backups, and the data files (Parquet) that sit next to the ontology. | Writes through FUSE are slow and sometimes surprising; the `.lorefile/` state folder works but is chatty. Use a bucket as the backup target, not as the working copy. |

**Recommendation.** Git for the ontology, a disk for the working copy, a bucket for data files and backups. Concretely:

```bash
# one-off: the repository that holds the spaces
git init spaces && cd spaces && git commit --allow-empty -m "start" && git remote add origin git@github.com:you/ontology.git

# the container: clone at boot; each accepted change is a commit by itself; push from Settings or every minute
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e GIT_SSH_COMMAND="ssh -i /run/secrets/deploy_key" \
  lorefile serve --dir /spaces --host 0.0.0.0 --allow-edit
# entrypoint sketch:  git clone "$REPO" /spaces 2>/dev/null || git -C /spaces pull;  (while sleep 60; do git -C /spaces push -q; done) &
```

**Data on S3 or GCS.** The rows a space points at can stay in a bucket: a `source:` of kind `s3` (or `httpfs`) with `path: s3://bucket/prefix` makes DuckDB read Parquet straight from there, with credentials from the environment (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, the AWS credential chain, or an HMAC key for GCS through its S3-compatible endpoint). The first use downloads DuckDB's `httpfs` extension, so the container needs outbound network once, or bake the extension into the image. Snowflake and BigQuery are never read by the binary: agents use their own connector and the user's own permissions.

Status: A works today. B is the default inside the app since 2026-11-26: a space whose repository is its own folder or the folder of spaces commits every accepted change with the summary in words as the message and `agent via MCP` or `person via page` as the author, the History page shows and undoes them, and Settings › Share takes any git address with Push and Pull. The per-minute push loop above is now optional, since the Push button does the same with the person's own git login; an automatic push on every commit when a remote is set is a one-line follow-up (`--push-edits`, in [BACKLOG.md](BACKLOG.md)). Clone-at-boot is still the entrypoint's job. A space nested inside some other project's repository stays uncommitted unless `--commit-edits`. C works with a FUSE mount; sync is your cron job.

**Hosted, later.** A hosted version may come later and would add one safe copy of each space, SSO, an audit log and GitHub sync, with any sync token held by the hosted service, never in a space and never in the open-source binary. It is not being built now; its earlier notes are in [BACKLOG.md](BACKLOG.md) under "Hosted version, later".

Which host gives you answer A (Fly.io with a volume at `/spaces`, Railway, Render or a VM), and why the app does not go on Vercel, is in [DEPLOY.md §3, "Vercel, Fly.io and friends"](DEPLOY.md#3-vercel-flyio-and-friends).

---

## 3. DuckDB: a library, not a service

Nothing is installed per brand, or at all. DuckDB is compiled into the `lorefile` binary the way SQLite is compiled into an app. There is no daemon, no port, no file unless a source is a `.duckdb` file you point at.

What happens at runtime:

- The first time a space is opened, the server builds an **in-memory catalog** for it: six small tables holding the ontology itself (`nodes`, `edges`, `constraints`, …) and one **view per data-bound type** that reads the rows where they live (a CSV or Parquet file, a `.duckdb` file, Postgres, S3). So "one DuckDB per space", all inside one process, each a few megabytes.
- It is **read-only**: sources are attached `READ_ONLY`, a query is one `SELECT`/`WITH`, at most 500 rows.
- It is **fenced, always**. After the views are built, the catalog is told the only folders it may touch (the space's `data/` and any folder a file source names), external access is switched off, and the configuration is locked. From then on `select * from read_csv('/etc/passwd')` and `install httpfs` fail, and a space whose fence cannot be set gets no SQL door. This is why the door reads local files only: Postgres, S3 and HTTP sources are declared for `where_is` and read by agents with their own connectors, never through DuckDB here. (An earlier build left the fence open for such spaces; the security review showed that gave any agent a read of the whole filesystem. It no longer does.)
- Memory and CPU are shared by every space in the process. For brands in one company that is fine. For separate customers, one container each (part 4) is also the resource boundary.

If you compile with `--no-default-features` there is no DuckDB at all: CSV and inline rows still work, `query` does not.

---

## 4. What production needs

The binary is ready to be put behind the things every internal web app sits behind. What it does not do itself is listed plainly.

| Area | Today | For production | Status |
|---|---|---|---|
| **Login** | One token, the way Jupyter and code-server do it. `serve` makes one on the first run, saves it to `<dir>/.lorefile/auth.toml` (readable by this user only) and prints a link that signs in; `--token` or `LOREFILE_TOKEN` set it instead. The page keeps it in an `HttpOnly; SameSite=Strict` cookie; API and MCP clients send `Authorization: Bearer <token>`. The same token is the approver's key for OAuth connections (below). Without it the API answers 401 and the page shows a login screen. Reads carry no CORS header and SQL is POST-only, as before. `--no-auth` turns the login off, and is refused on an address other machines can reach unless `--i-know` is added. | Set `LOREFILE_TOKEN` from your secret manager and give it to the people and agents who need it; change it and restart to rotate. For a login per person, put an identity-aware proxy in front (Google IAP, Cloudflare Access, oauth2-proxy, Tailscale) and keep the token as a second lock, or run `--no-auth --i-know` behind it. | Built: one shared token. Accounts and SSO are backlog. |
| **MCP for remote agents** | stdio (`lorefile mcp`) for an agent on the same machine, and MCP's Streamable HTTP transport at `POST /mcp` behind the same token: one JSON-RPC message or a batch per request, plain JSON replies, no sessions, the same tools and answers as stdio. `/s/<space>/mcp` serves one space only. | HTTPS in front. Claude Code connects with `claude mcp add --transport http …` (below). ChatGPT's and Claude's connectors (claude.ai, Desktop, mobile) connect from their vendor's cloud and sign in with OAuth, which Lorefile now has built in: it answers on any HTTPS address that reaches the server (below, "Connectors with OAuth"). | Built for Claude Code and any client that can send a header. OAuth built and tested with a scripted flow; not yet tried with real ChatGPT and Claude accounts. |
| **TLS** | Plain HTTP. The token travels in a header or a cookie, so beyond `127.0.0.1` it must go over HTTPS. The cookie gets `Secure` when the proxy marks the request as HTTPS (`X-Forwarded-Proto: https`, `Forwarded: proto=https` or Cloudflare's `CF-Visitor`). | Terminate TLS at the proxy or load balancer (Caddy, or the platform's own). | Standard. |
| **Edit mode** | Off by default. A write needs the token, and still carries the page's `X-Lorefile: write` header (a custom header, so a cross-site browser request never gets past the preflight) and, when the browser sends one, an `Origin` that matches the `Host`; it must be JSON and under 4 MB. Scripts can send the header on purpose. Shipped examples never take writes. Over MCP, `--allow-edit` decides whether a proposal is written or queued, as over stdio. | Turn it on behind the token or the proxy. Add `--commit-edits` so each change is a commit with who and why. | Built. |
| **Secrets** | By convention, YAML holds no credentials: a `source:` names an environment variable (`env: PG_URL`). This is not enforced; `add_source` refuses values that look like a password or token, nothing scans a full-file `propose_change`. | Mount from your secret manager as environment variables. Use read-only database roles. Review proposals before accepting. | Convention. |
| **Audit** | `.lorefile/activity.jsonl` per space (every edit, proposal, accept, dismiss, with who and when); git history when committing. | Ship the log and the proxy's access log to your SIEM. Retain per your policy. | Built (local file); shipping is yours. |
| **Backups** | Files on disk. | Git remote (part 2B) plus disk snapshots. | Yours. |
| **Health** | `GET /healthz` returns `{"ok":true}` without the token. Every other route, `/api/spaces` included, needs it. | Point the load balancer's health check at `/healthz`. | Built. |
| **Resource limits** | Request bodies are capped at 4 MB and each request runs on its own thread, so a stalled client holds a thread, not the server; writes still serialise on one lock. DuckDB caps rows at 500 and one statement; no memory cap or query timeout. | Container memory limit; a `--duckdb-memory` flag is roadmap. Rate-limit at the proxy. | Partly. |
| **Container hardening** | Non-root user, no shell needed, one static binary. | Add `--read-only` root filesystem with `/spaces` and `/tmp` writable. | Built in the Dockerfile; image untested here (no Docker on the build machine). |
| **Replicas** | One process owns one spaces folder. | Run one replica per spaces folder. Two replicas on one shared disk would race on writes. | By design; a hosted control plane handles this. |
| **Releases** | Built locally. | CI that builds, runs `cargo test`, validates every example and pack, publishes signed binaries and an image, runs `cargo audit`, produces an SBOM. | Not built. |

**The three things an auditor will ask.** Who can change the ontology (whoever holds the token, or the proxy's group, plus edit mode plus git review). Who changed what (the activity log and git). What can an agent reach (read-only, fenced DuckDB, the rows it can see are the rows its own warehouse login allows).

### Signing in, and MCP over HTTP

The first start prints everything needed to get in:

```
$ lorefile serve --dir spaces --allow-edit
Lorefile 0.1.0 · 2 spaces of yours in spaces, plus 8 shipped examples · edit mode on
Open    http://127.0.0.1:7070/?token=k3f9…   (first run: token saved to spaces/.lorefile/auth.toml)
MCP     http://127.0.0.1:7070/mcp   Authorization: Bearer k3f9…
        claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer k3f9…"
        or over stdio on this machine: lorefile mcp --dir spaces
Edit    on: …
```

- **The token.** Looked for in this order: `--token`, `LOREFILE_TOKEN`, `<dir>/.lorefile/auth.toml`; with none of them, 32 random bytes are made (base64url, 43 characters), saved to that file with mode 0600 and printed. A later start reads the same file and prints the same link. A token from `--token` or `LOREFILE_TOKEN` is not echoed back: the lines say where it came from. Letters, digits and `- . _ ~ + / =` only, so it fits a header and a cookie; `openssl rand -base64 32` makes a good one.
- **The page.** Open the printed link once: the server keeps the token as a cookie (`lorefile-<port>`, `HttpOnly`, `SameSite=Strict`, 400 days) and takes it out of the address bar. Or paste the token into the login screen. Settings shows both MCP commands with the token masked and a Copy button, and a Sign out link.
- **Scripts.** Send `Authorization: Bearer <token>`. `POST /api/login {"token": …}` sets the cookie for a client that wants one; `POST /api/logout` clears it. Writes also need the `X-Lorefile: write` header, as before.
- **Agents over HTTP.** `POST /mcp` with the bearer header, `Content-Type: application/json`, one JSON-RPC message or a batch. A request gets a JSON reply; notifications get 202 with no body; `GET /mcp` answers 405 (no event stream) and `DELETE /mcp` 204 (no sessions). The page's cookie does not open `/mcp`, and a browser `Origin` that does not match the `Host` is refused. Claude Code:

  ```bash
  claude mcp add --transport http lorefile https://lorefile.example.com/mcp --header "Authorization: Bearer $LOREFILE_TOKEN"
  ```

- **Claude Desktop, claude.ai and ChatGPT.** Their connectors are reached from the vendor's cloud, not from the laptop, so they need a public HTTPS address and sign in with OAuth (next section). On a laptop without a tunnel, Desktop uses stdio (`claude_desktop_config.json`, or `.mcp.json` for Claude Code in a project folder).
- **Turning it off.** `--no-auth` prints one warning line and serves without a login, as before this change: fine on `127.0.0.1`, or behind a proxy that already signs people in (`--no-auth --i-know` with `--host 0.0.0.0`). Without a token nothing stops a web page the user visits from reaching a server on their machine through DNS rebinding, so keep `--no-auth` to trials.

### Connectors with OAuth

ChatGPT's custom connectors and Claude's (claude.ai, Desktop, mobile) offer only OAuth or no sign-in, and have no field for a bearer token. So Lorefile runs a small OAuth authorization server of its own, built to `specs/2026-11-27-oauth-connectors.md`. Status: built and tested end to end with a scripted client and tunnel-style headers; not yet tried with real ChatGPT and Claude accounts, or behind each tunnel and proxy (DEPLOY.md §8 keeps that list).

- **When it answers.** While the login token is on, for requests marked HTTPS (`Forwarded: proto=https`, `X-Forwarded-Proto: https` or `CF-Visitor`). The public address is read from each request: `Forwarded: host=`, then `X-Forwarded-Host`, then `Host`, first valid value wins, and `:443` is dropped. So any tunnel or proxy works without telling Lorefile its address. Plain `http://127.0.0.1` requests see no change: no new routes, the same `WWW-Authenticate: Bearer`.
- **The start-up lines** add `OAuth   on for any HTTPS address that reaches this server … · N connections`, or the reason it is off: `--no-oauth`, `--no-auth`, or a login token under 32 characters (the server still starts). An address read from requests is printed only after someone holding the login token approved a connection on it (a `Seen` line with its connector URL, never the token), because anyone can send a request with a made-up name. With `--public-url`, the first request on that address prints its connector URL and a sign-in link there, following the same rule as the `Open` line. If the login token changed since the last start, a line says how many connections that disconnected.
- **`--public-url https://name`** (or `LOREFILE_PUBLIC_URL`) pins sign-ins to that one address: other names get today's plain 401. Use it for a team server, or behind a proxy that cannot pass the name. It must be `https://` and a host, with no path. `--no-oauth` turns OAuth off everywhere.
- **A proxy that hides the name.** nginx sends `Host: 127.0.0.1:7070` unless told otherwise. A request marked HTTPS with a loopback name cannot be a public address, so OAuth stays off for it and one line says what to add: `proxy_set_header Host $host; proxy_set_header X-Forwarded-Proto $scheme;`. Lorefile must sit at the root of its address (a subdomain, not a path prefix).
- **The addresses it adds**, open without the login like `/healthz`, and only on an OAuth address: `/.well-known/oauth-protected-resource` (and `…/mcp`, `…/s/<id>/mcp`), `/.well-known/oauth-authorization-server`, `POST /oauth/register` (DCR), `GET`/`POST /oauth/authorize` (the approval page), `POST /oauth/token`, `POST /oauth/revoke`. The two metadata documents carry `Access-Control-Allow-Origin: *`; nothing else gets a CORS header. Every OAuth reply carries `Cache-Control: no-store` and `Vary` on the address headers. `/api/oauth/connections` (list, and `POST …/<id>/revoke`) needs the login, like every `/api/` route.
- **Who may connect.** Anyone may register a client (kept in memory until its first connection, so a registration writes nothing to disk; at most 200 waiting, and one still inside its 10-minute sign-in window is never pushed out), but its return address must be ChatGPT's (`https://chatgpt.com/connector_platform_oauth_redirect`, or `https://chatgpt.com/connector/oauth/<id>`), Claude's (`https://claude.ai/api/mcp/auth_callback`, and the same on `claude.com`), or loopback on any port, matched exactly; `--oauth-redirect <url>` adds others. PKCE S256 is required, and every reply from the approval step carries `iss` (RFC 9207).
- **Who approves.** Whoever holds the login token: they paste it on the approval page, or press Allow in a browser already signed in to the page on that address. Approval is never automatic. The page shows the client's name (its own claim), what it asks for, and the host it returns to; requests expire after 10 minutes (at most 20 waiting per client, so a flood under one client cannot push out another's); the form needs a one-time request ID and a same-origin POST (no `Origin`, or the page's own; `Origin: null` is refused), and cannot be framed. After 10 wrong tokens in 10 minutes, typed tokens are refused for 10 minutes, while the login token and a signed-in browser still get through. That only slows the noise down: what stops guessing is the 32-character minimum.
- **Tokens.** `lfa_…` access tokens last an hour; `lfr_…` refresh tokens are replaced on every use and expire after 30 days without use. An old refresh token still works for 60 seconds after it was replaced (a client retrying after a dropped reply); after that, presenting it revokes the connection. Codes last 60 seconds and are single use; a second use by the same client with the right verifier revokes what the first issued, while anyone else replaying a code just gets `invalid_grant`. Each token opens only the MCP address (`/mcp` or `/s/<id>/mcp`) and the public address it was approved on. `<dir>/.lorefile/oauth.json` (mode 0600, written atomically) keeps SHA-256 hashes, never the tokens; codes and approval requests live in memory only. To notice a changed login token it keeps an HMAC of it keyed by a random secret in `<dir>/.lorefile/oauth.key` (also 0600), so a leaked `oauth.json` alone gives nothing to test guesses against. Tokens survive a restart; a changed login token, or a missing `oauth.key`, drops every connection.
- **What a connection may do.** `lorefile:read` gives every read tool. `lorefile:propose` adds `propose_patch` and `propose_change`; without it they are left out of `tools/list` and refused. `--allow-edit` still decides whether a suggestion is written or queued. A one-space connection reads that space only (`resources/read` refuses another space's `ontology://` address, and `spaces` lists just that one), and no connection is told the folders the spaces live in. The activity log and proposals name the connection: `agent via MCP (connection "ChatGPT", approved 27 Nov)`. The login token, Claude Code's header, keeps working on `/mcp` as before; it is checked first.
- **Seeing and removing.** Settings › Connections: who, which address, which scopes, when approved, when last used, Disconnect, Disconnect all. A client's own `POST /oauth/revoke` works too.
- **Honest limits.** No person behind an approval, only the shared token. The vendors hold the tokens and see the answers. A client's name is its own claim; the return-address list is what limits where codes can go. No IP allowlist in Lorefile (a proxy can limit `/mcp` and `/oauth/token` to the vendors' published ranges; the approval page must stay reachable from the person's browser). DCR is deprecated in the MCP spec (2026-07-28, kept for at least 12 months); client ID metadata documents (CIMD) are the next step. One replica, as before. Rate limits are coarse, global rather than per caller.


---

## 5. SOC2: what the tool gives you, what you still do

SOC2 audits your controls, not a product. The product can make the evidence cheap. Mapped to the trust criteria you will be asked about:

| Criterion | What the product provides | What the deployment adds |
|---|---|---|
| **Logical access** (CC6.1–6.3) | A login token on every route but the health check; read-only by default; edit mode is a flag; writes refused from other origins; shipped examples immutable; credentials never in files. | The token in a secret manager and rotated, or SSO in front for a login per person; group membership for edit mode, read-only warehouse roles, quarterly access reviews. |
| **Change management** (CC8.1) | Every write is compiled and validated first; an agent's proposal waits for a person; each accepted change is a commit in the space's own history; `owner:` fields name reviewers. | Protected branch, pull-request review, CI running `lorefile validate` on every space. |
| **Monitoring and logging** (CC7.2–7.3) | Per-space activity log with who/what/when; git history. | Central log collection, retention, alerting on unexpected edits. |
| **Confidentiality** (C1) | The ontology holds meaning and locations, not customer rows. Example data is synthetic. DuckDB is fenced to declared folders. | Encrypted disks (cloud default), bucket policies, data classification of any `data/` files you add. |
| **Availability** (A1) | Stateless process, restarts in a second, state is files. | Health checks, backups, a restore test. |
| **Vendor and dependency risk** (CC9) | Open source, Apache-2.0, one binary, pinned `Cargo.lock`. | `cargo audit` in CI, pinned base images, a rebuild cadence. |
| **Tenant isolation** | Spaces are separate folders and catalogs in one process, and the SQL door is fenced to each space's own folders. That is not a security boundary between customers. | One container per customer is required, not optional; per-tenant secrets. (What a hosted version would run.) |

---

## 6. Licence

Use **Apache-2.0** for the open-source core (the format, CLI, MCP server, web page, examples and packs). The `LICENSE` file is in the repository and `Cargo.toml` says `license = "Apache-2.0"`.

Why that one:

| Option | What it means | Why not / why |
|---|---|---|
| MIT | Do anything, keep the notice. | Fine, but no patent grant, which enterprise legal teams notice. |
| **Apache-2.0** | Do anything, keep the notice, contributors grant patents. | The standard for infrastructure (Kubernetes, DuckDB, Airflow). Maximises adoption, which is the point of an open format. |
| AGPL-3.0 | Anyone who hosts a modified version must publish their changes. | Stops a cloud vendor from hosting it quietly, but makes companies hesitate to run it at all, and the ontology format should spread freely. |
| BSL / Elastic | Source-available, not open source; converts later. | Conflicts with "open source" in the pitch and with contributors' expectations. |

What Apache-2.0 does not give away: the name. The [NOTICE](NOTICE) file carries the attribution, and anyone who redistributes the binary or the source keeps it (Apache-2.0 §4d). Whether "Lorefile" can be registered as a trademark is still being checked, so the README makes no trademark claim yet. A hosted control plane, if one is built, would be a separate service around the same binary. Ask for a Developer Certificate of Origin (`Signed-off-by`) on contributions.

---

## 7. Docker with S3 or GCS: would people run it that way?

Yes, and in three shapes, from simplest to most durable:

```bash
# 1. A disk. One container, a named volume, the token from your secret manager.
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN="$LOREFILE_TOKEN" lorefile serve --dir /spaces --host 0.0.0.0 --allow-edit

# 2. A bucket mounted as the folder (GCS shown; S3 is the same with mountpoint-s3 or s3fs).
gcsfuse --implicit-dirs your-bucket /mnt/spaces
docker run -d -p 7070:7070 -v /mnt/spaces:/spaces -e LOREFILE_TOKEN="$LOREFILE_TOKEN" lorefile serve --dir /spaces --host 0.0.0.0 --allow-edit

# 3. Git as the home, a bucket as the backup (the recommendation from part 2).
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN="$LOREFILE_TOKEN" lorefile serve --dir /spaces --host 0.0.0.0 --allow-edit
# cron on the host or a sidecar:  git -C /var/lib/docker/volumes/lorefile-spaces/_data push  &&  rclone sync /var/lib/docker/volumes/lorefile-spaces/_data gcs:your-bucket/spaces
```

On Cloud Run, mount the bucket as a volume (Cloud Run supports GCS volumes) and run one instance. On Kubernetes, a `PersistentVolumeClaim` at `/spaces` and one replica. On a VM, a disk and `systemd`.
