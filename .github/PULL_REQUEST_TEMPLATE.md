## Summary

- What changed and why.
- Keep this to 3-6 bullets focused on user-visible or architecture-impacting changes.

## Problem

- What issue or risk this PR addresses.
- Include context needed for reviewers to evaluate correctness quickly.

## Solution

- How the implementation solves the problem.
- Note important design decisions and tradeoffs.

## Submission Checklist

> If a section does not apply to this change, mark the item as `N/A` with a one-line reason. Do not delete items.

- [ ] Tests added or updated (happy path + at least one failure / edge case) per [`docs/TESTING-STRATEGY.md`](../docs/TESTING-STRATEGY.md#failure-path-requirement)
- [ ] **Diff coverage ≥ 80%** — changed lines (Vitest + cargo-llvm-cov merged via `diff-cover`) meet the gate enforced by [`.github/workflows/coverage.yml`](../.github/workflows/coverage.yml). Run `pnpm test:coverage` and `pnpm test:rust` locally; PRs below 80% on changed lines will not merge.
- [ ] Coverage matrix updated — added/removed/renamed feature rows in [`docs/TEST-COVERAGE-MATRIX.md`](../docs/TEST-COVERAGE-MATRIX.md) reflect this change (or `N/A: behaviour-only change`)
- [ ] All affected feature IDs from the matrix are listed in the PR description under `## Related`
- [ ] No new external network dependencies introduced (mock backend used per [`docs/TESTING-STRATEGY.md`](../docs/TESTING-STRATEGY.md#mock-policy))
- [ ] Manual smoke checklist updated if this touches release-cut surfaces ([`docs/RELEASE-MANUAL-SMOKE.md`](../docs/RELEASE-MANUAL-SMOKE.md))
- [ ] Linked issue closed via `Closes #NNN` in the `## Related` section

## Impact

- Runtime/platform impact (desktop/mobile/web/CLI), if any.
- Performance, security, migration, or compatibility implications.

## Related

<!--
Use a closing keyword so GitHub auto-closes the issue on merge. One per line.
Supported (case-insensitive): close/closes/closed, fix/fixes/fixed, resolve/resolves/resolved.
A bare "#123" reference is just a link — it does NOT close the issue.

  Closes #123
  Fixes  #456
-->

- Closes:
- Follow-up PR(s)/TODOs:
