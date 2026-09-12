import { type AssistantState, useAuiState } from '@assistant-ui/react';
import debugFactory from 'debug';

const debug = debugFactory('openhuman:assistant-ui:transcript');

/**
 * Optional-safe reads of the assistant-ui runtime for the CURRENT subtree.
 *
 * Two properties make these hooks usable from the transcript, and both are
 * load-bearing:
 *
 * 1. **They read the runtime from React context**, never from
 *    `state.thread.selectedThreadId`. `AssistantUiRuntimeProvider` is
 *    thread-parameterized and `ChatThreadView` is mounted by two hosts — the
 *    home chat (follows the selection) and `WorkflowCopilotPanel` (its own
 *    nested runtime on a dedicated builder thread). Reading the selection here
 *    would paint the home chat's state inside the copilot.
 *
 * 2. **They tolerate the runtime being absent.** Every selector goes through
 *    `s.optional.<scope>`, which resolves to `undefined` rather than throwing
 *    when no `AuiProvider` is above the component (assistant-ui's default
 *    client throws on a direct `s.thread` read). `ChatThreadView` is rendered
 *    without a runtime in several unit tests — including
 *    `ChatThreadView.renderPerf.test.tsx`, which must stay byte-identical — so
 *    a hook that threw outside a provider would make the transcript
 *    un-mountable there.
 *
 * Selectors are module-level constants: `useAuiState` keys its internal
 * memoization on selector identity, so an inline arrow would re-subscribe the
 * underlying `useSyncExternalStore` on every render of the chat's hot path.
 */

const selectIsRunning = (s: AssistantState) => s.optional.thread?.isRunning;

const selectCanEdit = (s: AssistantState) => s.optional.thread?.capabilities.edit;

const selectCanSwitchToBranch = (s: AssistantState) =>
  s.optional.thread?.capabilities.switchToBranch;

/**
 * Whether the runtime believes a turn is in flight.
 *
 * `undefined` means "no runtime mounted above this transcript" and is
 * deliberately distinct from `false` — the caller ORs a present `true` into its
 * own Redux-derived in-flight check rather than replacing it, so a surface with
 * no runtime keeps behaving exactly as it did.
 */
export function useAuiThreadRunning(): boolean | undefined {
  return useAuiState(selectIsRunning);
}

/**
 * The two capabilities the external-store adapter does NOT implement.
 *
 * `useOpenHumanExternalStore` supplies `onNew` / `onCancel` only;
 * it implements neither `onEdit` nor `setMessages`, which is what assistant-ui
 * requires for message editing and for the branch picker. The runtime reports
 * that faithfully, so this hook is the honest gate for those affordances rather
 * than a hard-coded `false` that would rot the day the adapter grows them.
 *
 * Both affordances in `components/assistant-ui/thread.tsx` are gated on this
 * (#5897): `UserActionBar` renders `ActionBarPrimitive.Edit` only when
 * `canEdit`, and `BranchPicker` returns `null` unless `canSwitchToBranch`.
 * Neither is reachable today, which is the point — an edit button that looks
 * supported and silently does nothing is worse than no button. See
 * `EDIT_AND_BRANCH_SEAM` below for where the edit composer itself attaches when
 * the adapter grows `onEdit` / `setMessages`.
 */
export function useAuiEditCapabilities(): { canEdit: boolean; canSwitchToBranch: boolean } {
  const canEdit = useAuiState(selectCanEdit) ?? false;
  const canSwitchToBranch = useAuiState(selectCanSwitchToBranch) ?? false;
  if (canEdit || canSwitchToBranch) {
    debug(
      '[assistant-ui] transcript capabilities changed edit=%s branch=%s',
      canEdit,
      canSwitchToBranch
    );
  }
  return { canEdit, canSwitchToBranch };
}

/**
 * THE EDIT / BRANCH SEAM.
 *
 * When the core gains a branch model and `useOpenHumanExternalStore` grows
 * `onEdit` + `setMessages`, two affordances become renderable and both belong
 * inside `TranscriptRow` (the memoized per-turn component), NOT here:
 *
 * - an edit composer, gated on `useAuiEditCapabilities().canEdit`, rendered
 *   from `ComposerPrimitive.Root` / `ComposerPrimitive.Input` inside a
 *   `MessagePrimitive.Root` for that turn;
 * - `BranchPickerPrimitive.Root` / `.Previous` / `.Number` / `.Count` /
 *   `.Next`, gated on `canSwitchToBranch`, rendered alongside the turn's
 *   existing copy / react / share action row.
 *
 * They are deliberately absent rather than rendered-and-inert: an edit button
 * that looks supported and silently does nothing is worse than no button.
 *
 * That rule was stated here but not enforced anywhere until #5897 — this hook
 * had zero production consumers while `ActionBarPrimitive.Edit` shipped
 * unconditionally, so the button was rendered, clickable and inert. The gate is
 * wired now; keep it wired when the affordances move into `TranscriptRow`.
 */
export const EDIT_AND_BRANCH_SEAM = Object.freeze({
  editComposer: 'TranscriptRow — gated on useAuiEditCapabilities().canEdit',
  branchPicker: 'TranscriptRow — gated on useAuiEditCapabilities().canSwitchToBranch',
});
