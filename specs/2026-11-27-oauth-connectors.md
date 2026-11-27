# Spec: OAuth sign-in, so ChatGPT's and Claude's connectors can reach Lorefile

Written 2026-11-27 for Vinay to decide on. Revised the same day after Vinay's review: Lorefile now works out its public address from each request, so it works with any tunnel and needs no flag (decision 2, 2.1, Part 3). **Status: built 2026-11-27**, option B with every decision line as recommended, including that revision. Tested through the real binary with a scripted client and tunnel-style headers, and the approval page and Settings › Connections checked by hand in a browser. **Not yet tried with real ChatGPT, claude.ai or Claude Code accounts, or through a real tunnel or proxy.** What differs from the text below is listed at the end, under "What changed during the build".

Today the only way to use Lorefile from ChatGPT is to switch the login off and put a public tunnel in front, so anyone with the URL can read the whole server. This spec adds a real sign-in for the two connectors that need one: ChatGPT's custom connectors (apps in developer mode) and Claude's custom connectors (claude.ai, Claude Desktop, mobile). Claude Code keeps its bearer header, unchanged.

The standing rules still hold ([BACKLOG.md](../BACKLOG.md)): the ontology never executes and never holds third-party credentials. Agents and builders write the YAML, people review. Status stays honest. Part 3 explains why OAuth breaks neither rule.

Read first: `src/serve.rs` (`resolve_login`, `signed_in`, `handle` and its `/mcp` branch, `mcp_http`, `login_page`), `src/main.rs` (`--token`, `--no-auth`, `--i-know`), `src/mcp.rs` (`Hub`), [PRODUCTION.md](../PRODUCTION.md) §4 "Signing in, and MCP over HTTP", [DEPLOY.md](../DEPLOY.md) §2B and §3, and BACKLOG.md item "OAuth for Claude Desktop's and ChatGPT's connectors".

---

## The gap, in one afternoon

**Before (today).** Vinay wants to ask ChatGPT about the Branda space on his laptop.

```
lorefile serve --dir spaces --no-auth      # with the login on, ChatGPT cannot connect at all
ngrok http 7070                            # → https://quiet-otter.ngrok-free.app

ChatGPT › Settings › Apps › Developer mode › Create
  MCP server URL   https://quiet-otter.ngrok-free.app/mcp
  Authentication   No authentication
```

It works, but nothing protects it. Anyone who has the URL can read every space, and with `--allow-edit` they can write too. The URL ends up in ChatGPT's settings, in ngrok's dashboard, in shell history and in screenshots. The `--i-know` guard does not catch this: Lorefile listens on 127.0.0.1, so it thinks only this machine can reach it, and it cannot see the tunnel. Claude Desktop's custom connectors have the same choice, sign in with OAuth or no sign-in at all, so on a laptop people fall back to stdio, which leaves out claude.ai on the web and on mobile.

**After.** Any tunnel, no extra flag:

```
lorefile serve --dir spaces                 # the login stays on
ngrok http 7070                             # or: cloudflared tunnel --url http://localhost:7070, or any other

Lorefile 0.2.0 · 1 space of yours in spaces, plus 8 shipped examples · edit mode off
Open    http://127.0.0.1:7070/?token=k3f9…   (token from spaces/.lorefile/auth.toml)
MCP     http://127.0.0.1:7070/mcp   Authorization: Bearer k3f9…
        claude mcp add --transport http lorefile http://127.0.0.1:7070/mcp --header "Authorization: Bearer k3f9…"
        or over stdio on this machine: lorefile mcp --dir spaces
OAuth   on for any HTTPS address that reaches this server (a tunnel or a proxy): give ChatGPT or Claude <that address>/mcp
Edit    off (start with --allow-edit to edit from the page)

… and the first time a request arrives through the tunnel:
Seen    https://quiet-otter.ngrok-free.app   connectors: https://quiet-otter.ngrok-free.app/mcp (choose OAuth)
        sign in there: https://quiet-otter.ngrok-free.app/?token=k3f9…
```

In ChatGPT he pastes the tunnel's URL with `/mcp` on the end and picks **OAuth**. A window opens on his own server:

```
┌────────────────────────────────────────────────────────────┐
│  Connect "ChatGPT" to Lorefile?                            │
│                                                            │
│  It asks to:                                               │
│    ✓ read every space on this server                       │
│    ☑ suggest changes (they wait for a person to accept)   │
│                                                            │
│  After you allow it, you go back to  chatgpt.com           │
│  Asked 12 seconds ago. Only allow it if you pressed        │
│  Connect yourself just now.                                │
│                                                            │
│  Login token  [·································]           │
│  (not shown when this browser is already signed in)        │
│                                                            │
│                               [ Deny ]   [ Allow ]         │
└────────────────────────────────────────────────────────────┘
```

He pastes the token (it is in the terminal) and presses Allow. ChatGPT says it is connected. Someone who only has the URL gets a 401 from `/mcp` and a page that asks for the token. Settings › Connections lists "ChatGPT · all spaces · read, suggest · approved 27 Nov · used 2 min ago [Disconnect]".

**A team server, after.** Same binary. The platform's HTTPS works like a tunnel, so no flag is needed. `--public-url` is there for a team that wants the server to answer sign-ins on one name only:

```
fly secrets set LOREFILE_TOKEN=$(openssl rand -base64 32)
# fly.toml:  serve --dir /spaces --host 0.0.0.0                                  (works)
#            serve --dir /spaces --host 0.0.0.0 --public-url https://acme-lorefile.fly.dev   (pinned to one name)
```

A Claude Team owner adds `https://acme-lorefile.fly.dev/s/brand-two/mcp` under Organization settings › Connectors. Each member presses Connect and sees the same page, scoped to "only the space brand-two". A member already signed in to the Lorefile page in that browser approves with one click. Anyone else pastes the shared token from the team's password manager.

---

## Decisions

One line each, with what I recommend. The parts below give the reasons. Approve, change or strike each line.

| # | Decision | Recommendation |
|---|---|---|
| 1 | The approach | **Option B, a small sign-in server built into Lorefile** (Part 4 compares it with a gateway in front, and with trusting an outside identity provider). |
| 2 | How OAuth turns on, and which address it uses | **On by itself, with any tunnel or proxy.** When the login token is on and a request arrives marked as HTTPS, Lorefile reads the public address from that request and runs OAuth on it (2.1 gives the rule). Plain `http://127.0.0.1` requests behave exactly as today. `--public-url https://…` is optional: it pins sign-ins to one address, for a team server or a proxy that hides the name. `--no-oauth` turns it off. With `--no-auth` there is no OAuth, and connectors use "No authentication" as today. |
| 3 | Who may approve a connection | Whoever holds the login token. A browser already signed in to the page on that address approves with one click. Anyone else pastes the token on the approval page. No accounts and no passwords. The approval is always shown and never automatic. |
| 4 | How clients register | Dynamic client registration (DCR), which both ChatGPT and Claude use when nothing else is offered. Anyone may register, but a client's return address (redirect URI) must be on a short list: ChatGPT's, Claude's, and this machine's loopback. `--oauth-redirect <url>` adds others. Client ID metadata documents (CIMD), which the MCP spec now prefers, come in phase 2 (Part 5). |
| 5 | Tokens | Random opaque strings (`lfa_…` access, `lfr_…` refresh), stored only as SHA-256 hashes in `<dir>/.lorefile/oauth.json` (mode 0600). An access token lasts 1 hour. A refresh token is replaced on every use and expires after 30 days without use. |
| 6 | What a connection may do | Two scopes: `lorefile:read` and `lorefile:propose`. The approval page lets the person untick "suggest changes". `--allow-edit` still decides whether a suggestion is written straight away or queued for a person. |
| 7 | Which spaces | Every MCP address is its own resource: `/mcp` (all spaces) and `/s/<id>/mcp` (one space). A token opens only the address it was issued for. |
| 8 | Claude Code's header | Unchanged. `Authorization: Bearer <login token>` keeps opening `/mcp`, so the header is the owner's key and OAuth tokens are the guests' keys. |
| 9 | Changing the login token | Disconnects every connection: grants made under the old token stop working on the next start, and the start-up line says so. That gives one lever that removes everything. |
| 10 | Seeing and removing connections | Settings › Connections: who, which address, which scopes, when approved, when last used, Disconnect, Disconnect all. |
| 11 | Issuer identification (RFC 9207) | On: every reply from the approval page carries `iss`. ChatGPT then uses its stable return address, and a token cannot be swapped between servers. |
| 12 | The activity log | A change suggested over a connection is logged as `agent via MCP (connection "ChatGPT", approved 27 Nov)`, not just `agent via MCP`. The name is the one the client gave itself. |
| 13 | A weak login token | OAuth stays off with a token shorter than 32 characters, because the approval page is on the public internet. The start-up line says why, and the server still starts, so an existing short `LOREFILE_TOKEN` breaks nothing. Generated tokens are 43 characters. |
| 14 | Scope of phase 1 | No CIMD, no outside identity provider, no per-person accounts, no IP allowlist in Lorefile. Each one is in Part 5 or Part 3. |

---

## Part 1 · What ChatGPT and Claude require today

Checked on 2026-11-27 against the MCP authorization spec (revision 2026-07-28, the newest) and the vendors' own docs (sources at the end). No vendor doc carries a date, so recheck before building.

| | ChatGPT (custom connector, developer mode) | Claude (custom connector: claude.ai, Desktop, mobile) | Claude Code |
|---|---|---|---|
| Connects from | OpenAI's servers (published egress ranges; it also presents an OpenAI client certificate) | Anthropic's servers, `160.79.104.0/21` | the user's own machine |
| Address it needs | public HTTPS | public HTTPS (the browser must reach the approval page too) | anything it can reach |
| Sign-in choices | OAuth, No authentication, Mixed. No API-key or bearer field. No machine-to-machine grant. | OAuth (CIMD, DCR, or your own client ID), No sign-in. A fixed request header (bearer token) is in beta for some organizations only. | `--header`, or OAuth through a loopback redirect |
| Discovery | 401 with `WWW-Authenticate: Bearer resource_metadata="…"`, or the well-known address. Then the authorization server's metadata (RFC 8414 or OpenID). | Same. Only a 401 starts sign-in. `resource` must equal the URL exactly as the user typed it, path included. Only the first authorization server listed is used. | Same |
| PKCE | `S256` must be listed in `code_challenge_methods_supported` | `S256` on every request, and listed | Same |
| Registration | CIMD if offered, else DCR (once per connection, reused), else a client ID typed in | CIMD if offered together with auth method `none`, else DCR as a public client, else your own client ID | CIMD (`https://claude.ai/oauth/claude-code-client-metadata`), else DCR |
| Return address | `https://chatgpt.com/connector_platform_oauth_redirect` when the server does RFC 9207 correctly, otherwise `https://chatgpt.com/connector/oauth/{callback_id}` | `https://claude.ai/api/mcp/auth_callback` (Anthropic may move it to `claude.com`) | `http://localhost:<any port>/callback`, `http://127.0.0.1:<any port>/callback` |
| `resource` parameter | sent on both the approval and the token request | sent (the MCP spec requires it) | sent |
| Refresh | expects normal expiry and replacement, gives no lifetimes | refreshes on a 401 and up to 5 minutes before expiry. Wants `invalid_grant` for a dead refresh token, and a replaced refresh token for public clients. | — |
| Token endpoint | form-encoded | form-encoded (415 otherwise). 10-second timeout, 30 seconds for refresh. | — |
| Who can add one | Developer mode on the web. OpenAI's guide lists Plus, Pro, Business, Enterprise and Edu. Its help centre names Business, Enterprise and Edu for full MCP support, and third-party guides say Plus and Pro only get read-only tools. **Check on the account you will use.** | Free (one connector), Pro, Max, Team, Enterprise. On Team and Enterprise an owner adds it and each member connects with their own account. | everyone |

**What the MCP spec (2026-07-28) asks of the server side**, in short:

- **The MCP server (the protected resource)** MUST publish protected-resource metadata (RFC 9728) with at least one entry in `authorization_servers`, and point to it from the 401, from the well-known address, or both. It MUST check that each token was issued for it (the audience, RFC 8707), MUST NOT accept any other token, and MUST NOT pass a token on to anyone. Invalid or expired tokens get a 401, and missing scopes SHOULD get a 403 `insufficient_scope`.
- **The authorization server** MUST speak OAuth 2.1, publish RFC 8414 or OpenID metadata whose `issuer` matches exactly, list `code_challenge_methods_supported`, serve everything over HTTPS, match return addresses exactly, and accept only `localhost` or HTTPS return addresses. It SHOULD issue short-lived access tokens, and MUST replace refresh tokens on every use for public clients. It SHOULD put `iss` on every reply from the approval step, and if it does, MUST say so with `authorization_response_iss_parameter_supported: true`. A later revision will make `iss` a MUST.
- **Registration.** Clients use, in order: a client ID they already hold, then CIMD if the server offers it, then DCR, then they ask the user. **DCR is deprecated as of 2026-07-28** but stays for at least 12 months. Servers SHOULD support CIMD, and a server that fetches client documents SHOULD guard against SSRF.

**What that means for Lorefile.** A connector gets in only through OAuth. The login token cannot be handed to ChatGPT, because it has no header field, and Claude's header field is beta for a few organizations only. Lorefile has to be its own small authorization server, or point to someone else's (Part 4). The smallest version both vendors accept today is: protected-resource metadata, authorization-server metadata, DCR, PKCE S256, form-encoded token endpoint, `iss`, exact return addresses, refresh tokens that are replaced on every use, and a public HTTPS address.

---

## Part 2 · The design (option B)

### 2.1 Which address: worked out from each request

OAuth needs absolute addresses: the issuer, the endpoints and the resource. They have to match, character for character, the address ChatGPT and Claude were given. Most people will not know that address in advance (a free tunnel gives a new one on each start), and every tunnel service is a little different. So Lorefile does not ask for it. It reads it from each request, the same way every web app behind a proxy does:

| What | Read from, first one present wins |
|---|---|
| The public name | `Forwarded: host=…` (RFC 7239), then `X-Forwarded-Host`, then `Host` |
| HTTPS or not | `Forwarded: proto=…`, then `X-Forwarded-Proto`, then `CF-Visitor` (`{"scheme":"https"}`), else plain HTTP |

Only the first value of each header counts, and a value that is not a plain host name (with an optional port) is ignored. The address is `https://<name>`, and the default port 443 is dropped.

- **When OAuth answers.** Only for requests marked HTTPS, and only while the login token is on. ChatGPT and Claude need HTTPS anyway. A request to `http://127.0.0.1:7070` sees no change at all: the same 401, no metadata, no new routes. That keeps every existing set-up and test as it is.
- **Several addresses at once.** An ngrok tunnel and a Cloudflare tunnel to the same server each get their own issuer, and each connection is tied to the address it was approved on. Settings › Connections shows which.
- **A proxy that hides the name.** nginx sends `Host: 127.0.0.1:7070` and no `X-Forwarded-Proto` unless told otherwise. A request marked HTTPS whose name is loopback cannot be a real public address, so Lorefile does not run OAuth on it, and prints once: "a proxy is hiding the public address; have it pass Host and X-Forwarded-Proto (nginx: `proxy_set_header Host $host; proxy_set_header X-Forwarded-Proto $scheme;`), or start with `--public-url`". A non-loopback name over plain HTTP gets a similar one-time hint: connectors need HTTPS.
- **`--public-url https://name` pins it.** Sign-ins are then answered only on that address. Requests through any other name get today's plain 401. This is for team servers and for proxies that cannot pass the name. It must be `https://` plus a host: no path, no query (`http://127.0.0.1:<port>` is allowed for tests).
- **TCP-only forwarders** (plain port forwarding, bore and the like) give no HTTPS, so they cannot serve ChatGPT or Claude. They still work for Claude Code with the header.
- **Lorefile must sit at the root of its address.** Behind a proxy that adds a path (`https://example.com/lorefile/`), the metadata addresses would not line up. Use a subdomain instead. This is a stated limit, not something we fix.
- **A new address means connecting again.** Tokens are tied to the address they were approved on. With a tunnel that changes its address on every start, ChatGPT keeps the old URL anyway, so you add the connector again. For daily use, a fixed tunnel name is easier (ngrok gives free accounts one; Cloudflare named tunnels have one). The README should say so, but nothing requires it.

Part 3 explains why trusting these headers is safe here, although it is a classic mistake elsewhere.

### 2.2 The addresses Lorefile adds

All of them are open without the login token, like `/healthz`. They answer only for requests marked HTTPS (2.1), and only on the pinned address when `--public-url` is set. Otherwise they return 404, as today. `<public>` below means the address worked out for that request, for example `https://quiet-otter.ngrok-free.app`.

| Address | What it is |
|---|---|
| `GET /.well-known/oauth-protected-resource` and `…/oauth-protected-resource/mcp` | Protected-resource metadata for the whole server (`resource` = `<public>/mcp`) |
| `GET /.well-known/oauth-protected-resource/s/<id>/mcp` | The same for one space (`resource` = `<public>/s/<id>/mcp`) |
| `GET /.well-known/oauth-authorization-server` | Authorization-server metadata (issuer = `<public>`) |
| `POST /oauth/register` | DCR (RFC 7591), JSON in and out |
| `GET /oauth/authorize` | The approval page |
| `POST /oauth/authorize` | Allow or Deny from that page |
| `POST /oauth/token` | Code for tokens, refresh for new tokens (form-encoded) |
| `POST /oauth/revoke` | A client giving a token back (RFC 7009) |
| `GET /api/oauth/connections`, `POST /api/oauth/connections/<id>/revoke` | For the page's Settings. These need the login, like every other `/api/` route. |

`/.well-known/openid-configuration` is not served. Lorefile is not an identity provider, and clients try the RFC 8414 address first. The two metadata documents carry `Access-Control-Allow-Origin: *`, since they are public and browser-based MCP tools read them. Nothing else gets a CORS header.

### 2.3 The flow, step by step

With ChatGPT as the example. Claude's is the same with its own return address.

1. ChatGPT posts `initialize` to `https://quiet-otter.ngrok-free.app/mcp` with no token. Lorefile answers `401` with
   `WWW-Authenticate: Bearer resource_metadata="https://quiet-otter.ngrok-free.app/.well-known/oauth-protected-resource/mcp", scope="lorefile:read lorefile:propose"`.
2. `GET /.well-known/oauth-protected-resource/mcp`:
   ```json
   { "resource": "https://quiet-otter.ngrok-free.app/mcp",
     "authorization_servers": ["https://quiet-otter.ngrok-free.app"],
     "scopes_supported": ["lorefile:read", "lorefile:propose"],
     "bearer_methods_supported": ["header"],
     "resource_name": "Lorefile" }
   ```
3. `GET /.well-known/oauth-authorization-server`:
   ```json
   { "issuer": "https://quiet-otter.ngrok-free.app",
     "authorization_endpoint": "https://quiet-otter.ngrok-free.app/oauth/authorize",
     "token_endpoint": "https://quiet-otter.ngrok-free.app/oauth/token",
     "registration_endpoint": "https://quiet-otter.ngrok-free.app/oauth/register",
     "revocation_endpoint": "https://quiet-otter.ngrok-free.app/oauth/revoke",
     "response_types_supported": ["code"],
     "grant_types_supported": ["authorization_code", "refresh_token"],
     "code_challenge_methods_supported": ["S256"],
     "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
     "scopes_supported": ["lorefile:read", "lorefile:propose"],
     "authorization_response_iss_parameter_supported": true }
   ```
   No `client_id_metadata_document_supported`, so both clients fall back to DCR. `offline_access` is not listed, as the MCP spec advises, but refresh tokens are always issued.
4. `POST /oauth/register` with `{"client_name": "ChatGPT", "redirect_uris": ["https://chatgpt.com/connector_platform_oauth_redirect"], "grant_types": ["authorization_code", "refresh_token"], "token_endpoint_auth_method": "none"}` returns `201` with `client_id: "lfc_8Qe…"`. If a client asks for `client_secret_post` or `client_secret_basic`, it gets a secret, shown once and stored only as a hash. ChatGPT's choice here is not documented, so this keeps both kinds working.
5. ChatGPT opens a window at `GET /oauth/authorize?response_type=code&client_id=lfc_8Qe…&redirect_uri=…&code_challenge=…&code_challenge_method=S256&state=…&scope=…&resource=https%3A%2F%2Fquiet-otter.ngrok-free.app%2Fmcp`. The client and its return address are checked **before anything else**. If either is wrong, the page shows an error and never redirects. If the rest is wrong (no `code_challenge`, a method other than `S256`, a `resource` that is not one of ours), the request goes back to the client as `error=invalid_request` with `iss`. Otherwise the approval page appears.
6. Allow returns `302 https://chatgpt.com/connector_platform_oauth_redirect?code=…&state=…&iss=https%3A%2F%2Fquiet-otter.ngrok-free.app`. Deny returns the same with `error=access_denied`.
7. `POST /oauth/token` with `grant_type=authorization_code&code=…&redirect_uri=…&client_id=…&code_verifier=…&resource=…` returns `{"access_token": "lfa_…", "token_type": "Bearer", "expires_in": 3600, "refresh_token": "lfr_…", "scope": "lorefile:read lorefile:propose"}` with `Cache-Control: no-store`.
8. Every `POST /mcp` from then on carries `Authorization: Bearer lfa_…`, and everything else works as today.

**Resource matching.** A `resource` is accepted if it is `<public>/mcp` or `<public>/s/<id>/mcp` for the address this request came through, ignoring a trailing slash. Codes and tokens remember the address (issuer) they were issued on. A token exchange or an MCP call through another address is refused. If it is missing, or is the bare origin (one doc suggests ChatGPT might send just the host), it means `<public>/mcp`. The real ChatGPT run (Tests) confirms which one it sends.

### 2.4 The approval page, and how the login token fits

The page is Lorefile's own login screen with a question on top: which client, which address (all spaces or one), which scopes, where you go back to (the redirect host, in bold), and how long ago the request was made.

- **Already signed in.** The popup arrives from another site, so the `SameSite=Strict` cookie is not sent with that first request. The page uses the trick the login screen already has: its script asks `/api/auth` from the same origin, where the cookie *is* sent, and hides the token box if the answer is "signed in". Then it is one click.
- **Not signed in.** The person pastes the login token. A correct token also signs that browser in, just as the login screen does (Sign out is in Settings), so the next approval is one click.
- **Behind a tunnel** the cookie belongs to the tunnel's address, not to `127.0.0.1`. That is why Lorefile prints a sign-in link for each new address the first time it sees one (2.8).
- **Forgery protection.** The form carries a one-time request ID, which expires after 10 minutes and is bound to the client, return address, PKCE challenge, resource, scopes, state and the address the page was shown on. The POST must come from the same origin (`Origin` equal to that address when the browser sends one), and the cookie is `SameSite=Strict`. The page sends `X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'`, so no other site can frame it and trick a click.

**Ways to approve without a password that were considered and not recommended:**

| Idea | Why not now |
|---|---|
| A pairing code printed in the server's terminal, typed into the page | It is the same effort as copying the token from that terminal, and on a team server the code would sit in shipped logs. |
| Approval requests queued in the page on `127.0.0.1` (like the proposals inbox), with the popup waiting until someone presses Allow there | Nice on a laptop, but no help on a team server, and it adds polling. It could come later if pasting the token proves annoying. |
| Approving automatically when the browser is signed in | That removes the one moment where a person can spot a request they did not start (Part 3). |

### 2.5 Tokens and where they live

`<dir>/.lorefile/oauth.json`, mode 0600, written to a temporary file and then renamed into place. `.lorefile/` already hides itself from git. It holds hashes, never tokens:

```json
{
  "version": 1,
  "login": "salted SHA-256 of the login token these grants were approved under",
  "clients": [
    { "id": "lfc_8Qe…", "name": "ChatGPT", "redirect_uris": ["https://chatgpt.com/connector_platform_oauth_redirect"],
      "auth": "none", "registered": "2026-11-27T10:02:11Z" }
  ],
  "connections": [
    { "id": "c3", "client": "lfc_8Qe…", "name": "ChatGPT",
      "resource": "https://quiet-otter.ngrok-free.app/mcp", "scopes": ["lorefile:read", "lorefile:propose"],
      "approved": "2026-11-27T10:02:40Z", "approved_with": "signed-in browser",
      "refresh": { "hash": "sha256:…", "expires": "2026-12-27T10:02:40Z" },
      "access": [ { "hash": "sha256:…", "expires": "2026-11-27T11:02:40Z" } ],
      "last_used": "2026-11-27T10:05:12Z" }
  ]
}
```

| Thing | Lifetime | Kept where |
|---|---|---|
| Authorization code | 60 seconds, single use. A second use revokes what the first one issued. | memory only (a restart mid-flow means pressing Connect again) |
| Approval request ID | 10 minutes, single use | memory only |
| Access token `lfa_…` | 1 hour | hash in the file, so a restart does not log every connector out |
| Refresh token `lfr_…` | replaced on every use, expires 30 days after last use. Presenting an old one revokes the connection, except within 60 seconds of replacement, which covers a client that retries after a dropped reply. | hash in the file |
| A connection | until Disconnect, a changed login token, or 30 days with no use | the file |
| A registered client with no connection | 24 hours, then deleted. At most 200 clients. | the file |

All tokens are 32 random bytes from the operating system in base64url (`new_token()` already does this), plus a prefix. The prefix lets people and secret scanners recognize what kind of token a leaked string is. Hashes are SHA-256: a 256-bit random token needs no slow hash. This is the one new dependency (`sha2`, small and pure Rust). There is no HTTP client and no JWT library.

### 2.6 Scopes and one-space connections

- `lorefile:read` gives every read tool: `summary`, `describe`, `search`, `validate`, `where_is`, `instances`, `query`, `spaces` and the rest.
- `lorefile:propose` gives `propose_patch` and `propose_change`. Without it, those two tools are left out of `tools/list`, and a call to them returns a tool error: "this connection may only read; disconnect and connect again with 'suggest changes' ticked". The edit flags still apply on top. Without `--allow-edit`, a suggestion waits for a person, exactly as over stdio.
- A token issued for `/s/brand-two/mcp` opens only that address. It does not open `/mcp` (all spaces) or another space. The resource must match exactly. That gives a team a way to connect ChatGPT to one brand.
- Asking for a resource of a space that does not exist gets metadata of the same shape as an existing one, so the metadata does not reveal which space names exist. `/mcp` then answers 404 after sign-in, as today.

### 2.7 What stays the same

| | After this change |
|---|---|
| stdio (`lorefile mcp`) | Unchanged |
| Claude Code with `--header "Authorization: Bearer <login token>"` | Unchanged, on `/mcp` and `/s/<id>/mcp`. The login token is checked first, then OAuth tokens. |
| Claude Code without a header, when OAuth is on | New and optional: it gets the 401 with metadata, opens the browser at the approval page through its loopback return address, and keeps its own tokens. |
| The page, the cookie, `?token=`, `/api/login` | Unchanged. The page's cookie still does not open `/mcp`. |
| `--no-auth` | Unchanged. No OAuth routes and no metadata, so connectors use "No authentication". |
| Plain HTTP (`http://127.0.0.1:7070`, or any request not marked HTTPS) | Byte for byte as today: no new routes, and the same `WWW-Authenticate: Bearer` on 401. The existing tests prove it. |
| `--no-oauth` | Byte for byte as today on every address, HTTPS included |
| `/mcp` Origin check, JSON-only, 4 MB cap, the same answers as stdio | Unchanged |
| MCP protocol version (`2025-06-18`) | Unchanged. Clients discover the sign-in from the 401, not from the version. |

### 2.8 The page and the start-up lines

- **Start-up lines** gain `OAuth` ("on for any HTTPS address that reaches this server", or the pinned address, or "off" with the reason: `--no-oauth`, `--no-auth`, or a token under 32 characters) and the number of connections. Lorefile cannot know a tunnel's address at start, so the **first HTTPS request through a new address** prints a `Seen` line: the connector URL (`<address>/mcp`, "choose OAuth, leave the client ID empty") and the sign-in link on that address. The sign-in link follows today's rule, so a token from `--token` or `LOREFILE_TOKEN` is not echoed. If the login token changed since the last start, one line says "the login token changed: N connections were disconnected".
- **The page knows its own address.** Opened through the tunnel, Settings › MCP for agents shows "ChatGPT and Claude: `https://quiet-otter.ngrok-free.app/mcp` [Copy]", taken from the address bar. Opened on `127.0.0.1`, it says "open this page through your tunnel to get the connector URL".
- **Settings › Connections:**

  ```
  Connections  (agents that signed in with OAuth)
    ChatGPT   all spaces        read, suggest   approved 27 Nov   used 2 min ago    [Disconnect]
    Claude    brand-two only    read            approved 27 Nov   used yesterday    [Disconnect]
    [Disconnect all]   Changing the login token also disconnects every one.
  ```

  Disconnect needs the login and the `X-Lorefile: write` header with a matching Origin, like every write. It does not need `--allow-edit`, because it changes no ontology.
- **Agents › Connect card**: when OAuth is on, three short recipes (ChatGPT, Claude, Claude Code) with the connector URL from the page's own address and a Copy button. When it is off, one line with the reason (`--no-oauth`, `--no-auth`, or a short token).

### 2.9 A laptop behind a tunnel, and a team server

| | Laptop behind a tunnel | Team server |
|---|---|---|
| Address | any HTTPS tunnel (ngrok, Cloudflare Tunnel, Tailscale Funnel, localtunnel, …). A fixed name saves re-adding the connector. | your domain, or the platform's (`<app>.fly.dev`), HTTPS from the platform or proxy. `--public-url` optional to pin it. |
| Lorefile listens on | `127.0.0.1` (the tunnel connects locally; no `--i-know`) | `0.0.0.0` behind the proxy |
| Login token | made on the first run, in `.lorefile/auth.toml` | `LOREFILE_TOKEN` from the secret manager, 32 characters or more |
| Who approves | you; one click once you have opened the `Seen` sign-in link | each member: one click if signed in to the page, else the shared token |
| `oauth.json` | in the spaces folder's `.lorefile/` | on the volume; one replica, as already required |
| When it is down | laptop asleep or tunnel stopped: the connector errors; tokens survive restarts | as any server |

---

## Part 3 · Security: what it stops, what it does not

| Threat | What stops it |
|---|---|
| Someone finds the URL | Every MCP call needs a token, and getting one needs the login token on the approval page |
| Guessing the login token on the approval page | The token's length: generated tokens are 256-bit, and OAuth stays off with a token under 32 characters, so guessing is out of reach. The comparison takes the same time whatever the input. After 10 wrong tries in 10 minutes, typed tokens are refused for 10 minutes (behind a tunnel every caller looks like 127.0.0.1, so this is global), while the login token and a signed-in browser still get through. That pause only slows the noise down and logs it; it is not what stops guessing. *(Changed after the security review: the pause used to block everyone, which let anyone keep approvals off.)* |
| A fake client registers its own return address to collect a code | Return addresses must be on the list, checked at registration and again at approval, with exact matching. Only the documented ChatGPT and Claude forms, plus loopback on any port, are allowed. `https://chatgpt.com.evil.example`, `https://chatgpt.com@evil.example` and `http://chatgpt.com` are all refused. The page shows the return host in bold. |
| Consent phishing: someone presses Connect in *their own* ChatGPT and sends you the approval link | No OAuth server can fully stop this. Your Allow would connect their account. The page says "only allow if you pressed Connect yourself just now" and shows the host you go back to. Requests expire after 10 minutes. (The "Asked N seconds ago" on the page counts from when the page opened, so on a phishing link it reads "just now": it is not a protection.) Every approval is listed in Connections with its time, and changing the token removes them all. |
| A stolen authorization code | PKCE S256 is required, so a code is useless without the secret the client kept. Single use, 60 seconds. A replay revokes what the code issued only when it comes from the same client with the right verifier; anyone else replaying it just gets `invalid_grant`, so a replay cannot be used to disconnect someone. |
| A stolen access token (a vendor breach, a log) | It lasts 1 hour and is tied to one MCP address. Disconnect, or change the login token. |
| A stolen refresh token | It is replaced on every use, and reuse of an old one revokes the connection. 30 days idle at most. |
| A leaked `oauth.json` (a backup, a misconfigured volume) | It holds hashes only, mode 0600, hidden from git. The login token's fingerprint is an HMAC keyed by a random secret in `oauth.key` (its own file, also 0600), so `oauth.json` alone gives nothing to test guesses of a human-chosen token against. |
| A web page the owner visits sends requests to the server | The approval POST must be same-origin, carry a one-time ID and the Strict cookie or the token. `/mcp` keeps its Origin check. The metadata is public on purpose. |
| Clickjacking the Allow button | `frame-ancestors 'none'`, `X-Frame-Options: DENY` |
| Using `/oauth/authorize` as an open redirect | A bad client or return address shows an error page and never redirects |
| Registration spam | Bodies up to 16 KB, at most 10 return addresses and a 100-character name. A client with no connection lives in memory only (no disk write per registration) and goes after a day. At most 200 wait at once: only one past its 10-minute sign-in window can make room, otherwise registration is refused (429). Approval requests: at most 20 waiting per client, its own oldest dropped first, so a flood under one client never pushes out another's. |
| Forged address headers (`Host`, `X-Forwarded-Host`, `Forwarded`) | See "Why reading the address from the request is safe here" below. In short: a forged name changes only the forger's own reply, and every grant stays tied to the address it was approved on. The terminal never shows an address read from a request until a connection was approved on it, and never with the token. *(Changed after the security review: the first version printed a `Seen` line with a sign-in link, token included, for any name a request claimed.)* |
| A code or token meant for another server (mix-up) | `iss` on every reply from the approval step, with the issuer the same everywhere |
| Token passthrough, confused deputy | Lorefile accepts only tokens it issued for its own addresses, and never sends a token anywhere |

**Why reading the address from the request is safe here.** Trusting `Host` or `X-Forwarded-Host` is a classic mistake in apps that put that name into links they email to other people, or into pages a cache keeps. Here, anyone can send any header, and these rules make that harmless:

1. **The address only shapes the sender's own reply.** A forged name produces metadata and an approval page that point at the forged name, and only the forger sees them.
2. **Every code, token and connection remembers the address it was approved on.** A real ChatGPT token for `quiet-otter` sent with a forged `evil.example` header is refused, because the address does not match. Sent with the right header, it is simply the owner of that token using it as intended.
3. **Approving still needs the login token**, whatever address the page claims to be on.
4. **Where a code goes back to never comes from the address.** It comes from the client's registered return address, which must be on the list (ChatGPT, Claude, loopback).
5. **Nothing is cached or reused across requests.** Every OAuth reply carries `Cache-Control: no-store` and `Vary: Host, X-Forwarded-Host, X-Forwarded-Proto, Forwarded`, so a cache in front cannot serve one address's metadata to another.
6. **No outbound requests.** Lorefile never fetches anything from the address it was given.

**What it does not do (honest limits):**

- **No person behind the approval.** Approving means holding the shared token. The log names the connection and the name the client gave itself, not a person. Per-person accounts and SSO stay in the backlog (option C, Part 5).
- **The vendors hold the tokens and see the answers.** Definitions, rules and metric logic flow into ChatGPT or Claude under their terms. The rows stay in the data MCPs, as today, but the ontology itself is shared with the vendor once you connect.
- **A client's name is its own claim.** A client can call itself "ChatGPT". The return-address list is what actually limits where codes can go.
- **The whole server is public behind the token.** A tunnel exposes the page as well as `/mcp`, just as a team server does.
- **No IP allowlist in Lorefile.** A proxy can limit `/mcp` and `/oauth/token` to Anthropic's `160.79.104.0/21` and OpenAI's published ranges. The approval page must stay reachable from the person's own browser.
- **DCR is deprecated in the MCP spec** (2026-07-28, kept at least 12 months). Both vendors still use it when CIMD is not offered. CIMD is phase 2.
- **One replica**, as today. Two servers on one `oauth.json` would race.
- **Coarse rate limits**, global rather than per caller.

**Still a keeper.** The tokens Lorefile issues and stores are keys to Lorefile itself, made by Lorefile. It never receives or stores an OpenAI, Anthropic or other third-party credential: the clients are public PKCE clients, and any client secret is one Lorefile issued. In phase 1 it makes no outbound request at all, which is one reason CIMD waits. The tools are unchanged (read, validate, propose), so a connected ChatGPT can do exactly what Claude Code over stdio can do, and no more. Nothing gets executed.

---

## Part 4 · Three options

| | A. A gateway in front (docs only) | **B. A small sign-in server in Lorefile** (recommended) | C. Trust an outside identity provider |
|---|---|---|---|
| What it is | An OAuth gateway made for MCP, or an identity-aware proxy that speaks MCP's sign-in (several cloud providers and open-source projects offer one), in front of Lorefile. Lorefile runs `--no-auth` on loopback behind it, or the gateway adds the bearer header. | Part 2: metadata, DCR, an approval page that uses the existing token, and opaque tokens in `.lorefile/oauth.json` | `--oauth-issuer https://acme.okta.com`: Lorefile publishes resource metadata pointing at Okta, Entra, Auth0, Keycloak or Google, and checks their signed (JWT) tokens |
| One person on a laptop | Needs a cloud account, a domain and the gateway's setup. Harder than the workaround it replaces. | Nothing extra: start any HTTPS tunnel | Needs an identity provider account and an app registration. Too much. |
| Team server | Good if the team already has such a gateway | Good: the shared token, now with per-connection control | Good, with real per-person sign-in and SSO |
| Code in Lorefile | None (a tested recipe in DEPLOY.md) | About a week: one new module, routing, the approval page, Settings, tests. One small crate. | 2 weeks or more: an HTTPS client at runtime, a JWT library, key rotation, per-provider docs. Many providers do not offer DCR or CIMD (Entra also needs the MCP URL registered as an application ID URI). |
| Outbound network from the binary | No | No | Yes (fetching the provider's public keys) |
| Who is behind a change | the gateway's user, if it forwards one | the connection's name | a named person |
| Risk | We document someone else's moving product | DCR deprecation (Part 5 covers it) | Setup friction, and a test matrix per provider |

**Recommendation: B now, with C kept as the later step.** B is the only option that makes the laptop case better than `--no-auth`, and it turns the team server into something a ChatGPT or Claude workspace can use, without new infrastructure, accounts or outbound calls. Build it so that C fits in later: the `/mcp` check becomes one function, `caller(request, resource) -> Option<Caller>`, which today answers "owner (login token)" or "connection c3 with these scopes". Later it can also answer "person priya@acme.com, from the issuer's token". The hosted Stella version would then run its own issuer with accounts and SSO, and the same open-source binary would only check its tokens.

---

## Part 5 · Later

1. **CIMD (phase 2, small to medium).** Advertise `client_id_metadata_document_supported: true`, and when a `client_id` is an HTTPS URL, fetch that document (a JSON file the client publishes, listing its name and return addresses). Fetch only from a short host list (chatgpt.com, claude.ai), with a timeout, a size cap and caching, and check that its `client_id` and return addresses match. This needs an HTTPS client at runtime (ureq with rustls is already in the lockfile, but only as a DuckDB build dependency) and Lorefile's first outbound request. Do it when either vendor stops using DCR, or by mid-2027 when DCR's 12-month window closes, whichever comes first.
2. **An outside issuer (option C)**, with accounts and SSO (BACKLOG "Accounts and single sign-on"), and the hosted version's issuer.
3. **Approval from the local page** (2.4), if pasting the token proves annoying.
4. **Tool annotations** (small, separate). Marking read tools `readOnlyHint: true` may stop ChatGPT asking for confirmation on every read (unverified; Lorefile's tools carry no annotations today).
5. **A `Host` allowlist** (BACKLOG item 3) for `--no-auth`, against DNS rebinding. Separate from this work: OAuth needs no allowlist because grants are tied to their address (Part 3).

---

## Build notes, file by file

| File | What changes |
|---|---|
| `src/oauth.rs` (new) | Working out the address from each request (2.1: `Forwarded`, `X-Forwarded-Host`, `Host`, the HTTPS marks, host-name checking, the loopback refusal) and the optional pin. The return-address list. The store (`oauth.json` load and save, atomic, 0600, pruning). PKCE S256 checking. Return-address matching. Resource canonicalisation. Handlers for the two metadata documents, register, authorize (GET page, POST decision), token, revoke. `check(bearer, resource) -> Option<Grant>`. The approval page HTML as a const next to `LOGIN_HTML`'s style. The failed-try limiter. An injectable clock for tests. |
| `src/serve.rs` | Route `/.well-known/oauth-*` and `/oauth/*` before the login gate (next to `/healthz`). In the `/mcp` branch, replace `signed_in(req, ctx, false)` with `caller(...)`: login token first, then `oauth::check`, and when OAuth is on, the 401 carries `resource_metadata` and `scope`. `Ctx` gains `oauth: Option<Arc<oauth::Server>>`. Pass the caller into `mcp_http` and on to `Hub`. Add `/api/oauth/connections` (list, revoke). Add the start-up lines, plus the one-time `Seen` line per new address and the one-time proxy hints. |
| `src/mcp.rs` | `Hub` gains `can_propose: bool` and an author label. `tools/list` leaves out the propose tools when it is false, and `tools/call` refuses them with the sentence from 2.6. Proposals and the activity log use the label. |
| `src/main.rs` | `--no-oauth`, optional `--public-url` (env `LOREFILE_PUBLIC_URL`) and repeatable `--oauth-redirect`. Checks on `--public-url`: https unless loopback, no path, no query. A login token under 32 characters leaves OAuth off with a line, and does not stop the start. |
| `src/ui/index.html` | Settings › Connections, the connector URL from the page's own address (2.8), and the Agents › Connect recipes |
| `Cargo.toml` | `sha2` |
| Docs | README: the ChatGPT footnote at the top, §4 "Connect your AI tool" (ChatGPT, Claude, Claude Code recipes), the serve line in the CLI list, the test count. PRODUCTION.md §4: the Login and MCP rows, the Claude Desktop bullet, a new subsection "Connectors with OAuth". DEPLOY.md: §2B, the Fly notes (works as is; `--public-url https://<app>.fly.dev` to pin), a short "tunnels" note (any HTTPS tunnel; a fixed name saves re-adding; the two nginx lines), status rows. BACKLOG.md: strike item 2 and add CIMD and the outside issuer. `fly.toml` comment. Then `python3 scripts/build-tutorial.py`. |
| `tests/oauth_http.rs` (new) | Below |

**Hand-off plan.** One build session, in order: `oauth.rs` with `serve.rs` and `main.rs` and their tests, then the scopes in `mcp.rs`, then the page, then the docs. Then one review session (the security table above is its checklist). Then a manual check by Vinay with real accounts, because it needs ChatGPT, Claude and an ngrok name.

---

## Tests

**Unit (in `oauth.rs`):**
- PKCE: the RFC 7636 test vector (verifier `dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` gives challenge `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM`). `plain` is refused, and so is a missing challenge.
- Return addresses: the exact ChatGPT, Claude and loopback forms pass (loopback on any port, `localhost`, `127.0.0.1`, `[::1]`). These are refused: `https://chatgpt.com.evil.example/…`, `https://chatgpt.com@evil.example/…`, `http://chatgpt.com/…`, a `#fragment`, `https://evil.example/?u=https://chatgpt.com`, and `http://192.168.1.5:3000/callback`.
- Resources: `/mcp`, `/mcp/`, `/s/brand-two/mcp`, a bare origin (which means `/mcp`), another host, and `/s/../mcp` are each mapped or refused correctly.
- Store: tokens never appear in the file, only hashes. Saving is atomic. Pruning works. A changed login fingerprint drops grants.
- Lifetimes with a fake clock: code after 60 seconds, access after 1 hour, refresh after 30 idle days, the 60-second grace window after replacement.

**Through the real binary (`tests/oauth_http.rs`, no network):**
1. Plain HTTP requests (no HTTPS mark) get 404 from the `/.well-known/*` and `/oauth/*` routes, and the `/mcp` 401 header is exactly as today. All existing tests pass unchanged. With `--no-oauth`, the same holds for HTTPS-marked requests too.
2. Requests with `Host: quiet-otter.example` and `X-Forwarded-Proto: https`: both metadata documents have the fields in 2.3 with `https://quiet-otter.example`, the issuer is identical everywhere, and the 401 carries `resource_metadata`. The scripted flows below all run this way, with no network.
3. The address rule, one case each: `X-Forwarded-Host` beats `Host`; `Forwarded: host=…;proto=https` beats both; `CF-Visitor: {"scheme":"https"}` counts as HTTPS; only the first of several comma-separated values is used; `:443` is dropped and other ports kept; a name with a slash, space, `@` or a path is ignored. HTTPS-marked with `Host: 127.0.0.1:7070` and no forwarded name gives no OAuth, plus the proxy hint once.
4. Two addresses at once: a connection approved through `a.example` gets a 401 through `b.example`, and a code issued on `a.example` is refused at `b.example`'s token endpoint.
5. Pinned: with `--public-url https://a.example`, HTTPS requests through `b.example` get today's plain 401 and 404s.
6. The whole flow, scripted: register (loopback return) → GET authorize (page, no redirect) → POST Allow with the token → 302 with `code`, `state` and `iss` → token exchange → `initialize` and `tools/list` with the access token give the same bytes as with the login token → refresh → the old refresh token is refused (after the grace window) and the connection is revoked.
7. Refusals: a wrong verifier (`invalid_grant`); a code used twice (`invalid_grant`, and what it issued stops working); a return address not on the list at registration (`400 invalid_redirect_uri`); an unregistered return address at approval (an error page, no `Location`); Deny (`access_denied` with `iss`); a POST Allow with no request ID or a foreign `Origin` (403); a wrong token on the page (the page again; paused after 10 tries); a token form-encoded as JSON at `/oauth/token` (400 or 415).
8. Audience: a token for `/s/brand-two/mcp` gets a 401 at `/mcp` and at `/s/other/mcp`. An expired token gets a 401 with `resource_metadata`.
9. Scopes: a read-only connection's `tools/list` has no propose tools, and `tools/call propose_patch` gets the tool error. With both scopes and no `--allow-edit`, a proposal is queued, as over stdio.
10. Restarts: the same token means access and refresh still work. A different `LOREFILE_TOKEN` means a 401 and `invalid_grant`, plus the start-up line.
11. `oauth.json` has mode 0600 and none of the issued token strings appear in it.
12. Claude Code's bearer header still works with OAuth on (the existing `tests/mcp_http.rs` cases, run again with the HTTPS-marked headers).
13. `--no-auth` gives no OAuth routes even for HTTPS-marked requests. A 20-character `LOREFILE_TOKEN` starts the server with OAuth off and the reason in the start-up line.

**By hand, once (results go into DEPLOY.md's status table):**
- ChatGPT developer mode → create a connector with an ngrok URL and OAuth → approve → ask a question that calls `summary` → Disconnect in Settings → the next question fails and ChatGPT offers to reconnect. Note which `resource` and which `token_endpoint_auth_method` ChatGPT sent, and whether it used the stable return address.
- claude.ai → Settings › Connectors → add the same URL → approve → use it in Claude Desktop and on mobile. Try a `/s/<id>/mcp` connection too.
- Claude Code → `claude mcp add --transport http lorefile https://…/mcp` with no header → `/mcp` → authenticate in the browser.
- The MCP Inspector's sign-in check against the same URL, as a second opinion on the metadata.
- Tunnels: the ChatGPT or Claude flow once each through ngrok, Cloudflare Tunnel (quick and named), Tailscale Funnel and localtunnel, and once behind Caddy and behind nginx with the two `proxy_set_header` lines. Record which header each one passes the name and HTTPS in. If one passes neither, it goes in the README with `--public-url` as its recipe.

---

## Sources

Checked 2026-11-27. The vendor pages carry no dates.

- MCP authorization, revision 2026-07-28: [overview](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/index), [authorization server discovery](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/authorization-server-discovery), [client registration](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/client-registration), [security considerations](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization/security-considerations); the previous revision, [2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)
- OpenAI: [Apps SDK authentication](https://developers.openai.com/apps-sdk/build/auth) (the same page as [plugins/build/auth](https://developers.openai.com/plugins/build/auth)), [developer mode](https://developers.openai.com/api/docs/guides/developer-mode), [Connectors in ChatGPT](https://help.openai.com/en/articles/12584461-connectors-in-chatgpt)
- Anthropic: [connector authentication](https://claude.com/docs/connectors/building/authentication), [adding an unlisted connector](https://claude.com/docs/connectors/custom/add-unlisted), [MCP tunnels](https://claude.com/docs/connectors/mcp-tunnels) and [OAuth through tunnels](https://claude.com/docs/connectors/mcp-tunnels/oauth) (an Enterprise research preview for private networks, not for a laptop)
- RFCs: 6749 and the OAuth 2.1 draft, 7591 (DCR), 7636 (PKCE), 7009 (revocation), 8252 (loopback redirects), 8414 (server metadata), 8707 (resource indicators), 9207 (`iss`), 9728 (protected-resource metadata)

---

## What changed during the build

Built 2026-11-27 on the branch `claude/oauth-connectors-956d8a`. New: `src/oauth.rs` and `tests/oauth_http.rs`. Changed: `src/serve.rs`, `src/main.rs`, `src/mcp.rs`, `src/ui/index.html`, `Cargo.toml` (`sha2` 0.10, the only new crate), the docs, `fly.toml`'s comment and `.claude/launch.json`. `cargo test` runs 126 tests (95 before; 114 at the first build, 12 more after the security review), and `cargo clippy --all-targets -- -D warnings` and `cargo check --no-default-features` pass on Rust 1.87, the only toolchain on the build machine. CI's newer clippy has not seen it yet.

**Where the build differs from the text above, and why:**

1. **The PKCE test vector had a typo.** The challenge for RFC 7636's verifier is `…K1t8URW…`, not `…K1t2URW…` (RFC 7636, Appendix B). Fixed in "Tests" above. The code was right; the first test run caught it.
2. **"404 from the routes" over plain HTTP.** Today an unknown route answers 401 without the login (the login gate) and 404 with it. The rule "byte for byte as today" wins, so over plain HTTP, through a name that is not the pinned one, with `--no-oauth`, and with a short token, the OAuth addresses answer exactly like that: 401, or 404 when signed in. Test 1 checks both.
3. **`/api/oauth/connections` exists in every mode.** With OAuth off it answers `{"on": false, "reason": "no-oauth" | "no-auth" | "short-token", "why": "…"}`, so the page can say why (decision 2.8). It is a new route behind the login, so `--no-oauth` is byte for byte as before on every existing route, not on this new one.
4. **Claude's return address is allowed on `claude.com` as well as `claude.ai`**, since Part 1 says Anthropic may move it. Same path, exact match.
5. **A client that names no `token_endpoint_auth_method` is registered as public (`none`)**, not with RFC 7591's default `client_secret_basic`. Every MCP client is public, and the reply says which method it got.
6. **Scopes.** Unknown scopes are ignored, `lorefile:read` is always included, and a request that names none of Lorefile's scopes asks for both: the person can untick "suggest changes" on the page.
7. **Detecting an old refresh token.** Each connection keeps the hashes of its last 20 replaced refresh tokens (`refresh.retired` in `oauth.json`). One of those within 60 seconds of its replacement gets a fresh pair (a retry); after that it revokes the connection. A refresh token more than 20 rotations old is just unknown: `invalid_grant`, and nothing is revoked.
8. **`Referrer-Policy: same-origin` on the approval page, not `no-referrer`.** With `no-referrer`, browsers send `Origin: null` on the form's POST, which the same-origin check would refuse. `same-origin` sends no referrer to ChatGPT or Claude either.
9. **While the form is paused** (10 wrong tokens in 10 minutes), a wrong typed token gets 429 and is not counted. *Superseded after the security review (fix 6):* the first build also refused the right token and a signed-in browser.
10. **The 401 from `/mcp` on an OAuth address adds `error="invalid_token"`** when a token was sent (RFC 6750), and its JSON body says that connectors sign in with OAuth. The 401 on plain HTTP is unchanged.
11. **The page's cookie gets `Secure`** when the request is marked HTTPS by any of the three marks (`Forwarded: proto=https`, `X-Forwarded-Proto`, `CF-Visitor`), not only `X-Forwarded-Proto`. Plain HTTP is unchanged.
12. **Small additions.** The server metadata also lists `revocation_endpoint_auth_methods_supported`. The `Seen` line stops after 50 addresses. Codes are capped at 500 in memory. *(The first build's pending-request cap of 500, oldest by the second, was replaced after the security review: fix 5.)* A read-only connection's `initialize` instructions leave out the propose sentences and say it may only read.
13. **The approval page's "Asked N seconds ago"** counts from when the page was first shown. Lorefile cannot know when the person pressed Connect, and every request still expires 10 minutes after the page was shown. It is informational only and is no longer listed as a protection in Part 3.
14. **Where the approval page lives.** Its HTML is a const in `src/oauth.rs`. The login screen's CSS became `PAGE_STYLE` in `src/serve.rs`, shared by the login screen (whose output is unchanged byte for byte), the approval page and its error page.
15. **The connection label's date** is the approval day in UTC ("approved 27 Nov").
16. **`.claude/launch.json`** gained `ontology-oauth` (port 7078, `--public-url http://127.0.0.1:7078`), used for the browser check. It writes `.lorefile/` into `examples/supply-chain` like `ontology-login` does; delete that folder afterwards.

**What was checked, and how:**

- Unit tests in `src/oauth.rs`: PKCE, return addresses, resources, the address rule, the store (hashes only, atomic, 0600, pruning, a changed login), lifetimes on a fake clock, one-address tokens and the tick box, the failed-try pause.
- `tests/oauth_http.rs`, 11 tests through the real binary with no network, covering the 13 cases in "Tests". Time passing (an expired access token, the end of the grace window) is done by editing `oauth.json` between two starts, not by waiting.
- In the in-app browser, against `ontology-oauth`: the approval page signed out, with a wrong token (the error, status 401), and signed in (one click, no token box). Allow redirected to a loopback listener with `code`, `state` and `iss`. The code was exchanged and `tools/list` answered. Settings showed the connector URL and the connection ("ChatGPT · all spaces · read, suggest · approved 27 Nov with the signed-in browser · used just now"), Disconnect gave the token a 401, and the empty list survived a restart. Agents › Connect showed the three recipes. Start-up and `Seen` lines were read from the server log. No console errors.

**Not verified (needs real accounts, so it is Vinay's by-hand run in "Tests"):** ChatGPT developer mode, claude.ai, Claude Desktop and mobile, Claude Code without a header, the MCP Inspector, and every tunnel and proxy (ngrok, Cloudflare quick and named, Tailscale Funnel, localtunnel, Caddy, nginx, Fly). DEPLOY.md §8 lists each one as untested. Still open from the text: which `resource` and `token_endpoint_auth_method` ChatGPT sends, whether it uses the stable return address, and which plans can add a connector.

### After the security review (2026-11-27)

A separate read-only review (the security table in Part 3 as its checklist) found 10 problems and showed 7 of them with curl against a running server. Vinay approved fixing all 10. Each fix has a test.

1. **A one-space token could read every space.** `resources/read` took `ontology://<any space>/…` even on `/s/<id>/mcp`, and the `spaces` tool listed every space with its folder and the server's root. Now a server for one space (`/s/<id>/mcp`, or `lorefile mcp --space`) refuses another space's `ontology://` address and lists only its own space in `spaces`. No OAuth connection is told `dir` or `root`, or a repository path in `summary`'s History line. A connection asking for a space that does not exist gets a 404 naming no other space. The owner's header still gets the folders and the full list. Tests: `a_one_space_token_stays_in_its_space_and_is_not_told_where_files_live`.
2. **`Seen` put the login token next to a forged name.** One request claiming `X-Forwarded-Host: lorefile-signin.evil.example` printed `https://lorefile-signin.evil.example/?token=<login token>` in the owner's terminal, and 60 forged names filled the 50-line cap. Now an address read from requests is printed only after a connection was approved on it (that took the login token), and never with the token: `Seen    <address>   a connection was approved on this address; its connector URL is <address>/mcp`. With `--public-url`, the first request on that address prints its sign-in link, following the `Open` line's rule, since the operator named it. Forged names print nothing, so they cannot crowd out the real one. The README now tells people to use the address their tunnel printed. Tests: `seen_never_puts_the_token_next_to_a_name_read_from_a_request`.
3. **Anyone replaying a used code revoked the connection.** Now a replay revokes only when it comes from the same client with the right verifier; any other replay gets `invalid_grant` and nothing else. Tests: unit `a_replayed_code_revokes_only_for_its_own_client_with_the_right_verifier`, and the review's own case in `refusals`.
4. **A registration flood evicted clients mid-sign-in, and each registration wrote the file.** Now a client lives in memory until its first connection, which is when it is written to `oauth.json`. At most 200 wait at once. Only a client past the 10-minute sign-in window can make room; otherwise registration is refused with 429. This is a blend of "refuse at the cap" and the review's "evict only old ones": refusing outright would have let one flood lock registration for a whole day. Tests: unit `floods_never_push_out_a_client_or_a_request_that_is_signing_in`, and the store test checks that a registration leaves `oauth.json` unchanged.
5. **A pending-request flood evicted the request being approved** (the oldest was picked by the second, so at random). Now each client keeps at most 20 waiting, its own oldest dropped first by arrival order, so a flood under one client never touches another's request. There is also a global cap of 5,000, past which the page says to wait. Clients evicted under fix 4 lose their waiting requests too, so the global cap is out of reach for unconnected clients (200 × 20 = 4,000). Test: the same flood test.
6. **The pause blocked a signed-in browser too**, so anyone could keep approvals off with 10 POSTs every 10 minutes. Now the login token (typed, or in a bearer header) and a signed-in browser always get through; only wrong typed tokens get the 429, and they are not counted during the pause. The threat table now says that the 32-character minimum stops guessing, not the pause. Tests: unit `wrong_tokens_pause_typed_tokens_but_not_the_login_itself`, and the cookie case in `refusals`.
7. **The login fingerprint was one salted SHA-256**, so a leaked `oauth.json` let someone test guesses of a human-chosen `LOREFILE_TOKEN` offline. Now it is `hmac-sha256:` keyed by 32 random bytes in `.lorefile/oauth.key` (0600, its own file). A missing key means the fingerprint cannot be trusted, so connections are dropped (the safe side). Grants written by the first build's format are dropped once on upgrade; that build was never released. Test: unit `the_login_fingerprint_is_keyed_by_a_secret_in_its_own_file` (RFC 4231 vector included).
8. **Template substitution let a client name pull in other fields** (`{{host}} {{request}}`). The approval and error pages are now filled in one pass, and a value is never searched again. Test: unit `templates_are_filled_in_one_pass`.
9. **`POST /api/oauth/connections/revoke` with no id panicked.** No id, an empty id, an id with a slash, or an unknown id now answer 400. Test: `disconnect_needs_a_real_id`.
10. **The small ones.** An approval sent with the bearer header is recorded as "login token", not "signed-in browser". `redirect_uri` may be left out at the token endpoint when authorize left it out, and must match when authorize named it. "Asked N seconds ago" is no longer listed as a protection. Tests: `the_approval_post_takes_no_origin_but_refuses_origin_null` (the bearer case) and unit `redirect_uri_may_be_left_out_at_the_token_endpoint_when_authorize_left_it_out`.

Also added from the review's "not covered" list:
- Confidential clients, with `client_secret_post` and `client_secret_basic`, on both the exchange and a refresh (`confidential_clients_prove_their_secret`).
- Revoking an access token while the connection stays (`revoking_an_access_token_keeps_the_connection`).
- An approval POST with no `Origin` (allowed) and with `Origin: null` (refused), in `the_approval_post_takes_no_origin_but_refuses_origin_null`.
- Two refreshes at once, both succeeding and the connection surviving (`concurrent_refreshes_keep_the_connection`).

Still not verified: everything that needs real accounts or real tunnels, as listed above.
