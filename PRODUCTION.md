# Running Stella Ontology in production

The questions this answers: how several brands share one server, where the files live (a disk, git, or an S3/GCS bucket), whether DuckDB is "installed per brand", what a production deployment still needs, what SOC2 asks of it, and which open-source licence to use. Plain English first, commands second, honest status at the end of each part.

---

## 1. One server, many spaces

A **space** is one ontology: one brand, one business unit, one client. The server holds many of them.

**Before** (one folder per brand):

```
stella-ontology serve --dir brand-one --port 7070
stella-ontology serve --dir brand-two --port 7071      # two servers, two ports, two MCP entries
```

**After** (one folder of spaces):

```
spaces/
  brand-one/        ← a space: ontology.yaml, types/, constraints/, actions/, data/ …
  brand-two/
  .stella/          ← the server's own state; it writes a .gitignore that hides itself
    examples/       ← the examples and packs that ship inside the binary, rewritten at start

stella-ontology serve --dir ./spaces --allow-edit    # one server, one port
stella-ontology mcp   --dir ./spaces                 # one MCP server for every space
```

The page shows one space at a time and has a picker at the top:

```
┌──────────────────────────────────────────────────────────────────┐
│ [brand-one · 12 things ▾]  brand-one v0.1.0   12 things · 13 links │
│   Your spaces                                                      │
│     brand-one · 12 things                                          │
│     brand-two · 4 things                                           │
│   Examples that ship with it                                       │
│     ecommerce · 12 things        ← read-only, to see how it is done │
│     supply-chain · 10 things                                        │
│   Packs for common MCPs                                             │
│     meta-ads · 12 things                                            │
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

A server dedicated to one space is one flag away: `stella-ontology mcp --dir ./spaces --space brand-two`.

**What is shared and what is not.** Folders, catalogs, logs and proposals are per space. The process, its flags and its credentials are shared: `--allow-edit` applies to every space, and there is no login per space. That is the right shape for brands inside one company. For separate customers, run one container per customer (part 4): a process boundary is what an auditor wants to see, and it is what Stella Cloud will do.

Status: built. `validate --dir ./spaces` checks every space, for CI.

---

## 2. Where the files live: disk, git, or a bucket

An ontology is a folder of small text files. The binary reads and writes a folder; it does not talk to buckets itself. So the question is how that folder gets onto the machine and how it survives a restart. Three answers.

| | How it works | Good for | Watch out for |
|---|---|---|---|
| **A. A persistent disk** | Docker volume, EBS, a GCE persistent disk, mounted at `/spaces`. | The simplest start. One container, one disk. | One replica only (two servers on one folder would race). Back the disk up yourself. |
| **B. Git is the home** (recommended for teams) | The container clones your repository at boot into `/spaces`; `--commit-edits` turns every accepted change into a commit; a small loop pushes. The disk becomes disposable. | Review, history, rollback, and the change-management evidence SOC2 wants, for free. | Needs a deploy key in the container's secret store. Push is a one-line loop today, not a flag (below). |
| **C. A bucket is the home** (S3, GCS) | A bucket is not a filesystem, so either mount it (`gcsfuse`, AWS `mountpoint-s3`, `s3fs`) at `/spaces`, or sync a disk to it (`rclone sync` on a timer). | Backups, and the data files (Parquet) that sit next to the ontology. | Writes through FUSE are slow and sometimes surprising; the `.stella/` state folder works but is chatty. Use a bucket as the backup target, not as the working copy. |

**Recommendation.** Git for the ontology, a disk for the working copy, a bucket for data files and backups. Concretely:

```bash
# one-off: the repository that holds the spaces
git init spaces && cd spaces && git commit --allow-empty -m "start" && git remote add origin git@github.com:you/ontology.git

# the container: clone at boot, commit each accepted change, push every minute
docker run -d -p 7070:7070 -v stella-spaces:/spaces -e GIT_SSH_COMMAND="ssh -i /run/secrets/deploy_key" \
  stella-ontology serve --dir /spaces --host 0.0.0.0 --allow-edit --commit-edits
# entrypoint sketch:  git clone "$REPO" /spaces 2>/dev/null || git -C /spaces pull;  (while sleep 60; do git -C /spaces push -q; done) &
```

**Data on S3 or GCS.** The rows a space points at can stay in a bucket: a `source:` of kind `s3` (or `httpfs`) with `path: s3://bucket/prefix` makes DuckDB read Parquet straight from there, with credentials from the environment (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`, the AWS credential chain, or an HMAC key for GCS through its S3-compatible endpoint). The first use downloads DuckDB's `httpfs` extension, so the container needs outbound network once, or bake the extension into the image. Snowflake and BigQuery are never read by the binary: agents use their own connector and the user's own permissions.

Status: A works today. B works with the entrypoint sketch above (`--commit-edits` is built; clone-at-boot and push are not flags yet). C works with a FUSE mount; sync is your cron job.

---

## 3. DuckDB: a library, not a service

Nothing is installed per brand, or at all. DuckDB is compiled into the `stella-ontology` binary the way SQLite is compiled into an app. There is no daemon, no port, no file unless a source is a `.duckdb` file you point at.

What happens at runtime:

- The first time a space is opened, the server builds an **in-memory catalog** for it: six small tables holding the ontology itself (`nodes`, `edges`, `constraints`, …) and one **view per data-bound type** that reads the rows where they live (a CSV or Parquet file, a `.duckdb` file, Postgres, S3). So "one DuckDB per space", all inside one process, each a few megabytes.
- It is **read-only**: sources are attached `READ_ONLY`, a query is one `SELECT`/`WITH`, at most 500 rows.
- It is **fenced**. After the views are built, the catalog is told the only folders it may touch (the space's `data/` and any folder a source names), external access is switched off, and the configuration is locked. From then on `select * from read_csv('/etc/passwd')` and `install httpfs` fail. The `tables` tool reports `fenced: true`. The fence stays open only for a space with a remote source (Postgres, S3), because those need external access at query time; such a space still has only the credentials you gave the process.
- Memory and CPU are shared by every space in the process. For brands in one company that is fine. For separate customers, one container each (part 4) is also the resource boundary.

If you compile with `--no-default-features` there is no DuckDB at all: CSV and inline rows still work, `query` does not.

---

## 4. What production needs

The binary is ready to be put behind the things every internal web app sits behind. What it does not do itself is listed plainly.

| Area | Today | For production | Status |
|---|---|---|---|
| **Login** | None. `serve` binds `127.0.0.1`; with `--host 0.0.0.0` anyone who can reach the port can read, and write when `--allow-edit` is on. | An identity-aware proxy in front: Google IAP, Cloudflare Access, oauth2-proxy, Tailscale. One group may reach the page; a smaller group reaches it when edit mode is on. | Not built in; by design for v1. Stella Cloud adds SSO. |
| **MCP for remote agents** | stdio only (an agent on the same machine). | The HTTP transport with OAuth, per the MCP spec, so ChatGPT and hosted agents can connect. | Not built (roadmap). Today a laptop or a sidecar process runs `mcp`. |
| **TLS** | Plain HTTP. | Terminate TLS at the proxy or load balancer. | Standard. |
| **Edit mode** | Off by default. Writes are refused cross-origin and must be JSON. Shipped examples never take writes. | Turn it on only behind the proxy. Add `--commit-edits` so each change is a commit with who and why. | Built. |
| **Secrets** | YAML never holds credentials: a `source:` names an environment variable (`${SNOWFLAKE_PASSWORD}`, `env: PG_URL`). | Mount from your secret manager as environment variables. Use read-only database roles. | Built. |
| **Audit** | `.stella/activity.jsonl` per space (every edit, proposal, accept, dismiss, with who and when); git history when committing. | Ship the log and the proxy's access log to your SIEM. Retain per your policy. | Built (local file); shipping is yours. |
| **Backups** | Files on disk. | Git remote (part 2B) plus disk snapshots. | Yours. |
| **Health** | `GET /api/spaces` returns 200 when the server is up. | Point the load balancer's health check at it. A dedicated `/healthz` is roadmap. | Works. |
| **Resource limits** | DuckDB caps rows at 500 and one statement; no memory cap or query timeout. | Container memory limit; a `--duckdb-memory` flag is roadmap. Rate-limit at the proxy. | Partly. |
| **Container hardening** | Non-root user, no shell needed, one static binary. | Add `--read-only` root filesystem with `/spaces` and `/tmp` writable. | Built in the Dockerfile; image untested here (no Docker on the build machine). |
| **Replicas** | One process owns one spaces folder. | Run one replica per spaces folder. Two replicas on one shared disk would race on writes. | By design; a hosted control plane handles this. |
| **Releases** | Built locally. | CI that builds, runs `cargo test`, validates every example and pack, publishes signed binaries and an image, runs `cargo audit`, produces an SBOM. | Not built. |

**The three things an auditor will ask.** Who can change the ontology (the proxy's group plus edit mode plus git review). Who changed what (the activity log and git). What can an agent reach (read-only, fenced DuckDB, the rows it can see are the rows its own warehouse login allows).

---

## 5. SOC2: what the tool gives you, what you still do

SOC2 audits your controls, not a product. The product can make the evidence cheap. Mapped to the trust criteria you will be asked about:

| Criterion | What the product provides | What the deployment adds |
|---|---|---|
| **Logical access** (CC6.1–6.3) | Read-only by default; edit mode is a flag; writes refused from other origins; shipped examples immutable; credentials never in files. | SSO in front, group membership for edit mode, read-only warehouse roles, quarterly access reviews. |
| **Change management** (CC8.1) | Every write is compiled and validated first; an agent's proposal waits for a person; `--commit-edits` makes each change a commit; `owner:` fields name reviewers. | Protected branch, pull-request review, CI running `stella-ontology validate` on every space. |
| **Monitoring and logging** (CC7.2–7.3) | Per-space activity log with who/what/when; git history. | Central log collection, retention, alerting on unexpected edits. |
| **Confidentiality** (C1) | The ontology holds meaning and locations, not customer rows. Example data is synthetic. DuckDB is fenced to declared folders. | Encrypted disks (cloud default), bucket policies, data classification of any `data/` files you add. |
| **Availability** (A1) | Stateless process, restarts in a second, state is files. | Health checks, backups, a restore test. |
| **Vendor and dependency risk** (CC9) | Open source, Apache-2.0, one binary, pinned `Cargo.lock`. | `cargo audit` in CI, pinned base images, a rebuild cadence. |
| **Tenant isolation** | Spaces are separate folders and catalogs in one process. | One container per customer; per-tenant secrets. (What Stella Cloud will run.) |

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

What Apache-2.0 does not give away: the hosted control plane (SSO, tenants, billing, hosted MCP, audit export) stays in Stella's private repository, and the name "Stella" stays a trademark, so a third party can host the binary but not call it Stella. Add a one-line trademark note to the README and a Developer Certificate of Origin (`Signed-off-by`) for contributions.

---

## 7. Docker with S3 or GCS: would people run it that way?

Yes, and in three shapes, from simplest to most durable:

```bash
# 1. A disk. One container, a named volume, the page behind your proxy.
docker run -d -p 7070:7070 -v stella-spaces:/spaces stella-ontology serve --dir /spaces --host 0.0.0.0 --allow-edit

# 2. A bucket mounted as the folder (GCS shown; S3 is the same with mountpoint-s3 or s3fs).
gcsfuse --implicit-dirs your-bucket /mnt/spaces
docker run -d -p 7070:7070 -v /mnt/spaces:/spaces stella-ontology serve --dir /spaces --host 0.0.0.0 --allow-edit

# 3. Git as the home, a bucket as the backup (the recommendation from part 2).
docker run -d -p 7070:7070 -v stella-spaces:/spaces stella-ontology serve --dir /spaces --host 0.0.0.0 --allow-edit --commit-edits
# cron on the host or a sidecar:  git -C /var/lib/docker/volumes/stella-spaces/_data push  &&  rclone sync /var/lib/docker/volumes/stella-spaces/_data gcs:your-bucket/spaces
```

On Cloud Run, mount the bucket as a volume (Cloud Run supports GCS volumes) and run one instance. On Kubernetes, a `PersistentVolumeClaim` at `/spaces` and one replica. On a VM, a disk and `systemd`.
