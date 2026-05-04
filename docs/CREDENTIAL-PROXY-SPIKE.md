# Credential Proxy Spike

Status: planned (post-PR-train, parallel with A-series)
Owner: TBD
Related: [ENVIRONMENT-CONTRACT-ROADMAP.md](ENVIRONMENT-CONTRACT-ROADMAP.md)

---

## Goal

A native Rust credential proxy inside openhuman core that lets skills make authenticated outbound HTTP calls **without ever holding real credentials**. Closes the credential half of the Environment Contract without adding third-party runtime dependencies (Docker, Postgres, Next.js dashboards).

Pattern adapted from [qwibitai/nanoclaw](https://github.com/qwibitai/nanoclaw) and [onecli/onecli](https://github.com/onecli/onecli) (both Apache-2.0 / MIT). Architecture only — no code copy.

## History

An earlier revision of this doc proposed an Infisical-based HTTPS proxy. That plan was pivoted after reviewing nanoclaw + OneCLI: shipping Docker + Postgres + a Next.js dashboard as a runtime dep is the wrong shape for a desktop app, and OneCLI has no Windows-support signal. Borrowing the pattern and implementing natively in Rust gives equivalent security with zero third-party install on end-user machines and no Windows testing gap.

---

## Non-goals

- HTTPS MITM interception (skip in v1; skills use localhost HTTP proxy explicitly)
- Multi-user / team credential vault (single-user desktop app scope)
- Postgres-backed storage (use existing openhuman SQLite)
- Web dashboard (use existing Tauri UI)
- Replace OpenAI / Anthropic API key env vars in core (those are core-config, not skill-credentials)
- Adopt OneCLI as a runtime dep (wrong shape; borrow pattern instead)

---

## Threat model

| Attacker | Capability | Defence |
|---|---|---|
| Malicious skill | reads env vars, files in mount, `/proc` | Keys never in env/files; skill gets `FAKE_KEY` placeholder |
| Malicious skill | exfiltrates placeholder key | Placeholder is useless off-host; rate-limited per skill |
| Local attacker on disk | reads SQLite DB | AES-256-GCM at rest, master key in OS keychain |
| Local attacker in memory | dumps process memory | Accepted limitation — core process holds decrypted keys briefly at request time |
| Network attacker | sniffs proxy traffic | Proxy only listens on `127.0.0.1`; outbound uses real TLS to upstream |

---

## Architecture

```text
Skill process
  ↓ HTTP to 127.0.0.1:<proxy_port>
  ↓ Authorization: Bearer <skill_access_token>
openhuman-core credential proxy
  ↓ resolve (skill_token, host, path) → credential_id
  ↓ decrypt credential from SQLite
  ↓ swap placeholder header / body values with real credential
  ↓ HTTPS to upstream
Upstream API (OpenAI, Slack, GitHub, etc.)
```

All five components (proxy listener, resolver, store, decryptor, forwarder) live inside openhuman core. Nothing external.

---

## Module layout

New domain under `src/openhuman/credentials/`:

```text
src/openhuman/credentials/
  mod.rs               # re-exports; controller registration
  proxy.rs             # hyper-based HTTP proxy, listens on 127.0.0.1
  store.rs             # SQLite-backed encrypted credential storage
  crypto.rs            # AES-256-GCM wrapper over `aes-gcm` crate
  keychain.rs          # OS keychain access via `keyring` crate
  rules.rs             # host+path pattern matching, placeholder substitution
  schemas.rs           # controller schemas (add_credential, list, delete, etc.)
  rpc.rs               # JSON-RPC handlers
  types.rs             # Credential, ProxyRule, SkillToken
  ops.rs               # business logic
  bus.rs               # event subscriber (for audit log)
  store_test.rs        # sibling tests
  proxy_test.rs        # sibling tests
```

Wired into `src/core/all.rs` via `all_credentials_registered_controllers`.

---

## Data model (SQLite)

```sql
CREATE TABLE credentials (
  id              TEXT PRIMARY KEY,              -- uuid
  name            TEXT NOT NULL,                 -- user-visible label, e.g. "OpenAI prod"
  host_pattern    TEXT NOT NULL,                 -- e.g. "api.openai.com"
  path_pattern    TEXT,                          -- e.g. "/v1/*", NULL = any
  injection_kind  TEXT NOT NULL,                 -- 'header' | 'query' | 'body-json'
  injection_spec  TEXT NOT NULL,                 -- JSON: {name, template}
  ciphertext      BLOB NOT NULL,                 -- AES-256-GCM(nonce || ciphertext || tag)
  created_at      TEXT NOT NULL,
  last_used_at    TEXT
);

CREATE TABLE skill_tokens (
  token_hash      TEXT PRIMARY KEY,              -- sha256 of bearer token
  skill_id        TEXT NOT NULL,
  credential_ids  TEXT NOT NULL,                 -- JSON array, grants
  rate_limit_rpm  INTEGER,                       -- NULL = unlimited
  created_at      TEXT NOT NULL,
  revoked_at      TEXT
);

CREATE TABLE proxy_audit (
  id              TEXT PRIMARY KEY,
  skill_id        TEXT NOT NULL,
  credential_id   TEXT,                          -- NULL = no match, denied
  host            TEXT NOT NULL,
  path            TEXT NOT NULL,
  status          INTEGER,                       -- HTTP status from upstream
  ts              TEXT NOT NULL
);
```

Master encryption key lives in OS keychain under service `openhuman`, account `credential-vault-master-key`. Generated on first run; never in plaintext on disk.

---

## Crypto details (non-negotiable)

- Crate: `aes-gcm` (RustCrypto). Implements AES-GCM, a FIPS-approved algorithm; note that the RustCrypto crate itself is not a FIPS 140-3 validated module — callers requiring module validation must swap in a validated backend.
- Key size: 256 bit, generated via `rand::rngs::OsRng`
- Nonce: 96-bit random per encryption, stored as first 12 bytes of ciphertext blob
- **Never reuse a nonce with the same key** — enforced by always generating fresh via `OsRng`
- Keychain access: `keyring` crate (cross-platform: macOS Keychain, Windows Credential Manager, Linux libsecret)
- No custom crypto anywhere. If a reviewer reads this and sees custom crypto, reject.

---

## Controller schemas (user-facing)

| Method | Purpose |
|---|---|
| `openhuman.credentials_add` | Add a credential (name, host_pattern, path_pattern, injection, plaintext_value) |
| `openhuman.credentials_list` | List (no plaintext) |
| `openhuman.credentials_delete` | Delete by id |
| `openhuman.credentials_rotate` | Replace plaintext, preserve id/rules |
| `openhuman.skill_tokens_issue` | Issue a bearer token for a skill, granting N credentials |
| `openhuman.skill_tokens_list` | List active tokens |
| `openhuman.skill_tokens_revoke` | Revoke by hash |
| `openhuman.proxy_audit_query` | Recent audit events |

UI surface in Tauri app: new Settings pane "Credentials" with add/list/delete + per-skill grant UI.

---

## Proxy behaviour

- Binds `127.0.0.1` on an ephemeral port at core startup; port published via existing core RPC so skills can discover
- `Proxy-Authorization: Bearer <skill_token>` required on every request
- Matches `(host, path)` against skill's granted credentials; first match wins, sorted by `match_priority ASC`, then `created_at ASC`, then `id ASC` (deterministic order)
- Substitutes placeholder per `injection_spec`:
  - `header`: sets `{name}: {template with ${SECRET}}`
  - `query`: appends `?{name}={template}`
  - `body-json`: JSONPath replace inside request body (JSON-content-type only)
- Forwards to upstream via `reqwest` with original method/body/headers (minus `Proxy-Authorization`)
- Streams response back verbatim
- Writes audit row (including denied requests) before responding

---

## Test plan

- Unit: `crypto.rs` — encrypt/decrypt round-trip, wrong-key rejection, nonce uniqueness
- Unit: `rules.rs` — pattern matching (exact, wildcard, path segments), substitution templates
- Integration: `proxy_test.rs` — spin up mock upstream (`httpmock`), send request through proxy, assert placeholder swap
- Integration: full JSON-RPC round-trip via existing `tests/json_rpc_e2e.rs` harness
- Security: a "malicious skill" test that tries to read the SQLite file directly, confirm it sees only ciphertext
- Cross-platform: CI matrix covers macOS, Linux, Windows (already exists for Tauri)

---

## Execution plan

**Week 1 — Design + prototype (parallel with A-series, no code collision)**
- [ ] Finalize this doc
- [ ] Prototype `crypto.rs` + `store.rs` in a scratch binary
- [ ] Confirm `keyring` crate works on macOS / Windows / Linux
- [ ] Confirm `aes-gcm` nonce handling against RustCrypto test vectors

**Week 2 — Implementation (parallel with A-series, collision only in `src/core/all.rs`)**
- [ ] Create `src/openhuman/credentials/` module per layout above
- [ ] Implement `store.rs`, `crypto.rs`, `keychain.rs`, `rules.rs`
- [ ] Implement `proxy.rs` hyper listener
- [ ] Wire controller schemas + JSON-RPC in `src/core/all.rs`
- [ ] Unit tests for every module

**Week 3 — Integration + UI**
- [ ] Integration tests via `tests/json_rpc_e2e.rs`
- [ ] Tauri settings UI for credential CRUD + per-skill grants
- [ ] Docs: update `ENVIRONMENT-CONTRACT-ROADMAP.md` with "credential half solved"

**Week 4+ — Container-per-skill spike (SEQUENTIAL after A-series lands)**
- [ ] Docker/Apple Container wrapper for skill execution
- [ ] Mount allowlist enforcement (pattern from nanoclaw's `~/.config/nanoclaw/mount-allowlist.json`)
- [ ] Inject `Proxy-Authorization` bearer into container env
- [ ] Validate cross-platform (CI Windows runner)

---

## Success criteria

- A skill making `reqwest::get("http://127.0.0.1:<port>/v1/models")` with its `Proxy-Authorization` bearer receives OpenAI's real response
- The skill process, inspected live with `ps -E` + `/proc/<pid>/environ` + full filesystem scan of its writable mounts, contains zero bytes of the real API key
- SQLite DB inspected directly (`sqlite3 openhuman.db "SELECT ciphertext FROM credentials"`) returns only ciphertext
- Revoking a skill token immediately blocks further requests (audit log shows `credential_id=NULL`)
- macOS + Linux + Windows CI all green

---

## Open questions

- **Per-request decryption vs. cached decryption in memory?** Cached = faster, wider memory-dump window. Per-request = slower, tight window. Default: per-request; add LRU with TTL only if latency measurably hurts.
- **How do skills discover the proxy port?** Current plan: published via existing core RPC. Alternative: fixed env var `OPENHUMAN_PROXY_URL` injected at skill spawn. Revisit during container spike.
- **Do we want OneCLI as an *optional* external backend for users who already run it?** Deferred — solve single-user case first, add adapter later if demand exists.

---

## References

- [nanoclaw security model](https://github.com/qwibitai/nanoclaw/blob/main/docs/SECURITY.md)
- [OneCLI gateway architecture](https://github.com/onecli/onecli/tree/main/apps/gateway)
- [RustCrypto `aes-gcm`](https://github.com/RustCrypto/AEADs/tree/master/aes-gcm)
- [`keyring` crate](https://github.com/hwchen/keyring-rs)
