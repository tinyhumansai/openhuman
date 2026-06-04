# Dev Workflow — Autonomous Issue Crusher

## Vision

An autonomous developer agent that runs on a schedule, picks GitHub issues assigned to the user, and raises PRs automatically. Users configure everything from a settings page — repo, fork/upstream, target branch, and frequency.

---

## Phase 1 — Config UI ✅ DONE

**PR:** [tinyhumansai/openhuman#2703](https://github.com/tinyhumansai/openhuman/pull/2703)
**Branch:** `feat/dev-workflow-panel`

### What was built
- Settings panel at **Settings > Advanced > Dev Workflow**
- Pre-checks GitHub connection via Composio `listConnections()`
- Fetches user's repos via `composio_execute('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER')`
- Auto-detects forks → shows upstream repo info
- Fetches branches from upstream via `composio_execute('GITHUB_LIST_BRANCHES')`
- Schedule presets: 30min, 1hr, 2hr, 6hr, daily
- Config saved to localStorage (temporary — Phase 2 moves to core)
- All UI strings internationalized via `t()` with 37 i18n keys

### What was also fixed
- Updated 5 outdated Composio GitHub tool slugs in the curated catalog (`tools.rs`)
- Removed 3 deprecated tools no longer in Composio's catalog
- Added `listGithubRepos()` wrapper and `ComposioGithubReposResponse` types to `composioApi.ts`

### Files created/modified
| File | Action |
|------|--------|
| `app/src/components/settings/panels/DevWorkflowPanel.tsx` | Created |
| `app/src/pages/Settings.tsx` | Route added |
| `app/src/components/settings/panels/DeveloperOptionsPanel.tsx` | Nav link added |
| `app/src/components/settings/hooks/useSettingsNavigation.ts` | Route type + breadcrumbs |
| `app/src/lib/composio/composioApi.ts` | `listGithubRepos()` wrapper |
| `app/src/lib/composio/types.ts` | `ComposioGithubRepo`, `ComposioGithubReposResponse` |
| `app/src/lib/i18n/en.ts` + all locale chunks | i18n keys |
| `src/openhuman/memory_sync/composio/providers/github/tools.rs` | Slug fixes |
| `src/openhuman/memory_sync/composio/providers/github/provider.rs` | Slug refs |
| `src/openhuman/memory_sync/composio/providers/github/sync.rs` | Comment refs |
| `src/openhuman/memory_sync/composio/providers/github/tests.rs` | Test assertions |

---

## Phase 2 — Wire Config to Execution

**Blocked on:** [tinyhumansai/openhuman#2707](https://github.com/tinyhumansai/openhuman/pull/2707) (codegraph + skills registry)

### 2a. Create Dev Workflow skill definition

Create a bundled skill at `src/openhuman/agent/agents/dev_workflow/` (or `skills/dev_workflow/`):

**`skill.toml`:**
```toml
id = "dev_workflow"
display_name = "Dev Workflow"
when_to_use = "Autonomous developer — picks GitHub issues assigned to the user and raises pull requests."
temperature = 0.3
max_iterations = 30
sandbox_mode = "sandboxed"

[model]
hint = "coding"

[tools]
named = [
  "shell", "file_read", "file_write", "git_operations",
  "grep", "glob", "list", "edit", "apply_patch",
  "web_fetch", "composio"
]

[[inputs]]
name = "repo"
description = "Fork repo full name (e.g. user/repo)"
required = true

[[inputs]]
name = "upstream"
description = "Upstream repo full name (e.g. org/repo)"
required = true

[[inputs]]
name = "target_branch"
description = "Branch to raise PRs against"
required = true

[[inputs]]
name = "fork_owner"
description = "GitHub username of the fork owner"
required = true
```

**`SKILL.md`:** (the agent's instructions)
```markdown
You are an autonomous developer agent. Your job is to pick a GitHub issue and deliver a PR.

## Per-run workflow

1. **Pick issue**: Use Composio GITHUB_LIST_REPOSITORY_ISSUES on the upstream repo,
   filtered to issues assigned to the user. Pick the oldest open issue with no linked PR.
2. **Clone & branch**: Clone the fork repo. Add upstream as remote. Fetch target branch.
   Create branch `dev-workflow/<issue-number>-<slug>` off upstream's target branch.
3. **Index**: Use codegraph_index to build a retrieval index of the repo.
4. **Implement**: Read the issue carefully. Use codegraph_search to find relevant files.
   Implement a minimal, correct fix/feature. Follow existing code style.
5. **Test**: Detect and run available test commands. Fix failures before proceeding.
6. **Push**: Commit with `Fixes #<number>`. Push branch to fork remote.
7. **Open PR**: Use Composio GITHUB_CREATE_A_PULL_REQUEST against upstream's target branch.
   Include issue link, summary, and test results.

## Rules
- One PR per run. After opening the PR, stop.
- If no suitable issue exists, exit cleanly.
- Never force-push. Never push to upstream directly.
- If too large/risky, comment on the issue and skip.
```

### 2b. Add `cron_add` RPC

**Not blocked on #2707** — can be done now.

The `cron_add` logic exists in the agent tool (`src/openhuman/cron/tools/add.rs`) but isn't exposed as an RPC controller. Need to add it to `src/openhuman/cron/schemas.rs`.

**Changes:**
- `src/openhuman/cron/schemas.rs` — Add `"add"` controller with inputs: `name`, `schedule`, `prompt`, `session_target`, `model`, `agent_id`, `delivery`, `delete_after_run`
- `app/src/utils/tauriCommands/cron.ts` — Add `openhumanCronAdd()` wrapper
- `app/src/utils/tauriCommands/index.ts` — Re-export

### 2c. Wire Save → Cron + Skill

Update `DevWorkflowPanel.tsx` save handler:
- When user clicks "Save Configuration" → call `openhumanCronAdd()` with:
  - `name: "dev-workflow-<repo>"`
  - `schedule: { kind: 'cron', expr: selectedSchedule }`
  - `agent_id: "dev_workflow"`
  - `prompt: "Run dev_workflow for repo ${repo}. Upstream: ${upstream}. Target: ${branch}. Fork owner: ${forkOwner}."`
  - `delivery: { mode: 'proactive' }`
- Remove localStorage — config lives in the cron job
- Show active cron job status instead of localStorage-based summary

---

## Phase 3 — Execution Polish

### Agent execution improvements
- Smarter issue picking: filter by labels, skip issues with existing PRs
- Git worktree support: don't pollute the user's working tree
- Codegraph integration: use `codegraph_index` + `codegraph_search` for file discovery
- Test detection: auto-detect `npm test`, `cargo test`, `pytest`, etc.

### Error handling
- Retry on transient failures (network, rate limits)
- Skip issues that are too complex (>N files changed)
- Comment on skipped issues explaining why

### Delivery & notifications
- `delivery.mode = 'proactive'` → shows in the active chat channel
- Notification when PR is opened
- Summary of what was done

### UI enhancements
- Show run history in the Dev Workflow panel
- Show active/paused status
- Manual trigger button ("Run now")
- View logs from past runs

---

## Dependency Graph

```
Phase 1 (Config UI)     ──── ✅ DONE
        │
        ▼
Phase 2a (Skill def)    ──── Blocked on #2707
Phase 2b (cron_add RPC) ──── NOT blocked, can start now
        │
        ▼
Phase 2c (Wire save)    ──── Blocked on 2a + 2b
        │
        ▼
Phase 3 (Polish)        ──── Blocked on Phase 2
```

---

## Related PRs & Issues

| Item | Link | Status |
|------|------|--------|
| Config UI | [openhuman#2703](https://github.com/tinyhumansai/openhuman/pull/2703) | Open (draft) |
| Codegraph + Skills | [openhuman#2707](https://github.com/tinyhumansai/openhuman/pull/2707) | Open (draft) |
| Backend repo endpoint | `tinyhumansai/backend#842` | Open |
