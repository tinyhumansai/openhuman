# Cursor Cloud Agents — parallel workflow

Operator playbook for running 15–20 [Cursor Cloud Agents](https://docs.cursor.com/agents/cloud) in parallel against OpenHuman. Companion to [`codex-pr-checklist.md`](codex-pr-checklist.md); the same merge gates apply.

This doc closes [`tinyhumansai/openhuman#1480`](https://github.com/tinyhumansai/openhuman/issues/1480).

## TL;DR

1. Write a **batch spec** — one JSON file naming N agents, their issues, branches, and owned paths.
2. **Validate** it (`pnpm agent-batch validate <spec>`) and **prove ownership disjointness** (`pnpm agent-batch overlap <spec>`).
3. Post **one launch comment** per agent (generated from the spec) into Cursor; each agent opens a branch and PR matching the spec.
4. Track progress with `pnpm agent-batch status <spec>` — markdown table of PR + CI per agent.
5. Pilot at N=3 before scaling to 15–20.

Concretely, none of this is "Cursor magic" — it is a JSON contract + three small scripts that fail loudly if humans break it.

## Why a contract

Running N agents in parallel breaks in three ways:

- **Branch / PR collisions** — two agents picking the same branch name, or opening duplicate PRs against the same issue.
- **File collisions** — two agents editing the same module, producing conflicting merges.
- **Quality drift** — agents skipping format / typecheck / coverage and pushing red PRs.

The batch spec is the single source of truth that prevents the first two. The third is enforced by upstream CI ([`.github/workflows/coverage.yml`](../../.github/workflows/coverage.yml), [`.github/workflows/pr-quality.yml`](../../.github/workflows/pr-quality.yml), [`.github/workflows/test.yml`](../../.github/workflows/test.yml)) — agents do not get to opt out.

## Batch spec

A batch is a JSON file living under `docs/agent-workflows/batches/` (gitignored — see [Privacy](#secrets-posture)) or generated ad hoc. Shape:

```json
{
  "batch_id": "pilot-2026-05-15",
  "base_repo": "tinyhumansai/openhuman",
  "base_branch": "main",
  "tracking_issue": 1480,
  "agents": [
    {
      "id": "a01",
      "issue": 1234,
      "title": "short slug for the branch name",
      "branch": "cursor/a01-1234-short-slug",
      "owned_paths": ["app/src/features/foo/", "src/openhuman/foo/"],
      "allowed_shared_paths": ["docs/TEST-COVERAGE-MATRIX.md"],
      "labels": ["cursor-agent", "pilot"]
    }
  ]
}
```

Field rules:

| Field                           | Required | Notes                                                                                                                                                                                             |
| ------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `batch_id`                      | yes      | Stable identifier — appears in PR bodies and the tracking comment.                                                                                                                                |
| `base_repo`                     | yes      | Always `tinyhumansai/openhuman` unless explicitly delegated.                                                                                                                                      |
| `base_branch`                   | yes      | `main`.                                                                                                                                                                                           |
| `tracking_issue`                | yes      | One upstream issue per batch; that issue's comment thread is the dashboard (AC #6).                                                                                                               |
| `agents[].id`                   | yes      | Two-char + digits, e.g. `a01`–`a20`. Unique within batch.                                                                                                                                         |
| `agents[].issue`                | yes      | Upstream issue number. **One issue per agent**, **one agent per issue.**                                                                                                                          |
| `agents[].branch`               | yes      | Must start with `cursor/` and contain the agent id and issue number.                                                                                                                              |
| `agents[].owned_paths`          | yes      | **Path prefixes** (directory ending in `/`) or exact files. **No globs.** Disjoint across agents — `overlap` enforces this.                                                                       |
| `agents[].allowed_shared_paths` | no       | Files the agent may touch even if another agent's prefix contains them (e.g. `docs/TEST-COVERAGE-MATRIX.md`, capability catalog). Best-effort only — overlap on these is **warned**, not blocked. |
| `agents[].labels`               | no       | PR labels. Always include `cursor-agent`. Add `docs` or `chore` to opt out of the soft `pr-quality` checks per [`pr-quality.yml`](../../.github/workflows/pr-quality.yml).                        |

### Why prefixes, not globs

A glob ownership model (`app/src/components/**`) is tempting but makes overlap detection ambiguous: does `app/src/**/*.test.ts` collide with `app/src/components/Foo/`? With prefixes the answer is mechanical: prefix containment in either direction = collision. CI files like `.github/workflows/*.yml`, the capability catalog at `src/openhuman/about_app/`, and similar shared surfaces should be assigned to **one** agent for the batch (or to no agent — the batch should be designed not to need them).

## Branch & PR conventions

- Branch off **`origin/main` (upstream)** at the moment the spec is written. Each agent fetches `origin/main` itself.
- Branch name format: `cursor/<id>-<issue>-<short-slug>` (e.g. `cursor/a04-1456-memory-namespace`). Enforced by `validate.mjs`.
- Push to the **forking remote the Cursor workspace is configured with**, not directly to `tinyhumansai/openhuman`. PRs are opened with `--head <fork-owner>:<branch>` against `tinyhumansai/openhuman:main`.
- **One PR per issue**, **one PR per branch**. If a retry is needed, update the existing PR; do not open a duplicate. Use the duplicate cleanup recipe in [`codex-pr-checklist.md`](codex-pr-checklist.md#duplicate-pr-cleanup).
- PR title: `<area>: <short imperative> (#<issue>)`. PR body **must** follow [`.github/PULL_REQUEST_TEMPLATE.md`](../../.github/PULL_REQUEST_TEMPLATE.md) verbatim, including the `## AI Authored PR Metadata` section.
- PR labels include at minimum `cursor-agent` and the batch id label `batch:<batch_id>` so the tracking comment can find them.

## Ownership boundaries

Disjointness rule: for any two agents A and B in the batch, no path prefix in `A.owned_paths` may be a prefix of any path in `B.owned_paths`, in either direction. `scripts/agent-batch/overlap.mjs` checks this and exits non-zero on a collision.

The rule applies to **prefixes**, not file existence. Two agents may own paths that don't exist yet (new modules), as long as no prefix contains another.

If two issues genuinely need the same module, **do not split them across agents**. Combine them into a single agent's scope or sequence the work.

## Quality gates

Agents run the same gates as any other PR. The launch comment instructs them explicitly — they do not get to drop any of these:

- **Format**: `pnpm --filter openhuman-app format:check`, `cargo fmt --manifest-path Cargo.toml --all --check`, and the Tauri shell equivalent if shell files changed.
- **Lint / typecheck**: `pnpm lint`, `pnpm typecheck`.
- **Tests (focused)**: targeted Vitest for changed files, focused Rust tests via `pnpm debug rust <filter>` for changed Rust.
- **Coverage**: agents must run `pnpm test:coverage` and `pnpm test:rust` locally and add tests for changed lines. The merge gate is `≥ 80% diff coverage`, enforced server-side by [`coverage.yml`](../../.github/workflows/coverage.yml). PRs below the threshold do not merge — agents that cannot reach the threshold must say so in the PR body, not paper over it.
- **PR checklist + coverage matrix**: [`pr-quality.yml`](../../.github/workflows/pr-quality.yml) checks the PR body and `docs/TEST-COVERAGE-MATRIX.md`. The `docs` and `chore` labels exempt a PR from these soft gates — use them only for PRs that genuinely change no behavior.

If the agent's environment cannot run a gate, the PR body must report the **exact command and error** under `### Validation Blocked`, not claim it passed. This is the same rule as the codex checklist.

## Secrets posture

Cursor Cloud Agents inherit env from the workspace. For OpenHuman, the cloud workspace MUST be configured with:

- **No** `STAGING_*` / `PRODUCTION_*` secrets.
- **No** `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, or any other LLM provider key used by the production agent runtime — agents do code work, not LLM calls into production providers.
- A scoped `GITHUB_TOKEN` with `contents:write` and `pull_requests:write` on the **fork** the workspace pushes to, plus `pull_requests:write` on `tinyhumansai/openhuman` for PR creation. **No `admin:*`, no `actions:write`, no `secrets:*`.**
- `OPENHUMAN_APP_ENV` MUST be unset or set to `dev`. Never `staging` or `production` — staging writes `~/.openhuman-staging/core.token` referenced by [`AGENTS.md`](../../AGENTS.md) "Cursor Cloud specific instructions" and that token is **per-developer**, not for shared cloud workspaces.
- `.env.local`, `app/.env.local`, and `core.token` files are gitignored and must not be committed.

The agent's own environment is the smallest blast-radius surface. Production credentials are out of scope for code-writing agents.

## Progress visibility

One tracking issue per batch (`tracking_issue` in the spec). The launch script posts a **single comment** on that issue containing a markdown table generated by `scripts/agent-batch/status.mjs`:

| Agent | Issue | Branch              | PR      | CI        | Coverage | Status       |
| ----- | ----- | ------------------- | ------- | --------- | -------- | ------------ |
| a01   | #1234 | `cursor/a01-1234-…` | `#1234` | ✓ green   | 87%      | merged       |
| a02   | #1235 | `cursor/a02-1235-…` | `#1235` | × failing | —        | needs review |

Re-running `status` rewrites the same comment (looked up by a `<!-- batch:<id> -->` marker) so the issue thread doesn't fill with stale tables. The script reads:

- `gh pr list --repo tinyhumansai/openhuman --label batch:<id> --json …` for PR + state.
- `gh pr checks <pr>` for CI rollup.
- The `diff-coverage.md` artifact from `coverage.yml`, if downloaded — otherwise coverage shows `—`.

No external dashboard — GitHub issues + labels + the table are the single pane of glass.

## Pilot-then-scale

Do not launch 15–20 agents on day one.

1. **N=3 pilot.** Pick three issues that are small (~200 LOC each), in three distinct domains, no overlap. Run the full flow: spec → validate → overlap → launch → status → merge.
2. **N=5 expansion.** Once the N=3 pilot has 3 green PRs (merged or at-review), expand to 5 in one batch. Watch CI queue times and rate limits.
3. **N=15–20 production.** Only after two clean expansion batches. At this scale, watch `gh api rate_limit`, GitHub Actions concurrency, and Cursor's per-workspace agent limit.

If a batch surfaces a class of failure (e.g. agents inventing API names, agents skipping `cargo fmt`), fix the **launch comment template** that all agents inherit — don't fix it case-by-case.

## Operator quickstart

```bash
# 1. Draft the spec
cp docs/agent-workflows/pilot-batch-example.json /tmp/my-batch.json
$EDITOR /tmp/my-batch.json

# 2. Validate shape + naming
pnpm agent-batch validate /tmp/my-batch.json

# 3. Prove ownership disjointness
pnpm agent-batch overlap /tmp/my-batch.json

# 4. Generate one launch comment per agent (paste into Cursor)
pnpm agent-batch launch /tmp/my-batch.json --print-only

# 5. After agents have pushed, refresh the tracking comment
pnpm agent-batch status /tmp/my-batch.json --post
```

All scripts are in [`scripts/agent-batch/`](../../scripts/agent-batch/). They are zero-dep Node, executable from the repo root, and exit non-zero on policy violations so they are CI-friendly.

## Reference

- [`pilot-batch-example.json`](pilot-batch-example.json) — canonical example with 3 disjoint agents.
- [`scripts/agent-batch/`](../../scripts/agent-batch/) — validate / overlap / launch / status implementations + `node:test` suites.
- [`codex-pr-checklist.md`](codex-pr-checklist.md) — parent checklist; per-agent rules inherit from it.
- [`AGENTS.md`](../../AGENTS.md) and [`CLAUDE.md`](../../CLAUDE.md) — repo-wide rules every agent must follow.
