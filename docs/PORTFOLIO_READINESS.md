# Portfolio Readiness

This note is for engineering review and portfolio handoff. It does not change
the product surface or marketing README.

## What This Repo Demonstrates

- A large local-first desktop AI product with TypeScript UI, Rust/Tauri shell,
  and a Python/Rust-adjacent core surface.
- Integration-heavy product architecture: local memory, account connectors,
  desktop shell commands, native tools, and testable app services.
- Real validation breadth: TypeScript compile, ESLint, Vitest, and Rust
  mock-backed tests all run locally.

## Current Validation Evidence

Run from the repo root:

```bash
pnpm run typecheck
pnpm run lint
pnpm --filter openhuman-app exec vitest run --config test/vitest.config.ts src/pages/__tests__/Conversations.test.tsx src/pages/__tests__/Conversations.render.test.tsx src/components/settings/panels/__tests__/RecoveryPhrasePanel.test.tsx
pnpm --filter openhuman-app exec prettier --check .
cargo fmt --manifest-path Cargo.toml --all --check
cargo fmt --manifest-path app/src-tauri/Cargo.toml --all --check
cargo check --manifest-path app/src-tauri/Cargo.toml
git diff --check
```

Latest clean-branch portfolio-readiness run:

- `pnpm run typecheck`: passed.
- `pnpm run lint`: passed with 35 warnings. The remaining warning family is
  React compiler `set-state-in-effect`.
- Focused Vitest coverage for the touched React areas passed: `3` files and
-  `24` tests across Conversations and Recovery Phrase panel tests.
- `pnpm --filter openhuman-app exec prettier --check .`: passed from `app/`.
- Rust format checks for the root and Tauri manifests passed.
- `cargo check --manifest-path app/src-tauri/Cargo.toml`: passed with existing
  Rust warnings.
- `git diff --check`: passed.

## Cleanup Performed

- Moved mnemonic/recovery-phrase mode resets into the explicit mode switch
  handlers.
- Removed dead sidebar label reset state now that conversation labels use a
  fixed tab model.
- Ignored local `.cocoindex_code/` index output so code-index experiments do
  not dirty the repo.
- Recorded validation evidence in `CODEX_WORKPAD.md`.

## Remaining Presentation Debt

- The lint output is not presentation-clean yet because 35 React compiler
  warnings remain.
- Most warnings are synchronous state updates inside effects. Some may be
  harmless legacy patterns, but they should be either refactored or explicitly
  accepted as a policy before this repo is used as a polished flagship example.
- Vitest currently emits repeated Node `localStorage is not available` warnings;
  tests pass, but the environment warning should be silenced or documented.

## Public Claim Boundary

Safe to claim:

- The repository has broad local validation across TypeScript, lint, JS tests,
  and Rust tests.
- The current cleanup reduced generic lint noise without changing product
  behavior.

Do not claim yet:

- "Lint-clean" or "warning-free."
- Full UI runtime readiness across every Tauri/desktop flow.
- That the remaining React compiler warnings have been reviewed and accepted.

## Next Slice

Create a narrow lint-policy slice:

1. Pick one warning family, starting with `react-hooks/set-state-in-effect`.
2. Classify warnings into real refactors vs accepted legacy patterns.
3. Fix the highest-risk components first.
4. Keep `pnpm run typecheck`, `pnpm run lint`, and the relevant Vitest tests
   green after each group.
