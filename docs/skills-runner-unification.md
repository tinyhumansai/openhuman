# Skills Runner Unification

**Status**: Proposed — implementation in progress on `run/codegraph-full`.
**Owner**: codegraph-skills track.
**Related**: PR #2802 (dev-workflow active-config UX), PR #2864 (smart issue selection / cron prompt embedding), the `feat/codegraph-skills` series that introduced `SkillsRunnerBody`.

---

## TL;DR

We currently maintain **two parallel scheduling UIs** for the same underlying primitive (`openhuman.cron_*` RPCs against an agent `prompt`):

1. **`SkillsRunnerBody`** (generic, every bundled skill) — picks any skill, dynamically renders inputs from `openhuman.skills_describe`, can run-now or save-as-cron. Lists saved schedules as a read-only summary with run/remove buttons.
2. **`DevWorkflowPanel`** (dev-workflow-only) — bespoke smart-issue-picker UI for `dev-workflow`, with an "active configuration" card, enable/disable toggle, embedded run history with per-run output viewer.

`DevWorkflowPanel` is just a hard-coded specialization of what `SkillsRunnerBody` already does — same cron RPC, same `agent` job type, same prompt-template scheme. The polish that lives in `DevWorkflowPanel` (toggle, run-history-with-output, active-config card) should be lifted into `SkillsRunnerBody`, and the dev-workflow-specific GitHub issue picker should be extracted as a conditionally-mounted sub-component. Then `DevWorkflowPanel` retires.

---

## Current state

### `app/src/components/skills/SkillsRunnerBody.tsx` (990 lines)

Generic skill runner used by both Settings → Developer Options → Skills Runner AND the `/skills` "Runners" tab.

| Concern | Location |
| --- | --- |
| Per-skill cron-name prefix | line 94 (`CRON_NAME_PREFIX = 'skill-run-'`) |
| Cron-name builder (skill + inputs hash) | lines 100-113 |
| Generic agent-prompt builder (re-fires `skill_run` at tick) | lines 116-127 |
| `scheduledJobs` state (filtered by name prefix) | line 233 |
| Saved-schedule render list | lines 828-875 |
| Per-schedule actions | `handleRunJobNow` (526-536), `handleRemoveJob` (538-548) — **no enable/disable** |
| Recent-runs viewer (cross-skill or scoped) | lines 882-985 |
| Inline log viewer (per run, auto-tail) | lines 440-519 |

### `app/src/components/settings/panels/DevWorkflowPanel.tsx` (792 lines)

| Concern | Location |
| --- | --- |
| `dev-workflow-`-prefixed cron-name | line 376 |
| Fork detection via Composio | lines 190-303 |
| Branch dropdown | lines 707-732 |
| Embedded skill-instructions prompt | lines 338-373 |
| **Active-config card** (rendered at top when any job exists) | lines 486-647 |
| **Enable/disable toggle** | lines 433-444 + render at 502-516 |
| **Run-history rows with per-run expandable output** | lines 591-645 (render), 306-322 (fetch via `openhumanCronRuns`) |
| **`last_output` viewer** | lines 580-589 |

### What's NOT in `SkillsRunnerBody` today

- Enable/disable toggle per schedule (the cron RPC supports `openhumanCronUpdate(id, { enabled })` — DevWorkflowPanel:439 — already wired generically).
- Per-schedule run history (`openhumanCronRuns(jobId, limit)` exists in `cron.ts`; SkillsRunnerBody currently shows recent runs only via `skillsApi.recentRuns()` which scans the skill log directory, not the cron `runs` table).
- "Active config card" — surface the most-recently-active schedule prominently rather than as one row in a list.
- Anything dev-workflow-specific (the GitHub repo / fork / branch picker workflow with smart auto-detection).

---

## Why unify

1. **UX symmetry.** Users with multiple `dev-workflow` schedules across different repos (a real use case Cyrus's PR #2802 was prepping for) need exactly the same UI as users with multiple `pr-review-shepherd` schedules across different repos. Today there's a privileged surface for one skill.
2. **Cron RPCs are already generic.** `openhuman.cron_update`, `cron_runs`, `cron_run` all take a `job_id` — no skill-specific logic in the core. We have zero RPC work to do.
3. **`DevWorkflowPanel` is a hard-coded specialization.** It bakes `dev-workflow-` into the cron name and the prompt template into the file. Both are anti-patterns once the generic runner exists: `SkillsRunnerBody` already builds prompts that re-fire `skill_run`, which routes through the skill registry and gets the up-to-date `system.md`/inputs without re-deploying the panel.
4. **Smaller surface to maintain.** Two views means two test suites, two i18n key namespaces (`settings.devWorkflow.*` and `settings.skillsRunner.*`), two render bugs to fix. Cyrus shipped 8 fixes to DevWorkflowPanel in 4 weeks; every one would have benefited the generic runner if they'd shared code.

---

## Unified information architecture

```
/skills
  ├─ Library tab (existing)
  └─ Runners tab (existing — this is where SkillsRunnerBody lives)

Runners tab body (SkillsRunnerBody, after unification):

   ┌─ Skill picker ─────────────────────────────────────────────┐
   │ Skill: [ dev-workflow                ▾ ]                   │
   └────────────────────────────────────────────────────────────┘

   ┌─ Saved schedules for this skill ───────────────────────────┐
   │                                                            │
   │  ★ ACTIVE       dev-workflow-tinyhumansai-openhuman        │
   │                 every 2 hours · next run in 23m            │
   │                 last: ok · 47s ago             [⏵ toggle]  │
   │                 [Run now] [▾ history (5)] [Remove]         │
   │                 ▾ history                                  │
   │                   2026-05-29 13:01  ok    51s              │
   │                     <expandable per-run output pre>        │
   │                   2026-05-29 11:01  ok    49s              │
   │                                                            │
   │  ──────────────────────────────────────────────────────    │
   │                                                            │
   │  ○                dev-workflow-graycyrus-openhuman         │
   │                 daily @ 9am · paused                       │
   │                 last: error · 4h ago           [⏵ toggle]  │
   │                 [Run now] [▾ history] [Remove]             │
   │                                                            │
   │  [ + Add new schedule for dev-workflow ]                   │
   └────────────────────────────────────────────────────────────┘

   ┌─ Configure & run ──────────────────────────────────────────┐
   │ [skill description — what_to_use]                          │
   │                                                            │
   │ Inputs:                                                    │
   │   <schema-driven form, today>                              │
   │                                                            │
   │ When skill_id === 'dev-workflow', ALSO render:             │
   │   <SmartIssuePicker subcomponent — fork detection,         │
   │    GitHub-connected repo dropdown, branch dropdown>        │
   │                                                            │
   │ [Run now]   [Save as schedule …]                           │
   └────────────────────────────────────────────────────────────┘

   ┌─ Recent runs (skill-scoped) ───────────────────────────────┐
   │ (existing — unchanged; reads skill_log scan)               │
   └────────────────────────────────────────────────────────────┘
```

Notes:
- The **★ ACTIVE** treatment lifts DevWorkflowPanel's "active configuration" pattern but generalises it: any enabled schedule gets the same emphasis. If multiple are enabled, the most recently run one is "active" — first in the list, larger card.
- The "+ Add new schedule" affordance just reveals the existing inputs form (we don't need a second form; we just gate the existing one behind a disclosure so the saved-schedules list reads cleaner).
- Run history per schedule uses `openhuman.cron_runs` (the same RPC DevWorkflowPanel already uses) — this is *separate* from "Recent runs" at the bottom which uses `skillsApi.recentRuns()` scanning the skill log directory. Both have value: cron-run history is structured per-schedule with status/duration; skill log scan catches run-now invocations that don't go through a schedule.

---

## Migration / deprecation path

### Phase 1 — Design doc (this file).
### Phase 2 — Smoke prototype.
- Add enable/disable toggle to existing `scheduledJobs.map(...)` row in `SkillsRunnerBody.tsx`.
- Mirror `openhumanCronUpdate(jobId, { enabled: !job.enabled })` from DevWorkflowPanel:439 exactly.
- New i18n keys: `settings.skillsRunner.scheduleEnabled`, `settings.skillsRunner.scheduleDisabled` (full 12-locale chunk parity).
- Vitest unit coverage for toggle.

### Phase 3 — Full incremental implementation (one commit per chunk).
1. Per-schedule run-history + expandable output viewer (port DevWorkflowPanel:593-635).
2. Active-config card pattern (port DevWorkflowPanel:502-547).
3. Extract `SmartIssuePicker` subcomponent into `app/src/components/skills/SmartIssuePicker.tsx`; conditionally render in `SkillsRunnerBody` when `skillId === 'dev-workflow'` (with a TODO for a schema-driven `[input].picker = "github-issue"` upgrade — see Open questions).
4. Deprecate `DevWorkflowPanel`: replace its body with a one-line "moved to /skills" notice + nav link; OR strip it from the Settings nav and delete the file. Update or remove `DevWorkflowPanel.test.tsx` accordingly. Decision in Phase 3 chunk 4 once we see whether the Settings nav strip is clean.
5. i18n parity audit (`pnpm i18n:check`) — fold any new keys into all 12 non-English chunk files.

### Phase 4 — Verification.
- Playwright via CDP `http://127.0.0.1:19222` against the running dev app on Vite port 1428.
- Confirm: schedule toggle + expand work; smart-issue picker shows only for `dev-workflow`; switching to `github-issue-crusher` / `pr-review-shepherd` shows the generic form.
- Save `G:/tmp/oh-skills-unified.png`.
- Run `pnpm typecheck` + `pnpm debug unit` on changed files.

---

## Risk + test plan

### Risks

| Risk | Mitigation |
| --- | --- |
| **Breaking existing dev-workflow users** — they have a configured cron job named `dev-workflow-<repo>`; the new generic runner uses `skill-run-<skillId>-<hash>` as the prefix. | Filter saved-schedules list by **both** prefixes when `skillId === 'dev-workflow'` so existing jobs surface. Don't migrate names — they keep working. |
| **i18n drift** — there's already significant pre-existing drift in `en-N.ts` chunks for `settings.skillsRunner.*` keys (audit log shows ~50 keys in `en.ts` not in chunks). | Address chunk drift for the **new** keys this PR adds (toggle, history, smart-picker). Pre-existing drift is out of scope but worth a follow-up. |
| **`DevWorkflowPanel` removal breaks the deep link or settings nav route.** | Check `settings/AppSettings.tsx` (or wherever the nav is wired) and either preserve the route as a redirect to `/skills` or update the nav entry. |
| **Coverage gate (≥80% on changed lines)** on a 990-line component. | Add focused tests per Phase 3 chunk: toggle test (Phase 2), history-expand test (chunk 1), active-card render test (chunk 2), conditional-picker test (chunk 3). |
| **Stale-component test deletion regressing dev-workflow coverage.** | If `DevWorkflowPanel.test.tsx` is deleted, ensure equivalent coverage exists in `SkillsRunnerBody.test.tsx` for the smart-picker path. |

### Test plan

Per phase commit:

- **Phase 2**: `SkillsRunnerBody.test.tsx` — render one saved job, click toggle, assert `openhumanCronUpdate` called with `{ enabled: false }`, assert refresh-list invoked.
- **Phase 3.1**: render one saved job with `runHistory`, click expand, assert per-run output `<pre>` visible. Assert `openhumanCronRuns(jobId, 5)` called on render.
- **Phase 3.2**: render multiple jobs, assert most-recent-active sorted to top with `data-testid="active-schedule"` (or equivalent). Assert non-enabled jobs sorted below.
- **Phase 3.3**: render with `skillId === 'dev-workflow'`, assert `SmartIssuePicker` present. Render with `skillId === 'github-issue-crusher'`, assert it's absent. Subcomponent's own tests cover Composio loading/error paths (move from `DevWorkflowPanel.test.tsx`).
- **Phase 3.4**: if DevWorkflowPanel becomes a redirect, assert nav still routes correctly; if deleted, delete the test file.

---

## Open questions

1. **Schema-driven pickers vs. hard-coded `if (skillId === 'dev-workflow')`.** The clean answer is to extend `skill.toml`'s `[[inputs]]` schema with an optional `picker = "github-issue"` field, and let the runner route to a `SmartIssuePicker` subcomponent based on the picker key. The shortcut answer is the hard-coded `if`. Phase 3 chunk 3 ships the shortcut with a `TODO(picker-schema)` comment. The schema upgrade is a follow-up issue worth filing — it would also benefit `RepoPicker` and `BranchPicker`, which today route by *name convention* (`REPO_INPUT_NAMES` / `BRANCH_INPUT_NAMES` sets in SkillsRunnerBody:42-55), which is brittle.
2. **One enabled schedule per (skill, inputs) combo, or many?** Today DevWorkflowPanel allows only one (it looks up `name?.startsWith('dev-workflow')` and updates in place). `SkillsRunnerBody` allows many (the cron-name hash includes input values, so two different repos produce two jobs). The unified UX should keep "many" — but display only one as the prominent "ACTIVE" card. Pre-existing `dev-workflow-<repo>` jobs will surface in the list once we add the dual-prefix filter.
3. **Run history retention.** DevWorkflowPanel pulls last 5; SkillsRunnerBody's bottom "Recent runs" scans last 10 log files. Unify on a single source? For now, keep both — the per-schedule history is structured cron data, the bottom list is a cross-cutting "what ran lately" surface useful even with no schedules. Worth re-evaluating once both surfaces are in production for a few weeks.
4. **`last_output` field on `CoreCronJob` vs. per-run output on `CoreCronRun`.** Today both exist (`cron.ts:42-43` and `cron.ts:51`). DevWorkflowPanel renders `existingJob.last_output` in the active card AND per-run `run.output` in history rows. After unification, drop the duplicated `last_output` block; the per-run history already shows the most recent run's output. Lightweight change.
5. **Deprecation timing for the Settings → Developer Options → Dev Workflow nav entry.** Strip immediately (Phase 3 chunk 4), or leave as a redirect for one release? Leaning toward strip — the user is the maintainer, the panel is dev-only, and `/skills` is more discoverable than a buried Settings sub-page.
