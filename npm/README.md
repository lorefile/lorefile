# Lorefile

A team's business knowledge, written down in plain YAML files that AI agents read: what counts as a sale, where the orders live, which number the business runs on, what an agent may never do. One `lorefile` binary serves it three ways: an MCP server for agents, a CLI, and a web page for the people who review it.

This page is about installing it. The guide to using it is the project README on GitHub: <https://github.com/lorefile/lorefile#readme>.

## Install

Pick one. Each gives you the same `lorefile` command.

| Way | Command | Needs |
|---|---|---|
| npx | `npx lorefile serve` | Node.js 18 or newer |
| Docker | `docker run -p 7070:7070 -v lorefile-spaces:/spaces ghcr.io/lorefile/lorefile` | Docker |
| cargo | `cargo install lorefile --locked` | Rust 1.87 or newer |
| Install script | `curl -fsSL https://raw.githubusercontent.com/lorefile/lorefile/main/scripts/install.sh \| sh` | macOS or Linux |

### npx

```bash
npx lorefile serve --dir spaces
```

The first run downloads the Lorefile binary built for your computer from the project's GitHub Releases (the release with the same version as this package), checks its SHA-256 checksum against the one published with the package, and keeps it in `~/.cache/lorefile/<version>/` (under `$XDG_CACHE_HOME` if you set it; `%LOCALAPPDATA%\lorefile\` on Windows). Later runs start straight away. Nothing runs when the package is installed; the download happens on first use.

`npm install --global lorefile` gives you a plain `lorefile` command instead.

To run the MCP server for Claude Code over stdio:

```bash
claude mcp add lorefile -- npx -y lorefile mcp --dir /path/to/spaces
```

Other MCP clients (Claude Desktop, Cursor) take the same command in their config:

```json
{ "mcpServers": { "lorefile": { "command": "npx", "args": ["-y", "lorefile", "mcp", "--dir", "/path/to/spaces"] } } }
```

The launcher prints only to stderr, so stdout stays clean for the MCP messages.

### Docker

```bash
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN=choose-a-long-secret ghcr.io/lorefile/lorefile
```

Then open <http://127.0.0.1:7070>. Images are published for `linux/amd64` and `linux/arm64`, tagged with each version and `latest` (and `edge` for test builds). The container runs as a non-root user (uid 10001) and keeps your spaces on the `/spaces` volume. It starts read-only; to let the page and agents write, pass the command yourself:

```bash
docker run -d -p 7070:7070 -v lorefile-spaces:/spaces -e LOREFILE_TOKEN=choose-a-long-secret \
  ghcr.io/lorefile/lorefile serve --dir /spaces --host 0.0.0.0 --allow-edit
```

### cargo install

```bash
cargo install lorefile --locked
```

This compiles DuckDB from source, which takes 5 to 10 minutes. `cargo install lorefile --locked --no-default-features --features rdf` builds the slim version in a minute or two (see below).

### Install script (macOS and Linux, no Node needed)

```bash
curl -fsSL https://raw.githubusercontent.com/lorefile/lorefile/main/scripts/install.sh | sh
```

It downloads the latest release for your computer, checks the checksum, puts `lorefile` in `~/.local/bin` (or `/usr/local/bin` when run as root, e.g. with `sudo sh`), and prints what it did. Optional settings go before `sh`:

| Setting | What it does |
|---|---|
| `LOREFILE_VERSION=0.2.0` | that version instead of the latest |
| `LOREFILE_INSTALL_DIR=/opt/bin` | install there (asks `sudo` if the folder needs it) |
| `LOREFILE_SLIM=1` | the slim build |

For example: `curl -fsSL …/install.sh | LOREFILE_SLIM=1 sh`.

### Binaries

Every release on <https://github.com/lorefile/lorefile/releases> has an archive per platform, each with a `.sha256` file next to it, and one `checksums.txt`:

| Platform | Archive |
|---|---|
| macOS, Apple silicon | `lorefile-<version>-aarch64-apple-darwin.tar.gz` |
| macOS, Intel | `lorefile-<version>-x86_64-apple-darwin.tar.gz` |
| Linux x86_64 (glibc 2.35 or newer) | `lorefile-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 (glibc 2.35 or newer) | `lorefile-<version>-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x86_64 | `lorefile-<version>-x86_64-pc-windows-msvc.zip` |

macOS builds need macOS 11 or newer. On Alpine and other musl Linux, use Docker or `cargo install`.

**Full or slim.** Each platform comes in two builds. The full one (about 70 MB) includes DuckDB, so the `query` tool and the Data & SQL page can run SQL over local CSV, Parquet and DuckDB files. The slim one (`-slim` in the name, about 10 MB) leaves DuckDB out; everything else works the same, the Apache Ossie and OWL / SKOS / SHACL import and export included. `npx` uses the full build; if a release has no full build for your platform, it uses the slim one and says so.

## First run: the login token

`lorefile serve` asks for a token before it shows the page or answers the API, the way Jupyter does. On the first run it makes one, saves it in `<dir>/.lorefile/auth.toml`, and prints a login URL with the token in it:

```
Open  http://127.0.0.1:7070/?token=k3f9…
MCP   http://127.0.0.1:7070/mcp    Authorization: Bearer k3f9…
```

Open that URL once and the browser remembers you. To choose the token yourself, set `LOREFILE_TOKEN` (or pass `--token`); that is how Docker and servers get theirs. The same token lets agents use MCP over HTTP, which is what you need when Lorefile runs in a container:

```bash
claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer <token>"
```

## Launcher settings

| Variable | What it does |
|---|---|
| `LOREFILE_BINARY` | run this binary (say, your own build) instead of downloading one |
| `XDG_CACHE_HOME` | where the downloaded binary is kept: `<it>/lorefile/<version>/` |
| `LOREFILE_DOWNLOAD_URL` | a folder URL holding the release archives, for a mirror |
| `LOREFILE_REPO` | download from another GitHub repository (a fork) |

Behind a proxy that Node cannot get through, download the archive for your computer from the releases page and set `LOREFILE_BINARY` to the `lorefile` binary inside it.

## Licence

Apache-2.0. Each release archive carries the licences of the software compiled into it, DuckDB's included. Built by the team at [Stella](https://stellaheystella.com).
