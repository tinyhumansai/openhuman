# OpenHuman App Knip Cleanup Implementation Plan

> **Execution note:** Carry this plan out from
> `/Users/enamakel/work/workflow-openhuman/worktrees/knip-cleanup/openhuman` on
> `chore/knip-cleanup`. Use Node 24 (`source "$HOME/.nvm/nvm.sh" && nvm use 24`)
> for every pnpm/Knip command. Do not push, open a PR, edit Rust, touch another
> umbrella submodule, or expand the cleanup beyond `app/`.

**Goal:** Make Knip's OpenHuman app analysis trustworthy, then remove unreachable
TypeScript/React files, unused package dependencies, and unnecessary exported
surface without changing runtime behavior.

**Design:** Treat Knip as a static-analysis regression test, not as an automatic
deletion list. First model every Vite, Vitest, WebdriverIO, Playwright, Tauri,
package-script, and shell-launched entry point. Then process findings in
subsystem-sized slices, independently checking static imports, barrel exports,
string references, package scripts, test configuration, Tauri configuration,
and shell/workflow invocation before changing a file. Delete a declaration only
when it is unused; when the declaration is used locally, remove only `export`.
Keep intentional contracts through the narrowest documented Knip exception.

**Scope:** `app/knip.json`, `app/package.json`, the applicable pnpm lockfiles,
and Knip-confirmed dead or over-exported TypeScript/TSX under `app/src/` and
`app/test/`. Generated assets, `app/src-tauri/`, vendored sources, Rust, public
docs, and unrelated formatting/refactors are out of scope.

---

## Mandatory execution discipline

Apply these rules to every task below:

1. Start with `git status --short --branch`. Stop if another agent has modified
   a file needed by the task; coordinate instead of reverting or overwriting it.
2. Run the stated **red** Knip query before editing and save the relevant output
   in the terminal/log, not in the repository.
3. Independently verify every candidate with `rg` before modifying it:

   ```bash
   rg -n '<symbol-or-file-stem>' app package.json scripts .github \
     --glob '!app/pnpm-lock.yaml' --glob '!pnpm-lock.yaml' \
     --glob '!app/src-tauri/vendor/**' --glob '!node_modules/**' \
     --glob '!target/**'
   ```

   Also inspect `app/package.json`, `app/index.html`, `app/vite.config.ts`,
   `app/test/vitest.config.ts`, `app/test/wdio.conf.ts`,
   `app/playwright.config.ts`, `app/scripts/`, `app/src-tauri/tauri.conf.json`,
   and `.github/workflows/` whenever the candidate could be loaded by name,
   convention, a script, or Tauri.

4. Make the smallest change:
   - delete a file/declaration only when no runtime, build, test, documentation,
     generated-manifest, tooling, or dynamic-loading role exists;
   - remove only `export` when the declaration is used in its own module;
   - remove one side of a duplicate named/default export and update imports to
     the retained form;
   - keep intentional public/test seams with the narrowest possible
     `app/knip.json` exception and an adjacent reason.
5. Run the stated **green** checks. A targeted Knip category must have no
   actionable findings in the files handled by the step and must introduce no
   new `files`, `dependencies`, or `unlisted` findings.
6. Run Prettier only on the explicitly touched files:

   ```bash
   pnpm --filter openhuman-app exec prettier --check <explicit paths>
   ```

7. Review `git diff --check` and `git diff -- <explicit paths>`.
8. Commit immediately after validation. Manually enumerate every touched path;
   never use a glob, directory, `git add`, or command substitution:

   ```bash
   atomic-commit "<scoped message>" -- path/to/file1 path/to/file2
   ```

9. Do not begin the next task until the current task is committed.

If deleting dead production code leaves a test whose only purpose was to test
that dead code, independently verify that fact and delete the orphan test in
the same atomic commit. A test is not evidence that a production entry point is
live.

## Baseline

With Node 24, the approved-design baseline currently reports:

- 34 unused files;
- `lottie-react` and `react-ga4` as unused dependencies;
- direct but undeclared imports of `webdriverio` and `@wdio/globals`;
- 131 unused value exports;
- 276 unused exported types;
- one unused enum-member group;
- 41 duplicate exports.

The counts may shrink after entry-point correction. Always use the post-config
report as the deletion authority.

---

### Task 1: Model all real app and tooling entry points

**Files:**

- Modify: `app/knip.json`
- Modify only if direct dependency evidence requires it:
  `app/package.json`, `pnpm-lock.yaml`, `app/pnpm-lock.yaml`

**Step 1: Record the red baseline**

Run:

```bash
source "$HOME/.nvm/nvm.sh"
nvm use 24
pnpm --filter openhuman-app exec knip --config knip.json --reporter compact
```

Expected: non-zero with `test/wdio.conf.ts` incorrectly reported as unused,
unlisted `webdriverio`/`@wdio/globals`, and the remaining cleanup candidates.

**Step 2: Prove each indirect entry**

Run and inspect:

```bash
rg -n 'src/main\.tsx|vite\.config|vitest\.config|wdio\.conf|playwright\.config|build-parallel|e2e-run-session' \
  app/index.html app/package.json app/scripts scripts .github
rg -n 'test/e2e/specs|test/playwright/specs|src/test/setup' \
  app/test app/playwright.config.ts app/package.json app/scripts .github
rg -n 'webdriverio|@wdio/globals|@wdio/types' app/test app/package.json
```

Evidence that must be preserved:

- `app/index.html` loads `src/main.tsx`;
- package build scripts execute `scripts/build-parallel.mjs`, which invokes the
  Vite build;
- unit scripts load `test/vitest.config.ts`, whose `setupFiles` loads
  `src/test/setup.ts` and whose `include` globs load app tests;
- `app/scripts/e2e-run-session.sh` executes
  `pnpm exec wdio run test/wdio.conf.ts`;
- `test/wdio.conf.ts` loads all `test/e2e/specs/**/*.spec.ts`;
- Playwright configuration loads `test/playwright/specs/**/*.spec.ts`;
- WebdriverIO types/globals imported by source are direct dev dependencies,
  even if another WDIO package currently installs them transitively;
- Tauri/Vite/package-script binaries are real even when referenced only in a
  package script or shell script.

**Step 3: Correct Knip configuration narrowly**

Update `app/knip.json` so the verified configuration/spec/tooling files are
explicit entries. At minimum the graph must include:

```json
{
  "entry": [
    "src/main.tsx",
    "vite.config.ts",
    "test/vitest.config.ts",
    "test/wdio.conf.ts",
    "playwright.config.ts",
    "test/e2e/specs/**/*.spec.ts",
    "test/playwright/specs/**/*.spec.ts"
  ]
}
```

Retain `project` coverage for app and test TypeScript. Add a package-script or
tooling script as an entry only when Knip does not discover its verified
invocation. Do not add a broad `scripts/**`, `test/**`, or `src/**` entry merely
to make findings disappear.

Remove an `ignoreDependencies` or `ignoreBinaries` entry only after the
corrected graph recognizes the real usage. Keep an exception only when the
package manager, Tauri, or a shell script invokes it in a way Knip cannot model,
and document the exact invocation next to that exception.

**Step 4: Declare direct WDIO imports**

If the corrected graph still reports the existing imports as unlisted, add
`@wdio/globals` and `webdriverio` to `app/devDependencies` at the same `9.24.x`
family used by the other WDIO packages. Add `@wdio/types` too if Knip or
`pnpm why @wdio/types` confirms `test/wdio.conf.ts` relies on it transitively.
Regenerate only the lockfile sections affected by those declarations. Do not
upgrade unrelated packages.

**Step 5: Run the green checks**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --reporter compact
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app exec tsc -p test/tsconfig.e2e.json --noEmit
pnpm --filter openhuman-app exec prettier --check knip.json package.json
git diff --check
```

Expected: `test/wdio.conf.ts` and all verified framework/tooling entries are no
longer unused; `webdriverio` and `@wdio/globals` are no longer unlisted; no
broad ignore has hidden app source.

**Step 6: Commit**

Use:

```bash
atomic-commit "chore(app): make knip entry analysis accurate" -- app/knip.json app/package.json pnpm-lock.yaml app/pnpm-lock.yaml
```

Omit any listed path that was not actually changed, while continuing to name
every changed path explicitly.

---

### Task 2: Remove independently verified unreachable files

**Candidate files (post-Task-1 Knip must still report each before deletion):**

- `app/src/agentworld/theme/AgentWorldThemeBridge.tsx`
- `app/src/assets/icons/GoogleIcon.tsx`
- `app/src/components/ConnectionBadge.tsx`
- `app/src/components/LottieAnimation.tsx`
- `app/src/components/accounts/RespondQueuePanel.tsx`
- `app/src/components/chat/CycleUsagePill.tsx`
- `app/src/components/chat/TokenUsagePill.tsx`
- `app/src/components/intelligence/SyncBudgetDialog.tsx`
- `app/src/components/intelligence/SyncConfirmDialog.tsx`
- `app/src/components/routines/RoutineCard.tsx`
- `app/src/components/routines/RoutineRunHistory.tsx`
- `app/src/components/settings/components/PageBackButton.tsx`
- `app/src/components/settings/components/SettingsHeader.tsx`
- `app/src/components/settings/panels/autocomplete/AppFilterSection.tsx`
- `app/src/components/settings/panels/autocomplete/CompletionStyleSection.tsx`
- `app/src/components/settings/panels/billing/BillingHistoryTab.tsx`
- `app/src/components/settings/panels/billing/BillingPaymentsTab.tsx`
- `app/src/components/settings/panels/billing/BillingPlansTab.tsx`
- `app/src/components/settings/panels/billing/PayAsYouGoCard.tsx`
- `app/src/components/skills/SkillResourceTree.tsx`
- `app/src/features/conversations/components/LimitPill.tsx`
- `app/src/features/conversations/index.ts`
- `app/src/hooks/useIntelligenceApiFallback.ts`
- `app/src/hooks/useIntelligenceStats.ts`
- `app/src/hooks/useScreenIntelligenceItems.ts`
- `app/src/lib/ai/skillsAgentContext.ts`
- `app/src/pages/Routines.tsx`
- `app/src/pages/onboarding/components/ConfigureLaterCallout.tsx`
- `app/src/pages/onboarding/pages/ApiKeysPage.tsx`
- `app/src/pages/onboarding/pages/ChatProviderPage.tsx`
- `app/src/pages/onboarding/pages/CustomMemoryPage.tsx`
- `app/src/pages/onboarding/steps/ReferralApplyStep.tsx`
- `app/test/e2e/helpers/rpc-preflight.ts`

`app/test/wdio.conf.ts` is explicitly **not** a deletion candidate.

**Step 1: Red-check the file set**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include files --reporter compact
```

Expected: only files that survived Task 1 remain in the candidate set above.
Drop any file from this task if entry-point correction made it reachable.

**Step 2: Verify every file independently**

For every remaining file, search the full repository for its path, filename,
stem, exported component/hook names, route paths, lazy import strings,
`import.meta.glob`, index/barrel exports, package scripts, test configs, Tauri
resources, and documentation references.

Pay special attention to:

- `Routines.tsx`: current routes redirect legacy `/routines`; do not remove
  translations or redirects merely because the old page is dead.
- onboarding pages: commented-out JSX is not a live entry; do not remove shared
  onboarding types or current wizard routes.
- `GoogleIcon.tsx`: a separate local `GoogleIcon` exists in
  `components/oauth/providerConfigs.tsx`; keep the live local implementation.
- `useScreenIntelligenceItems.ts`: inspect
  `hooks/__tests__/useScreenIntelligenceItems.test.ts`; delete that test only if
  it is an orphan that duplicates dead behavior rather than importing a live
  unit.
- `rpc-preflight.ts`: comments claiming it can be called are not execution;
  verify neither WDIO config nor a spec imports it.
- files in settings, routines, intelligence, and billing: search route
  registries and lazy imports in addition to static imports.

**Step 3: Delete only confirmed-dead files**

Delete the files that remain unreachable. Remove now-empty barrel exports or
now-empty directories only when they contain no other tracked assets. Do not
delete localization strings, styles, or tests merely because their names are
similar.

**Step 4: Green-check the slice**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include files --reporter compact
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
git diff --check
```

Expected: no verified file candidate remains; unit tests, types, and lint pass.

**Step 5: Commit**

Commit with every deleted file and any orphan test/barrel update named
explicitly:

```bash
atomic-commit "refactor(app): remove unreachable frontend files" -- <explicit deleted and modified paths>
```

Do not use the angle-bracket text literally; replace it with the full explicit
path list.

---

### Task 3: Remove unused package dependencies

**Files:**

- Modify: `app/package.json`
- Modify: `pnpm-lock.yaml`
- Modify: `app/pnpm-lock.yaml`

**Step 1: Red-check dependency findings**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include dependencies,unlisted --reporter compact
rg -n "from ['\"](lottie-react|react-ga4)['\"]|import\\(['\"](lottie-react|react-ga4)['\"]\\)" \
  app/src app/test app/vite.config.ts app/scripts
```

Expected: after Task 2 deletes `LottieAnimation.tsx`, neither package has a
live import. A stale test comment mentioning `react-ga4` is not a dependency.

**Step 2: Remove only confirmed-unused declarations**

Remove `lottie-react` and `react-ga4` from `app/package.json`. Update the root
workspace lock and the tracked `app/pnpm-lock.yaml` without upgrading unrelated
packages. Inspect both lockfile diffs and revert any resolver churn unrelated
to these two removals and the direct WDIO declarations from Task 1.

**Step 3: Green-check dependencies**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include dependencies,unlisted --reporter compact
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app exec prettier --check package.json
git diff --check
```

Expected: no actionable unused or unlisted dependency finding.

**Step 4: Commit**

```bash
atomic-commit "chore(app): remove unused frontend dependencies" -- app/package.json pnpm-lock.yaml app/pnpm-lock.yaml
```

---

### Task 4: Normalize duplicate exports and barrels

**Files:** The post-Task-3 duplicate report, initially including 41 findings
across:

- `app/src/components/chat/`
- `app/src/components/flows/`
- `app/src/components/intelligence/`
- `app/src/components/mcp-setup/`
- `app/src/components/meetings/`
- `app/src/components/skills/`
- `app/src/components/userErrors/`
- `app/src/features/conversations/`
- `app/src/features/human/`
- `app/src/features/meet/`
- `app/src/features/share/`
- `app/src/hooks/`
- `app/src/lib/attachments.ts`
- `app/src/lib/mcp/rateLimiter.ts`
- `app/src/services/api/flowsApi.ts`

**Step 1: Red-check duplicates**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include duplicates --reporter compact
```

**Step 2: Choose one canonical export per symbol**

For each duplicate, search every import and barrel re-export. Retain the import
shape already used by live consumers; remove the redundant named/default
export only. Update imports only when necessary to converge on that canonical
shape. Do not rename components or refactor their implementation.

For `attachments.ts`, verify whether
`ATTACHMENT_MAX_IMAGE_SIZE_BYTES` and `ATTACHMENT_MAX_SIZE_BYTES` are aliases
before removing either export. For `rateLimiter.ts`, distinguish an actual
duplicate re-export from two different implementations. For `flowsApi.ts`,
retain the import shape used by the majority of live callers.

Process and commit this task in at most three subsystem commits if the explicit
path list becomes too large:

1. chat/flows/intelligence;
2. meetings/skills/human/share/hooks;
3. lib/services/barrels.

Each commit must independently pass:

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include duplicates --reporter compact
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
git diff --check
```

Commit each validated slice with explicit paths:

```bash
atomic-commit "refactor(app): remove duplicate <subsystem> exports" -- <explicit paths>
```

Expected at task completion: zero unexplained duplicate-export findings.

---

### Task 5: Internalize Agent World and mascot exports

**Files:** Post-Task-4 unused values/types in:

- `app/src/agentworld/**`
- `app/src/features/human/Mascot/**`
- `app/src/features/meet/MascotFrameProducer.tsx`
- `app/src/features/meet/useMeetingMascots.ts`

**Step 1: Red-check the subsystem**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include exports,types --reporter compact \
  | rg 'src/(agentworld|features/human/Mascot|features/meet/)'
```

**Step 2: Verify public-looking barrels**

Inspect `agentworld/iso/index.ts` and `features/human/Mascot/index.ts` against
all imports. A barrel item unused inside this private app workspace is not
automatically a public package API. Remove dead barrel re-exports, but keep the
underlying declaration when imported directly or used in its defining module.
For component props, manifest types, renderer types, and hook result types used
only locally, remove `export` rather than deleting the type.

Do not modify runtime rendering, sprite/room registration, Rive asset lookup,
manifest discovery, or Tauri resource paths.

**Step 3: Green-check and commit**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --include exports,types --reporter compact \
  | rg 'src/(agentworld|features/human/Mascot|features/meet/)' && exit 1 || true
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
git diff --check
atomic-commit "refactor(app): internalize agent world and mascot APIs" -- <explicit paths>
```

The `rg` pipeline must be interpreted carefully: only the absence of actionable
findings for this subsystem is green; a Knip process failure must not be hidden.

---

### Task 6: Internalize conversations, flows, orchestration, and hook exports

**Files:** Post-Task-5 unused values/types in:

- `app/src/components/flows/**`
- `app/src/features/conversations/**`
- `app/src/hooks/useFlow*.ts`
- `app/src/hooks/useRunsPendingApprovalSet.ts`
- `app/src/lib/flows/**`
- `app/src/lib/orchestration/**`
- `app/src/pages/FlowCanvasPage.tsx`
- `app/src/services/api/flowsApi.ts`
- `app/src/services/api/workflowRunsApi.ts`

**Step 1: Red-check and classify**

Run Knip for `exports,types`, filter to these paths, then classify each finding:
delete an unused declaration, internalize a locally used declaration, or retain
an intentional test seam. Search tests before touching constants such as
`VALIDATION_DEBOUNCE_MS`, model hints, timeline kinds, flow API helpers, and
copilot seed types.

**Step 2: Apply the smallest visibility/dead-code changes**

Preserve runtime hook registration, event names, API payload shapes, and flow
node-kind registries. Do not remove a flow API method solely because the React
app does not call it until repository-wide and documented-extension searches
also show it is private and unused.

**Step 3: Validate affected tests**

Run the specific test files that import any changed module, followed by:

```bash
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
pnpm --filter openhuman-app exec knip --config knip.json --include exports,types --reporter compact
git diff --check
```

**Step 4: Commit**

Split into a flows commit and a conversations/orchestration commit if needed,
with all paths explicit:

```bash
atomic-commit "refactor(app): trim unused flow module exports" -- <explicit paths>
atomic-commit "refactor(app): trim unused conversation exports" -- <explicit paths>
```

Run and commit each command only after its own validation.

---

### Task 7: Internalize settings, layout, UI, meetings, and feature exports

**Files:** Post-Task-6 unused values/types in:

- `app/src/components/layout/**`
- `app/src/components/ui/**`
- `app/src/components/settings/**`
- `app/src/components/meetings/**`
- `app/src/components/channels/**`
- `app/src/components/oauth/**`
- `app/src/components/skills/**`
- remaining `app/src/components/**`
- remaining `app/src/features/**`

**Step 1: Red-check and inspect barrel contracts**

Run the `exports,types` report filtered to these paths. Search all imports from
`components/ui`, `components/settings/controls`, settings route registries,
meeting components, and skill components before editing their index files.

**Step 2: Remove only unnecessary external visibility**

Typical safe changes are:

- `export interface FooProps` to `interface FooProps` when only the component
  in that file uses it;
- `export const` to `const` when local code/tests do not import it;
- remove a barrel re-export when every consumer imports directly;
- delete a value/type only when no local use remains.

Preserve settings route registry data, Tauri command names, analytics IDs,
documented extension points, and intentional test seams.

**Step 3: Green-check and commit in bounded slices**

Use two commits if necessary:

1. settings/layout/UI;
2. meetings/channels/skills/remaining features.

For each slice:

```bash
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
pnpm --filter openhuman-app exec knip --config knip.json --include exports,types --reporter compact
git diff --check
atomic-commit "refactor(app): trim unused <subsystem> exports" -- <explicit paths>
```

---

### Task 8: Internalize services, stores, libraries, and shared types

**Files:** Post-Task-7 unused values/types in:

- `app/src/services/**`
- `app/src/store/**`
- `app/src/lib/**`
- `app/src/types/**`
- `app/src/utils/**`
- remaining `app/src/hooks/**`
- `app/src/polyfills.ts`

**Step 1: Red-check and separate API shape from TypeScript visibility**

Run Knip for `exports,types`. For RPC/API modules, distinguish an exported
TypeScript declaration from a serialized field or method string: removing
`export` must not change the runtime object, payload, enum value, or command
name. Search the Rust/Tauri boundary and docs whenever a string might be an IPC
contract.

**Step 2: Process high-risk modules conservatively**

Explicitly verify:

- `services/coreRpcClient.ts`, `services/rpcMethods.ts`, and all `services/api/*`
  against frontend imports, tests, Tauri command strings, and documented RPC
  contracts;
- Redux slice actions/selectors against `store/index.ts`, middleware,
  persistence, tests, and dynamic dispatch;
- `lib/agentworld/invokeApiClient.ts` types against all Agent World consumers;
- `lib/mcp/*` tool classification and rate-limit symbols against tests and
  string-driven tool handling;
- `polyfills.ts` is imported for side effects by `src/main.tsx`; remove only
  unused exports, never the file or its side effects;
- platform, notification, webview, tunnel, attachment, and theme constants
  against runtime branches and tests.

**Step 3: Apply and validate in three atomic slices**

1. services/API clients;
2. stores/selectors;
3. libraries/types/utils/hooks.

For every slice:

```bash
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
pnpm --filter openhuman-app exec knip --config knip.json --include exports,types --reporter compact
git diff --check
atomic-commit "refactor(app): trim unused <subsystem> exports" -- <explicit paths>
```

Do not batch the three slices into one commit.

---

### Task 9: Clean test-helper exports and unused enum members

**Files:** Post-Task-8 findings in:

- `app/test/e2e/helpers/**`
- `app/test/e2e/mock-server.ts`
- `app/test/playwright/helpers/**`
- `app/src/lib/mcp/errorHandler.ts`

**Step 1: Red-check**

```bash
pnpm --filter openhuman-app exec knip --config knip.json \
  --include exports,types,enumMembers --reporter compact
```

**Step 2: Verify test discovery and helper use**

Search all WDIO and Playwright specs before internalizing or deleting a helper.
Test specs and configs are entry points; helper files are not. Preserve helpers
called through the WDIO lifecycle hooks or fixture registration even when no
spec imports them directly.

For `ErrorCategory`, search string values and computed property access in
addition to enum member names. Remove `CONTACT`, `GROUP`, `MEDIA`, `PROFILE`,
`AUTH`, `ADMIN`, `SEARCH`, and `DRAFT` only if none is used dynamically and
removing them does not alter serialized behavior.

**Step 3: Green-check**

```bash
pnpm --filter openhuman-app exec tsc -p test/tsconfig.e2e.json --noEmit
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
pnpm --filter openhuman-app exec knip --config knip.json \
  --include exports,types,enumMembers --reporter compact
git diff --check
```

**Step 4: Commit**

```bash
atomic-commit "refactor(app): trim unused test helper exports" -- <explicit paths>
```

---

### Task 10: Resolve only genuinely intentional residual findings

**Files:**

- Modify only if needed: `app/knip.json`
- Modify only if needed: files containing deliberate public/test seams

**Step 1: Run the complete report**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --reporter compact
pnpm --filter openhuman-app exec knip --config knip.json --production --reporter compact
```

**Step 2: Adversarially verify every residual**

For every residual finding, repeat the full static/dynamic search. A finding may
remain only when there is concrete evidence of convention-based loading,
package/Tauri/tooling invocation, generated input, documented public contract,
or intentional test seam that Knip cannot model.

Prefer modeling another precise entry or dependency over ignoring a finding.
If an exception is unavoidable, scope it to the exact file/package/symbol
supported by Knip's schema and add a one-line reason naming the loader or
contract. Do not add directory-wide ignores, `src/**`, `test/**`, or global
export suppression.

**Step 3: Green-check and commit exceptions separately**

```bash
pnpm --filter openhuman-app exec knip --config knip.json --reporter compact
pnpm --filter openhuman-app exec knip --config knip.json --production --reporter compact
pnpm --filter openhuman-app compile
pnpm --filter openhuman-app test -- --run
pnpm --filter openhuman-app lint
git diff --check
atomic-commit "chore(app): document intentional knip exceptions" -- app/knip.json <explicit affected paths>
```

Skip the commit entirely if no exceptions are needed.

---

### Task 11: Final independent verification

**Files:** No planned edits.

**Step 1: Confirm the branch and clean worktree**

```bash
git status --short --branch
git log --oneline --decorate -15
```

Expected: branch `chore/knip-cleanup`; every completed slice has its own atomic
commit; no uncommitted implementation changes.

**Step 2: Re-run the complete quality gate from a fresh Node 24 shell**

```bash
source "$HOME/.nvm/nvm.sh"
nvm use 24
pnpm knip
pnpm knip:production
pnpm typecheck
pnpm test
pnpm lint
pnpm format:check
pnpm build
```

Expected: every command exits zero. If `format:check` finds pre-existing,
out-of-scope failures, prove they exist at the design commit before claiming an
exception; otherwise fix only formatting in files touched by this cleanup and
amend it through a new atomic formatting commit with explicit paths.

**Step 3: Verify no scope leakage**

```bash
git diff --name-only eb2bda897..HEAD
git diff --stat eb2bda897..HEAD
git diff --check eb2bda897..HEAD
git status --short
```

Expected: implementation changes are confined to `app/`; the approved design
and this plan are the only `docs/` changes; no Rust, vendor, generated asset,
other submodule, or umbrella gitlink changed.

**Step 4: Review behavior preservation**

Inspect the complete diff and confirm:

- no route, command, event, RPC method, analytics ID, payload field, or visible
  copy changed;
- Vite, Vitest, WDIO, Playwright, Tauri, package-script, and shell-launched
  entry points remain tracked by Knip or have a precise documented exception;
- removed files have no imports, string references, generated-manifest role,
  tooling role, or documentation contract;
- dependency changes contain no unrelated upgrades;
- every remaining export is live or deliberately retained.

Do not push or open a PR. Report the commit list, deleted files/dependencies,
final Knip result, every validation command and exit status, and any narrowly
documented exception to the requesting agent.
