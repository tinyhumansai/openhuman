# React Reuse Foundation Implementation Plan

Date: 2026-07-24

Design: `docs/specs/2026-07-24-react-reuse-foundation-design.md`

Branch: `react-reuse-foundation`

## Scope and invariants

This plan implements the approved React reuse foundation in the design's delivery
order. It consolidates repeated presentation and client-side state mechanics while
preserving RPC contracts, authorization decisions, persistence formats, copy,
layout, and domain-specific state unions.

The following remain out of scope:

- HTTP JSON-RPC transport consolidation.
- A general form schema or universal status model.
- Drawer/full-screen-gate/consent-overlay unification.
- Socket event alias consolidation.
- Skeleton and live-stream atoms.
- Card polymorphism unless a migration below proves it is required. None currently
  does, so `app/src/components/ui/Card.tsx` is not changed by this plan.

Run every command below from the repository root.

For every task, follow the same discipline:

1. Add the focused test and run the stated targeted command to see it fail for the
   intended missing behavior.
2. Add the minimum shared implementation and migrate only the named consumers.
3. Run the targeted tests, then `pnpm typecheck`.
4. Review `git diff --check` and the task-scoped diff.
5. Commit only the explicitly listed files with the task's `atomic-commit` command
   before starting the next task.

## Task 1: Strengthen `ModalShell` and promote the global `ConfirmDialog`

**Files**

- Create: `app/src/components/ui/ModalShell.test.tsx`
- Create: `app/src/components/ui/ConfirmDialog.tsx`
- Create: `app/src/components/ui/ConfirmDialog.test.tsx`
- Modify: `app/src/components/ui/ModalShell.tsx`
- Modify: `app/src/components/ui/index.ts`
- Modify: `app/src/agentworld/components/ConfirmDialog.tsx`
- Modify: `app/src/agentworld/components/ConfirmDialog.test.tsx`

### Red

Add `ModalShell.test.tsx` coverage for:

- focus moves into the dialog on mount and returns to the previously focused
  element on unmount;
- Escape and backdrop close when `closePolicy` permits them;
- Escape, backdrop, and the close button do not close while closing is disabled;
- the close button is omitted when closing is disabled;
- `footer` renders in a dedicated border-top slot after the content;
- an explicit `aria-describedby` value is forwarded.

Add `ConfirmDialog.test.tsx` coverage for string and React-node bodies, neutral and
destructive confirmation tones, busy labels, disabled actions, Escape/backdrop
suppression while busy, and focus restoration.

Run:

```bash
pnpm test -- app/src/components/ui/ModalShell.test.tsx app/src/components/ui/ConfirmDialog.test.tsx
```

Expected failure: the new props and global component do not exist.

### Green

Change `ModalShell` to export these concrete contracts:

```ts
export interface ModalClosePolicy {
  escape?: boolean;
  backdrop?: boolean;
  button?: boolean;
}

export interface ModalShellProps {
  children: ReactNode;
  onClose: () => void;
  title: ReactNode;
  titleId: string;
  subtitle?: ReactNode;
  icon?: ReactNode;
  footer?: ReactNode;
  maxWidthClassName?: string;
  contentClassName?: string;
  panelClassName?: string;
  labelledBy?: string;
  describedBy?: string;
  closePolicy?: ModalClosePolicy;
}
```

Default every close-policy field to `true`. Treat an explicit `false` independently:
for example, a dialog may retain a close button while suppressing backdrop close.
Keep portal rendering and focus restoration. Render `footer`, when present, as:

```tsx
<div className="border-t border-line-subtle px-5 py-4">{footer}</div>
```

Create the global confirmation contract:

```ts
export interface ConfirmDialogProps {
  title: ReactNode;
  body: ReactNode;
  titleId?: string;
  confirmLabel?: ReactNode;
  cancelLabel?: ReactNode;
  busy?: boolean;
  busyLabel?: ReactNode;
  confirmDisabled?: boolean;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}
```

`ConfirmDialog` must compose `ModalShell`, place actions in its `footer`, use
`Button` with `tone="danger"` only when `destructive` is true, and set all three
close-policy fields to `false` while busy. `confirmDisabled` disables only the
confirm action and does not change dismissal policy. Default `titleId` to
`"confirm-dialog-title"`, `destructive` to `false`, and labels to
`"Confirm"`, `"Cancel"`, and `"Working…"`.

Export both components and their public types from `app/src/components/ui/index.ts`.
Replace the Agent World implementation with a compatibility wrapper that maps its
existing `message` prop to global `body`, retains its destructive-by-default
behavior, and delegates all interaction to the global component. Keep the existing
default export so current imports do not break yet.

### Verify

```bash
pnpm test -- app/src/components/ui/ModalShell.test.tsx app/src/components/ui/ConfirmDialog.test.tsx app/src/agentworld/components/ConfirmDialog.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(ui): establish reusable dialog primitives" -- app/src/components/ui/ModalShell.tsx app/src/components/ui/ModalShell.test.tsx app/src/components/ui/ConfirmDialog.tsx app/src/components/ui/ConfirmDialog.test.tsx app/src/components/ui/index.ts app/src/agentworld/components/ConfirmDialog.tsx app/src/agentworld/components/ConfirmDialog.test.tsx
```

## Task 2: Migrate ordinary confirmations to the shared dialog boundary

**Files**

- Modify: `app/src/components/intelligence/SyncConfirmDialog.tsx`
- Create: `app/src/components/intelligence/SyncConfirmDialog.test.tsx`
- Modify: `app/src/components/settings/panels/TeamMembersPanel.tsx`
- Modify: `app/src/components/settings/panels/__tests__/TeamMembersPanel.test.tsx`

### Red

Add a focused `SyncConfirmDialog` test that asserts dialog naming, loading/error/
ready body states, disabled confirmation before an estimate is available, backdrop
and Escape cancellation, and focus restoration. Extend the Team Members tests to
assert that remove-member and role-change confirmations are accessible dialogs,
close by Escape when idle, and cannot close or double-submit while their action is
busy.

Run:

```bash
pnpm test -- app/src/components/intelligence/SyncConfirmDialog.test.tsx app/src/components/settings/panels/__tests__/TeamMembersPanel.test.tsx
```

Expected failure: the hand-built overlays lack the shared close/focus behavior.

### Green

Refactor `SyncConfirmDialog` to render `ConfirmDialog` with:

- `title={t('syncConfirm.title')}`;
- a React-node `body` containing estimating, error, estimate, and budget-note
  states;
- `confirmLabel={t('syncConfirm.proceed')}`;
- `cancelLabel={t('syncConfirm.cancel')}`;
- `confirmDisabled={!estimate}`;
- `destructive={false}`.

Keep the existing RPC and cancellation guard unchanged. Disable confirmation when
there is no estimate, while preserving the current ability to cancel during
estimation or after an estimate error.

Replace both Team Members overlays with global `ConfirmDialog` instances. Preserve
all localized copy and the existing `confirmRemoveMember`/
`confirmChangeRole` functions. The remove action is destructive; role change is
neutral. Render the existing error with `ErrorBanner` inside `body`. Pass each
in-flight ID comparison as `busy` so shared close policy prevents dismissal during
the mutation.

### Verify

```bash
pnpm test -- app/src/components/intelligence/SyncConfirmDialog.test.tsx app/src/components/settings/panels/__tests__/TeamMembersPanel.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(ui): reuse confirmation dialogs" -- app/src/components/intelligence/SyncConfirmDialog.tsx app/src/components/intelligence/SyncConfirmDialog.test.tsx app/src/components/settings/panels/TeamMembersPanel.tsx app/src/components/settings/panels/__tests__/TeamMembersPanel.test.tsx
```

## Task 3: Strengthen and adopt existing loading, error, input, and chip primitives

**Files**

- Create: `app/src/components/ui/LoadingState.test.tsx`
- Modify: `app/src/components/ui/LoadingState.tsx`
- Create: `app/src/components/ui/Input.test.tsx`
- Modify: `app/src/components/ui/Input.tsx`
- Modify: `app/src/components/layout/ChipTabs.tsx`
- Modify: `app/src/components/layout/ChipTabs.test.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.test.tsx`
- Modify: `app/src/agentworld/pages/IdentitiesSection.tsx`
- Modify: `app/src/agentworld/pages/IdentitiesSection.test.tsx`
- Modify: `app/src/components/channels/mcp/McpCatalogBrowser.tsx`
- Modify: `app/src/components/channels/mcp/McpCatalogBrowser.test.tsx`

### Red

Test these exact contracts:

```ts
export interface ErrorBannerProps {
  children?: ReactNode;
  message?: ReactNode;
  action?: ReactNode;
  size?: "sm" | "md";
  className?: string;
}

export interface InputProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "size"
> {
  inputSize?: "sm" | "md" | "lg";
  invalid?: boolean;
  monospace?: boolean;
}

export interface ChipTabItem<T extends string> {
  id: T;
  label: ReactNode;
  testId?: string;
  controls?: string;
  labelledBy?: string;
}

export interface ChipTabsProps<T extends string> {
  // existing props remain
  compact?: boolean;
}
```

`ErrorBanner` must have `role="alert"`, accept React content, render an optional
action, and retain the existing `message` call sites. `Input` must set
`aria-invalid` when invalid and add `font-mono` only for `monospace`.
`ChipTabs` must connect tab and panel IDs with `aria-controls`/`aria-labelledby`,
use roving `tabIndex` (`0` selected, `-1` unselected), and reduce chip padding when
`compact`.

Add consumer assertions showing the Flow Runs drawer uses shared loading/error
states, Identities uses the shared error banner, and MCP catalog search is the
shared `Input`.

Run:

```bash
pnpm test -- app/src/components/ui/LoadingState.test.tsx app/src/components/ui/Input.test.tsx app/src/components/layout/ChipTabs.test.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/agentworld/pages/IdentitiesSection.test.tsx app/src/components/channels/mcp/McpCatalogBrowser.test.tsx
```

Expected failure: accessibility/variant props and consumer adoption are absent.

### Green

Implement only the tested semantic variants. In `FlowRunsDrawer`, replace the
hand-built spinner and error box with `CenteredLoadingState` and `ErrorBanner`
while retaining the existing test IDs on wrappers. In `IdentitiesSection`, delete
its local `ErrorBanner` and import the shared primitive. In `McpCatalogBrowser`,
replace the plain search input with `Input`, retaining all value, placeholder,
event, and test attributes.

Do not convert semantically distinct warning, validation, or fatal-gate messages.
Do not change `Card.tsx`.

### Verify

```bash
pnpm test -- app/src/components/ui/LoadingState.test.tsx app/src/components/ui/Input.test.tsx app/src/components/layout/ChipTabs.test.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/agentworld/pages/IdentitiesSection.test.tsx app/src/components/channels/mcp/McpCatalogBrowser.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(ui): strengthen and adopt existing primitives" -- app/src/components/ui/LoadingState.tsx app/src/components/ui/LoadingState.test.tsx app/src/components/ui/Input.tsx app/src/components/ui/Input.test.tsx app/src/components/layout/ChipTabs.tsx app/src/components/layout/ChipTabs.test.tsx app/src/components/flows/FlowRunsDrawer.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/agentworld/pages/IdentitiesSection.tsx app/src/agentworld/pages/IdentitiesSection.test.tsx app/src/components/channels/mcp/McpCatalogBrowser.tsx app/src/components/channels/mcp/McpCatalogBrowser.test.tsx
```

## Task 4: Extract the Agent World `StatusBlock`

**Files**

- Create: `app/src/agentworld/components/StatusBlock.tsx`
- Create: `app/src/agentworld/components/StatusBlock.test.tsx`
- Modify: `app/src/agentworld/pages/FeedSection.tsx`
- Modify: `app/src/agentworld/pages/DirectorySection.tsx`
- Modify: `app/src/agentworld/pages/LedgerSection.tsx`
- Modify: `app/src/agentworld/pages/JobsSection.tsx`
- Modify: `app/src/agentworld/pages/BountiesSection.tsx`
- Modify: `app/src/agentworld/pages/ProfilesSection.tsx`
- Modify: `app/src/agentworld/pages/ProfileViewer.tsx`
- Modify: `app/src/agentworld/pages/ExploreSection/index.tsx`

### Red

Add component tests for:

```ts
export type StatusBlockTone =
  | "neutral"
  | "info"
  | "success"
  | "warning"
  | "danger";

export interface StatusBlockProps {
  tone?: StatusBlockTone;
  title: ReactNode;
  body?: ReactNode;
  loading?: boolean;
  action?: ReactNode;
  className?: string;
}
```

Assert semantic tone classes, optional body/action omission, loading spinner
accessibility, and a stable `data-testid="agentworld-status-block"`.

Run:

```bash
pnpm test -- app/src/agentworld/components/StatusBlock.test.tsx
```

Expected failure: the shared component does not exist.

### Green

Implement the component with a semantic internal tone map; callers must not pass
Tailwind color strings. Preserve the existing centered `h-64`, title, body width,
and text sizing. When `loading` is true, render the shared `Spinner` before the
title and expose `aria-busy="true"`.

Delete all eight local `StatusBlock` implementations and import the shared one.
Map existing colors to tones without changing copy:

- muted/ordinary empty states -> `neutral`;
- primary/ocean states -> `info`;
- amber/payment-required states -> `warning`;
- coral/red errors -> `danger`;
- sage successful states -> `success`.

Use `className` only for layout differences already present; do not recreate tone
classes at call sites.

### Verify

```bash
pnpm test -- app/src/agentworld/components/StatusBlock.test.tsx app/src/agentworld/pages/FeedSection.test.tsx app/src/agentworld/pages/DirectorySection.test.tsx app/src/agentworld/pages/LedgerSection.test.tsx app/src/agentworld/pages/JobsSection.test.tsx app/src/agentworld/pages/BountiesSection.test.tsx app/src/agentworld/pages/ProfilesSection.test.tsx app/src/agentworld/pages/ProfileViewer.test.tsx app/src/agentworld/pages/ExploreSection/ExploreSection.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(agentworld): share status block presentation" -- app/src/agentworld/components/StatusBlock.tsx app/src/agentworld/components/StatusBlock.test.tsx app/src/agentworld/pages/FeedSection.tsx app/src/agentworld/pages/DirectorySection.tsx app/src/agentworld/pages/LedgerSection.tsx app/src/agentworld/pages/JobsSection.tsx app/src/agentworld/pages/BountiesSection.tsx app/src/agentworld/pages/ProfilesSection.tsx app/src/agentworld/pages/ProfileViewer.tsx app/src/agentworld/pages/ExploreSection/index.tsx
```

## Task 5: Extract one Agent World wallet identity state contract

**Files**

- Create: `app/src/agentworld/hooks/useMyAgentId.ts`
- Create: `app/src/agentworld/hooks/useMyAgentId.test.ts`
- Modify: `app/src/agentworld/pages/DirectorySection.tsx`
- Modify: `app/src/agentworld/pages/JobsSection.tsx`
- Modify: `app/src/agentworld/pages/BountiesSection.tsx`
- Modify: `app/src/agentworld/pages/MessagingSection.tsx`
- Modify: `app/src/agentworld/pages/ProfileViewer.tsx`

### Red

Test this discriminated state:

```ts
export type MyAgentIdState =
  | { status: "loading" }
  | { status: "disconnected" }
  | { status: "ready"; agentId: string }
  | { status: "error"; error: Error };

export function useMyAgentId(): MyAgentIdState;
```

Cover a Solana account, no Solana account, rejected fetch, stale completion after
unmount, and ignoring non-Solana accounts. The error state must wrap non-`Error`
rejections in `new Error(String(value))`; it must not log the error content.

Run:

```bash
pnpm test -- app/src/agentworld/hooks/useMyAgentId.test.ts
```

Expected failure: the shared hook does not exist.

### Green

Implement one `fetchWalletStatus()` call per mounted hook instance and the explicit
state transitions above. Replace the five local `useMyAgentId(): string | null`
copies in Directory, Jobs, Bounties, Messaging, and Profile Viewer. Adapt their
existing null checks to:

```ts
const myAgent = useMyAgentId();
const myAgentId = myAgent.status === "ready" ? myAgent.agentId : null;
```

Leave Profiles, Feed, and `WalletAddressChip` on their existing richer resource
flows. Profiles chains wallet resolution into profile and directory fallbacks,
Feed distinguishes an unknown transport failure from a proven missing wallet, and
`WalletAddressChip` owns retry behavior. Forcing those behaviors through this
identity-only hook would change their state contracts. They can be reconsidered
later behind a richer wallet-resource abstraction.

### Verify

```bash
pnpm test -- app/src/agentworld/hooks/useMyAgentId.test.ts app/src/agentworld/pages/DirectorySection.test.tsx app/src/agentworld/pages/JobsSection.test.tsx app/src/agentworld/pages/BountiesSection.test.tsx app/src/agentworld/pages/MessagingSection.test.tsx app/src/agentworld/pages/ProfileViewer.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(agentworld): centralize wallet identity resolution" -- app/src/agentworld/hooks/useMyAgentId.ts app/src/agentworld/hooks/useMyAgentId.test.ts app/src/agentworld/pages/DirectorySection.tsx app/src/agentworld/pages/JobsSection.tsx app/src/agentworld/pages/BountiesSection.tsx app/src/agentworld/pages/MessagingSection.tsx app/src/agentworld/pages/ProfileViewer.tsx
```

## Task 6: Extract Agent World form layout primitives

**Files**

- Create: `app/src/agentworld/components/FormField.tsx`
- Create: `app/src/agentworld/components/FormField.test.tsx`
- Create: `app/src/agentworld/components/FormActions.tsx`
- Create: `app/src/agentworld/components/FormActions.test.tsx`
- Modify: `app/src/agentworld/pages/JobsSection.tsx`
- Modify: `app/src/agentworld/pages/JobsSection.test.tsx`
- Modify: `app/src/agentworld/pages/BountiesSection.tsx`
- Modify: `app/src/agentworld/pages/BountiesSection.test.tsx`

### Red

Test these contracts:

```ts
export interface FormFieldProps {
  id: string;
  label: ReactNode;
  children: ReactElement;
  description?: ReactNode;
  error?: ReactNode;
  required?: boolean;
  className?: string;
}

export interface FormActionsProps {
  children: ReactNode;
  align?: "start" | "end" | "stretch";
  className?: string;
}
```

`FormField` must clone its single form control child to set `id`,
`aria-describedby`, `aria-invalid`, and `required` without overwriting explicit
child values. Description and error IDs must be `${id}-description` and
`${id}-error`; error content has `role="alert"`. `FormActions` must map alignments
to stable flex layouts, with `end` as the default.

Run:

```bash
pnpm test -- app/src/agentworld/components/FormField.test.tsx app/src/agentworld/components/FormActions.test.tsx
```

Expected failure: the primitives do not exist.

### Green

Migrate the Post Job, Apply, Open Dispute, Create Bounty, Submit Work, Add Comment,
and bounty dispute forms in the named page files. Use globally unique IDs prefixed
by the form (`post-job-title`, `create-bounty-title`, `submit-work-url`). Replace
plain text inputs with shared `Input` where their behavior matches it. Keep
textareas/selects native and pass them as `FormField` children. Replace repeated
right-aligned action wrappers with `FormActions`.

Do not move form state, validation, RPC calls, or submit handlers into the
primitives.

### Verify

```bash
pnpm test -- app/src/agentworld/components/FormField.test.tsx app/src/agentworld/components/FormActions.test.tsx app/src/agentworld/pages/JobsSection.test.tsx app/src/agentworld/pages/BountiesSection.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(agentworld): share form field layout" -- app/src/agentworld/components/FormField.tsx app/src/agentworld/components/FormField.test.tsx app/src/agentworld/components/FormActions.tsx app/src/agentworld/components/FormActions.test.tsx app/src/agentworld/pages/JobsSection.tsx app/src/agentworld/pages/JobsSection.test.tsx app/src/agentworld/pages/BountiesSection.tsx app/src/agentworld/pages/BountiesSection.test.tsx
```

## Task 7: Extract accessible expandable resource-row mechanics

**Files**

- Create: `app/src/agentworld/components/ExpandableResourceRow.tsx`
- Create: `app/src/agentworld/components/ExpandableResourceRow.test.tsx`
- Modify: `app/src/agentworld/pages/JobsSection.tsx`
- Modify: `app/src/agentworld/pages/JobsSection.test.tsx`
- Modify: `app/src/agentworld/pages/BountiesSection.tsx`
- Modify: `app/src/agentworld/pages/BountiesSection.test.tsx`

### Red

Test this composition API:

```ts
export interface ExpandableResourceRowProps {
  id: string;
  expanded: boolean;
  onToggle: () => void;
  summary: ReactNode;
  children: ReactNode;
  className?: string;
  expandedClassName?: string;
  summaryClassName?: string;
  detailClassName?: string;
}
```

The summary must be a real button with `aria-expanded` and
`aria-controls="${id}-details"`. The details wrapper must use that ID,
`role="region"`, and `aria-labelledby="${id}-toggle"`, and be absent while
collapsed. The shared component owns the rotating chevron and disclosure
mechanics; it does not own job/bounty content or side effects.

Run:

```bash
pnpm test -- app/src/agentworld/components/ExpandableResourceRow.test.tsx
```

Expected failure: the shared disclosure component does not exist.

### Green

Wrap `JobRow` and `BountyRow` with the shared component. Pass their current summary
markup through `summary` and current detail markup through `children`. Preserve
the job list's border-row shape and bounty grid card shape with the explicit class
props. Keep Bounty's detail-fetch effect in `BountyRow`, keyed by `expanded` and
`bountyId`; do not move it into the presentation primitive.

Add consumer assertions for unique control/detail IDs and keyboard activation.

### Verify

```bash
pnpm test -- app/src/agentworld/components/ExpandableResourceRow.test.tsx app/src/agentworld/pages/JobsSection.test.tsx app/src/agentworld/pages/BountiesSection.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(agentworld): share expandable resource rows" -- app/src/agentworld/components/ExpandableResourceRow.tsx app/src/agentworld/components/ExpandableResourceRow.test.tsx app/src/agentworld/pages/JobsSection.tsx app/src/agentworld/pages/JobsSection.test.tsx app/src/agentworld/pages/BountiesSection.tsx app/src/agentworld/pages/BountiesSection.test.tsx
```

## Task 8: Consolidate flow-run list query coordination

**Files**

- Create: `app/src/hooks/useFlowRunsQuery.ts`
- Create: `app/src/hooks/__tests__/useFlowRunsQuery.test.ts`
- Modify: `app/src/components/flows/FlowRunsSidebar.tsx`
- Modify: `app/src/components/flows/FlowRunsSidebar.test.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.test.tsx`
- Modify: `app/src/pages/WorkflowRunsPage.tsx`
- Modify: `app/src/pages/WorkflowRunsPage.test.tsx`

### Red

Test this API:

```ts
export type FlowRunsQueryScope =
  | { kind: "flow"; flowId: string | null }
  | { kind: "all" };

export interface UseFlowRunsQueryOptions {
  scope: FlowRunsQueryScope;
  enabled?: boolean;
}

export interface UseFlowRunsQueryResult {
  runs: FlowRun[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  refreshSilently: () => Promise<void>;
}

export function useFlowRunsQuery(
  options: UseFlowRunsQueryOptions,
): UseFlowRunsQueryResult;
```

Cover initial loading, disabled/null-flow reset, flow/all endpoint selection, flow
changes, error normalization, silent refresh preserving visible data/error state,
latest-request-wins for both initial and silent requests, and no state updates
after unmount.

Run:

```bash
pnpm test -- app/src/hooks/__tests__/useFlowRunsQuery.test.ts
```

Expected failure: the hook does not exist.

### Green

Implement the query hook with a request generation and mounted guard. `refresh`
sets `loading` and clears the foreground error. `refreshSilently` changes only
`runs` on success and drops/logs failures without exposing raw payloads. Choose
`listFlowRuns(flowId)` for `kind: 'flow'` and `listAllFlowRuns()` for `kind: 'all'`.
Key effects on `scope.kind` and `scope.kind === 'flow' ? scope.flowId : null`, not
the caller's object identity. Guard success, failure, and `finally` updates with the
same generation so a stale request cannot clear a newer request's loading state.

Migrate Sidebar, Drawer, and All Runs Page to the hook. Keep:

- Drawer selection reset on `flowId` change;
- `useFlowRunsLiveRefresh(runs, refreshSilently)`;
- `useFlowRunStarted(() => void refreshSilently(), flowId?)`;
- All Runs Page's separate `listFlows()` name lookup and its own error handling.

For All Runs Page, start the run query and `listFlows()` together. Track
`flowNamesLoading`/`flowNamesError` locally and derive page state as
`loading || flowNamesLoading` and `error ?? flowNamesError`; a silent run refresh
must not reload names.

Delete each consumer's duplicated generation refs and run-list load functions.
Do not move flow-name loading into `useFlowRunsQuery`.

### Verify

```bash
pnpm test -- app/src/hooks/__tests__/useFlowRunsQuery.test.ts app/src/components/flows/FlowRunsSidebar.test.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/pages/WorkflowRunsPage.test.tsx app/src/hooks/__tests__/useFlowRunsLiveRefresh.test.ts app/src/hooks/__tests__/useFlowRunStarted.test.ts
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(flows): centralize run list queries" -- app/src/hooks/useFlowRunsQuery.ts app/src/hooks/__tests__/useFlowRunsQuery.test.ts app/src/components/flows/FlowRunsSidebar.tsx app/src/components/flows/FlowRunsSidebar.test.tsx app/src/components/flows/FlowRunsDrawer.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/pages/WorkflowRunsPage.tsx app/src/pages/WorkflowRunsPage.test.tsx
```

## Task 9: Share one pending-approval polling source

**Files**

- Create: `app/src/hooks/flowPendingApprovalsStore.ts`
- Create: `app/src/hooks/__tests__/flowPendingApprovalsStore.test.ts`
- Modify: `app/src/hooks/useFlowPendingApprovals.ts`
- Modify: `app/src/hooks/__tests__/useFlowPendingApprovals.test.ts`
- Modify: `app/src/hooks/useRunsPendingApprovalSet.ts`
- Modify: `app/src/hooks/__tests__/useRunsPendingApprovalSet.test.ts`

### Red

Test a module-scoped external store with this public surface:

```ts
export interface FlowPendingApprovalsSnapshot {
  approvals: PendingApproval[];
  error: string | null;
  polling: boolean;
}

export function subscribeFlowPendingApprovals(listener: () => void): () => void;
export function getFlowPendingApprovalsSnapshot(): FlowPendingApprovalsSnapshot;
export function retainFlowPendingApprovalsPolling(): () => void;
export function refreshFlowPendingApprovals(): Promise<void>;

export function useFlowPendingApprovalsSource(
  enabled: boolean,
): FlowPendingApprovalsSnapshot;
```

Use `useSyncExternalStore` in the hook. Cover one immediate fetch for two concurrent
subscribers, one timer regardless of subscriber count, continued polling until the
last enabled consumer releases, immutable snapshots, retry after transient failure,
and timer/request cleanup. Provide a test-only reset export guarded by
`import.meta.env.MODE === 'test'` or an internal exported reset named
`resetFlowPendingApprovalsStoreForTests`.

Run:

```bash
pnpm test -- app/src/hooks/__tests__/flowPendingApprovalsStore.test.ts app/src/hooks/__tests__/useFlowPendingApprovals.test.ts app/src/hooks/__tests__/useRunsPendingApprovalSet.test.ts
```

Expected failure: the shared source is absent and current hooks poll independently.

### Green

Poll every 2 seconds while the retain count is positive. Keep the last successful
approval list on failure and expose a normalized error. Never log approval payloads
or user-authored error text.

Refactor `useFlowPendingApprovals(flowId, runId)` to select approvals matching both
IDs from the shared snapshot. Keep its `decidingId` and `decide` mutation locally;
after a successful decision, call `refreshFlowPendingApprovals()` so every consumer
reconciles.

Refactor `useRunsPendingApprovalSet(runs)` to retain polling only while any run is
`running` and derive the set from the shared snapshot. Keep
`resolveDisplayStatus` unchanged. Remove both private polling loops.

### Verify

```bash
pnpm test -- app/src/hooks/__tests__/flowPendingApprovalsStore.test.ts app/src/hooks/__tests__/useFlowPendingApprovals.test.ts app/src/hooks/__tests__/useRunsPendingApprovalSet.test.ts app/src/components/flows/FlowRunsSidebar.test.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/pages/WorkflowRunsPage.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(flows): share pending approval polling" -- app/src/hooks/flowPendingApprovalsStore.ts app/src/hooks/__tests__/flowPendingApprovalsStore.test.ts app/src/hooks/useFlowPendingApprovals.ts app/src/hooks/__tests__/useFlowPendingApprovals.test.ts app/src/hooks/useRunsPendingApprovalSet.ts app/src/hooks/__tests__/useRunsPendingApprovalSet.test.ts
```

## Task 10: Add a reusable latest-only async guard

**Files**

- Create: `app/src/hooks/useLatestAsync.ts`
- Create: `app/src/hooks/__tests__/useLatestAsync.test.ts`
- Modify: `app/src/agentworld/pages/ExploreSection/index.tsx`
- Modify: `app/src/agentworld/pages/ExploreSection/ExploreSection.test.tsx`

### Red

Test this mechanics-only API:

```ts
export interface LatestAsyncGuard {
  begin: () => number;
  isLatest: (generation: number) => boolean;
  invalidate: () => void;
}

export function useLatestAsync(): LatestAsyncGuard;
```

`begin()` returns a monotonically increasing generation, `isLatest()` is true only
for the newest generation while mounted, `invalidate()` makes all prior generations
stale, and unmount invalidates outstanding work. The returned methods must be
referentially stable.

Run:

```bash
pnpm test -- app/src/hooks/__tests__/useLatestAsync.test.ts
```

Expected failure: the hook does not exist.

### Green

Implement with refs and stable callbacks; it must not store results or dictate
loading/error state. Use it in the four Explore async resource hooks in
`ExploreSection/index.tsx`, replacing their repeated cancellation booleans with
`begin()`/`isLatest()` checks. Preserve PaymentRequiredError and wallet-error
classification exactly.

Add a consumer regression where request B resolves before request A and A cannot
overwrite B.

### Verify

```bash
pnpm test -- app/src/hooks/__tests__/useLatestAsync.test.ts app/src/agentworld/pages/ExploreSection/ExploreSection.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(hooks): add latest async guard" -- app/src/hooks/useLatestAsync.ts app/src/hooks/__tests__/useLatestAsync.test.ts app/src/agentworld/pages/ExploreSection/index.tsx app/src/agentworld/pages/ExploreSection/ExploreSection.test.tsx
```

## Task 11: Add reusable debounced values

**Files**

- Create: `app/src/hooks/useDebouncedValue.ts`
- Create: `app/src/hooks/__tests__/useDebouncedValue.test.ts`
- Modify: `app/src/components/channels/mcp/McpCatalogBrowser.tsx`
- Modify: `app/src/components/channels/mcp/McpCatalogBrowser.test.tsx`
- Modify: `app/src/components/channels/mcp/McpServersTab.tsx`
- Modify: `app/src/components/channels/mcp/McpServersTab.test.tsx`

### Red

Test:

```ts
export function useDebouncedValue<T>(value: T, delayMs: number): T;
```

Cover initial value, delayed update, replacement of a pending timer, delay changes,
referential values, and unmount cleanup.

Run:

```bash
pnpm test -- app/src/hooks/__tests__/useDebouncedValue.test.ts
```

Expected failure: the hook does not exist.

### Green

Implement trailing debounce with `useEffect` and `window.setTimeout`. Normalize a
negative delay to zero so a bad configuration cannot leave a value permanently
stale.

Replace the manual debounce refs in `McpCatalogBrowser` (250 ms) and
`McpServersTab` (300 ms). Effects should fetch from the debounced value, while
inputs remain controlled by the immediate value. Preserve initial fetch and
pagination reset behavior.

### Verify

```bash
pnpm test -- app/src/hooks/__tests__/useDebouncedValue.test.ts app/src/components/channels/mcp/McpCatalogBrowser.test.tsx app/src/components/channels/mcp/McpServersTab.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(hooks): share debounced values" -- app/src/hooks/useDebouncedValue.ts app/src/hooks/__tests__/useDebouncedValue.test.ts app/src/components/channels/mcp/McpCatalogBrowser.tsx app/src/components/channels/mcp/McpCatalogBrowser.test.tsx app/src/components/channels/mcp/McpServersTab.tsx app/src/components/channels/mcp/McpServersTab.test.tsx
```

## Task 12: Add cleanup-safe clipboard feedback

**Files**

- Create: `app/src/hooks/useClipboardFeedback.ts`
- Create: `app/src/hooks/__tests__/useClipboardFeedback.test.ts`
- Modify: `app/src/agentworld/pages/ProfileViewer.tsx`
- Modify: `app/src/agentworld/pages/ProfileViewer.test.tsx`
- Modify: `app/src/agentworld/components/WalletAddressChip.tsx`
- Modify: `app/src/agentworld/components/WalletAddressChip.test.tsx`
- Modify: `app/src/pages/Invites.tsx`
- Create: `app/src/pages/Invites.test.tsx`

### Red

Test:

```ts
export type ClipboardFeedbackStatus = "idle" | "copied" | "error";

export interface UseClipboardFeedbackOptions {
  resetAfterMs?: number;
  writeText?: (value: string) => Promise<void>;
}

export interface UseClipboardFeedbackResult {
  status: ClipboardFeedbackStatus;
  copy: (value: string) => Promise<boolean>;
  reset: () => void;
}

export function useClipboardFeedback(
  options?: UseClipboardFeedbackOptions,
): UseClipboardFeedbackResult;
```

Cover success, failure, boolean result, timer replacement on repeated copy, manual
reset, and no post-unmount updates. Default to `navigator.clipboard.writeText` and
2 seconds. Do not retain the copied value in React state or log it.

Run:

```bash
pnpm test -- app/src/hooks/__tests__/useClipboardFeedback.test.ts
```

Expected failure: the hook does not exist.

### Green

Migrate Profile Viewer share-link copying, Wallet Address Chip, and Invites.
Preserve their existing labels, analytics, and visible success durations by passing
the current timeout explicitly where it differs from 2 seconds. Preserve consumer
error copy by mapping `status === 'error'`; do not migrate recovery phrases,
credentials, or other sensitive clipboard flows.

### Verify

```bash
pnpm test -- app/src/hooks/__tests__/useClipboardFeedback.test.ts app/src/agentworld/pages/ProfileViewer.test.tsx app/src/agentworld/components/WalletAddressChip.test.tsx app/src/pages/Invites.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(hooks): share clipboard feedback" -- app/src/hooks/useClipboardFeedback.ts app/src/hooks/__tests__/useClipboardFeedback.test.ts app/src/agentworld/pages/ProfileViewer.tsx app/src/agentworld/pages/ProfileViewer.test.tsx app/src/agentworld/components/WalletAddressChip.tsx app/src/agentworld/components/WalletAddressChip.test.tsx app/src/pages/Invites.tsx app/src/pages/Invites.test.tsx
```

## Task 13: Compose Escape and outside-pointer dismissal

**Files**

- Create: `app/src/hooks/useDismissLayer.ts`
- Create: `app/src/hooks/__tests__/useDismissLayer.test.ts`
- Modify: `app/src/components/ui/ModalShell.tsx`
- Modify: `app/src/components/ui/ModalShell.test.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.test.tsx`

### Red

Test:

```ts
export interface UseDismissLayerOptions {
  onDismiss: () => void;
  enabled?: boolean;
  dismissOnEscape?: boolean;
  dismissOnOutsidePointer?: boolean;
}

export interface DismissLayerBindings {
  layerRef: RefObject<HTMLElement | null>;
  onPointerDownCapture: (event: React.PointerEvent) => void;
}

export function useDismissLayer(
  options: UseDismissLayerOptions,
): DismissLayerBindings;
```

The hook composes `useEscapeKey`, calls `onDismiss` only when the pointer event
target lies outside `layerRef.current`, observes each policy independently, uses
the latest callback without re-registering listeners, and performs no action after
unmount.

Run:

```bash
pnpm test -- app/src/hooks/__tests__/useDismissLayer.test.ts
```

Expected failure: the hook does not exist.

### Green

Use the hook in `ModalShell`, with the dialog panel as `layerRef` and backdrop
policy mapped from `closePolicy.backdrop`. Use it in `FlowRunsDrawer`, with its
`aside` as the layer and dismissal disabled while the inspector is open. Replace
the drawer's separate Escape hook and backdrop button handler, retaining the
backdrop as an accessible button and current test IDs.

Do not migrate drawers, gates, or overlays with materially different stacking or
consent behavior in this task.

### Verify

```bash
pnpm test -- app/src/hooks/__tests__/useDismissLayer.test.ts app/src/components/ui/ModalShell.test.tsx app/src/components/flows/FlowRunsDrawer.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(hooks): share dismiss layer mechanics" -- app/src/hooks/useDismissLayer.ts app/src/hooks/__tests__/useDismissLayer.test.ts app/src/components/ui/ModalShell.tsx app/src/components/ui/ModalShell.test.tsx app/src/components/flows/FlowRunsDrawer.tsx app/src/components/flows/FlowRunsDrawer.test.tsx
```

## Task 14: Extract approval decision presentation

**Files**

- Create: `app/src/components/approvals/ApprovalDecisionCard.tsx`
- Create: `app/src/components/approvals/ApprovalDecisionCard.test.tsx`
- Modify: `app/src/components/flows/FlowRunPendingApprovalCard.tsx`
- Create: `app/src/components/flows/FlowRunPendingApprovalCard.test.tsx`
- Modify: `app/src/components/chat/FlowApprovalRequestCard.tsx`
- Modify: `app/src/components/chat/__tests__/FlowApprovalRequestCard.test.tsx`

### Red

Test this presentation-only contract:

```ts
export interface ApprovalDecisionAction {
  id: string;
  label: ReactNode;
  busyLabel?: ReactNode;
  variant: "primary" | "secondary";
  tone?: "default" | "danger";
  title?: string;
}

export interface ApprovalDecisionCardProps {
  ariaLabel: string;
  summary: ReactNode;
  metadata?: ReactNode;
  actions: ApprovalDecisionAction[];
  busyActionId?: string | null;
  onAction: (actionId: string) => void;
  testId?: string;
  className?: string;
}
```

Assert alert-dialog semantics, summary/metadata composition, action order, busy
label only on the active action, all actions disabled while any action is busy,
action IDs passed to the callback, and danger tone forwarding.

Run:

```bash
pnpm test -- app/src/components/approvals/ApprovalDecisionCard.test.tsx
```

Expected failure: the shared card does not exist.

### Green

Move only amber card chrome and action rendering into the shared component.
`FlowRunPendingApprovalCard` retains local `ApprovalDecision` state and maps
`approve_once`, `approve_always_for_flow`, and `deny` descriptors.
`FlowApprovalRequestCard` retains its `decideApproval` call, resolution callback,
error state, and domain copy, mapping its existing actions to descriptors.

Do not merge tool/chat/flow authorization logic. Do not pass RPC callbacks into
the shared presentation component.

### Verify

```bash
pnpm test -- app/src/components/approvals/ApprovalDecisionCard.test.tsx app/src/components/flows/FlowRunPendingApprovalCard.test.tsx app/src/components/chat/__tests__/FlowApprovalRequestCard.test.tsx app/src/components/chat/__tests__/ApprovalRequestCard.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(approvals): share decision card presentation" -- app/src/components/approvals/ApprovalDecisionCard.tsx app/src/components/approvals/ApprovalDecisionCard.test.tsx app/src/components/flows/FlowRunPendingApprovalCard.tsx app/src/components/flows/FlowRunPendingApprovalCard.test.tsx app/src/components/chat/FlowApprovalRequestCard.tsx app/src/components/chat/__tests__/FlowApprovalRequestCard.test.tsx
```

## Task 15: Extract flow-run status presentation

**Files**

- Create: `app/src/components/flows/FlowRunStatus.tsx`
- Create: `app/src/components/flows/FlowRunStatus.test.tsx`
- Modify: `app/src/components/flows/FlowRunInspectorDrawer.tsx`
- Modify: `app/src/components/flows/__tests__/FlowRunInspectorDrawer.test.tsx`
- Modify: `app/src/components/flows/FlowRunsSidebar.tsx`
- Modify: `app/src/components/flows/FlowRunsSidebar.test.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.tsx`
- Modify: `app/src/components/flows/FlowRunsDrawer.test.tsx`
- Modify: `app/src/pages/WorkflowRunsPage.tsx`
- Modify: `app/src/pages/WorkflowRunsPage.test.tsx`

### Red

Test:

```ts
export type FlowRunStatusPresentation = "badge" | "dot";

export interface FlowRunStatusProps {
  status: FlowRunStatus;
  label: string;
  presentation?: FlowRunStatusPresentation;
  className?: string;
  testId?: string;
}
```

Pin every `FlowRunStatus` to the existing accent and dot classes. Badge output
must show the supplied localized label; dot output must be `aria-hidden`.

Run:

```bash
pnpm test -- app/src/components/flows/FlowRunStatus.test.tsx
```

Expected failure: status maps currently live in consumer/inspector files.

### Green

Move `FLOW_RUN_STATUS_ACCENT` and `FLOW_RUN_STATUS_DOT` into the new module and
keep named exports temporarily for callers that need classes during migration.
Keep `FLOW_RUN_STATUS_KEY` as flow-domain vocabulary, exported from the same module.
Use `FlowRunStatus` in Sidebar, Drawer, and All Runs Page for badges/dots. Update
Inspector imports. Delete `STATUS_CLASS` from All Runs Page.

Do not broaden the component to channel, job, bounty, or feedback status unions.

### Verify

```bash
pnpm test -- app/src/components/flows/FlowRunStatus.test.tsx app/src/components/flows/__tests__/FlowRunInspectorDrawer.test.tsx app/src/components/flows/FlowRunsSidebar.test.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/pages/WorkflowRunsPage.test.tsx
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(flows): share run status presentation" -- app/src/components/flows/FlowRunStatus.tsx app/src/components/flows/FlowRunStatus.test.tsx app/src/components/flows/FlowRunInspectorDrawer.tsx app/src/components/flows/__tests__/FlowRunInspectorDrawer.test.tsx app/src/components/flows/FlowRunsSidebar.tsx app/src/components/flows/FlowRunsSidebar.test.tsx app/src/components/flows/FlowRunsDrawer.tsx app/src/components/flows/FlowRunsDrawer.test.tsx app/src/pages/WorkflowRunsPage.tsx app/src/pages/WorkflowRunsPage.test.tsx
```

## Task 16: Make workflow-proposal parsing canonical

**Files**

- Modify: `app/src/lib/workflows/workflowProposal.ts`
- Modify: `app/src/lib/workflows/workflowProposal.test.ts`
- Modify: `app/src/services/api/flowsApi.ts`
- Modify: `app/src/services/api/__tests__/flowsApi.test.ts`
- Modify: `app/src/services/api/flowsApi.test.ts`

### Red

Add parity tests proving `buildWorkflow()` maps its raw proposal through
`coerceWorkflowProposal`, including malformed payloads, missing
`require_approval`, invalid summary steps, and explicit false approval. Add a
regression that a valid payload produces the same object through the string parser
and API mapper.

Run:

```bash
pnpm test -- app/src/lib/workflows/workflowProposal.test.ts app/src/services/api/__tests__/flowsApi.test.ts app/src/services/api/flowsApi.test.ts
```

Expected failure: `flowsApi.ts` still owns an independent mapper.

### Green

Import `coerceWorkflowProposal` into `flowsApi.ts`, delete the duplicated mapping
body, and either:

- replace `mapWorkflowProposal(result.proposal)` with
  `coerceWorkflowProposal(result.proposal)` and remove the exported mapper; or
- keep `mapWorkflowProposal` as a deprecated one-line compatibility export:

```ts
export const mapWorkflowProposal = coerceWorkflowProposal;
```

Choose the one-line alias only if current imports/tests require the public symbol.
There must be exactly one implementation of type/name/graph/summary/default-
approval validation after this task.

### Verify

```bash
pnpm test -- app/src/lib/workflows/workflowProposal.test.ts app/src/services/api/__tests__/flowsApi.test.ts app/src/services/api/flowsApi.test.ts
pnpm typecheck
git diff --check
```

### Commit

```bash
atomic-commit "refactor(flows): canonicalize workflow proposal parsing" -- app/src/lib/workflows/workflowProposal.ts app/src/lib/workflows/workflowProposal.test.ts app/src/services/api/flowsApi.ts app/src/services/api/__tests__/flowsApi.test.ts app/src/services/api/flowsApi.test.ts
```

## Final branch verification

After all 16 task commits, run targeted aggregate coverage first:

```bash
pnpm test -- app/src/components/ui/ModalShell.test.tsx app/src/components/ui/ConfirmDialog.test.tsx app/src/components/ui/LoadingState.test.tsx app/src/components/ui/Input.test.tsx app/src/components/layout/ChipTabs.test.tsx app/src/agentworld/components/StatusBlock.test.tsx app/src/agentworld/hooks/useMyAgentId.test.ts app/src/agentworld/components/FormField.test.tsx app/src/agentworld/components/FormActions.test.tsx app/src/agentworld/components/ExpandableResourceRow.test.tsx app/src/hooks/__tests__/useFlowRunsQuery.test.ts app/src/hooks/__tests__/flowPendingApprovalsStore.test.ts app/src/hooks/__tests__/useLatestAsync.test.ts app/src/hooks/__tests__/useDebouncedValue.test.ts app/src/hooks/__tests__/useClipboardFeedback.test.ts app/src/hooks/__tests__/useDismissLayer.test.ts app/src/components/approvals/ApprovalDecisionCard.test.tsx app/src/components/flows/FlowRunStatus.test.tsx app/src/lib/workflows/workflowProposal.test.ts
```

Then run every required frontend gate:

```bash
pnpm typecheck
pnpm lint
pnpm test
pnpm build
pnpm format:check
git diff --check
git status --short
```

This refactor does not change Rust domains, controller schemas, JSON-RPC methods,
or transport behavior, so Rust/JSON-RPC implementation and E2E additions are not
applicable. Existing consumer-flow coverage remains the E2E proof for the
migrated UI; the work here is verified with focused interaction tests plus the
complete frontend suite and production build.

The branch is ready for review only when:

- all eight former Agent World `StatusBlock` definitions are gone;
- the five former page-local `useMyAgentId` implementations use the shared hook;
- the richer Feed, Profiles, and WalletAddressChip wallet-resource flows remain
  behaviorally unchanged;
- Sidebar, Drawer, and All Runs Page use `useFlowRunsQuery`;
- pending approvals issue one poll for concurrent list/detail consumers;
- the duplicated mapper body is absent from `flowsApi.ts`;
- every gate above passes;
- the worktree contains no uncommitted production or test changes.

Do not push or open a PR unless separately requested.
