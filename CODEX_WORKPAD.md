# CODEX Workpad

## Portfolio Readiness Pass - 2026-05-12

### Scope

- No product functionality changes.
- Validation-focused pass for presentation readiness.
- Existing dirty `.gitignore` and `docs/architecture/` changes were left intact.

### Validation Evidence

Commands run:

```bash
pnpm run typecheck
pnpm run lint
pnpm run test
pnpm run test:rust
git diff --check
```

Results:

- `pnpm run typecheck`: passed.
- `pnpm run lint`: passed with 38 warnings, mostly React compiler `set-state-in-effect` warnings plus one stale eslint-disable.
- `pnpm run test`: passed, 214 test files, 1 skipped; 1,977 tests passed, 3 skipped.
- `pnpm run test:rust`: passed via `scripts/test-rust-with-mock.sh`.
- `git diff --check`: passed.

### Follow-Up Debt

- Triage React compiler lint warnings before treating lint output as presentation-clean.
- Decide whether Node `localStorage is not available` warnings in Vitest should be silenced with an explicit test environment/localStorage configuration.

## Lint Warning Cleanup - 2026-05-13

Original source checkout scope:

- Removed one stale `eslint-disable-next-line no-console` directive from
  `app/src/services/meetCallService.ts`.
- No behavior change.

Validation:

```bash
pnpm run lint
```

Result: passed with 37 warnings, down from 38. Remaining warnings are React
compiler `set-state-in-effect` warnings plus one `no-explicit-any` warning.

Follow-up cleanup:

- Replaced the remaining `no-explicit-any` cast in the source checkout's
  `app/src/lib/mcp/transport.ts` with a typed MCP event handler map. This
  transport cleanup was already obsolete on the clean `origin/main` extraction
  branch and is not part of the PR branch.
- `pnpm run lint`: passed with 36 warnings, down from 37. Remaining warnings
  are React compiler `set-state-in-effect` warnings.
- `pnpm run typecheck`: passed.
- `git diff --check`: passed.

Follow-up cleanup:

- Moved mnemonic/recovery-phrase mode resets out of effects and into the mode
  switch event handlers in `app/src/pages/Mnemonic.tsx` and
  `app/src/components/settings/panels/RecoveryPhrasePanel.tsx`.
- Removed dead sidebar label reset state from `app/src/pages/Conversations.tsx`
  so the fixed tab model owns empty label categories directly.
- `pnpm run lint`: passed with 33 warnings, down from 36. Remaining warnings
  are React compiler `set-state-in-effect` warnings.

## Portfolio Readiness Note - 2026-05-13

Added `docs/PORTFOLIO_READINESS.md` with:

- validation evidence,
- cleanup summary,
- remaining React compiler warning debt,
- public claim boundary,
- next lint-policy slice.

No product behavior changes.

## Clean PR Extraction - 2026-05-13

Implementation branch: `codex/openhuman-portfolio-lint-readiness-upstream`.
Base: `upstream/main` at `83bc5648`.

The source checkout was already on `codex/operator-mvp-plan` with generated
architecture artifacts and several cleanup changes. The clean extraction keeps
only the still-relevant portfolio lint/readiness slice:

- `.gitignore` ignores local `.cocoindex_code/` index output.
- `app/src/components/settings/panels/RecoveryPhrasePanel.tsx` moves recovery
  phrase mode resets into the explicit mode switch handler.
- `app/src/pages/Mnemonic.tsx` moves mnemonic mode resets into the explicit
  mode switch handler.
- `app/src/pages/Conversations.tsx` removes dead label reset state now that the
  fixed tab model owns label categories.
- `docs/PORTFOLIO_READINESS.md` records validation evidence and the remaining
  warning boundary.

Excluded:

- Generated `docs/architecture/` scan artifacts.
- Obsolete source-checkout changes that were already absent from current
  `upstream/main`, including the MCP transport cast cleanup and removed meet call
  service cleanup.
- Broader Core RPC, config, or scanner-memory architecture refactors.

Gemini secondary review:

- Pre-review was attempted with `gemini-2.5-flash`, but Gemini repeatedly
  returned 429 model-capacity errors. This branch therefore has local validation
  evidence but no completed Gemini review for the OpenHuman slice.

Validation on the upstream-based clean extraction branch:

```bash
pnpm install
```

Result: passed after the upstream rebase installed the missing current
workspace packages from the local pnpm store. Build scripts were left
unapproved.

```bash
pnpm run lint
```

Result: passed with 35 warnings. Remaining warnings are
`react-hooks/set-state-in-effect`.

```bash
pnpm run typecheck
```

Result: passed.

```bash
pnpm --filter openhuman-app exec vitest run --config test/vitest.config.ts src/pages/__tests__/Conversations.test.tsx src/pages/__tests__/Conversations.render.test.tsx src/components/settings/panels/__tests__/RecoveryPhrasePanel.test.tsx
```

Result: passed, `3` files and `24` tests. Vitest still emitted the known Node
`localStorage is not available` warning.

```bash
git diff --check
```

Result: passed.

```bash
pnpm --filter openhuman-app exec prettier --check .
```

Result: passed from `app/`.

```bash
cargo fmt --manifest-path Cargo.toml --all --check
cargo fmt --manifest-path app/src-tauri/Cargo.toml --all --check
cargo check --manifest-path app/src-tauri/Cargo.toml
```

Result: passed. `cargo check` emitted existing Rust warnings.
