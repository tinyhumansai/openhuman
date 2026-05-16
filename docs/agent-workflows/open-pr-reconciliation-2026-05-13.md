# Open PR Reconciliation Handoff - 2026-05-13

Snapshot taken from `tinyhumansai/openhuman` on 2026-05-13 after fetching
`upstream/main` at `2b64ea8a` (`chore(release): v0.53.43`). The queue is
moving quickly; refresh the PR list before acting.

This is a reconciliation-first handoff. It does not change product code.

## Current Queue

- Open PRs: 28.
- Merge-state shape: 23 `BLOCKED`, 5 `DIRTY`.
- Draft PRs: #1644, #1519, #1420, #1383.
- High-risk conflict/rebase set: #1671, #1646, #1518, #1462, #1420.
- Broadest PRs by changed files: #1420 (93 files), #1518 (90 files), #1671
  (19 files), #1519 (13 files), #1677 (12 files), #1488 (11 files).

| PR | State | Merge | Files | Size | Updated | Notes |
| --- | --- | --- | ---: | ---: | --- | --- |
| #1678 | ready | BLOCKED | 2 | +280/-16 | 2026-05-13 19:29Z | Triage prompt-guard rejection handling. |
| #1677 | ready | BLOCKED | 12 | +2623/-1 | 2026-05-13 19:25Z | Gameplay review workflow; failing broad CI and review requested. |
| #1676 | ready | BLOCKED | 4 | +238/-3 | 2026-05-13 18:54Z | Backend 4xx observability classifier. |
| #1672 | ready | BLOCKED | 2 | +68/-8 | 2026-05-13 18:07Z | Socket sustained-outage classifier; cancelled lightweight checks observed. |
| #1671 | ready | DIRTY | 19 | +1772/-265 | 2026-05-13 18:09Z | BYO Composio API key; conflicts with Composio/integrations/settings surfaces. |
| #1657 | ready | BLOCKED | 8 | +344/-1 | 2026-05-13 17:31Z | Gmail unsubscribe agent; overlaps tools/memory email surfaces. |
| #1656 | ready | BLOCKED | 8 | +52/-103 | 2026-05-13 15:13Z | Local AI test isolation. |
| #1646 | ready | DIRTY | 6 | +265/-20 | 2026-05-13 17:06Z | Portfolio readiness docs/lint; conflicts with active UI pages and docs. |
| #1645 | ready | BLOCKED | 2 | +39/-1 | 2026-05-13 18:25Z | Provider function argument parse guard. |
| #1644 | draft | BLOCKED | 9 | +1474/-0 | 2026-05-13 17:12Z | Deep-work automation scripts; draft. |
| #1641 | ready | BLOCKED | 3 | +458/-155 | 2026-05-13 20:02Z | Windows FS retry; changes requested. |
| #1636 | ready | BLOCKED | 2 | +212/-1 | 2026-05-13 18:04Z | Stale credentials lock recovery. |
| #1635 | ready | BLOCKED | 3 | +88/-31 | 2026-05-13 16:04Z | Screen-intelligence idempotent session start. |
| #1634 | ready | BLOCKED | 10 | +325/-51 | 2026-05-13 20:00Z | Agent max-iteration observability; checks still running. |
| #1633 | ready | BLOCKED | 10 | +344/-84 | 2026-05-13 20:08Z | Budget-exhausted observability; PR checklist failed and review requested. |
| #1632 | ready | BLOCKED | 7 | +474/-37 | 2026-05-13 20:07Z | Transient backend/integrations observability; checks still running. |
| #1630 | ready | BLOCKED | 2 | +298/-4 | 2026-05-13 18:28Z | Integrations local-AI URL fallback. |
| #1623 | ready | BLOCKED | 1 | +80/-0 | 2026-05-13 19:23Z | Vision RAM-tier observability skip. |
| #1620 | ready | BLOCKED | 1 | +57/-10 | 2026-05-13 17:59Z | UTF-8 memory ingest slicing. |
| #1589 | ready | BLOCKED | 3 | +377/-0 | 2026-05-13 14:42Z | Contributor reward invite automation. |
| #1561 | ready | BLOCKED | 3 | +744/-0 | 2026-05-13 18:38Z | Memory benchmark fixtures. |
| #1519 | draft | BLOCKED | 13 | +1065/-19 | 2026-05-12 07:07Z | Learning summarizer; draft. |
| #1518 | ready | DIRTY | 90 | +3727/-1295 | 2026-05-13 06:33Z | Chinese i18n; very broad UI surface. |
| #1488 | ready | BLOCKED | 11 | +646/-126 | 2026-05-13 18:09Z | Orchestrator delegation collapse. |
| #1462 | ready | DIRTY | 5 | +886/-789 | 2026-05-10 23:43Z | "Files Reviewed"; stale/conflicting, many failed checks, changes requested. |
| #1420 | draft | DIRTY | 93 | +10060/-35 | 2026-05-10 21:31Z | iOS client; largest branch, draft and conflicting. |
| #1383 | draft | BLOCKED | 1 | +57/-0 | 2026-05-10 21:30Z | Worker clone disposition docs. |
| #1321 | ready | BLOCKED | 4 | +84/-13 | 2026-05-10 21:31Z | Core-state rewards timeout UX. |

## Conflict Clusters

These files are touched by multiple open PRs and should be treated as hot
surfaces. Do not launch implementation work here until the relevant PRs are
rebased, merged, or closed.

| Surface | PRs | Risk |
| --- | --- | --- |
| `src/core/observability.rs` | #1676, #1634, #1633, #1632, #1623 | Highest overlap. Pick a canonical observability stack order before rebasing. |
| `app/src-tauri/src/lib.rs` | #1634, #1633, #1632, #1420 | Tauri shell overlap; avoid mixing iOS shell work with observability fixes. |
| `src/main.rs` | #1634, #1633, #1632 | Observability initialization overlap. |
| `src/openhuman/integrations/client.rs` | #1676, #1632, #1630 | Integrations error classification and local-AI fallback interact. |
| `app/src/App.tsx` | #1518, #1420 | Broad UI architecture conflict between i18n and iOS routing. |
| `app/src/pages/Settings.tsx` | #1671, #1420 | BYO Composio settings conflicts with iOS settings changes. |
| `src/core/all.rs`, `src/openhuman/about_app/catalog.rs`, `src/openhuman/mod.rs` | #1677, #1420 | Core module/catalog registration overlap. |
| `src/openhuman/agent/harness/*`, `src/openhuman/channels/runtime/dispatch.rs` | #1634, #1519, #1488 | Agent runtime, learning, and delegation work should not proceed independently. |
| `src/openhuman/credentials/profiles.rs` | #1641, #1636 | Windows retry and stale-lock recovery need one canonical credential-lock PR. |
| `Cargo.toml`, `Cargo.lock`, `pnpm-lock.yaml` | #1462, #1420 | Dependency/lockfile churn; #1462 appears stale and should not be used as a base. |

## Validation Gates

OpenHuman PRs carry both local and CI gates. For code PRs, the minimum gate is
the smallest focused command that proves the changed surface plus the relevant
global merge checks:

- PR body/template gate: `pnpm pr:checklist <body-file>`.
- Formatting: `pnpm --filter openhuman-app format:check`; Rust changes also
  need `cargo fmt --manifest-path Cargo.toml --all --check` and, for Tauri,
  `cargo fmt --manifest-path app/src-tauri/Cargo.toml --all --check`.
- TypeScript: `pnpm typecheck` for app-facing TypeScript changes.
- Focused tests: Vitest file-level tests for changed React/TS behavior; `pnpm
  debug rust <filter>` for Rust behavior.
- Coverage gate: CI enforces changed-line diff coverage at or above 80%.
- Docs-only changes: run the PR checklist parser and any available markdown
  link/check workflow locally when dependencies are installed.

Current blocker pattern:

- `DIRTY` PRs need rebase/conflict resolution before any CI result matters.
- `BLOCKED` PRs are mostly mergeable but waiting on checks, reviews, checklist
  failures, or branch protection.
- #1677 and #1462 have concrete failed CI/check evidence and should not be
  treated as safe to merge without repair.
- #1633 has a failed PR checklist and requested changes; fix the body/checklist
  before spending compute on broad tests.
- Before rebasing or repairing any existing PR, refresh metadata with `gh pr
  view <number> --repo tinyhumansai/openhuman --json
  body,changedFiles,mergeStateStatus,mergeable,state,statusCheckRollup`.
- If validating an existing PR body, feed the actual body into the checklist
  parser: `gh pr view <number> --repo tinyhumansai/openhuman --json body --jq
  .body | pnpm pr:checklist -`.

## Recommended Reconciliation Order

1. Close or explicitly supersede stale/meta PRs before implementation:
   #1462 should be audited first because it is conflicting, has broad failed
   checks, and has an unclear title. If it has no unique current work, close it
   with a pointer to the canonical replacement.
2. Reconcile observability as a single stack:
   compare #1623, #1632, #1633, #1634, #1676, and #1672. Choose an order from
   narrowest independent classifier to broadest runtime initialization. Rebase
   one PR at a time and rerun focused observability tests plus checklist.
   #1676 is the likely base candidate because it changes the backend 4xx
   classifier that broader observability filters build on.
3. Reconcile credentials-lock PRs:
   decide whether #1636 or #1641 is canonical, then port any unique tests/fixes
   into the kept branch.
4. Keep broad feature branches out of the merge queue until smaller fixes land:
   #1420, #1518, #1671, #1677, #1519, and #1488 all overlap hot surfaces.
5. Only after the queue is quieter, launch implementation work outside these
   hot files.

Do not batch-rebase PRs from the conflict-cluster table. Rebase and validate one
branch at a time so failures can be attributed to the branch being repaired.

## Next Safe Implementation Slice

Do not start in observability, settings, Tauri shell, agent harness, or iOS/i18n
routing. The next safe slice is a small reconciliation task:

- Target: canonicalize #1462 disposition.
- Work:
  - Refresh metadata first with `gh pr view 1462 --repo tinyhumansai/openhuman
    --json body,changedFiles,mergeStateStatus,mergeable,state,statusCheckRollup`.
  - Fetch `refs/pull/1462/head` and compare it against `upstream/main`.
  - Identify whether the branch contains any unique, still-relevant code.
  - Inspect lockfile/dependency churn in `Cargo.toml`, `Cargo.lock`,
    `pnpm-lock.yaml`, and `scripts/mock-api-core.mjs`; these may have been
    superseded by the `v0.53.43` release merge.
  - If no unique work remains, close #1462 as stale/superseded and link this
    handoff plus the canonical replacement PRs.
  - If useful work remains, create a new narrow issue/branch for only that
    surface; do not rebase the full PR.
- Validation:
  - `git diff --name-status upstream/main...refs/tmp/pr-1462`
  - `git log --left-right --cherry-pick --oneline upstream/main...refs/tmp/pr-1462`
  - `gh pr view 1462 --repo tinyhumansai/openhuman --json body --jq .body |
    pnpm pr:checklist -`
  - `pnpm pr:checklist <generated-body>` only if a replacement PR is opened.

After #1462 is resolved, the next implementation-grade slice should be the
observability-stack canonicalization because it is blocking the most active
ready PRs.
