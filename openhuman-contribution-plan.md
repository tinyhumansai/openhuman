# OpenHuman Contribution Plan

**Target repo:** [`tinyhumansai/openhuman`](https://github.com/tinyhumansai/openhuman) (upstream, GNU-licensed)
**Plan drafted:** 2026-05-25
**Status:** Awaiting fork setup and order confirmation before PR 1 starts
**Scope:** 4 separate PRs, each branched off latest `upstream/main`, opened against `tinyhumansai/openhuman:main`

---

## Table of contents

1. [Why these four](#why-these-four)
2. [Ground rules](#ground-rules-from-claudemd)
3. [PR 1 — Docs truth-up](#pr-1--docs-truth-up)
4. [PR 2 — Backend stub closure](#pr-2--backend-stub-closure)
5. [PR 3 — MCP Servers UI panel](#pr-3--mcp-servers-ui-panel)
6. [PR 4 — LSP tool backend](#pr-4--lsp-tool-backend)
7. [Branch and push flow](#branch-and-push-flow)
8. [What I need from you to start](#what-i-need-from-you-to-start)

---

## Why these four

OpenHuman is highly active — 1,716 commits in the last 2 months, v0.54.0 latest. An audit across `src/openhuman/` (~87 Rust domains) and `app/src/` (React + Tauri v2) surfaced four classes of authentic gaps. The PRs below are ordered ascending by risk so the early ones establish a working relationship with maintainers before we tackle the flagship.

| PR | Class | Effort | Risk | User-visible |
| --- | --- | --- | --- | --- |
| 1 | Docs truth-up | 1–2 days | Low | Indirectly (contributor onboarding) |
| 2 | Backend stub closure | 2–3 days | Low–Med | No (internal correctness) |
| 3 | MCP Servers UI panel | 3–5 days | Med | **Yes — flagship** |
| 4 | LSP tool backend | 3–5 days | Med–High (exploratory) | Yes (via agent tool surface) |

---

## Ground rules (from CLAUDE.md)

- **Never write on `main`** — each PR gets its own branch off latest `upstream/main`.
- **Don't bundle unrelated changes** — these are 4 separate PRs, not one giant one.
- **Push to your fork** (`<yourname>/openhuman`), open PRs against `tinyhumansai/openhuman:main` with `--head <yourname>:<branch>`. Treat `upstream` remote as fetch-only.
- **≥80% coverage on changed lines** — enforced by `.github/workflows/coverage.yml` via `diff-cover` over Vitest + cargo-llvm-cov lcov outputs. Any new Rust or TS code needs tests in the same PR.
- **Pre-merge checks**: `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, `cargo fmt`, `cargo check`. Pre-push hook runs `pnpm rust:check`.
- **i18n rule**: every user-visible string goes through `useT()` from `app/src/lib/i18n/I18nContext`. Add keys to `app/src/lib/i18n/en.ts` in the same PR.
- **Capability catalog**: if a PR adds/removes a user-facing feature, update `src/openhuman/about_app/` in the same work.
- **No new JS injection** into CEF child webviews (`acct_*`, third-party origins) — relevant to PR 3 if MCP servers ever expose a webview.
- **No dynamic imports** in production `app/src` code — static `import` / `import type` only.

---

## PR 1 — Docs truth-up

**Branch:** `docs/truth-up-and-architecture-pages`
**Estimated effort:** 1–2 days
**Risk:** Low — zero compilation, fast review cycle, establishes our quality bar.

### Deliverables

#### 1.1 Fix stale QuickJS references in architecture docs

- **File:** `gitbooks/developing/architecture.md` (audit flagged lines ~60–100)
- **Why:** CLAUDE.md says *"Skills runtime removed: the QuickJS / `rquickjs` runtime that previously executed skill packages is gone. `src/openhuman/skills/` is now a metadata-only domain."* The architecture doc still describes QuickJS as active.
- **Action:** Rewrite the affected section to reflect the metadata-only reality. Point to the external skills registry repo.
- **Verification first:** Open the file and confirm the stale text exists. If the audit was wrong and the doc is already correct, skip this and report.

#### 1.2 Backfill localized READMEs

- **Files:** `README.zh-CN.md`, `README.ja-JP.md`, `README.ko.md`
- **Why:** English `README.md` has a "Contributing from source" section (~10 lines with build commands). Localized versions don't — non-EN speakers can't get from zero to a working build via their native README.
- **Action:** Diff each localized README against `README.md` to confirm the gap. Add the equivalent translated block.
- **Caveat to flag in PR body:** Translating from canonical English without native fluency in zh/ja/ko. Native-speaker review pass welcome.

#### 1.3 Add architecture pages for the 3 most-active domains

Under `gitbooks/developing/architecture/`:

- **`memory-tree.md`** — hierarchical Markdown chunks, ≤3k token summaries, SQLite store, Obsidian vault mirroring. Source: read `src/openhuman/memory_tree/` (consolidated in #2556 two weeks ago).
- **`mcp-client.md`** — multi-registry, boot-spawn, setup agent. Source: read `src/openhuman/mcp_registry/` (split in #2559 two weeks ago).
- **`security.md`** — sandboxing layers (bubblewrap / firejail / landlock). Source: read `src/openhuman/security/`.

Each page: ~1 page of prose + brief module map + key invariants. Linked from the parent `architecture.md`.

### Out of scope for PR 1

The remaining ~80 missing architecture pages. We pick the 3 hottest; rest is a separate effort.

### Validation

- `pnpm format:check` for any markdown that lives in linted dirs.
- No tests needed (docs only).
- Manual re-read of each file post-edit.

---

## PR 2 — Backend stub closure

**Branch:** `feat/close-backend-stubs`
**Estimated effort:** 2–3 days
**Risk:** Low–Med — real Rust changes, but each piece is tiny and isolated.

### Deliverables

#### 2.1 Wire FTS5 episodic-memory insert

- **File:** `src/openhuman/tools/impl/system/insert_sql_record.rs:137`
- **Current state:** Tool returns `"episodic memory write not yet wired (FTS5/SQLite insert pending)"` for all calls.
- **Verification first:** Read the file. Read `src/openhuman/memory_store/` to find the existing FTS5 schema and any `INSERT` helpers already written for other call sites.
- **Action:** Implement the missing INSERT using the same patterns. Add unit tests for happy path + duplicate-rowid + empty-body edge cases.

#### 2.2 Add `webview_notifications/rpc.rs`

- **Path:** `src/openhuman/webview_notifications/`
- **Current state:** Domain has `schemas.rs` but no `rpc.rs` — audit suggests this blocks frontend integration.
- **Verification first:** Read `schemas.rs` to see declared controllers. Read a neighbor domain (e.g. `notifications/rpc.rs`) for the established pattern. Grep call sites to confirm `rpc.rs` is actually expected and not intentionally absent.
- **Action (if confirmed needed):** Implement handlers, register in `mod.rs`, wire into `src/core/all.rs`. Tests per handler.
- **Drop if intentional:** If `webview_notifications` is intentionally event-bus-only (no RPC), drop this deliverable and report.

#### 2.3 Add unit tests for `src/core/dispatch.rs`

- **File:** `src/core/dispatch.rs` (362 LOC, zero unit tests today)
- **Why:** Critical RPC routing path — method resolution, auth ordering, controller-registry vs legacy-alias precedence. Untested today.
- **Action:** Write `dispatch_tests.rs` covering: valid method routing, unknown method, auth failure, malformed payload, controller-registry vs legacy-alias resolution order, error response shape.

### Validation

- `pnpm test:rust`
- `cargo check --bin openhuman-core`
- `cargo fmt`
- The diff-cover gate runs against changed Rust lines.

---

## PR 3 — MCP Servers UI panel

**Branch:** `feat/mcp-servers-ui-panel`
**Estimated effort:** 3–5 days
**Risk:** Med — biggest scope, but backend is done and patterns are established.
**Why this is the flagship:** Backend (`src/openhuman/mcp_registry/`: 11 files including `boot.rs`, `bus.rs`, `connections.rs`, `ops.rs`, `registries/`, `setup.rs`, `setup_ops.rs`, `store.rs`, `types.rs`) is already merged in #2559. UI is stubbed — `<McpComingSoonPanel />` rendered at `app/src/pages/Skills.tsx:1119` (PR #2570). User-visible feature unlock.

### Deliverables

#### 3.1 New components under `app/src/components/mcp/`

- **`McpRegistryList.tsx`** — list of registered MCP servers (from `mcp_registry` backend). Each row: name, transport (stdio / http / sse), status, last-seen.
- **`McpServerCard.tsx`** — single-server detail panel: tool inventory, last error, enable/disable toggle.
- **`McpAddServerModal.tsx`** — wizard to add a server: pick registry, enter config, run `setup` agent (setup agent landed in #2559).
- Bind to existing Redux conventions: Redux Toolkit slices, no ad-hoc `localStorage`.

#### 3.2 Wire into `Skills.tsx`

- Replace `<McpComingSoonPanel />` at `app/src/pages/Skills.tsx:1119` with the new list/detail UI.
- Keep the existing tab structure intact.

#### 3.3 RPC client bindings

- New bindings in `app/src/services/api/` for the existing `openhuman.mcp_*` JSON-RPC methods exposed by `src/openhuman/mcp_registry/rpc.rs`.
- **Verification first:** Read `rpc.rs` to confirm exact method names and schemas.

#### 3.4 Fix `McpStatusBadge.tsx` i18n + a11y

- **File:** `app/src/components/channels/mcp/McpStatusBadge.tsx`
- **Verified violation:** Lines 7–25 hardcode 'Connected' / 'Connecting' / 'Disconnected' / 'Error' — slipped past the #2577 i18n sweep.
- **Action:** Route through `useT()` with new keys in `en.ts`. Add `role="status"` and `aria-live="polite"` to the badge `<span>`.
- **Why bundled here, not PR 1:** Same MCP surface area — reviewers will see both together as one feature delivery. CLAUDE.md says don't bundle *unrelated* changes; this is related.

#### 3.5 Vitest coverage

- Per-component tests for `McpRegistryList`, `McpServerCard`, `McpAddServerModal`, `McpStatusBadge`.
- ≥80% line coverage on changed lines (merge gate).
- Use helpers in `app/src/test/`. No real network.

#### 3.6 i18n keys

- All new strings added to `app/src/lib/i18n/en.ts`.
- Use the existing `mcp.*` namespace (already partly populated for `channels.mcp.*`).

#### 3.7 E2E spec

- New spec at `app/test/e2e/specs/mcp-servers-flow.spec.ts`.
- Flow: open Skills → MCP tab → list renders → open Add Server modal → cancel.
- Use documented helpers from `element-helpers.ts` (`clickNativeButton`, `waitForWebView`, `clickToggle`) — never raw XCUI types.

#### 3.8 Capability catalog update

- Update `src/openhuman/about_app/` — MCP server management is now user-facing.

### Validation

- `pnpm typecheck && pnpm lint && pnpm test && pnpm test:e2e:build && pnpm rust:check`
- Manual smoke: launch desktop app, navigate to Skills → MCP, walk the flow.

---

## PR 4 — LSP tool backend

**Branch:** `feat/lsp-tool-backend`
**Estimated effort:** 3–5 days
**Risk:** Med–High — exploratory; language-server integration surfaces cross-platform discovery and lifecycle questions.

### Current state

- `src/openhuman/tools/impl/system/lsp.rs` — schema complete (`kind`, `language`, `file`, `line`, `character`, `symbol`); `execute()` returns "not yet implemented"; gated by `OPENHUMAN_LSP_ENABLED=1`.

### Deliverables

#### 4.1 Read and map the existing schema

- Open `lsp.rs` in full. Confirm the tool kinds the schema commits to (likely `hover`, `definition`, `references`, `completion`, etc.).

#### 4.2 New `src/openhuman/lsp/` domain

Per CLAUDE.md's "new functionality goes in a dedicated subdirectory" rule. Files:

- `client.rs` — LSP JSON-RPC framing (Content-Length headers, request/response correlation).
- `pool.rs` — per-language server pool, lazy spawn, idle shutdown.
- `discovery.rs` — cross-platform server binary discovery (`rust-analyzer`, `typescript-language-server`, `pyright`).
- `types.rs` — LSP request/response shapes.
- `schemas.rs` + `rpc.rs` — controller pattern from CLAUDE.md (`all_controller_schemas`, `all_registered_controllers`, `handle_*`).
- Wire exports into `src/core/all.rs`.

#### 4.3 JSON-RPC bridge over stdio

- Implement the LSP wire protocol over stdio to the spawned server.
- Survey existing crates first (`tower-lsp` is server-side; client path may need rolling our own or finding a maintained `lsp-client`).

#### 4.4 Tests

- Mock LSP server (or a small real one like a `tower-lsp`-based echo) to test: request framing, response correlation, timeout, server crash recovery.

#### 4.5 Keep behind env gate

- Keep `OPENHUMAN_LSP_ENABLED=1` until the feature is hardened. Ship behind the flag.

#### 4.6 Capability catalog

- Update `src/openhuman/about_app/`.

### Risk callouts I'll surface in the PR body

- Cross-platform server discovery — where is `rust-analyzer` on Windows / Linux / macOS / asdf / mise / path-relative?
- Server install — do we auto-install or just error if missing?
- Workspace root detection for multi-root projects.
- Per CLAUDE.md's philosophy: if any of these turn into rabbit holes mid-PR, **surface the limitation rather than ship a half-baked solution**.

### Validation

- `pnpm test:rust`
- `cargo check`
- `cargo fmt`
- Manual smoke test against a real `rust-analyzer` on this machine.

---

## Branch and push flow

For each PR:

```bash
git fetch upstream
git checkout -b <branch> upstream/main
# ... work ...
git push -u origin <branch>
gh pr create \
  --repo tinyhumansai/openhuman \
  --base main \
  --head <yourname>:<branch> \
  --title "<conventional commit title>" \
  --body "$(cat <<'EOF'
<follow .github/PULL_REQUEST_TEMPLATE.md verbatim>
EOF
)"
```

Between PRs I'll pause and let you review the diff before kicking off the next one.

---

## What I need from you to start

1. **Fork name** — so I can prep the `git push` and `gh pr create --head` commands correctly when each PR is ready.
2. **Confirm the order** — docs → stubs → MCP UI → LSP. If you'd rather start with the MCP UI flagship, say so.
3. **Translation preference for PR 1.2** — do you want me to do the zh/ja/ko translation, or stub the English content with a `<!-- TODO: translate -->` marker for a native speaker to fill in?

Once you greenlight, I'll create the branch for PR 1 and start by **reading** the files to confirm every audit claim — no edits until each one is verified true against current code.
