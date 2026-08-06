# React Reuse Foundation

Date: 2026-07-24
Status: Proposed

## Purpose

The React application has useful shared primitives, but features do not consistently
promote repeated behavior into them. This work establishes a small set of reusable
UI and state-management seams, then migrates proven duplicates without redesigning
the affected screens.

The governing rule is: when a second implementation repeats the same semantics,
either reuse or extract a shared owner, or document why the two implementations
must remain separate. Similar styling alone is not enough to justify an abstraction.

## Goals

- Remove exact and near-exact React implementations already repeated at least twice.
- Prefer adopting existing primitives before adding new ones.
- Give shared behavior a clear ownership level: global UI, settings, Agent World,
  flows, or a feature-local helper.
- Preserve current user-visible behavior while improving accessibility and
  consistency.
- Add regression coverage at the shared boundary before migrating consumers.
- Land every independently validated slice as an atomic commit.

## Non-goals

- A wholesale redesign or restyling of the application.
- A schema-driven form framework.
- One universal status model spanning unrelated domains.
- Combining drawers, blocking gates, sheets, and ordinary dialogs into one component.
- Rewriting every large component solely to reduce its line count.
- Changing RPC contracts, authorization semantics, or persistence formats.

## Design principles

### Promote to the narrowest stable owner

Global presentation primitives belong under `app/src/components/ui`. Domain
vocabulary and state mappings stay with the domain. For example, a global `Badge`
owns shape and tone, while `JobStatusBadge` owns the mapping from job statuses to
tones.

Agent World-only presentation belongs under `app/src/agentworld/components`, and
Agent World data hooks belong under `app/src/agentworld/hooks`. Flow-run query
coordination belongs under `app/src/hooks` because it is consumed by pages and
feature components.

### Preserve domain semantics

Extraction should consolidate mechanics, not erase meaning. Approval cards may
share layout and request lifecycle, but tool, flow, and gate authorization actions
remain distinct. Loading and error primitives expose variants rather than treating
all warnings, validation messages, and fatal errors as the same state.

### Prefer composition over configuration engines

Shared components accept ordinary React content and small semantic props. Feature
code continues to own validation, RPC calls, and domain-specific rendering.

## Planned slices

### 1. Adopt and strengthen existing UI primitives

Extend existing primitives only where current consumers require a stable capability:

- `ModalShell`: optional footer, close policy, and content/layout slots.
- `ErrorBanner`: alert semantics, React content, size, and optional action.
- `Input`: invalid and monospace presentation where raw inputs currently duplicate it.
- `ChipTabs`: per-item accessibility relationships and compact presentation.
- `Card`: polymorphic output only if required by semantic list or section consumers.

Migrate exact matches before flexible variants. Drawers, full-screen gates, and
consent overlays remain separate.

Promote the Agent World confirmation dialog to
`app/src/components/ui/ConfirmDialog.tsx`. It composes `ModalShell` and supports
custom body content, destructive or neutral confirmation, busy labels, and
close-disabled behavior.

### 2. Extract Agent World reuse seams

Add:

- `StatusBlock`: semantic tone, title, optional body, loading state, and optional
  action. It replaces the eight current copies.
- `useMyAgentId`: resolves the Solana wallet identity with explicit loading,
  disconnected, ready, and error states.
- Small form primitives: `FormField` and `FormActions`. They own labels,
  descriptions, error association, and action layout, but not form state.
- `ExpandableResourceRow`: disclosure mechanics and accessibility for job and
  bounty rows.

Skeleton atoms and a live-stream indicator are follow-ups only if the initial
migrations demonstrate that their geometry and semantics remain stable.

### 3. Consolidate shared hooks and query coordination

Add narrowly scoped hooks:

- `useFlowRunsQuery`: initial load, latest-request protection, silent refresh,
  and flow-run live refresh.
- A shared pending-approvals source with selector hooks so list and detail views do
  not poll the same endpoint independently.
- `useLatestAsync`: latest-request and unmount protection without dictating how a
  caller stores or merges returned data.
- `useDebouncedValue`: timer cleanup for search inputs.
- `useClipboardFeedback`: copy status, failure state, timer replacement, and
  unmount cleanup. Sensitive clipboard flows opt in separately.
- `useDismissLayer`: composition around the existing `useEscapeKey`, with explicit
  outside-pointer behavior.

Socket subscription consolidation follows only after canonical-versus-legacy event
alias behavior is decided and covered by tests.

### 4. Consolidate repeated domain presentation and parsing

- `ApprovalDecisionCard` shares approval presentation and action rendering while
  callers supply domain-specific action descriptors.
- Flow-run status presentation shares tone and dot primitives without merging
  distinct status unions.
- The workflow-proposal parser has one canonical implementation used by the API
  mapping layer.

HTTP JSON-RPC transport consolidation is intentionally a later, non-React slice. It
will be planned separately if the React-focused changes finish cleanly.

## Component contracts

Shared UI components use semantic props rather than accepting arbitrary Tailwind
color classes. They forward accessibility attributes and allow targeted
`className` overrides for layout, not replacement of their core visual contract.

Dialog and drawer primitives must provide:

- an accessible name;
- Escape and backdrop behavior controlled by explicit policy;
- focus placement and restoration;
- a real close control when closing is allowed;
- portal rendering where required;
- no action after unmount.

Async hooks expose discriminated state where the distinction affects rendering.
They must prevent stale requests from overwriting newer state and must not log raw
payloads or user-authored error content.

## Migration strategy

Each slice follows this order:

1. Add or extend the shared boundary with focused tests.
2. Migrate two representative consumers.
3. Run related tests and TypeScript validation.
4. Migrate remaining exact matches.
5. Run related tests again and commit the slice.

Consumers with materially different behavior remain untouched and are recorded as
intentional exceptions. No migration should combine a behavior change with the
reuse refactor.

## Testing

- Shared components receive rendering, interaction, keyboard, ARIA, busy-state,
  and focus-restoration tests as applicable.
- Hooks receive success, error, unmount, stale-response, timer-cleanup, and
  concurrent-consumer tests as applicable.
- Existing consumer tests remain green; targeted tests are added when a migrated
  behavior was previously uncovered.
- Each slice runs its related Vitest files plus `pnpm typecheck`.
- The completed branch runs `pnpm lint`, `pnpm test`, and `pnpm build`.
- Rust domains, controller schemas, JSON-RPC methods, and transport behavior are
  unchanged, so no Rust/JSON-RPC implementation or new E2E scenario is needed.
  Existing consumer-flow E2E coverage remains applicable; focused interaction
  tests and the full frontend suite verify the migrated UI behavior.

## Delivery order

1. Global `ConfirmDialog` and simple `ModalShell` migrations.
2. Existing loading/error primitive adoption.
3. Agent World `StatusBlock`.
4. Agent World `useMyAgentId`.
5. Agent World form and expandable-row primitives.
6. Flow-run query and pending-approval coordination.
7. Clipboard, debounce, latest-async, and dismiss-layer hooks.
8. Approval presentation, flow status presentation, and workflow parser.

Every item is independently reviewable and committed with `atomic-commit`.

## Success criteria

- The eight Agent World status blocks share one implementation.
- Repeated ordinary confirmations use the global dialog primitives.
- Agent World wallet identity resolution has one hook and one state contract.
- Flow-run surfaces share one race-safe query lifecycle.
- Concurrent flow-run surfaces share pending-approval polling.
- Existing exact loading, error, input, and chip duplicates use their established
  primitives.
- No migrated feature loses behavior, accessibility, or test coverage.
- Frontend lint, unit tests, type checking, and production build pass.
