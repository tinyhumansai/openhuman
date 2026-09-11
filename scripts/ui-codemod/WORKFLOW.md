# UI migration workflow

`scripts/ui-codemod/report.mjs` produces the work list; the `ui-migration-wave`
workflow consumes it one wave at a time.

## Why waves, and why one checkout

Agents run **concurrently in a single checkout**, partitioned by file. They must
never be given a nested git worktree: this repo is a submodule of a superproject,
and a nested worktree there shares `.git/modules` HEAD and silently corrupts the
gitlink (see the superproject CLAUDE.md).

Disjoint file ownership is therefore the only thing keeping two agents off one
file, which is why `report.mjs` guarantees buckets are disjoint and why the
workflow chunks a large bucket rather than handing 41 files to one agent.

## Shared files no migration agent may touch

Each is shared across every bucket, so a concurrent edit is a lost write:

| File | Owner |
| --- | --- |
| `src/components/ui/index.ts` | the barrel agent, after the primitives phase |
| `src/lib/i18n/*.ts` | nobody in a wave — agents report needed keys instead |
| `src/test/setup.ts`, `tailwind.config.js`, `pnpm-lock.yaml` | nobody: each forces the full serial CI suite |
| `src/services/analyticsInteractions.ts` | nobody: read-only `data-analytics-id` contract |

## Running a wave

```bash
node scripts/ui-codemod/report.mjs              # see what is left
node scripts/ui-codemod/report.mjs --json       # workflow input
node scripts/ui-codemod/report.mjs --bucket components/settings
```

Then invoke the `ui-migration-wave` workflow with `{ primitives: [...], buckets: [...] }`.
Between waves, run the full suite and the diff-cover dry run yourself — the
workflow's gate is scoped to the files it touched, which is faster but narrower.
