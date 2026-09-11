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
import type { Viewport } from '@xyflow/react';
import createDebug from 'debug';
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate, useParams } from 'react-router-dom';

import type {
  EditableFlowCanvasHandle,
  EditorSaveMeta,
} from '../components/flows/canvas/EditableFlowCanvas';
import FlowCanvas from '../components/flows/canvas/FlowCanvas';
import { FlowPreauthorizationOverlay } from '../components/flows/FlowPreauthorizationCard';
import FlowRunsSidebar from '../components/flows/FlowRunsSidebar';
import WorkflowCopilotPanel, {
  type RepairPromptContext,
} from '../components/flows/WorkflowCopilotPanel';
import {
  getCopilotThreadId,
  setCopilotThreadId as setCopilotThreadIdCache,
} from '../components/flows/workflowCopilotThreads';
import { ToastContainer } from '../components/intelligence/Toast';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  CenteredLoadingState,
  ConfirmDialog,
  ErrorBanner,
  ToggleGroupItem,
  ToggleGroupRoot,
} from '../components/ui';
import { useFlowPreauthorization } from '../hooks/useFlowPreauthorization';
import { asFlowCanvasDraftState } from '../lib/flows/canvasDraft';
import {
  normalizeWorkflowGraphForDirtyCheck,
  workflowGraphToXyflow,
} from '../lib/flows/graphAdapter';
import { buildPreviewGraph, diffGraphs } from '../lib/flows/graphDiff';
import type { WorkflowGraph } from '../lib/flows/types';
import { useT } from '../lib/i18n/I18nContext';
import {
  createFlow,
  type Flow,
  getFlow,
  runFlowDetached,
  updateFlow,
} from '../services/api/flowsApi';
import type { WorkflowProposal } from '../store/chatRuntimeSlice';
import type { ToastNotification } from '../types/intelligence';

/**
 * Seed for opening the canvas copilot preloaded from a failed run's "Fix with
 * agent" action (Phase 5c). Rides in `location.state` (ephemeral). The graph is
 * supplied by the editor itself, so only the run context travels here.
 */
interface CopilotRepairSeed {
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
interface CopilotBuildSeed {
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
interface CopilotPrefillSeed {
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
function asCopilotRepairSeed(state: unknown): CopilotRepairSeed | null {
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

/** How long the run-error banner stays up before it auto-dismisses. */
const RUN_ERROR_AUTO_DISMISS_MS = 12_000;

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
 * Run-start failures bubble up through several `thiserror`-wrapped tinyflows
 * layers, each re-tagging the one beneath it (e.g. "capability error: graph
 * error: capability error: code node exited non-zero (timed_out=false):").
 * The nesting is meaningless to the user — strip repeated "<word> error: "
 * wrapper prefixes so only the innermost, actionable tail is shown. Scoped
 * to the run-error banner only (`handleRun`'s catch); other error surfaces
 * in this file keep the raw message since their shapes differ. Pure and
 * exported for direct unit testing without rendering `FlowEditor`.
 */
export function formatRunError(message: string): string {
  const wrapperPrefix = /^\w+\s+error:\s*/i;
  let rest = message;
  let match = wrapperPrefix.exec(rest);
  while (match && match[0].length < rest.length) {
    rest = rest.slice(match[0].length);
    match = wrapperPrefix.exec(rest);
  }
  const trimmed = rest.replace(/:\s*$/, '').trim();
  return trimmed || message.trim();
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
      className="h-5 w-5"
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

/**
 * The canvas header's Back control. One component rather than the three
 * byte-identical copies this file used to carry (the editor, the load-state
 * page and the draft page each declared their own `backButton`), which is how
 * the three drifted apart in the first place.
 */
function CanvasBackButton({ onBack }: { onBack: () => void }) {
  const { t } = useT();
  return (
    <Button
      type="button"
      variant="tertiary"
      size="sm"
      iconOnly
      data-testid="flow-canvas-back"
      aria-label={t('flows.canvas.backToList')}
      // The header row is `items-start`, so a control shorter than the heading
      // sits high against it rather than centred. `text-2xl`'s line box is
      // exactly 2rem, so an `h-8` button fills it and lands on the title's
      // optical centre; `w-8` keeps it square. The default `size="sm"` metrics
      // (30px) miss by 2px, which is enough to read as misaligned next to a
      // 24px heading.
      className="h-8 w-8"
      onClick={onBack}>
      <BackIcon />
    </Button>
  );
}

/**
 * The chrome the canvas shows when there is no graph to edit — loading, load
 * error, not found, or a draft whose ephemeral `location.state` is gone. All
 * four rendered their own `PanelPage` with the same title/leading/body classes;
 * the only thing that differed was the message in the middle. They share the
 * standard page shell now, so a canvas that failed to load reads as the same
 * page as one that loaded.
 */
function CanvasStatePage({ onBack, children }: { onBack: () => void; children: ReactNode }) {
  const { t } = useT();
  return (
    <div className="h-full p-4" data-testid="flow-canvas-page">
      <SettingsTabbedPage
        title={t('flows.canvas.title')}
        description={t('flows.canvas.description')}
        leading={<CanvasBackButton onBack={onBack} />}
        scrollable={false}>
        {children}
      </SettingsTabbedPage>
    </div>
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
  // F4/F5 fix: the editable canvas is remounted on every Save/Accept/Reject
  // (keyed on `canvasVersion` below), which previously wiped BOTH the
  // pan/zoom viewport (React Flow's `fitView` refits on every mount) and the
  // undo history. This ref lives on `FlowEditor` — keyed on `flow.id`, not
  // `canvasVersion` — so it survives those remounts and can seed the fresh
  // canvas's `defaultViewport`, skipping `fitView` and preserving pan/zoom
  // across a Save. (The undo-history wipe is a known, accepted limitation —
  // Save/Accept are commit points, so resetting undo there is semantically
  // fine; see the PR description.)
  const viewportRef = useRef<Viewport | null>(null);
  const handleViewportChange = useCallback((vp: Viewport) => {
    viewportRef.current = vp;
    // Grep-friendly, numeric-only (no PII) — fires on every pan/zoom, so kept
    // to plain numbers rather than the full `Viewport` object.
    log('viewport capture: x=%d y=%d zoom=%d', vp.x, vp.y, vp.zoom);
  }, []);
  const [saveMeta, setSaveMeta] = useState<EditorSaveMeta>({
    dirty: false,
    hasErrors: false,
    saving: false,
  });
  const [leaveConfirm, setLeaveConfirm] = useState(false);
  // Which header action (run/save) is awaiting confirmation, if any —
  // every icon click opens a confirm popup before it fires.
  const [confirmAction, setConfirmAction] = useState<'run' | 'save' | null>(null);
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
  //
  // F-m2 fix: `chatFirst` itself must ALSO latch at mount, not re-derive from
  // the live `initialBuildSeed` prop every render. `onBuildSeedConsumed`
  // (`clearBuildSeed` in the parent) strips `copilotBuild` from
  // `location.state` once the copilot has dispatched the seeded build turn —
  // deliberately, so a later remount doesn't re-fire it. But if that turn's
  // FIRST reply was a clarifying question (no proposal yet, so `graphRevealed`
  // is still false), re-deriving `chatFirst` from the now-cleared prop flipped
  // it to `false` mid-conversation, which un-hid the blank trigger-only canvas
  // while the user was still answering the agent's question. Reading
  // `initialBuildSeed` only inside a `useState` initializer freezes the value
  // as of first mount, immune to the prop later going away.
  const [chatFirst] = useState(() => initialBuildSeed?.chatFirst === true);
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
  // `next`) always; only bump `canvasVersion` — remounting the editable canvas
  // to re-seed from the canonical persisted graph — when that response
  // actually DIFFERS from what was sent (F4/F5 fix: an unconditional bump
  // here wiped the canvas's undo history and viewport on every Save even when
  // the server echoed the graph back unchanged, and double-remounted on
  // Accept, which already bumps once for its own preview→draft transition).
  //
  // Declared ahead of `handleAcceptProposal` (below), which calls it directly
  // to persist an accepted proposal immediately.
  //
  // Returns `flowId`/`flowEnabled` alongside `remounted` so a caller wanting a
  // "Save & enable" follow-up (`handleAcceptProposal`'s `opts.enable`) knows
  // exactly which flow id to arm and whether the persisted flow already came
  // back enabled — without having to re-derive it from component state, which
  // is especially important for the draft-create path: `flowId` (the prop)
  // is still `null` in THIS closure even after `createFlow` resolves, since
  // the draft only becomes a real flow id via the `navigate(...)` below, not
  // a state update this same render can observe.
  const handleSave = useCallback(
    async (
      next: WorkflowGraph,
      overrideName?: string,
      overrideRequireApproval?: boolean,
      // When true, a draft-create does NOT navigate to `/flows/:id` itself —
      // the caller owns navigation timing. `handleAcceptProposal`'s
      // "Save & enable" needs this: it must run `setFlowEnabled` on the
      // just-created flow BEFORE the route change unmounts this page, else the
      // enable RPC resolves against an unmounted component (its loading/error
      // state is lost and the new page shows the flow still disabled). The
      // `wasDraft` flag in the return tells the caller navigation is now its
      // responsibility.
      deferDraftNavigation?: boolean
    ): Promise<{ remounted: boolean; flowId: string; flowEnabled: boolean; wasDraft: boolean }> => {
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
        log('save: draft persisted as flow id=%s enabled=%s', created.id, created.enabled);
        if (!deferDraftNavigation) {
          navigate(`/flows/${created.id}`, { replace: true });
        }
        // Navigating replaces this whole page (new `flowId` route param), so
        // "remounted" is moot for a draft-create — no caller branches on it.
        // `flowId`/`flowEnabled` DO matter — a "Save & enable" caller reads
        // them to arm the just-created flow (B29 Rule 1 always persists an
        // automatic-trigger draft disabled, regardless of the caller's
        // intent), and this RPC response is the only place that id/enabled
        // pair is available before the route change lands. `wasDraft` lets a
        // `deferDraftNavigation` caller know it now owns the navigation.
        return {
          remounted: false,
          flowId: created.id,
          flowEnabled: created.enabled,
          wasDraft: true,
        };
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
      // `persistedGraphRef`/`setDraftGraph` run UNCONDITIONALLY regardless of
      // whether a remount fires below — the dirty diff (`initialDirty`, B21)
      // is computed against `persistedGraphRef`, not the canvas's own mount
      // baseline, so it stays correct either way. Only the remount is gated.
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
      // F4/F5 fix: only remount the canvas (wiping its undo history and,
      // pre-Fix-A, its viewport) when the server actually normalized the
      // graph into something different from what was sent (issue B21 — schema
      // migration, id defaults, port normalization, etc.). When the response
      // is a byte-for-byte echo of `next` (the common case), re-seeding from
      // it would be a no-op remount — most visibly on Accept, whose own
      // preview→draft transition already bumps `canvasVersion` once, so a
      // guaranteed-normalized-away Save bump right after it was a pure
      // double-remount (undo wipe + dirty flash) with no behavioral upside.
      const graphChanged = JSON.stringify(persisted) !== JSON.stringify(next);
      if (graphChanged) {
        setCanvasVersion(v => v + 1);
      }
      log(
        'save: flow id=%s persisted — canvas re-synced from response nodes=%d edges=%d graphChanged=%s enabled=%s',
        flowId,
        persisted.nodes.length,
        persisted.edges.length,
        graphChanged,
        updated.enabled
      );
      return { remounted: graphChanged, flowId, flowEnabled: updated.enabled, wasDraft: false };
    },
    [isDraft, flowId, name, requireApproval, navigate]
  );

  // Deferred draft navigation while the pre-authorization card is open — the
  // `/flows/:id` route change would unmount the card mid-decision, so a
  // draft-save that surfaces the card parks its navigation here and fires it
  // from the hook's `onSettled` once the user decided either way.
  const preauthNavRef = useRef<string | null>(null);
  const preauth = useFlowPreauthorization({
    onSettled: useCallback(
      (outcome: 'no-card' | 'approved' | 'denied', settledFlowId: string) => {
        if (preauthNavRef.current === settledFlowId) {
          preauthNavRef.current = null;
          log(
            'preauth settled outcome=%s id=%s — firing deferred draft navigation',
            outcome,
            settledFlowId
          );
          navigate(`/flows/${settledFlowId}`, { replace: true });
        } else {
          log('preauth settled outcome=%s id=%s — no deferred navigation', outcome, settledFlowId);
        }
      },
      [navigate]
    ),
  });

  // Adapter for the canvas's own `onSave` prop, whose type (`void |
  // Promise<void>`) is shared with the read-only viewer and every other
  // consumer — `handleSave`'s richer `{ remounted }` return (needed by
  // `handleAcceptProposal` below) isn't part of that contract.
  //
  // After a successful persist this also runs the save+enable
  // pre-authorization check: a flow that is (or came back) enabled with
  // missing permission grants surfaces the consolidated Approve-all/Deny
  // card. For a draft-create the navigation to `/flows/:id` is deferred
  // until the card settles (see `preauthNavRef`).
  const onCanvasSave = useCallback(
    async (next: WorkflowGraph) => {
      const result = await handleSave(next, undefined, undefined, true);
      if (result.wasDraft) {
        let cardShown = false;
        try {
          cardShown = await preauth.checkAfterSave(result.flowId, result.flowEnabled);
        } catch (err) {
          // The flow is already persisted — on failure we must still
          // navigate, or `isDraft` stays true and a Save retry would call
          // `createFlow` again, duplicating the flow (same guard as
          // `handleAcceptProposal`'s enable arm).
          log('save: pre-authorization check failed id=%s err=%o', result.flowId, err);
        }
        if (cardShown) {
          log('save: preauth card shown id=%s — deferring draft navigation', result.flowId);
          preauthNavRef.current = result.flowId;
        } else {
          log('save: no preauth card id=%s — navigating to flow route', result.flowId);
          navigate(`/flows/${result.flowId}`, { replace: true });
        }
      } else {
        preauth
          .checkAfterSave(result.flowId, result.flowEnabled)
          .catch(err =>
            log('save: pre-authorization check failed id=%s err=%o', result.flowId, err)
          );
      }
    },
    [handleSave, preauth, navigate]
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
  //
  // `opts.enable` (PR1 — "Save & enable") mirrors `WorkflowProposalCard.save()`
  // in the main chat surface: after a successful save, explicitly arm the
  // flow via `setFlowEnabled`. This is needed because `createFlow` with an
  // automatic trigger (schedule/app_event/webhook) ALWAYS persists disabled
  // (B29 Rule 1, `flowsApi.ts`) regardless of what the caller passed — Rule 1
  // exists to stop a copilot autosave from silently arming an unattended
  // automation, but "Save & enable" is the user's own explicit arming click,
  // not a silent autosave, so it must follow up. Plain "Accept & save" (no
  // `opts`) must NOT enable and must NOT force-disable an already-enabled
  // existing flow — it's simply omitted from the call.
  const handleAcceptProposal = useCallback(
    async (proposal: WorkflowProposal, opts?: { enable?: boolean }) => {
      log('copilot proposal accepted: enable=%s', Boolean(opts?.enable));
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
        const {
          remounted,
          flowId: savedFlowId,
          flowEnabled,
          wasDraft,
        } = await handleSave(
          proposedGraph,
          overrideName,
          proposal.requireApproval,
          // Defer a draft-create's navigation so a "Save & enable" arms the
          // flow BEFORE this page unmounts — see `deferDraftNavigation`.
          true
        );
        // The canvas remounted once already (this handler's own bump above)
        // with `forcedDirty` seeded `true` — correct pre-persist, but that
        // instance's `forcedDirty` is only ever cleared by ITS OWN save()/
        // discard(), neither of which fires here (we persisted directly,
        // above). A second remount (`remounted === true`, server actually
        // normalized the graph) reseeds a fresh instance with the now-correct
        // `initialDirty`, so nothing else to do. Otherwise (the common
        // echoed-back-unchanged case) explicitly sync the still-current
        // instance so it doesn't read dirty forever (see
        // `EditableFlowCanvasHandle.clearForcedDirty`'s doc comment).
        if (!remounted) {
          canvasRef.current?.clearForcedDirty();
        }
        log(
          'copilot proposal accepted: persisted remounted=%s flowId=%s flowEnabled=%s',
          remounted,
          savedFlowId,
          flowEnabled
        );

        // "Save & enable": follow up with an explicit arm, same as
        // `WorkflowProposalCard.save()`. Fires unconditionally when
        // requested (idempotent if the flow already came back enabled) —
        // simpler than special-casing an already-enabled flow, and this is
        // still inside the same try/catch so a failure here also leaves the
        // proposal visible for retry rather than silently vanishing.
        let preauthCardShown = false;
        if (opts?.enable) {
          log('copilot proposal accepted: enabling flow id=%s', savedFlowId);
          try {
            // Routes through the pre-authorization check: enables directly
            // when no grants are missing, otherwise surfaces the consolidated
            // Approve-all/Deny card and defers the enable to "Approve all".
            const enabledNow = await preauth.beginEnable(savedFlowId);
            preauthCardShown = !enabledNow;
            log(
              'copilot proposal accepted: enable settled id=%s cardShown=%s',
              savedFlowId,
              preauthCardShown
            );
          } catch (enableErr) {
            // The flow IS saved at this point. On a DRAFT we must still
            // navigate to the created flow (below) or a retry would create a
            // duplicate — so we can't keep the proposal for an in-place retry;
            // swallow here and let the user arm it from the flow page (matches
            // the "Saved, but could not enable" guidance). On an EXISTING flow
            // there's no navigation, so rethrow to keep the proposal visible
            // for retry, preserving the pre-existing behavior.
            if (!wasDraft) throw enableErr;
            log(
              'copilot proposal accepted: enable failed on draft; flow saved-but-disabled id=%s err=%o',
              savedFlowId,
              enableErr
            );
          }
        }

        // Draft navigation was deferred so the "Save & enable" arm could run
        // first; now that persist + enable have settled, move to the real flow
        // route. A non-draft accept stays on its existing `/flows/:id` page.
        // When the pre-authorization card is open, navigation is parked until
        // the user decides (the route change would unmount the card).
        if (wasDraft) {
          if (preauthCardShown) {
            preauthNavRef.current = savedFlowId;
          } else {
            navigate(`/flows/${savedFlowId}`, { replace: true });
          }
        }
      } catch (err) {
        log('copilot proposal accepted: save/enable failed err=%o', err);
        // Rethrow: the draft above is already applied unconditionally, so no
        // data is lost by rethrowing. This lets the caller — the copilot
        // panel's own `accept`/`acceptAndEnable` handler — see the failure
        // and skip `clearProposal()`, keeping the proposal card visible for
        // retry instead of silently vanishing while nothing was actually
        // saved (or saved-but-not-enabled). `acceptSaving`/`acceptState`
        // there still resets via its own `finally`.
        throw err;
      }
    },
    [titleDraft, renaming, t, isDraft, handleSave, preauth, navigate]
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
  //
  // F-m3 fix: normalize BOTH sides through the same `workflowGraphToXyflow` /
  // `xyflowToWorkflowGraph` round-trip the canvas itself performs (see
  // `normalizeWorkflowGraphForDirtyCheck`'s doc comment) before comparing,
  // rather than diffing `editorGraph` against the raw, possibly
  // position-less `persistedGraphRef.current` directly. A graph saved
  // without per-node `position` (e.g. agent-authored) otherwise reads as
  // dirty the instant a REMOUNTED canvas instance (e.g. after a copilot
  // Reject) reports its now-positioned `editorGraph` back — even with zero
  // real edits — because `persistedGraphRef.current` was never updated to
  // match. Normalizing here (rather than pre-normalizing the ref at seed
  // time) keeps this correct on the very FIRST mount too, where `editorGraph`
  // is still raw and `persistedGraphRef.current` must compare equal to it
  // before any canvas effect has run.
  const initialDirty = useMemo(
    () =>
      JSON.stringify(normalizeWorkflowGraphForDirtyCheck(editorGraph, meta)) !==
        JSON.stringify(normalizeWorkflowGraphForDirtyCheck(persistedGraphRef.current, meta)) ||
      name !== persistedNameRef.current,
    [editorGraph, name, meta]
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

  // Run the *persisted* flow and hand its run id to the canvas so it can
  // overlay live per-node status (Phase 3e). Runs the saved version — not the
  // (possibly dirty) draft — matching the "Save is explicit, running is live"
  // model. The durable run row + poller remain the source of truth.
  //
  // F-M1 fix: uses `runFlowDetached` (`openhuman.flows_run_detached`), which
  // registers the run and returns its id immediately, INSTEAD of the old
  // `runFlow` (`openhuman.flows_run`), which blocked until the run reached a
  // terminal status — by the time that resolved, every `flow:run_progress`
  // event for the run had already fired and been dropped, because
  // `useFlowRunProgress` only subscribes once `activeRunId` is set (see its
  // doc comment). `setActiveRunId` below now runs BEFORE the engine has
  // executed a single node, so the subscription is live for the whole run.
  const handleRun = useCallback(async () => {
    if (flowId === null) return; // drafts aren't runnable until saved
    setRunning(true);
    setRunError(null);
    try {
      log('run: starting (detached) flow id=%s', flowId);
      const result = await runFlowDetached(flowId);
      log('run: started (detached) flow id=%s run_id=%s', flowId, result.run_id);
      setActiveRunId(result.run_id);
    } catch (err) {
      const message = errorMessage(err);
      log('run: failed id=%s err=%o', flowId, err);
      setRunError(message);
    } finally {
      setRunning(false);
    }
  }, [flowId]);

  // Auto-dismiss the run-error banner so a stale failure doesn't linger
  // forever over the canvas. Re-runs (and thus restarts the timer) whenever
  // `runError` changes — including a new failure replacing an old one, since
  // `handleRun` always routes through `setRunError(null)` before setting the
  // next message, so a later error is always a distinct effect run.
  useEffect(() => {
    if (!runError) return;
    const timer = setTimeout(() => setRunError(null), RUN_ERROR_AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [runError]);

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

  const backButton = <CanvasBackButton onBack={handleBack} />;

  // A draft has nothing persisted to run yet — the canvas's Save (which creates
  // the flow) is the only gate, so no Run affordance until it's saved.
  const runButton = isDraft ? undefined : (
    <Button
      type="button"
      variant="primary"
      size="sm"
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
    <ToggleGroupRoot
      type="single"
      variant="secondary"
      size="sm"
      value={sidePanel ?? ''}
      onValueChange={next => setSidePanel(next === 'copilot' || next === 'legend' ? next : null)}
      aria-label={t('flows.canvas.sidePanelToggle')}
      className="rounded-lg border border-line bg-surface p-0.5">
      <ToggleGroupItem
        value="copilot"
        data-testid="flow-canvas-copilot-toggle"
        className="border-0">
        {t('flows.copilot.open')}
      </ToggleGroupItem>
      <ToggleGroupItem value="legend" data-testid="flow-canvas-legend-toggle" className="border-0">
        {t('flows.canvas.legendTab')}
      </ToggleGroupItem>
    </ToggleGroupRoot>
  );

  // Save lives in the header (the canvas keeps only undo/redo), as an icon
  // button behind a confirm popup, driving the editable canvas through
  // `canvasRef`.
  //
  // Discard was beside it and is gone. Its ✕ glyph read as "close" next to a
  // back button that also leaves, so it competed with navigation while
  // actually meaning "revert to the last saved graph". Dropping edits now goes
  // through Back, whose dirty guard already offers exactly that choice.
  // `EditableFlowCanvasHandle.discard()` stays — it is the canvas's own API and
  // its specs still cover it; nothing in this page calls it any more.
  const saveActions = (
    <div className="flex items-center gap-1.5">
      {saveMeta.dirty && (
        <Badge variant="warning" className="rounded-full" data-testid="flow-editor-dirty">
          {t('flows.editor.unsaved')}
        </Badge>
      )}
      <Button
        type="button"
        variant="primary"
        size="sm"
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

  // Editable title: the page's `h1` IS this input, so renaming is discoverable
  // without a separate edit affordance.
  //
  // Deliberately a raw `<input>` rather than the `ui/Input` primitive. That
  // component joins its class list with `[...].join(' ')` — no `cn`, no
  // tailwind-merge — so `inputSize="sm"`'s `h-8 px-2.5 text-sm` stays in the
  // attribute alongside anything a caller passes, and which one applies is
  // decided by Tailwind's stylesheet order rather than by the caller. That is
  // how this rendered as a small, inset control instead of a page heading. A
  // control that has to inherit its host's typography has no business going
  // through a sized primitive at all.
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
      // `[font:inherit]` + `text-inherit` take the whole type ramp from the
      // enclosing `h1` — family, size, weight, line-height, colour — so this
      // cannot drift from `SettingsTabbedPage`'s heading the way a restated
      // `text-2xl font-semibold` would the next time that heading changes.
      // `p-0` puts the text on the same left edge a static title would sit on;
      // `-mx-1 px-1` gives the hover/focus highlight some room back without
      // moving the text, since a padding-only affordance would inset it.
      className="-mx-1 w-full max-w-lg truncate rounded-md border-0 bg-transparent p-0 px-1 text-inherit [font:inherit] hover:bg-surface-hover focus:bg-surface-hover focus:outline-hidden focus:ring-2 focus:ring-primary-500/30 disabled:opacity-50"
    />
  );

  return (
    // The standard page shell, with `scrollable={false}` so the canvas owns its
    // own viewport instead of living inside a scroller. This was a `PanelPage`
    // with a `text-base` title and a hairline; every other page in the app
    // opens with `SettingsTabbedPage`'s 2xl heading over a full-bleed divider,
    // and there was no reason for the builder to be the exception once it was
    // back in the shell.
    <div className="h-full p-4" data-testid="flow-canvas-page">
      {/* Run history + "Fix with agent" go through the shell's dynamic sidebar
          region, like every other page's rail. This used to be an in-page
          `hidden lg:flex w-60 border-r` column because the route was chromeless
          and had no slot to project into — so the builder drew a second sidebar
          in the space the real one would have occupied, and it only existed at
          `lg` and up. Drafts have no runs yet, so they project nothing and the
          region stays empty. */}
      {!isDraft && flowId && (
        <SidebarContent>
          <div className="h-full overflow-hidden">
            <FlowRunsSidebar flowId={flowId} />
          </div>
        </SidebarContent>
      )}
      <SettingsTabbedPage
        title={titleNode}
        description={t('flows.canvas.description')}
        leading={backButton}
        headerAction={headerActions}
        scrollable={false}
        // The canvas is a single full-bleed surface, so it runs to the content
        // card's edges instead of floating as an inset rectangle inside it.
        // The header keeps the gutter, so the title still lines up with every
        // other page's.
        bodyFullBleed>
        <div className="flex h-full w-full">
          <div className={`relative h-full flex-1 ${hideGraph ? 'hidden' : ''}`}>
            <FlowCanvas
              key={`canvas-${canvasVersion}`}
              ref={canvasRef}
              editable
              nodes={nodes}
              edges={edges}
              meta={meta}
              onSave={onCanvasSave}
              onDirtyChange={setDirty}
              onSaveMetaChange={setSaveMeta}
              activeRunId={activeRunId}
              onGraphChange={handleGraphChange}
              addedNodeIds={preview?.addedNodeIds}
              removedNodeIds={preview?.removedNodeIds}
              saveDisabled={preview !== null}
              initialDirty={initialDirty}
              showPalette={sidePanel === 'legend'}
              savedViewport={viewportRef.current}
              onViewportChange={handleViewportChange}
            />

            {runError && (
              // top-14 (not top-3) so this never overlaps the canvas's own
              // top-right undo/redo controls, which sit at top-3; max-w-md
              // caps how wide a long nested error can grow. When the legend
              // palette is open it also docks at top-14/right-3 (w-48), so we
              // pull the banner's right edge in past it (right-56) rather than
              // centering across the full width, which would let a long
              // message reach under the palette and swallow its clicks.
              <div
                className={`pointer-events-none absolute left-3 top-14 z-20 flex justify-center ${
                  sidePanel === 'legend' ? 'right-56' : 'right-3'
                }`}>
                <Alert
                  variant="destructive"
                  density="compact"
                  data-testid="flow-canvas-run-error"
                  className="pointer-events-auto w-full max-w-md items-start gap-2">
                  <AlertDescription className="flex-1">
                    {t('flows.editor.runFailed')}: {formatRunError(runError)}
                  </AlertDescription>
                  <Button
                    type="button"
                    variant="tertiary"
                    size="xs"
                    iconOnly
                    onClick={() => setRunError(null)}
                    aria-label={t('common.dismiss')}
                    title={t('common.dismiss')}
                    data-testid="flow-canvas-run-error-dismiss"
                    className="shrink-0 text-coral-500 hover:bg-transparent hover:text-coral-700 dark:text-coral-300 dark:hover:text-coral-100">
                    <svg
                      className="h-3.5 w-3.5"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M6 18L18 6M6 6l12 12"
                      />
                    </svg>
                  </Button>
                </Alert>
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

          {/* Both confirms are `ConfirmDialog` now. They were two hand-rolled
            overlays — an `absolute`/`fixed inset-0` scrim over a `max-w-sm`
            card — that between them reimplemented the shell twice and had no
            focus trap, no scroll lock and no focus restore. `ConfirmDialog`'s
            own spec already reserved these exact test ids for the migration. */}
          {leaveConfirm && (
            <ConfirmDialog
              testId="flow-leave-confirm"
              titleId="flow-leave-confirm-title"
              title={t('flows.editor.leaveTitle')}
              body={t('flows.editor.leaveBody')}
              destructive
              cancelLabel={t('flows.editor.leaveStay')}
              cancelTestId="flow-leave-stay"
              confirmLabel={t('flows.editor.leaveDiscard')}
              confirmTestId="flow-leave-discard"
              onCancel={() => setLeaveConfirm(false)}
              onConfirm={() => {
                log('back: confirmed leave — discarding unsaved edits');
                goBack();
              }}
            />
          )}

          {/* Confirm popup for the header's Run / Save icon buttons. */}
          {confirmAction && (
            <ConfirmDialog
              testId="flow-action-confirm"
              titleId="flow-action-confirm-title"
              title={t(`flows.editor.confirm.${confirmAction}Title`)}
              body={t(`flows.editor.confirm.${confirmAction}Body`)}
              cancelLabel={t('flows.editor.confirm.cancel')}
              cancelTestId="flow-action-cancel"
              confirmLabel={t('flows.editor.confirm.confirm')}
              confirmTestId="flow-action-confirm-accept"
              onCancel={() => setConfirmAction(null)}
              onConfirm={() => {
                const action = confirmAction;
                setConfirmAction(null);
                if (action === 'run') void handleRun();
                else if (action === 'save') canvasRef.current?.save();
              }}
            />
          )}

          {/* Consolidated save+enable pre-authorization card (Approve all / Deny). */}
          {preauth.pending && (
            <FlowPreauthorizationOverlay
              entries={preauth.pending.manifest.entries}
              busy={preauth.busy}
              errorMsg={preauth.errorKey ? t('flows.enableApproval.error') : null}
              onApproveAll={() => void preauth.approveAll()}
              onDeny={() => void preauth.deny()}
            />
          )}
        </div>
      </SettingsTabbedPage>
    </div>
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

  return (
    <CanvasStatePage onBack={() => navigate('/flows')}>
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
    </CanvasStatePage>
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

  return (
    <CanvasStatePage onBack={() => navigate('/flows')}>
      <div className="flex h-full items-center justify-center p-4">
        <p className="text-sm text-content-muted" data-testid="flow-canvas-draft-missing">
          {t('flows.canvas.draftMissing')}
        </p>
      </div>
    </CanvasStatePage>
  );
}
