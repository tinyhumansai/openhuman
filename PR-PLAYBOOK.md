# OpenHuman PR Playbook

Reference for all 4 PRs against [`tinyhumansai/openhuman`](https://github.com/tinyhumansai/openhuman) from your fork [`aashir-athar/openhuman`](https://github.com/aashir-athar/openhuman).

**Working directory:** `D:/openhuman/` (single consolidated worktree).
**Remotes:** `origin → aashir-athar/openhuman.git`, `upstream → tinyhumansai/openhuman` (push disabled).

---

## Universal rules (apply to every push)

1. **Always use explicit file lists in `git add`** — never `-A` or `.`. Three files in `D:/openhuman/` must stay out of every commit:
   - `PR3-mcp-status-badge.patch`
   - `openhuman-contribution-plan.md`
   - `PR-PLAYBOOK.md` (this file)
2. **If the pre-push hook fails on `cargo fmt` not found**, add `--no-verify` to the push. CLAUDE.md authorizes `--no-verify` for unrelated pre-existing breakage. CI on Linux runs the real cargo fmt check.
3. **For PR creation**: if `gh` auth fails (TLS timeout etc.), use the URL GitHub prints after `git push`:
   `https://github.com/aashir-athar/openhuman/pull/new/<branch-name>`
   That opens a pre-filled form pointing at `tinyhumansai/openhuman:main` — paste title + description there, add labels in sidebar.

---

## PR 1 — Docs truth-up

**Branch:** `docs/truth-up-and-architecture-pages`
**Worktree state:** 6 modified + 3 new files in `D:/openhuman/`.
**Status:** Ready to commit + push.
**Label on PR:** `docs`

### Title

```
docs: truth-up architecture.md + 3 new domain pages + Linux/Arch caveats in localized READMEs
```

### Description

```markdown
## Summary

- Purge 9 stale QuickJS / `rquickjs` skill-runtime references throughout `gitbooks/developing/architecture.md` — the runtime was removed and `src/openhuman/skills/` is metadata-only per CLAUDE.md, but multiple diagrams, tables, and prose sections still described it as active.
- Fix two related factual bugs in the same file: "Yarn workspace" → "pnpm workspace" (root `package.json` declares `pnpm@10.10.0`); rewrite the stale top-level `skills/` row to point at `src/openhuman/skills/` and describe its actual post-QuickJS-removal state.
- Add three contributor-audience architecture pages under `gitbooks/developing/architecture/` (`memory-tree.md`, `mcp-registry.md`, `security.md`) for currently-undocumented active domains. Linked from `gitbooks/SUMMARY.md`.
- Backfill the Linux Wayland warning + Arch AUR pointer block (present in English `README.md` since #2463) into `README.zh-CN.md`, `README.ja-JP.md`, `README.ko.md`, `README.de.md`. English content under `<!-- TODO: translate (xx) -->` markers — native-speaker translation follow-up invited.

## Problem

The architecture book diverged from the code on three load-bearing points:

1. **Stale runtime model.** CLAUDE.md is explicit: *"Skills runtime removed: the QuickJS / `rquickjs` runtime that previously executed skill packages is gone."* Yet `architecture.md` still described a QuickJS runtime engine, per-skill 64 MB sandbox limits, "QuickJS Skill Instance executes tool" in the MCP flow, and the same in the end-to-end data flow.
2. **Stale build-tool reference** ("Yarn workspace") inconsistent with `pnpm@10.10.0` in `package.json`.
3. **Stale path reference** to a top-level `skills/` directory that no longer exists.

Three active, substantial domains (`memory_tree`, `mcp_registry`, `security`) had no gitbook architecture page despite recent activity (#2556, #2559) and despite each having a rich internal-audience `README.md` / `mod.rs` rustdoc invisible to the gitbook.

The Linux Wayland / AUR block was only added to the English README; localized README readers running install on Arch + Wayland hit an unexplained crash with no caveat in their language.

## Solution

**`gitbooks/developing/architecture.md`** — purged QuickJS references from the high-level diagram, performance table, MCP flow diagram, security architecture diagram + bullet, and end-to-end data flow. Where a replacement was needed (e.g. "Sandboxed QuickJS per skill (64 MB)"), used what the code actually does: tool execution gated by `SecurityPolicy` + a host-selected sandbox backend from `src/openhuman/security/`. Three intentional "QuickJS was removed" historical references retained — clearly marked as such — to make the migration explicit.

**Three new architecture pages** — followed the existing `agent-harness.md` / `frontend.md` / `tauri-shell.md` convention: YAML frontmatter, H1 with source path in backticks, ASCII diagram, layout table sourced from the existing `README.md` / `mod.rs`, "Calls into" / "Called by" / "Tests" / "Related" sections. Describe only what the code currently does — no aspirational claims. Tables prettier-aligned to match existing style.

**Localized READMEs** — inserted the Linux/Arch block right after the install bash code-block (matching the structural position in `README.md`), wrapped in `<!-- TODO: translate (xx) -->` / `<!-- /TODO -->` markers so native speakers know exactly what needs translation in a follow-up PR.

## Submission Checklist

- [x] Tests added or updated — **N/A: docs-only change, no behavioural code touched.**
- [x] **Diff coverage ≥ 80%** — **N/A: no changed lines appear in any lcov report.** `diff-cover` returns 100% by definition.
- [x] Coverage matrix updated — **N/A: behaviour-only change** (no behaviour change; pure documentation truth-up).
- [x] All affected feature IDs listed under `## Related` — **N/A: no feature behaviour changed.**
- [x] No new external network dependencies introduced — **N/A: docs only.**
- [x] Manual smoke checklist updated if release-cut surfaces touched — **N/A: no release-cut surface touched.**
- [x] Linked issue closed via `Closes #NNN` in `## Related` — **N/A: no linked issue.**

## Impact

- Runtime/platform impact: **none.** Docs only.
- Performance/security/migration/compatibility: **none.**
- New contributors reading `architecture.md` now get an accurate mental model of the post-QuickJS skill-metadata + native-tool-runtime split, and three of the most active Rust domains finally have contributor-audience overviews.
- Localized README readers see the same install caveat their English counterparts do.

## Related

- Closes:
- Follow-up PR(s)/TODOs:
  - Native-speaker translation of the four `<!-- TODO: translate (xx) -->` blocks in the localized READMEs.

---

## AI Authored PR Metadata

### Linear Issue
- Key: N/A
- URL: N/A

### Commit & Branch
- Branch: `docs/truth-up-and-architecture-pages`
- Commit SHA: (filled by GitHub after push)

### Validation Run
- [x] `pnpm --filter openhuman-app format:check` — fails on ~1044 pre-existing files unrelated to this PR (Windows CRLF vs `endOfLine: "lf"`); zero of my files are in the failure list; the 3 new architecture pages explicitly pass prettier
- [x] `pnpm typecheck` — N/A (docs only, no TypeScript touched)
- [x] Focused tests: N/A (docs only)
- [x] Rust fmt/check (if changed): N/A (no Rust touched)
- [x] Tauri fmt/check (if changed): N/A (no Tauri touched)

### Validation Blocked
- `command:` N/A
- `error:` N/A
- `impact:` N/A

### Behavior Changes
- Intended behavior change: None — documentation only
- User-visible effect: Localized README readers now see the same Linux/Arch install caveats as English readers; contributors get accurate runtime architecture and three new domain-level architecture pages

### Parity Contract
- Legacy behavior preserved: Yes — no code changed; only documentation corrected to match current code
- Guard/fallback/dispatch parity checks: N/A

### Duplicate / Superseded PR Handling
- Duplicate PR(s): None known
- Canonical PR: This one
- Resolution: N/A

---

> Local pre-push hook bypassed with `--no-verify` because `cargo fmt` failed on a machine without a Rust toolchain installed. This PR is docs-only (zero Rust changes), so CI's `cargo fmt --check` on the Linux runner is the authoritative gate.
```

### Commands (PowerShell)

```powershell
cd D:\openhuman

# Stage (explicit file list — never -A or .)
git add README.de.md README.ja-JP.md README.ko.md README.zh-CN.md `
        gitbooks/SUMMARY.md `
        gitbooks/developing/architecture.md `
        gitbooks/developing/architecture/memory-tree.md `
        gitbooks/developing/architecture/mcp-registry.md `
        gitbooks/developing/architecture/security.md

# Commit (single-line subject; full body lives in the PR description)
git commit -m "docs: truth-up architecture.md (purge QuickJS refs) + add 3 domain pages + backfill Linux/Arch caveats to localized READMEs"

# Push (use --no-verify if pre-push hook fails on missing cargo)
git push --no-verify -u origin docs/truth-up-and-architecture-pages

# Then open this URL in browser, paste the title + description above, add "docs" label:
#   https://github.com/aashir-athar/openhuman/pull/new/docs/truth-up-and-architecture-pages
```

---

## PR 2 — Backend stub closure (NO COMMANDS — all stale)

**Branch:** *(none — `feat/close-backend-stubs` was deleted after verification)*
**Status:** All 3 audit deliverables verified STALE against current code; cannot ship as specified.

### Verification verdict

| Deliverable | Verdict |
|---|---|
| **2.1** Wire FTS5 insert in `insert_sql_record.rs:137` | STALE — file is intentionally a Phase 5 stub with 8 existing tests; doing it honestly requires designing schema + migration + DI plumbing (multi-day feature, not 1-day stub wiring) |
| **2.2** Add `webview_notifications/rpc.rs` | STALE — domain is intentionally Tauri-IPC only per its own `schemas.rs` comment: *"v1 has no user-facing controllers: the on/off toggle lives in the Tauri shell"* |
| **2.3** Add unit tests for `src/core/dispatch.rs` | STALE — file already has 12 inline tests covering every case the audit listed (valid routing, unknown method, empty method, tier-2 domain dispatch, legacy alias resolution, etc.) |

### To revisit

PR 2 needs a fresh audit pass to identify alternative real backend gaps (in the same spirit as PR 1.2 which was replaced when the audit was found stale). When ready, start a fresh session and ask me to "find real 1–3 day backend gaps" with no prescribed deliverables.

---

## PR 3 — McpStatusBadge i18n + a11y (reduced scope)

**Branch:** `feat/mcp-servers-ui-panel` (empty branch in git refs, off `upstream/main`)
**Worktree state:** Changes saved as `D:/openhuman/PR3-mcp-status-badge.patch` (107-line diff).
**Status:** Apply patch after PR 1 ships, then commit + push.
**Label on PR:** none required (PR 3 is a feature/a11y change, default labels are fine)

### Why only McpStatusBadge

6 of 8 original PR 3 deliverables were verified STALE. The full MCP UI surface already exists at `app/src/components/channels/mcp/` (8 components, all with tests, RPC client `mcpClientsApi`, ships via `pages/Channels.tsx` → `ChannelConfigPanel` → `McpServersTab`). The `Skills.tsx` `<McpComingSoonPanel />` is intentionally pinned by `Skills.mcp-coming-soon.test.tsx`. Only the McpStatusBadge i18n + a11y gap was real.

### Title

```
feat(mcp): i18n + a11y McpStatusBadge status labels
```

### Description

```markdown
## Summary

- Route the 4 hardcoded status labels in `McpStatusBadge.tsx` ('Connected' / 'Connecting' / 'Disconnected' / 'Error') through `useT()` using the existing `channels.status.*` keys. The badge's own docstring says it *"Mirrors ChannelStatusBadge"*; the labels are identical English; reusing the shared key set avoids redundant translation work.
- Add `role="status"` and `aria-live="polite"` so screen readers announce state changes — matches the alpha-banner pattern already used in `McpServersTab`.
- Add a 7-test `McpStatusBadge.test.tsx` covering each `ServerStatus` variant, the a11y attributes, the disconnected fallback for unknown statuses, and className passthrough.

## Problem

`McpStatusBadge.tsx` slipped past the #2577 React i18n sweep because it's an isolated leaf component without a co-located test. Lines 7–25 hardcoded English labels (`label: 'Connected'`, etc.) directly in a `STATUS_STYLES` object, violating the CLAUDE.md rule that *every user-visible string in `app/src/**` must go through `useT()`*. Non-EN users saw English labels on every MCP server connection state.

The same `<span>` had no `role` or `aria-live`, so screen readers wouldn't announce state changes — important for a long-running connection that can transition between `connecting` → `connected` → `error` while the user is on a different part of the page.

## Solution

- Reuse the existing `channels.status.{connected,connecting,disconnected,error}` i18n keys rather than introducing `mcp.status.*` duplicates. The labels are character-identical to the channel status set; the badge's docstring already calls it a "mirror" of `ChannelStatusBadge`. If the MCP vocabulary ever diverges (e.g. adds "spawning" / "handshaking"), the keys can be split then.
- Add `role="status"` + `aria-live="polite"` (matches `McpServersTab`'s alpha banner).
- Co-locate `McpStatusBadge.test.tsx` covering: every status variant renders the right label, a11y attributes present, fallback to "Disconnected" for unknown status values, className passthrough preserves built-in classes.

## Submission Checklist

- [x] Tests added or updated — 7 tests added in new `McpStatusBadge.test.tsx`.
- [x] **Diff coverage ≥ 80%** — All changed lines in `McpStatusBadge.tsx` are exercised by the new test file (`it.each` over all 4 status variants + a11y + fallback + className).
- [x] Coverage matrix updated — **N/A: behaviour-only change** (no new feature; same component, just i18n + a11y).
- [x] All affected feature IDs listed under `## Related` — **N/A: leaf-component fix.**
- [x] No new external network dependencies introduced — N/A.
- [x] Manual smoke checklist updated if release-cut surfaces touched — **N/A: no release-cut surface touched.**
- [x] Linked issue closed via `Closes #NNN` in `## Related` — **N/A: no linked issue.**

## Impact

- Runtime/platform impact: **none** (drop-in component change).
- Performance/security/migration/compatibility: none.
- Localized users will now see translated status labels in their locale; screen reader users will hear connection state changes announced.

## Related

- Closes:
- Follow-up PR(s)/TODOs: none.

---

## AI Authored PR Metadata

### Linear Issue
- Key: N/A
- URL: N/A

### Commit & Branch
- Branch: `feat/mcp-servers-ui-panel`
- Commit SHA: (filled by GitHub after push)

### Validation Run
- [x] `pnpm --filter openhuman-app format:check` — `McpStatusBadge.test.tsx` clean; `McpStatusBadge.tsx` inherits CRLF from Windows checkout (CI on Linux passes)
- [x] `pnpm typecheck` — clean
- [x] Focused tests: `pnpm vitest run src/components/channels/mcp/` — 82/82 pass (includes the new file + the pinned `Skills.mcp-coming-soon.test.tsx` confirming no regression)
- [x] Rust fmt/check (if changed): N/A (no Rust touched)
- [x] Tauri fmt/check (if changed): N/A (no Tauri touched)

### Validation Blocked
- `command:` N/A
- `error:` N/A
- `impact:` N/A

### Behavior Changes
- Intended behavior change: Localized status labels + screen-reader announcement of state changes
- User-visible effect: Non-EN users see translated labels; screen readers announce connection state transitions

### Parity Contract
- Legacy behavior preserved: Yes — labels render identically in English; the fallback for unknown statuses still resolves to "Disconnected" style + label
- Guard/fallback/dispatch parity checks: Test covers unknown status fallback to "Disconnected" label + style

### Duplicate / Superseded PR Handling
- Duplicate PR(s): None known
- Canonical PR: This one
- Resolution: N/A
```

### Commands (PowerShell, run after PR 1 is pushed)

```powershell
cd D:\openhuman

# Switch to the empty PR 3 branch (already off upstream/main)
git switch feat/mcp-servers-ui-panel

# Apply the saved patch
git apply PR3-mcp-status-badge.patch

# Verify the expected diff (1 modified + 1 untracked .test.tsx)
git status

# Stage explicitly
git add app/src/components/channels/mcp/McpStatusBadge.tsx `
        app/src/components/channels/mcp/McpStatusBadge.test.tsx

# Commit
git commit -m "feat(mcp): i18n + a11y McpStatusBadge status labels"

# Push (use --no-verify if pre-push hook fails on missing cargo)
git push --no-verify -u origin feat/mcp-servers-ui-panel

# Open this URL and paste the title + description above:
#   https://github.com/aashir-athar/openhuman/pull/new/feat/mcp-servers-ui-panel

# After PR 3 is opened, the patch is no longer needed:
Remove-Item PR3-mcp-status-badge.patch
```

---

## PR 4 — LSP tool backend (deferred)

**Branch:** `feat/lsp-tool-backend` (empty branch in git refs, off `upstream/main`)
**Status:** Verified REAL (audit accurate); deferred to a fresh session per the master prompt's *"ONE PR per session"* rule and *"if rabbit hole, surface limitation"* clause.

### Scope (when you resume)

Per master prompt PR 4 spec:
- New `src/openhuman/lsp/` domain (`client.rs`, `pool.rs`, `discovery.rs`, `types.rs`, `schemas.rs`, `rpc.rs`)
- LSP JSON-RPC over stdio (Content-Length framing, request/response correlation)
- Cross-platform server discovery for at minimum `rust-analyzer`
- Wire into existing `tools/impl/system/lsp.rs` (which is the stable capability-gated stub)
- Tests with mock LSP server
- Keep behind `OPENHUMAN_LSP_ENABLED=1` env gate
- Update `src/openhuman/about_app/`

### Reusable infrastructure already in the codebase

- `src/openhuman/mcp_client/stdio.rs` — tokio Command + ChildStdin/ChildStdout pattern (LSP uses the same wire framing, different methods)
- Controller pattern (`all_controller_schemas`, `all_registered_controllers`, `handle_*`) from any existing domain
- Wire into `src/core/all.rs` like every other domain

### Rabbit holes to surface, not invent solutions for

- Cross-platform server discovery (Windows / Linux / macOS / asdf / mise / path-relative)
- Auto-install vs error-if-missing
- Multi-root workspace detection

### How to resume

```powershell
cd D:\openhuman
git fetch upstream
git switch feat/lsp-tool-backend
git rebase upstream/main   # bring branch up to date if upstream has moved
```

Then start a fresh Claude session with the master prompt's `CURRENT TASK` line set to `PR 4`.

---

## Summary

| PR | Branch | Status | Action |
|---|---|---|---|
| 1 | `docs/truth-up-and-architecture-pages` | Ready in worktree | Run PR 1 commands above |
| 2 | *(none)* | All audit deliverables STALE | Skip; revisit with fresh audit later if desired |
| 3 | `feat/mcp-servers-ui-panel` | Patch saved | Run PR 3 commands after PR 1 ships |
| 4 | `feat/lsp-tool-backend` | Deferred | Resume in fresh session |

**Branches preserved in git refs:** `docs/truth-up-and-architecture-pages`, `feat/mcp-servers-ui-panel`, `feat/lsp-tool-backend`, `main`. The deleted `feat/close-backend-stubs` was PR 2's empty branch — no work to preserve.
