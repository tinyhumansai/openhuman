# Weekly Code-Review Report

Scheduled aggregation of slow-moving code-health signals that per-PR CI does
not catch.

## What runs

Workflow: [`.github/workflows/weekly-code-review.yml`](../.github/workflows/weekly-code-review.yml).
Script: [`scripts/weekly-code-review.sh`](../scripts/weekly-code-review.sh).

The aggregator currently collects:

| Check           | Source                              | What it catches                                   |
| --------------- | ----------------------------------- | ------------------------------------------------- |
| Unused code     | `pnpm exec knip` (in `app/`)        | Unused files, exports, dependencies, types        |
| Rust advisories | `cargo audit` on core + Tauri shell | Published RustSec advisories against `Cargo.lock` |
| TODO backlog    | `grep` over `src/` + `app/src/`     | `TODO` / `FIXME` / `XXX` / `HACK` drift           |

Each sub-check is **best-effort**: a missing tool or transient failure is
reported inline in the Markdown, not fatal. A full lane going red never stops
the rest of the report from being produced.

## Schedule + manual trigger

- Cron: every Monday at **06:00 UTC** (`0 6 * * 1`).
- Manual: **Actions → Weekly Code Review → Run workflow**.
- Concurrency: one run at a time; subsequent triggers queue rather than cancel.

## Outputs

1. **Tracking issue** — created fresh every run, labeled `weekly-code-review`.
   Previous open reports are closed with a "superseded" comment so the
   maintainer triage view only shows the latest week.
2. **Artifact** — `weekly-code-review-<run-id>` with:
   - `report.md` — the human-readable body also used for the issue.
   - `report.json` — machine-readable digest (parsed check outputs) for any
     downstream tooling.
     Retention: 90 days.

## Running locally

From the repo root:

```bash
bash scripts/weekly-code-review.sh            # writes to weekly-code-review-out/
bash scripts/weekly-code-review.sh ./out      # custom dir
```

Dependencies: `pnpm` for knip, `cargo-audit` for Rust advisories, `python3`
for the JSON shaping. Missing tools are skipped with a note in the report.

## Triaging a report

- **Unused code** — knip findings are suggestions; check the linked file
  before deleting. Legitimate deletions land in a `chore(cleanup)` PR.
- **Rust advisories** — bump the affected crate (`cargo update -p <crate>`
  for a patch, or pin a workaround) and re-run `cargo audit` locally.
- **TODO backlog** — the counter is a direction signal, not an action item
  on its own. Watch for a rising trend over successive weeks.

## Disabling / overrides

- **One-off skip** — cancel the scheduled run from the Actions tab.
- **Pause indefinitely** — comment out the `schedule:` block in
  `.github/workflows/weekly-code-review.yml`. `workflow_dispatch` still works.
- **Retire** — delete the workflow + `scripts/weekly-code-review.sh` and
  remove the `weekly-code-review` label. No other code references them.

## Intentionally out of scope for the first cut

- npm audit: Yarn v1's `audit` output is messy and noisy; revisit when the
  project moves to Yarn berry or adopts `audit-ci` / GitHub's dependency
  review action.
- Bundle-size diff: needs a baseline to be meaningful; separate workflow.
- AI-assisted review: CodeRabbit already runs per-PR; duplicating weekly
  would be noise, not signal.
