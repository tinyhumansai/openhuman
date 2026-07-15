/**
 * FlowCanvasPage (issue B5b / Phase 3) — the Workflow Canvas builder at
 * `/flows/:id`. Loads one saved flow via `flows_get`, converts its
 * `WorkflowGraph` (`Flow.graph`, opaque `unknown` on the wire type — see
 * `services/api/flowsApi.ts`) to xyflow's shape via `graphAdapter.ts`, and
 * renders it in the *editable* `FlowCanvas` (drag / connect / add / delete /
 * config, plus Phase 3c validation UX and Phase 3d draft/dirty state).
 *
 * This page owns the two host-level pieces of Phase 3d the canvas can't:
 *  - **Save persistence** — `onSave` runs `flows_update(id, { graph })`. NO
 *    autosave: a saved+enabled flow is live, so an accidental save would fire
 *    real schedules. Save is only ever the explicit button in the canvas.
 *  - **Unsaved-changes guard** — the canvas reports its dirty state up via
 *    `onDirtyChange`; while dirty we (a) warn on a hard tab close/reload via
 *    `beforeunload`, and (b) intercept the in-page Back button with a confirm
 *    dialog. (App-wide route interception would need a data router; this app
 *    mounts a `HashRouter`, so full `useBlocker` interception isn't available —
 *    the Back button is this page's only in-app navigation affordance.)
 */
import createDebug from 'debug';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useParams } from 'react-router-dom';

import type {
  EditableFlowCanvasHandle,
  EditorSaveMeta,
} from '../components/flows/canvas/EditableFlowCanvas';
import FlowCanvas from '../components/flows/canvas/FlowCanvas';
import FlowRunsSidebar from '../components/flows/FlowRunsSidebar';
import WorkflowCopilotPanel, {
  type RepairPromptContext,
} from '../components/flows/WorkflowCopilotPanel';
import {
  getCopilotThreadId,
  setCopilotThreadId as setCopilotThreadIdCache,
} from '../components/flows/workflowCopilotThreads';
import { ToastContainer } from '../components/intelligence/Toast';
import PanelPage from '../components/layout/PanelPage';
import Button from '../components/ui/Button';
import { CenteredLoadingState, ErrorBanner } from '../components/ui/LoadingState';
import { asFlowCanvasDraftState } from '../lib/flows/canvasDraft';
import { workflowGraphToXyflow } from '../lib/flows/graphAdapter';
import { buildPreviewGraph, diffGraphs } from '../lib/flows/graphDiff';
import type { WorkflowGraph } from '../lib/flows/types';
import { useT } from '../lib/i18n/I18nContext';
import { createFlow, type Flow, getFlow, runFlow, updateFlow } from '../services/api/flowsApi';
import type { WorkflowProposal } from '../store/chatRuntimeSlice';
import type { ToastNotification } from '../types/intelligence';

/**
 * Seed for opening the canvas copilot preloaded from a failed run's "Fix with
 * agent" action (Phase 5c). Rides in `location.state` (ephemeral). The graph is
 * supplied by the editor itself, so only the run context travels here.
 */
export interface CopilotRepairSeed {
  runId: string;
  error?: string | null;
  failingNodeIds?: string[];
}

/**
 * Seed for opening the canvas copilot preloaded from the Flows prompt bar
 * (instant-create): the flow was just created blank, and the copilot should
 * open already building the described workflow. Rides in `location.state`
 * (ephemeral — lost on hard reload, which just leaves a blank flow to edit).
 */
export interface CopilotBuildSeed {
  /** The user's free-text workflow description from the prompt bar. */
  description: string;
  /**
   * Open the copilot chat-first: the graph pane stays hidden and the copilot
   * panel fills the width until the build produces real nodes, at which point
   * the normal split view returns ("graph appears later"). Set by the prompt
   * bar's "Start building" CTA; the "Build" CTA leaves it unset for the
   * classic graph-canvas open.
   */
  chatFirst?: boolean;
}

/** Narrow an opaque `location.state` to a {@link CopilotBuildSeed}. */
export function asCopilotBuildSeed(state: unknown): CopilotBuildSeed | null {
  if (!state || typeof state !== 'object') return null;
  const seed = (state as Record<string, unknown>).copilotBuild;
  if (!seed || typeof seed !== 'object') return null;
  const description = (seed as Record<string, unknown>).description;
  if (typeof description !== 'string' || description.trim().length === 0) return null;
  // Only carry `chatFirst` when explicitly true so the classic ("Build") path
  // keeps returning a bare `{ description }` seed — no behavioural drift there.
  const chatFirst = (seed as Record<string, unknown>).chatFirst === true;
  return chatFirst ? { description, chatFirst: true } : { description };
}

/**
 * Seed for opening the canvas copilot with its input PRE-FILLED (never
 * auto-sent) — the Suggested Workflows "Build this" action navigates here
 * with the suggestion's `build_prompt` so the user can review/edit it before
 * pressing Send themselves. Rides in `location.state` (ephemeral — lost on
 * hard reload, which just leaves a blank flow with an empty copilot input).
 */
export interface CopilotPrefillSeed {
  /** The text to populate the copilot's composer with, unsent. */
  text: string;
  /**
   * The builder mode the FIRST Send after this prefill should use (mirrors
   * `CopilotBuildSeed`'s auto-sent `mode: 'build'` turn). Suggested
   * Workflows' "Build this" always seeds `'build'` — the flow this prefill
   * targets was JUST created blank, matching the server's `BuildMode::Build`
   * contract ("the flow already exists ... design the graph and verify it
   * with dry_run_workflow") — rather than the panel's default `revise` turn,
   * which would treat the blank graph as an existing draft to merely tweak.
   * Defaults to `'build'` if omitted (the only seed producer today), so an
   * older/partial route state still gets the correct first-send mode.
   */
  mode?: 'build' | 'create';
}

/** Narrow an opaque `location.state` to a {@link CopilotPrefillSeed}. */
export function asCopilotPrefillSeed(state: unknown): CopilotPrefillSeed | null {
  if (!state || typeof state !== 'object') return null;
  const seed = (state as Record<string, unknown>).copilotPrefill;
  if (!seed || typeof seed !== 'object') return null;
  const text = (seed as Record<string, unknown>).text;
  if (typeof text !== 'string' || text.trim().length === 0) return null;
  const rawMode = (seed as Record<string, unknown>).mode;
  const mode = rawMode === 'build' || rawMode === 'create' ? rawMode : 'build';
  return { text, mode };
}

/** Narrow an opaque `location.state` to a {@link CopilotRepairSeed}. */
export function asCopilotRepairSeed(state: unknown): CopilotRepairSeed | null {
  if (!state || typeof state !== 'object') return null;
  const record = state as Record<string, unknown>;
  const seed = record.copilotRepair;
  if (!seed || typeof seed !== 'object') return null;
  const s = seed as Record<string, unknown>;
  if (typeof s.runId !== 'string') return null;
  return {
    runId: s.runId,
    error: typeof s.error === 'string' ? s.error : null,
    failingNodeIds: Array.isArray(s.failingNodeIds)
      ? s.failingNodeIds.filter((v): v is string => typeof v === 'string')
      : undefined,
  };
}

const log = createDebug('app:flows:canvas');

/** Which panel (if any) the canvas side rail shows. Driven by the header toggle. */
type SidePanel = 'copilot' | 'legend' | null;

type LoadState =
  | { status: 'loading' }
  | { status: 'notFound' }
  | { status: 'error'; message: string }
  | { status: 'ready'; flow: Flow };

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * True when `title` is "unclaimed" — either blank or exactly the localized
 * generic placeholder (`t('flows.page.newWorkflow')`, "New workflow") — i.e.
 * nothing user-meaningful has been set yet. Used to decide whether accepting
 * a copilot proposal is allowed to adopt `proposal.name` as the flow title:
 * it must never clobber a user-chosen or description-derived name. Pure and
 * exported for direct unit testing without rendering `FlowEditor`.
 */
export function isPlaceholderTitle(title: string, placeholder: string): boolean {
  const trimmed = title.trim();
  return trimmed === '' || trimmed === placeholder.trim();
}

function BackIcon() {
  return (
    <svg
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg className="h-4 w-4" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M7 5l12 7-12 7V5z" />
    </svg>
  );
}

function SaveIcon() {
  // Floppy disk.
  return (
    <svg
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
      <path d="M17 21v-8H7v8M7 3v5h8" />
    </svg>
  );
}

function DiscardIcon() {
  return (
    <svg
      className="h-4 w-4"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path d="M18 6L6 18M6 6l12 12" />
    </svg>
  );
}

/**
 * A flow ready for the editable canvas — either a persisted flow (`flowId` set)
 * or an unsaved draft handed in from the chat `WorkflowProposalCard` "Open in
 * canvas" action (`flowId === null`, Phase 4e).
 */
interface EditorFlow {
  /** Persisted flow id, or `null` for an unsaved draft. */
  flowId: string | null;
  name: string;
  graph: WorkflowGraph;
  /** "Require approval" toggle carried into `flows_create` when saving a draft. */
  requireApproval: boolean;
}

/** The editable canvas body — split out so its hooks only mount once a flow loads. */
function FlowEditor({
  editorFlow,
  initialCopilotSeed = null,
  initialBuildSeed = null,
  onBuildSeedConsumed,
  initialPrefillSeed = null,
  onPrefillSeedConsumed,
  locationKey,
}: {
  editorFlow: EditorFlow;
  initialCopilotSeed?: CopilotRepairSeed | null;
  /** Prompt-bar instant-create seed: open the copilot already building this. */
  initialBuildSeed?: CopilotBuildSeed | null;
  /** Clear the route's build seed once the copilot has dispatched it (#4597). */
  onBuildSeedConsumed?: () => void;
  /** Suggested Workflows seed: open the copilot with its input pre-filled (unsent). */
  initialPrefillSeed?: CopilotPrefillSeed | null;
  /** Clear the route's prefill seed once the copilot has consumed it. */
  onPrefillSeedConsumed?: () => void;
  /**
   * The route's `location.key` (issue B22) — react-router mints a fresh one on
   * every navigation, including a same-path repeat navigation (e.g. the "Fix
   * with agent" action from {@link FlowRunsSidebar}, which stays on this same
   * `/flows/:id` route and so does NOT remount `FlowEditor`). Folded into the
   * copilot panel's `key` below so a repair seed arriving without a page-level
   * remount still forces a fresh panel mount — required for the panel's
   * once-per-mount auto-fire guard (`repairSentRef`) to actually fire again.
   */
  locationKey: string;
}) {
  const { t } = useT();
  const navigate = useNavigate();
  const [dirty, setDirty] = useState(false);
  // Save/Discard now live in the page header; the canvas reports its Save state
  // up and exposes save()/discard() via this handle.
  const canvasRef = useRef<EditableFlowCanvasHandle>(null);
  const [saveMeta, setSaveMeta] = useState<EditorSaveMeta>({
    dirty: false,
    hasErrors: false,
    saving: false,
  });
  const [leaveConfirm, setLeaveConfirm] = useState(false);
  // Which header action (run/save/discard) is awaiting confirmation, if any —
  // every icon click opens a confirm popup before it fires.
  const [confirmAction, setConfirmAction] = useState<'run' | 'save' | 'discard' | null>(null);
  // Active run id (== thread_id) driving the canvas's live per-node overlay
  // (Phase 3e). Set when the user runs the flow; the canvas subscribes to the
  // `flow:run_progress` feed for it via `useFlowRunProgress`.
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);

  const { flowId, graph, requireApproval } = editorFlow;
  // Draft (unsaved) canvases have no persisted id yet; Save creates the flow
  // rather than updating one, and there is nothing runnable to run.
  const isDraft = flowId === null;

  // Editable flow name. `name` is the committed value (used by Save + the run
  // header); `titleDraft` is the in-progress input buffer. Renaming a persisted
  // flow is metadata-only (`flows_update({ name })`) — it never touches the
  // graph, so it can't fire a schedule, and is safe to persist on blur/Enter
  // without the graph's explicit-Save gate. A draft just updates locally; the
  // name rides into `flows_create` when the draft is first Saved.
  const [name, setName] = useState(editorFlow.name);
  const [titleDraft, setTitleDraft] = useState(editorFlow.name);
  const [renaming, setRenaming] = useState(false);
  // Last name actually persisted to the backend (mirrors `persistedGraphRef`
  // below, same rationale): a manual rename persists immediately via
  // `commitRename`, but adopting a copilot proposal's name (below) only
  // updates local state — Save is still the gate for THAT change. Tracking
  // the real baseline (rather than diffing against the initial `editorFlow.name`
  // prop) lets `handleSave` include `name` in its `flows_update` payload only
  // when it actually diverges from what's on the server.
  const persistedNameRef = useRef<string>(editorFlow.name);

  const commitRename = useCallback(async () => {
    const trimmed = titleDraft.trim();
    if (!trimmed || trimmed === name) {
      setTitleDraft(name);
      return;
    }
    if (isDraft) {
      log('rename (draft): %s', trimmed);
      setName(trimmed);
      return;
    }
    setRenaming(true);
    try {
      log('rename: flow id=%s name=%s', flowId, trimmed);
      await updateFlow(flowId, { name: trimmed });
      setName(trimmed);
      persistedNameRef.current = trimmed;
    } catch (err) {
      log('rename failed: id=%s err=%o', flowId, err);
      setTitleDraft(name);
    } finally {
      setRenaming(false);
    }
  }, [titleDraft, name, isDraft, flowId]);

  // ── Canvas copilot + draft overlay (Phase 5c) ─────────────────────────────
  // `draftGraph` is the current ACCEPTED draft (starts as the loaded graph),
  // kept in sync with manual canvas edits via `onGraphChange`. A copilot
  // proposal enters `preview`: the canvas re-seeds (bump `canvasVersion`) with
  // the proposed graph plus ghosted removed nodes, painted diff-style. Accept
  // commits the proposed graph into `draftGraph`; Reject reverts to the frozen
  // base. NOTHING here persists — the canvas's own Save is the only gate.
  // The canvas side panel shows one of two things — the Copilot, or the read-only
  // node Legend — or nothing. The header toggle switches between them. The
  // Copilot is shown by DEFAULT (and any build/prefill/repair seed also targets
  // it); the user can switch to the Legend or collapse the rail entirely.
  const [sidePanel, setSidePanel] = useState<SidePanel>('copilot');
  const copilotOpen = sidePanel === 'copilot';
  // Toggle a panel: selecting the active one again closes the side panel.
  const toggleSidePanel = useCallback(
    (panel: Exclude<SidePanel, null>) => setSidePanel(cur => (cur === panel ? null : panel)),
    []
  );
  // Issue B22: a repair seed can also arrive WITHOUT a `FlowEditor` remount —
  // "Fix with agent" clicked from `FlowRunsSidebar` stays on this same
  // `/flows/:id` route (only `location.state`/`location.key` change), so the
  // `useState` initializer above (mount-only) won't re-open the panel for it.
  // Re-assert the copilot whenever a new repair seed prop arrives — done during
  // render (React's "adjusting state when a prop changes" pattern) rather
  // than a `useEffect`, so it lands in the same render pass instead of an
  // extra one.
  const [seenCopilotSeed, setSeenCopilotSeed] = useState(initialCopilotSeed);
  if (initialCopilotSeed !== seenCopilotSeed) {
    setSeenCopilotSeed(initialCopilotSeed);
    if (initialCopilotSeed) setSidePanel('copilot');
  }
  // Per-workflow copilot thread: seeded from the session cache so opening/closing
  // the panel (or switching flows and back) resumes the same conversation
  // instead of starting a fresh `workflow_builder` thread each time.
  const [copilotThreadId, setCopilotThreadId] = useState<string | null>(() =>
    getCopilotThreadId(flowId)
  );
  const handleCopilotThreadId = useCallback(
    (id: string | null) => {
      setCopilotThreadId(id);
      setCopilotThreadIdCache(flowId, id);
    },
    [flowId]
  );
  const [draftGraph, setDraftGraph] = useState<WorkflowGraph>(graph);
  const [preview, setPreview] = useState<{
    proposal: WorkflowProposal;
    base: WorkflowGraph;
    addedNodeIds: Set<string>;
    removedNodeIds: Set<string>;
  } | null>(null);
  const [canvasVersion, setCanvasVersion] = useState(0);

  // Chat-first open ("Start building" from the prompt bar): keep the graph pane
  // hidden and let the copilot fill the surface until the build produces real
  // nodes — "graph appears later". `graphRevealed` latches true the first time
  // a proposal preview arrives or the draft gains a node beyond the lone
  // trigger, and never flips back (so rejecting a proposal can't re-hide it).
  const chatFirst = initialBuildSeed?.chatFirst === true;
  const [graphRevealed, setGraphRevealed] = useState(!chatFirst);
  if (!graphRevealed && (preview !== null || draftGraph.nodes.length > 1)) {
    setGraphRevealed(true);
  }
  // Only actually hide the graph while the copilot is open — closing the panel
  // must always leave the (possibly empty) canvas visible, never a blank pane.
  const hideGraph = chatFirst && !graphRevealed && copilotOpen;

  // Last-persisted graph, independent of canvas remounts (fixes a P1: the
  // editable canvas seeds its own dirty baseline from whatever graph it's
  // mounted with, so bumping `canvasVersion` on Accept — remounting the
  // canvas with the just-accepted proposal as its "initial" graph — made an
  // unsaved accepted proposal instantly read as clean; the accepted change
  // was then lost on back/reload instead of gating behind the required Save.
  // Only ever updated by a real Save (`handleSave` below), so a diff against
  // it survives any number of accept/reject/preview remounts.
  const persistedGraphRef = useRef<WorkflowGraph>(graph);

  // Persist the live graph. A saved flow updates in place via `flows_update`; a
  // draft is created via `flows_create` (the single persistence gate — an
  // agent's `propose_workflow` never reaches this RPC), then we replace into
  // the new flow's canonical `/flows/:id` canvas so further saves update it.
  // Rejections propagate so the canvas surfaces the failure inline (and leaves
  // the draft dirty).
  //
  // Issue B21: `flows_update` re-validates/normalizes the graph server-side
  // (schema migration, id defaults, port normalization, etc.) before
  // persisting, so the canonical saved shape can differ from what the client
  // sent. Re-sync the canvas draft from the RESPONSE (not just the just-sent
  // `next`) and bump `canvasVersion` so the editable canvas re-seeds from the
  // canonical persisted graph immediately — matching what a navigate-away-
  // and-back remount would show, without requiring one.
  //
  // Declared ahead of `handleAcceptProposal` (below), which calls it directly
  // to persist an accepted proposal immediately.
  const handleSave = useCallback(
    async (next: WorkflowGraph, overrideName?: string, overrideRequireApproval?: boolean) => {
      // `overrideName` covers the copilot-Accept call site: it calls
      // `setName(proposal.name)` and `handleSave(...)` in the same handler,
      // but `name` in THIS closure is still the pre-update value — React
      // state updates don't land until the next render. Falling back to the
      // (possibly stale) `name` keeps the normal manual-Save call site
      // (which never passes an override) unaffected.
      const effectiveName = overrideName ?? name;
      // Same stale-closure concern for `requireApproval`: an accepted
      // proposal carries its own approval policy (`WorkflowProposal.
      // requireApproval`), which must win over the currently-loaded canvas
      // policy — otherwise Accept would silently keep the old flow's policy
      // instead of the one the agent proposed. A plain manual Save never
      // passes an override, so `requireApproval` (the loaded flow's current
      // policy) is unaffected.
      const effectiveRequireApproval = overrideRequireApproval ?? requireApproval;
      if (isDraft) {
        log(
          'save: creating draft name=%s nodes=%d edges=%d requireApproval=%s',
          effectiveName,
          next.nodes.length,
          next.edges.length,
          effectiveRequireApproval
        );
        const created = await createFlow(effectiveName, next, effectiveRequireApproval);
        log('save: draft persisted as flow id=%s', created.id);
        navigate(`/flows/${created.id}`, { replace: true });
        return;
      }
      // Only include `name` / `requireApproval` in the update payload when
      // they actually diverge from what's already persisted (a manual
      // rename already persisted `name` via `commitRename`; a copilot-
      // adopted placeholder name or proposal-driven policy has not) — keeps
      // the update metadata-safe and avoids needless writes.
      const nameChanged = effectiveName !== persistedNameRef.current;
      const requireApprovalChanged = overrideRequireApproval !== undefined;
      log(
        'save: flow id=%s nodes=%d edges=%d nameChanged=%s requireApprovalChanged=%s',
        flowId,
        next.nodes.length,
        next.edges.length,
        nameChanged,
        requireApprovalChanged
      );
      const updated = await updateFlow(flowId, {
        graph: next,
        ...(nameChanged ? { name: effectiveName } : {}),
        ...(requireApprovalChanged ? { requireApproval: effectiveRequireApproval } : {}),
      });
      const persisted = updated.graph as WorkflowGraph;
      persistedGraphRef.current = persisted;
      persistedNameRef.current = updated.name;
      setDraftGraph(persisted);
      if (updated.name !== name) {
        // Re-sync BOTH title states from the response — leaving `titleDraft`
        // stale would show the pre-save value in the input and could
        // resubmit it verbatim on a later blur.
        setName(updated.name);
        setTitleDraft(updated.name);
      }
      setCanvasVersion(v => v + 1);
      log(
        'save: flow id=%s persisted — canvas re-synced from response nodes=%d edges=%d',
        flowId,
        persisted.nodes.length,
        persisted.edges.length
      );
    },
    [isDraft, flowId, name, requireApproval, navigate]
  );

  const handleGraphChange = useCallback(
    (next: WorkflowGraph) => {
      // Freeze the draft while a proposal is under review — the preview graph
      // (with ghosts) must not overwrite the real draft.
      if (preview) return;
      setDraftGraph(next);
    },
    [preview]
  );

  const handleProposal = useCallback(
    (proposal: WorkflowProposal) => {
      const proposedGraph = proposal.graph as WorkflowGraph;
      const d = diffGraphs(draftGraph, proposedGraph);
      log('copilot proposal: added=%d removed=%d', d.addedNodeIds.size, d.removedNodeIds.size);
      setPreview({
        proposal,
        base: draftGraph,
        addedNodeIds: d.addedNodeIds,
        removedNodeIds: d.removedNodeIds,
      });
      setCanvasVersion(v => v + 1);
    },
    [draftGraph]
  );

  // Accept now REVIEWS + SAVES in one step: the canvas copilot's own inline
  // proposal card previously only applied the proposal to the local
  // `draftGraph` and left persistence to a separate header Save click the
  // user rarely noticed — the proposal would look "accepted" while nothing
  // was actually saved (confirmed live: a flow persisted as empty
  // trigger-only after the user said "looks good"/"save it" and the agent
  // had no further action to take). Accept now immediately persists the
  // just-applied graph via `handleSave`, matching what a manual Save click
  // right after Accept would have done. A failed save is non-fatal: the
  // proposal stays applied to the (now dirty) draft and the header Save
  // button remains the manual retry — we never crash or revert the draft.
  const handleAcceptProposal = useCallback(
    async (proposal: WorkflowProposal) => {
      log('copilot proposal accepted');
      const proposedGraph = proposal.graph as WorkflowGraph;
      setDraftGraph(proposedGraph);
      setPreview(null);
      setCanvasVersion(v => v + 1);

      // Adopt the proposal's name into the flow title, but ONLY while the
      // title is still the generic placeholder — never clobber a user-chosen
      // or description-derived meaningful name.
      //
      // Check the VISIBLE `titleDraft`, not the committed `name` — `name`
      // only updates on blur/Enter via `commitRename`, so if the user is
      // mid-typing a custom title (or a rename is still in flight) when a
      // proposal is accepted, `name` can still read as the stale placeholder
      // while `titleDraft` already holds the user's real input. Deciding off
      // `name` would silently clobber that in-progress input. Also skip
      // entirely while `renaming` is true — an in-flight `commitRename`
      // persist must not race with a local proposal-driven rename.
      const proposedName = proposal.name?.trim();
      // `handleSave` closes over `name`, which — even after `setName` below —
      // won't reflect the adopted name until the NEXT render (stale-closure
      // pitfall). Pass it through explicitly as an override instead of
      // relying on `name` to have updated in time.
      let overrideName: string | undefined;
      if (
        proposedName &&
        !renaming &&
        isPlaceholderTitle(titleDraft, t('flows.page.newWorkflow'))
      ) {
        // Log shape, not the user-authored name (no PII in logs).
        log(
          'copilot proposal accepted: adopting proposed name into placeholder title, isDraft=%s',
          isDraft
        );
        setName(proposedName);
        setTitleDraft(proposedName);
        overrideName = proposedName;
      }

      // Persist immediately — this is the actual fix. Do NOT route through
      // `canvasRef.current?.save()`: the canvas is mid-remount (the
      // `canvasVersion` bump above) so the ref's imperative handle is stale;
      // call `handleSave` directly with the known-good proposed graph.
      try {
        await handleSave(proposedGraph, overrideName, proposal.requireApproval);
        log('copilot proposal accepted: persisted');
      } catch (err) {
        log('copilot proposal accepted: save failed err=%o', err);
        // Rethrow: the draft above is already applied unconditionally, so no
        // data is lost by rethrowing. This lets the caller — the copilot
        // panel's own `accept` handler — see the failure and skip
        // `clearProposal()`, keeping the proposal card visible for retry
        // instead of silently vanishing while nothing was actually saved.
        // `acceptSaving` there still resets via its own `finally`.
        throw err;
      }
    },
    [titleDraft, renaming, t, isDraft, handleSave]
  );

  const handleRejectProposal = useCallback(() => {
    log('copilot proposal rejected');
    setPreview(null);
    setCanvasVersion(v => v + 1);
  }, []);

  // The graph the canvas renders: the proposed+ghosted preview while reviewing,
  // else the accepted draft.
  const editorGraph = useMemo(
    () =>
      preview
        ? buildPreviewGraph(
            preview.base,
            preview.proposal.graph as WorkflowGraph,
            preview.removedNodeIds
          )
        : draftGraph,
    [preview, draftGraph]
  );
  const { nodes, edges } = useMemo(() => workflowGraphToXyflow(editorGraph), [editorGraph]);
  const meta = useMemo(
    () => ({ schema_version: graph.schema_version, id: flowId ?? undefined, name }),
    [graph.schema_version, flowId, name]
  );
  // Also dirty when a copilot-adopted proposal name has changed the flow's
  // `name` without yet persisting it (`persistedNameRef` only advances on a
  // real Save/rename) — a name-only proposal (same graph, new name) must
  // still enable Save, or the adopted title can never be persisted.
  const initialDirty = useMemo(
    () =>
      JSON.stringify(editorGraph) !== JSON.stringify(persistedGraphRef.current) ||
      name !== persistedNameRef.current,
    [editorGraph, name]
  );

  // Repair seed for the copilot: bind the run context to the CURRENT draft.
  const copilotRepairSeed = useMemo<RepairPromptContext | null>(
    () =>
      initialCopilotSeed
        ? {
            runId: initialCopilotSeed.runId,
            error: initialCopilotSeed.error,
            failingNodeIds: initialCopilotSeed.failingNodeIds,
            graph: draftGraph,
          }
        : null,
    // Only seed once (on the initial draft) — a later draft edit must not
    // re-fire the repair turn.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [initialCopilotSeed]
  );

  // Warn on hard tab close / reload while there are unsaved edits.
  useEffect(() => {
    if (!dirty) return;
    const handler = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', handler);
    return () => window.removeEventListener('beforeunload', handler);
  }, [dirty]);

  // Run the *persisted* flow and hand its thread_id to the canvas so it can
  // overlay live per-node status (Phase 3e). Runs the saved version — not the
  // (possibly dirty) draft — matching the "Save is explicit, running is live"
  // model. The durable run row + poller remain the source of truth.
  const handleRun = useCallback(async () => {
    if (flowId === null) return; // drafts aren't runnable until saved
    setRunning(true);
    setRunError(null);
    try {
      log('run: starting flow id=%s', flowId);
      const result = await runFlow(flowId);
      log('run: started flow id=%s thread_id=%s', flowId, result.thread_id);
      setActiveRunId(result.thread_id);
    } catch (err) {
      const message = errorMessage(err);
      log('run: failed id=%s err=%o', flowId, err);
      setRunError(message);
    } finally {
      setRunning(false);
    }
  }, [flowId]);

  // Return to wherever the user came from rather than always the list. React
  // Router stamps the initial history entry with key 'default', so when this
  // page was the first thing loaded (deep link / fresh load) there's nothing to
  // go back to — fall back to the workflows list so Back never dead-ends.
  const goBack = useCallback(() => {
    if (locationKey === 'default') {
      log('back: no prior history — falling back to /flows');
      navigate('/flows');
    } else {
      log('back: navigating to previous page');
      navigate(-1);
    }
  }, [locationKey, navigate]);

  const handleBack = useCallback(() => {
    if (dirty) {
      log('back: dirty — prompting for confirmation');
      setLeaveConfirm(true);
      return;
    }
    goBack();
  }, [dirty, goBack]);

  const backButton = (
    <Button
      type="button"
      variant="tertiary"
      size="xs"
      iconOnly
      data-testid="flow-canvas-back"
      aria-label={t('flows.canvas.backToList')}
      onClick={handleBack}>
      <BackIcon />
    </Button>
  );

  // A draft has nothing persisted to run yet — the canvas's Save (which creates
  // the flow) is the only gate, so no Run affordance until it's saved.
  const runButton = isDraft ? undefined : (
    <Button
      type="button"
      variant="primary"
      size="xs"
      analyticsId="flow-canvas-run"
      iconOnly
      data-testid="flow-canvas-run"
      aria-label={running ? t('flows.editor.running') : t('flows.editor.run')}
      title={running ? t('flows.editor.running') : t('flows.editor.run')}
      disabled={running}
      onClick={() => setConfirmAction('run')}>
      <PlayIcon />
    </Button>
  );

  // Segmented toggle for the side rail: Copilot | Legend. Clicking the active
  // segment again collapses the rail (full-width graph). Replaces the old
  // single copilot on/off button.
  const sidePanelToggle = (
    <div
      role="group"
      aria-label={t('flows.canvas.sidePanelToggle')}
      className="inline-flex items-center rounded-lg border border-line bg-surface p-0.5">
      {(
        [
          { key: 'copilot', label: t('flows.copilot.open'), testId: 'flow-canvas-copilot-toggle' },
          {
            key: 'legend',
            label: t('flows.canvas.legendTab'),
            testId: 'flow-canvas-legend-toggle',
          },
        ] as const
      ).map(tab => {
        const active = sidePanel === tab.key;
        return (
          <button
            key={tab.key}
            type="button"
            aria-pressed={active}
            data-testid={tab.testId}
            onClick={() => toggleSidePanel(tab.key)}
            className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
              active
                ? 'bg-primary-500 text-content-inverted shadow-sm'
                : 'text-content-secondary hover:bg-surface-hover'
            }`}>
            {tab.label}
          </button>
        );
      })}
    </div>
  );

  // Save / Discard moved out of the canvas into the header (the canvas keeps
  // only undo/redo), as icon buttons. Each opens a confirm popup before firing;
  // they drive the editable canvas through `canvasRef`.
  const saveActions = (
    <div className="flex items-center gap-1.5">
      {saveMeta.dirty && (
        <span
          className="rounded-full bg-amber-100 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:bg-amber-500/15 dark:text-amber-300"
          data-testid="flow-editor-dirty">
          {t('flows.editor.unsaved')}
        </span>
      )}
      <Button
        type="button"
        variant="tertiary"
        size="xs"
        iconOnly
        data-testid="flow-editor-discard"
        aria-label={t('flows.editor.discard')}
        title={t('flows.editor.discard')}
        disabled={!saveMeta.dirty || saveMeta.saving}
        onClick={() => setConfirmAction('discard')}>
        <DiscardIcon />
      </Button>
      <Button
        type="button"
        variant="primary"
        size="xs"
        iconOnly
        data-testid="flow-editor-save"
        aria-label={saveMeta.saving ? t('flows.editor.saving') : t('flows.editor.save')}
        title={saveMeta.hasErrors ? t('flows.editor.saveBlocked') : t('flows.editor.save')}
        disabled={!saveMeta.dirty || saveMeta.hasErrors || saveMeta.saving || preview !== null}
        onClick={() => setConfirmAction('save')}>
        <SaveIcon />
      </Button>
    </div>
  );

  // Keep the save actions and Run button adjacent; the panel toggle sits apart.
  const headerActions = (
    <div className="flex items-center gap-2">
      {sidePanelToggle}
      <div className="flex items-center gap-1.5">
        {saveActions}
        {runButton}
      </div>
    </div>
  );

  // Editable title: an unstyled input that reads as the page heading until
  // focused, so renaming is discoverable without a separate edit affordance.
  const titleNode = (
    <input
      type="text"
      value={titleDraft}
      disabled={renaming}
      data-testid="flow-canvas-title"
      aria-label={t('flows.canvas.renameLabel')}
      onChange={e => setTitleDraft(e.target.value)}
      onBlur={() => void commitRename()}
      onKeyDown={e => {
        if (e.key === 'Enter') {
          e.preventDefault();
          e.currentTarget.blur();
        } else if (e.key === 'Escape') {
          setTitleDraft(name);
          e.currentTarget.blur();
        }
      }}
      className="w-full max-w-md truncate rounded-md border border-transparent bg-transparent px-1 py-0.5 text-base font-semibold text-content hover:border-line focus:border-primary-400 focus:outline-none disabled:opacity-60"
    />
  );

  return (
    <PanelPage
      testId="flow-canvas-page"
      title={titleNode}
      leading={backButton}
      action={headerActions}
      contentClassName="h-full p-0">
      <div className="flex h-full w-full">
        {/* Run history + "Fix with agent" as an inline left rail (persisted flows
            only). The app sidebar is hidden on this route (chromeless), so this
            can't use the shell `SidebarContent` slot — render it in-page. */}
        {!isDraft && flowId && (
          <div className="hidden h-full w-60 flex-shrink-0 border-r border-line lg:flex">
            <FlowRunsSidebar flowId={flowId} />
          </div>
        )}
        <div className={`relative h-full flex-1 ${hideGraph ? 'hidden' : ''}`}>
          <FlowCanvas
            key={`canvas-${canvasVersion}`}
            ref={canvasRef}
            editable
            nodes={nodes}
            edges={edges}
            meta={meta}
            onSave={handleSave}
            onDirtyChange={setDirty}
            onSaveMetaChange={setSaveMeta}
            activeRunId={activeRunId}
            onGraphChange={handleGraphChange}
            addedNodeIds={preview?.addedNodeIds}
            removedNodeIds={preview?.removedNodeIds}
            saveDisabled={preview !== null}
            initialDirty={initialDirty}
            showPalette={sidePanel === 'legend'}
          />

          {runError && (
            <div className="pointer-events-none absolute inset-x-3 top-3 z-20 flex justify-center">
              <div
                role="alert"
                data-testid="flow-canvas-run-error"
                className="pointer-events-auto rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
                {t('flows.editor.runFailed')}: {runError}
              </div>
            </div>
          )}

          {leaveConfirm && (
            <div
              className="absolute inset-0 z-30 flex items-center justify-center bg-black/30 p-4"
              data-testid="flow-leave-confirm">
              <div className="w-full max-w-sm rounded-xl border border-line bg-surface p-4 shadow-xl">
                <h2 className="text-sm font-semibold text-content">
                  {t('flows.editor.leaveTitle')}
                </h2>
                <p className="mt-1 text-xs text-content-muted">{t('flows.editor.leaveBody')}</p>
                <div className="mt-4 flex justify-end gap-2">
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    data-testid="flow-leave-stay"
                    onClick={() => setLeaveConfirm(false)}>
                    {t('flows.editor.leaveStay')}
                  </Button>
                  <Button
                    type="button"
                    variant="primary"
                    tone="danger"
                    size="sm"
                    data-testid="flow-leave-discard"
                    onClick={() => {
                      log('back: confirmed leave — discarding unsaved edits');
                      goBack();
                    }}>
                    {t('flows.editor.leaveDiscard')}
                  </Button>
                </div>
              </div>
            </div>
          )}
        </div>

        {copilotOpen && (
          <WorkflowCopilotPanel
            // Stable ('copilot') across manual open/close and build-seed
            // navigations (unaffected — those always land on a fresh
            // `FlowEditor` mount already, see `locationKey`'s doc comment).
            // Repair seeds fold in `locationKey` so a same-route "Fix with
            // agent" click (no `FlowEditor` remount) still forces a fresh
            // panel mount, resetting the once-per-mount `repairSentRef` guard
            // so the repair turn actually (re)fires (issue B22).
            key={initialCopilotSeed ? `copilot-repair-${locationKey}` : 'copilot'}
            graph={preview?.base ?? draftGraph}
            flowId={flowId}
            onProposal={handleProposal}
            onAccept={handleAcceptProposal}
            onReject={handleRejectProposal}
            onClose={() => setSidePanel(null)}
            repairSeed={copilotRepairSeed}
            buildSeed={initialBuildSeed}
            onBuildSeedConsumed={onBuildSeedConsumed}
            prefillSeed={initialPrefillSeed}
            onPrefillSeedConsumed={onPrefillSeedConsumed}
            seedThreadId={copilotThreadId}
            onThreadIdChange={handleCopilotThreadId}
            fullWidth={hideGraph}
          />
        )}

        {/* Confirm popup for the header's Run / Save / Discard icon buttons. */}
        {confirmAction && (
          <div
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4"
            data-testid="flow-action-confirm">
            <div className="w-full max-w-sm rounded-xl border border-line bg-surface p-4 shadow-xl">
              <h2 className="text-sm font-semibold text-content">
                {t(`flows.editor.confirm.${confirmAction}Title`)}
              </h2>
              <p className="mt-1 text-xs text-content-muted">
                {t(`flows.editor.confirm.${confirmAction}Body`)}
              </p>
              <div className="mt-4 flex justify-end gap-2">
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  data-testid="flow-action-cancel"
                  onClick={() => setConfirmAction(null)}>
                  {t('flows.editor.confirm.cancel')}
                </Button>
                <Button
                  type="button"
                  variant="primary"
                  tone={confirmAction === 'discard' ? 'danger' : undefined}
                  size="sm"
                  data-testid="flow-action-confirm-accept"
                  onClick={() => {
                    const action = confirmAction;
                    setConfirmAction(null);
                    if (action === 'run') void handleRun();
                    else if (action === 'save') canvasRef.current?.save();
                    else if (action === 'discard') canvasRef.current?.discard();
                  }}>
                  {t('flows.editor.confirm.confirm')}
                </Button>
              </div>
            </div>
          </div>
        )}
      </div>
    </PanelPage>
  );
}

export default function FlowCanvasPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const { id } = useParams<{ id: string }>();
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  // The app sidebar is hidden entirely on this route via App's `chromeless`
  // check, so the builder owns the full viewport — nothing to do here.
  // "Fix with agent" (Phase 5c) navigates here with a repair seed in
  // `location.state` so the copilot opens preloaded with the failed run.
  const copilotSeed = useMemo(() => asCopilotRepairSeed(location.state), [location.state]);
  // The Flows prompt bar's instant-create path navigates here with a build
  // seed so the copilot opens already building the described workflow.
  const buildSeed = useMemo(() => asCopilotBuildSeed(location.state), [location.state]);
  // The Suggested Workflows "Build this" action navigates here with a prefill
  // seed so the copilot opens with its input pre-filled (unsent) from the
  // suggestion's `build_prompt`.
  const prefillSeed = useMemo(() => asCopilotPrefillSeed(location.state), [location.state]);

  // Strip the ephemeral build seed from `location.state` once the copilot has
  // dispatched it. The panel's own `buildSentRef` guard is per-mount, so
  // closing + reopening the copilot remounts it and would otherwise re-fire the
  // same `build` turn against the still-present route seed (issue #4597).
  // Preserve any other state fields (e.g. a repair seed) — only drop
  // `copilotBuild`.
  const clearBuildSeed = useCallback(() => {
    // Named `routeState` (not `state`) to avoid shadowing the component-level
    // `state` from `useState<LoadState>` that drives this file's render switch.
    const routeState = location.state;
    if (!routeState || typeof routeState !== 'object' || !('copilotBuild' in routeState)) return;
    const next = { ...(routeState as Record<string, unknown>) };
    delete next.copilotBuild;
    log('build seed consumed — clearing route state: id=%s', id);
    // Navigate with an object (not a bare pathname) so the current search and
    // hash are preserved — a string target would drop them.
    navigate(
      { pathname: location.pathname, search: location.search, hash: location.hash },
      { replace: true, state: next }
    );
  }, [id, location.state, location.pathname, location.search, location.hash, navigate]);

  // Strip the ephemeral prefill seed from `location.state` once the copilot
  // has consumed it (populated its input) — same rationale as
  // `clearBuildSeed`: a remount (close + reopen the copilot) must not re-fill
  // the input a second time against the still-present route seed. Only drops
  // `copilotPrefill`, preserving any sibling state fields.
  const clearPrefillSeed = useCallback(() => {
    const routeState = location.state;
    if (!routeState || typeof routeState !== 'object' || !('copilotPrefill' in routeState)) return;
    const next = { ...(routeState as Record<string, unknown>) };
    delete next.copilotPrefill;
    log('prefill seed consumed — clearing route state: id=%s', id);
    navigate(
      { pathname: location.pathname, search: location.search, hash: location.hash },
      { replace: true, state: next }
    );
  }, [id, location.state, location.pathname, location.search, location.hash, navigate]);

  useEffect(() => {
    // Guards a stale response from clobbering newer state: this effect
    // re-runs on every `:id` change without the component remounting (same
    // route, different param), and on unmount, so a slow fetch for a
    // previous id (or one that resolves after the component is gone) must
    // not call `setState` once superseded. Same pattern as
    // `useFlowRunPoller.ts`'s `cancelled`/`mountedRef` guard.
    let cancelled = false;

    if (!id) {
      log('load: no id in route params');
      setState({ status: 'notFound' });
      return;
    }

    log('load: fetching flow id=%s', id);
    setState({ status: 'loading' });

    void (async () => {
      try {
        const flow = await getFlow(id);
        if (cancelled) {
          log('load: fetched flow id=%s but superseded/unmounted, dropping', id);
          return;
        }
        log('load: fetched flow id=%s name=%s', flow.id, flow.name);
        setState({ status: 'ready', flow });
      } catch (err) {
        if (cancelled) return;
        const message = errorMessage(err);
        log('load: failed id=%s err=%o', id, err);
        if (message.toLowerCase().includes('not found')) {
          setState({ status: 'notFound' });
        } else {
          setState({ status: 'error', message });
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [id]);

  if (state.status === 'ready') {
    // Keyed by flow id so switching flows cleanly re-seeds the editable canvas's
    // controlled node/edge state (which only reads its props at mount).
    const flow = state.flow;
    return (
      <FlowEditor
        key={flow.id}
        editorFlow={{
          flowId: flow.id,
          name: flow.name,
          graph: flow.graph as WorkflowGraph,
          requireApproval: flow.require_approval,
        }}
        initialCopilotSeed={copilotSeed}
        initialBuildSeed={buildSeed}
        onBuildSeedConsumed={clearBuildSeed}
        initialPrefillSeed={prefillSeed}
        onPrefillSeedConsumed={clearPrefillSeed}
        locationKey={location.key}
      />
    );
  }

  const backButton = (
    <Button
      type="button"
      variant="tertiary"
      size="xs"
      iconOnly
      data-testid="flow-canvas-back"
      aria-label={t('flows.canvas.backToList')}
      onClick={() => navigate('/flows')}>
      <BackIcon />
    </Button>
  );

  return (
    <PanelPage
      testId="flow-canvas-page"
      title={t('flows.canvas.title')}
      leading={backButton}
      contentClassName="h-full p-0">
      {state.status === 'loading' && (
        <div className="flex h-full items-center justify-center">
          <CenteredLoadingState label={t('flows.canvas.loading')} />
        </div>
      )}

      {state.status === 'error' && (
        <div className="p-4" data-testid="flow-canvas-error">
          <ErrorBanner message={state.message || t('flows.canvas.loadError')} />
        </div>
      )}

      {state.status === 'notFound' && (
        <div className="flex h-full items-center justify-center p-4">
          <p className="text-sm text-content-muted" data-testid="flow-canvas-not-found">
            {t('flows.canvas.notFound')}
          </p>
        </div>
      )}
    </PanelPage>
  );
}

/**
 * FlowCanvasDraftPage (Phase 4e) — the editable Workflow Canvas hosting an
 * UNSAVED draft handed in from the chat `WorkflowProposalCard` "Open in canvas"
 * action, at `/flows/draft`. The candidate graph rides in `location.state`
 * (ephemeral — see `lib/flows/canvasDraft.ts`); NOTHING is fetched or persisted
 * on open. The canvas's own Save button remains the single persistence gate
 * (it calls `flows_create` for a draft), so opening a draft never touches
 * `flows_create`/`flows_update`. If there's no draft in state (e.g. a hard
 * reload dropped it, or the route was hit directly), we show an empty state
 * rather than a broken canvas.
 */
export function FlowCanvasDraftPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const draft = useMemo(() => asFlowCanvasDraftState(location.state), [location.state]);

  // Non-fatal import warnings (Phase 4d) shown as dismissible toasts over the
  // draft canvas. Seeded once from the draft state so unmapped n8n node types /
  // untranslated expressions aren't silently lost on the way in.
  const [toasts, setToasts] = useState<ToastNotification[]>(() =>
    (draft?.importWarnings ?? []).map((message, i) => ({
      id: `import-warning-${i}`,
      type: 'warning',
      title: t('flows.import.warningTitle'),
      message,
    }))
  );
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(item => item.id !== id));
  }, []);

  if (draft) {
    return (
      <>
        <FlowEditor
          editorFlow={{
            flowId: null,
            name: draft.name,
            graph: draft.graph,
            requireApproval: draft.requireApproval,
          }}
          locationKey={location.key}
        />
        <ToastContainer notifications={toasts} onRemove={removeToast} />
      </>
    );
  }

  const backButton = (
    <Button
      type="button"
      variant="tertiary"
      size="xs"
      iconOnly
      data-testid="flow-canvas-back"
      aria-label={t('flows.canvas.backToList')}
      onClick={() => navigate('/flows')}>
      <BackIcon />
    </Button>
  );

  return (
    <PanelPage
      testId="flow-canvas-page"
      title={t('flows.canvas.title')}
      leading={backButton}
      contentClassName="h-full p-0">
      <div className="flex h-full items-center justify-center p-4">
        <p className="text-sm text-content-muted" data-testid="flow-canvas-draft-missing">
          {t('flows.canvas.draftMissing')}
        </p>
      </div>
    </PanelPage>
  );
}
