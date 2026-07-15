/**
 * FlowCanvasPage (issue B5b / Phase 3) — the editable Workflow Canvas builder
 * at `/flows/:id`. Asserts the loading → canvas happy path, the not-found state
 * (mirrors the Rust `flows_get` "not found" error), the generic error state,
 * and the Phase 3d host wiring: Save persists via `flows_update`, and the
 * unsaved-changes guard intercepts the Back button while dirty.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { createMemoryRouter, MemoryRouter, Route, RouterProvider, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Flow } from '../../services/api/flowsApi';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import FlowCanvasPage, {
  asCopilotBuildSeed,
  asCopilotPrefillSeed,
  FlowCanvasDraftPage,
  isPlaceholderTitle,
} from '../FlowCanvasPage';

const getFlow = vi.hoisted(() => vi.fn());
const updateFlow = vi.hoisted(() => vi.fn());
const createFlow = vi.hoisted(() => vi.fn());
const validateFlow = vi.hoisted(() => vi.fn());
const listFlowConnections = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({
  getFlow,
  updateFlow,
  createFlow,
  validateFlow,
  listFlowConnections,
}));

// Stub the copilot panel: it drives the real chat runtime (redux + socket),
// which is out of scope here — we only assert the host opens it and hands the
// right seed through.
const copilotPanelProps = vi.hoisted(() => ({ current: null as Record<string, unknown> | null }));
vi.mock('../../components/flows/WorkflowCopilotPanel', () => ({
  default: (props: Record<string, unknown>) => {
    copilotPanelProps.current = props;
    return <div data-testid="stub-copilot-panel" />;
  },
}));

// The page auto-collapses the app sidebar via `useRootSidebar` (redux-backed);
// this test renders without a Provider, so stub the hook to no-ops.
vi.mock('../../components/layout/shell/RootShellLayout', () => ({
  useRootSidebar: () => ({ visible: true, toggle: () => {}, show: () => {}, hide: () => {} }),
}));

function makeFlow(overrides: Partial<Flow> = {}): Flow {
  return {
    id: 'test-id',
    name: 'Daily digest',
    enabled: true,
    graph: {
      schema_version: 1,
      id: 'test-id',
      name: 'Daily digest',
      nodes: [
        {
          id: 't',
          kind: 'trigger',
          name: 'Start',
          config: {},
          ports: [],
          position: { x: 0, y: 0 },
        },
      ],
      edges: [],
    },
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    last_run_at: null,
    last_status: null,
    require_approval: false,
    ...overrides,
  };
}

function renderAtFlowId(id: string) {
  return render(
    <MemoryRouter initialEntries={[`/flows/${id}`]}>
      <Routes>
        <Route path="/flows/:id" element={<FlowCanvasPage />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('FlowCanvasPage', () => {
  beforeEach(() => {
    getFlow.mockReset();
    updateFlow.mockReset();
    createFlow.mockReset();
    validateFlow.mockReset();
    listFlowConnections.mockReset();
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    listFlowConnections.mockResolvedValue([]);
    updateFlow.mockResolvedValue(makeFlow());
    createFlow.mockResolvedValue(makeFlow({ id: 'created-id', name: 'Daily digest' }));
  });

  it('shows a loading state while the flow is being fetched', () => {
    getFlow.mockReturnValue(new Promise(() => {})); // never resolves
    renderAtFlowId('test-id');

    expect(screen.getByText('Loading workflow…')).toBeInTheDocument();
  });

  it('loads the flow and renders the canvas with the flow name as the title', async () => {
    getFlow.mockResolvedValue(makeFlow());
    renderAtFlowId('test-id');

    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());
    expect(getFlow).toHaveBeenCalledWith('test-id');
    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('Daily digest');
  });

  it('renames a persisted flow via the editable title (metadata-only update)', async () => {
    getFlow.mockResolvedValue(makeFlow());
    updateFlow.mockResolvedValue(makeFlow({ name: 'Renamed' }));
    renderAtFlowId('test-id');

    const title = await screen.findByTestId('flow-canvas-title');
    fireEvent.change(title, { target: { value: 'Renamed' } });
    fireEvent.blur(title);

    await waitFor(() => expect(updateFlow).toHaveBeenCalledWith('test-id', { name: 'Renamed' }));
    // Name-only update — no graph in the payload, so it can't fire a schedule.
    expect(updateFlow.mock.calls[0][1]).not.toHaveProperty('graph');
  });

  it('shows a not-found state when the flow does not exist', async () => {
    getFlow.mockRejectedValue(new Error("flow 'missing-id' not found"));
    renderAtFlowId('missing-id');

    await waitFor(() => expect(screen.getByTestId('flow-canvas-not-found')).toBeInTheDocument());
  });

  it('shows an error state for any other failure', async () => {
    getFlow.mockRejectedValue(new Error('core unreachable'));
    renderAtFlowId('test-id');

    await waitFor(() => expect(screen.getByTestId('flow-canvas-error')).toBeInTheDocument());
    expect(screen.getByText('core unreachable')).toBeInTheDocument();
  });

  it('ignores a stale response for a superseded id after navigating to a new one', async () => {
    // Deferred promises so the test controls resolution order precisely: the
    // first (old-id) fetch resolves AFTER the second (new-id) one, mimicking
    // a slow response for a page the user has since navigated away from.
    let resolveFirst!: (flow: Flow) => void;
    const firstFetch = new Promise<Flow>(resolve => {
      resolveFirst = resolve;
    });
    getFlow.mockImplementation((id: string) =>
      id === 'old-id' ? firstFetch : Promise.resolve(makeFlow({ id: 'new-id', name: 'New flow' }))
    );

    const router = createMemoryRouter([{ path: '/flows/:id', element: <FlowCanvasPage /> }], {
      initialEntries: ['/flows/old-id'],
    });
    render(<RouterProvider router={router} />);

    // Navigate away before the old id's fetch resolves.
    router.navigate('/flows/new-id');
    await waitFor(() => expect(screen.getByTestId('flow-canvas-title')).toHaveValue('New flow'));

    // Now let the stale old-id fetch resolve — it must not clobber the
    // already-rendered new-id state.
    resolveFirst(makeFlow({ id: 'old-id', name: 'Old flow (stale)' }));
    await Promise.resolve();
    await Promise.resolve();

    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('New flow');
    expect(screen.queryByDisplayValue('Old flow (stale)')).not.toBeInTheDocument();
  });

  function renderEditor(id = 'test-id') {
    return render(
      <MemoryRouter initialEntries={[`/flows/${id}`]}>
        <Routes>
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
          <Route path="/flows" element={<div data-testid="flows-list">Flows list</div>} />
        </Routes>
      </MemoryRouter>
    );
  }

  it('persists the live graph via flows_update when Save is clicked', async () => {
    getFlow.mockResolvedValue(makeFlow());
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    // Edit the graph (add a node) so it is dirty, then Save.
    // The node palette is the "Manual" tab — hidden by default now the Copilot
    // shows — so switch to it before adding a node.
    fireEvent.click(screen.getByTestId('flow-canvas-legend-toggle'));
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    fireEvent.click(screen.getByTestId('flow-editor-save'));
    // Save (and Run/Discard) open a confirm popup before firing.
    fireEvent.click(screen.getByTestId('flow-action-confirm-accept'));

    await waitFor(() => expect(updateFlow).toHaveBeenCalledTimes(1));
    const [calledId, update] = updateFlow.mock.calls[0];
    expect(calledId).toBe('test-id');
    expect(update.graph.nodes.map((n: { kind: string }) => n.kind).sort()).toEqual([
      'agent',
      'trigger',
    ]);
  });

  // Issue B21: `flows_update` re-validates/normalizes the graph server-side
  // before persisting, so the canonical response can legitimately differ from
  // what the client sent (schema migration, id defaults, etc.). Previously the
  // canvas re-baselined against its OWN pre-save nodes/edges and ignored the
  // response entirely — the canonical shape only ever appeared after a
  // navigate-away-and-back remount refetched it via `flows_get`. Assert the
  // canvas now reflects the SAVE RESPONSE's graph immediately, with no
  // navigation and no remount of `FlowCanvasPage`.
  it('re-syncs the canvas from the flows_update response on save, without a remount (B21)', async () => {
    getFlow.mockResolvedValue(makeFlow());
    // The server "normalizes" the saved graph: it accepts the client's
    // trigger+agent nodes but also injects a third node the client never
    // added (standing in for a server-side migration/default-fill), and
    // renames the trigger. A stale canvas would keep showing only the two
    // client-added nodes named "Start"/"New agent".
    updateFlow.mockResolvedValue(
      makeFlow({
        graph: {
          schema_version: 1,
          id: 'test-id',
          name: 'Daily digest',
          nodes: [
            {
              id: 't',
              kind: 'trigger',
              name: 'Start (normalized)',
              config: {},
              ports: [],
              position: { x: 0, y: 0 },
            },
            {
              id: 'new-agent-0',
              kind: 'agent',
              name: 'New agent',
              config: {},
              ports: [],
              position: { x: 80, y: 80 },
            },
            {
              id: 'server-added',
              kind: 'transform',
              name: 'Server-added node',
              config: {},
              ports: [],
              position: { x: 160, y: 160 },
            },
          ],
          edges: [],
        },
      })
    );
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    // The node palette is the "Manual" tab — hidden by default now the Copilot
    // shows — so switch to it before adding a node.
    fireEvent.click(screen.getByTestId('flow-canvas-legend-toggle'));
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    expect(screen.getAllByTestId('flow-node')).toHaveLength(2);

    fireEvent.click(screen.getByTestId('flow-editor-save'));
    // Save (and Run/Discard) open a confirm popup before firing.
    fireEvent.click(screen.getByTestId('flow-action-confirm-accept'));
    await waitFor(() => expect(updateFlow).toHaveBeenCalledTimes(1));

    // The canvas now shows the RESPONSE's three nodes (including the one the
    // client never added and the renamed trigger) — no navigation, no
    // `flows_get` refetch, no remount required.
    await waitFor(() => expect(screen.getAllByTestId('flow-node')).toHaveLength(3));
    expect(screen.getByText('Start (normalized)')).toBeInTheDocument();
    expect(screen.getByText('Server-added node')).toBeInTheDocument();
    // Still the same page/component — proving this wasn't a navigate-away
    // remount refetch in disguise.
    expect(getFlow).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('flow-canvas-page')).toBeInTheDocument();
  });

  it('does not prompt when navigating Back with no unsaved changes', async () => {
    getFlow.mockResolvedValue(makeFlow());
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('flow-canvas-back'));
    // Pristine → straight to the list, no confirmation dialog.
    await waitFor(() => expect(screen.getByTestId('flows-list')).toBeInTheDocument());
    expect(screen.queryByTestId('flow-leave-confirm')).not.toBeInTheDocument();
  });

  it('prompts before leaving when dirty, and discards to navigate away', async () => {
    getFlow.mockResolvedValue(makeFlow());
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    // Make it dirty, then click Back — a confirmation dialog blocks navigation.
    // The node palette is the "Manual" tab — hidden by default now the Copilot
    // shows — so switch to it before adding a node.
    fireEvent.click(screen.getByTestId('flow-canvas-legend-toggle'));
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    fireEvent.click(screen.getByTestId('flow-canvas-back'));
    expect(screen.getByTestId('flow-leave-confirm')).toBeInTheDocument();
    expect(screen.queryByTestId('flows-list')).not.toBeInTheDocument();

    // Staying dismisses the dialog and keeps the editor mounted.
    fireEvent.click(screen.getByTestId('flow-leave-stay'));
    expect(screen.queryByTestId('flow-leave-confirm')).not.toBeInTheDocument();
    expect(screen.getByTestId('flow-canvas')).toBeInTheDocument();

    // Re-open the prompt and confirm leaving → navigates to the list.
    fireEvent.click(screen.getByTestId('flow-canvas-back'));
    fireEvent.click(screen.getByTestId('flow-leave-discard'));
    await waitFor(() => expect(screen.getByTestId('flows-list')).toBeInTheDocument());
  });

  // -------------------------------------------------------------------------
  // Draft canvas (Phase 4e) — the chat "Open in canvas" action lands here with
  // the proposed graph in router state. Opening it must NEVER persist.
  // -------------------------------------------------------------------------
  const draftGraph = {
    schema_version: 1,
    name: 'Proposed flow',
    nodes: [
      { id: 't', kind: 'trigger', name: 'Start', config: {}, ports: [], position: { x: 0, y: 0 } },
    ],
    edges: [],
  };

  function renderDraft(state: unknown) {
    return render(
      <MemoryRouter initialEntries={[{ pathname: '/flows/draft', state }]}>
        <Routes>
          <Route path="/flows/draft" element={<FlowCanvasDraftPage />} />
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
          <Route path="/flows" element={<div data-testid="flows-list">Flows list</div>} />
        </Routes>
      </MemoryRouter>
    );
  }

  it('renders the draft canvas from router state without fetching or persisting', async () => {
    renderDraft({ name: 'Proposed flow', graph: draftGraph, requireApproval: true });

    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());
    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('Proposed flow');
    // A draft is not fetched, is not runnable, and has persisted nothing.
    expect(getFlow).not.toHaveBeenCalled();
    expect(createFlow).not.toHaveBeenCalled();
    expect(updateFlow).not.toHaveBeenCalled();
    expect(screen.queryByTestId('flow-canvas-run')).not.toBeInTheDocument();
  });

  it('creates (never updates) the flow when a draft is saved', async () => {
    renderDraft({ name: 'Proposed flow', graph: draftGraph, requireApproval: true });
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    // Edit to make it dirty, then Save → the single persistence gate fires
    // `flows_create` (with the require-approval flag), not `flows_update`.
    // The node palette is the "Manual" tab — hidden by default now the Copilot
    // shows — so switch to it before adding a node.
    fireEvent.click(screen.getByTestId('flow-canvas-legend-toggle'));
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    fireEvent.click(screen.getByTestId('flow-editor-save'));
    // Save (and Run/Discard) open a confirm popup before firing.
    fireEvent.click(screen.getByTestId('flow-action-confirm-accept'));

    await waitFor(() => expect(createFlow).toHaveBeenCalledTimes(1));
    const [name, graph, requireApproval] = createFlow.mock.calls[0];
    expect(name).toBe('Proposed flow');
    expect(requireApproval).toBe(true);
    expect(graph.nodes.map((n: { kind: string }) => n.kind).sort()).toEqual(['agent', 'trigger']);
    expect(updateFlow).not.toHaveBeenCalled();
  });

  it('shows an empty state when the draft route is hit with no draft in state', () => {
    renderDraft(null);
    expect(screen.getByTestId('flow-canvas-draft-missing')).toBeInTheDocument();
    expect(screen.queryByTestId('flow-canvas')).not.toBeInTheDocument();
  });
});

describe('isPlaceholderTitle', () => {
  it('treats an empty or whitespace-only title as a placeholder', () => {
    expect(isPlaceholderTitle('', 'New workflow')).toBe(true);
    expect(isPlaceholderTitle('   ', 'New workflow')).toBe(true);
  });

  it('treats the localized generic placeholder as a placeholder', () => {
    expect(isPlaceholderTitle('New workflow', 'New workflow')).toBe(true);
    expect(isPlaceholderTitle('  New workflow  ', 'New workflow')).toBe(true);
  });

  it('does not treat a user-chosen or description-derived name as a placeholder', () => {
    expect(isPlaceholderTitle('My flow', 'New workflow')).toBe(false);
    expect(isPlaceholderTitle('Standup reminder', 'New workflow')).toBe(false);
  });
});

// -----------------------------------------------------------------------------
// Copilot proposal name adoption — accepting a `propose_workflow` proposal
// carries a top-level `name` the canvas previously dropped, leaving the flow
// titled the generic placeholder even when the agent proposed a real name.
// -----------------------------------------------------------------------------
function makeProposal(overrides: Partial<WorkflowProposal> = {}): WorkflowProposal {
  return {
    name: 'Standup reminder',
    graph: {
      schema_version: 1,
      name: 'Standup reminder',
      nodes: [
        {
          id: 't',
          kind: 'trigger',
          name: 'Start',
          config: {},
          ports: [],
          position: { x: 0, y: 0 },
        },
        {
          id: 'a',
          kind: 'agent',
          name: 'Send reminder',
          config: {},
          ports: [],
          position: { x: 80, y: 80 },
        },
      ],
      edges: [],
    },
    requireApproval: false,
    summary: { trigger: 'manual', steps: [] },
    ...overrides,
  };
}

describe('FlowCanvasPage copilot proposal name adoption', () => {
  beforeEach(() => {
    copilotPanelProps.current = null;
    getFlow.mockReset();
    updateFlow.mockReset();
    createFlow.mockReset();
    validateFlow.mockReset();
    listFlowConnections.mockReset();
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    listFlowConnections.mockResolvedValue([]);
    // Accept now persists immediately (review + save in one step, see
    // `handleAcceptProposal`) — default every test to a successful save so
    // tests that only care about the title/name adoption aren't tripped up
    // by an unmocked (`undefined`-resolving) `updateFlow`/`createFlow`.
    updateFlow.mockResolvedValue(makeFlow());
    createFlow.mockResolvedValue(makeFlow({ id: 'created-id' }));
  });

  function renderEditor(id = 'test-id') {
    return render(
      <MemoryRouter initialEntries={[`/flows/${id}`]}>
        <Routes>
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
          <Route path="/flows" element={<div data-testid="flows-list">Flows list</div>} />
        </Routes>
      </MemoryRouter>
    );
  }

  // `handleAcceptProposal` is async (it awaits the persist call) — drive it
  // through `act(async () => …)` so React flushes every state update the
  // resulting save produces before the test asserts on them.
  function acceptProposal(proposal: WorkflowProposal = makeProposal()) {
    return act(async () => {
      await (copilotPanelProps.current?.onAccept as (p: WorkflowProposal) => Promise<void>)(
        proposal
      );
    });
  }

  it('adopts the proposal name when the title is the generic placeholder', async () => {
    getFlow.mockResolvedValue(makeFlow({ name: 'New workflow' }));
    updateFlow.mockResolvedValue(makeFlow({ name: 'Standup reminder' }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());
    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('New workflow');

    await acceptProposal();

    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('Standup reminder');
  });

  it('adopts the proposal name when the title is blank', async () => {
    getFlow.mockResolvedValue(makeFlow({ name: '' }));
    updateFlow.mockResolvedValue(makeFlow({ name: 'Standup reminder' }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    await acceptProposal();

    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('Standup reminder');
  });

  it('does not clobber a user-set title when accepting a proposal', async () => {
    getFlow.mockResolvedValue(makeFlow({ name: 'My flow' }));
    // The name is unchanged by this accept, so the accept-triggered save's
    // response echoes it back unchanged too — matching a real server, which
    // only touches `name` when it's part of the update payload.
    updateFlow.mockResolvedValue(makeFlow({ name: 'My flow' }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    await acceptProposal();

    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('My flow');
  });

  // Regression (CodeRabbit on #4886): the committed `name` only updates on
  // blur/Enter (`commitRename`), so while the user is still typing a custom
  // title the committed `name` can read as the stale placeholder even though
  // the visible input already holds real user input. Adoption must check the
  // VISIBLE `titleDraft`, or it clobbers in-progress typing.
  it('does not clobber an in-progress (uncommitted) title edit when accepting a proposal', async () => {
    getFlow.mockResolvedValue(makeFlow({ name: 'New workflow' }));
    // The uncommitted edit never reaches `name`, so the accept-triggered
    // save's payload/response name is unchanged from the loaded flow.
    updateFlow.mockResolvedValue(makeFlow({ name: 'New workflow' }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    // User is mid-typing a custom title — not yet committed via blur/Enter.
    fireEvent.change(screen.getByTestId('flow-canvas-title'), {
      target: { value: 'My in-progress title' },
    });
    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('My in-progress title');

    await acceptProposal();

    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('My in-progress title');
  });

  it('Accept on an existing flow fires updateFlow(flowId, { graph, name }) immediately, with no separate Save click', async () => {
    getFlow.mockResolvedValue(makeFlow({ name: 'New workflow' }));
    updateFlow.mockResolvedValue(makeFlow({ name: 'Standup reminder' }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    await acceptProposal();

    expect(updateFlow).toHaveBeenCalledTimes(1);
    const [calledId, update] = updateFlow.mock.calls[0];
    expect(calledId).toBe('test-id');
    expect(update.name).toBe('Standup reminder');
    expect(update.graph).toBeDefined();
    // The accept-triggered save re-syncs the canvas from the response and
    // clears the dirty baseline — no lingering "Unsaved changes" badge, and
    // no manual Save click was fired to get here.
    expect(screen.queryByTestId('flow-editor-dirty')).not.toBeInTheDocument();
  });

  // Regression (CodeRabbit on #4886): accepting a proposal that changes only
  // the top-level `name` (graph unchanged) previously left the editor's dirty
  // state false — since the graph-only diff saw no change — so Save stayed
  // disabled and the adopted title could never be persisted. Accept now saves
  // immediately regardless of dirty tracking, so this asserts the
  // accept-triggered save still fires — and carries the adopted name — even
  // when the graph itself didn't change.
  it('persists a name-only accepted proposal (graph unchanged) via the accept-triggered save', async () => {
    const flow = makeFlow({ name: 'New workflow' });
    getFlow.mockResolvedValue(flow);
    updateFlow.mockResolvedValue(makeFlow({ name: 'Standup reminder' }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    // Clean on load.
    expect(screen.queryByTestId('flow-editor-dirty')).not.toBeInTheDocument();

    await acceptProposal(makeProposal({ name: 'Standup reminder', graph: flow.graph }));

    await waitFor(() =>
      expect(screen.getByTestId('flow-canvas-title')).toHaveValue('Standup reminder')
    );
    expect(updateFlow).toHaveBeenCalledTimes(1);
    const [, update] = updateFlow.mock.calls[0];
    expect(update.name).toBe('Standup reminder');
  });

  // Regression (CodeRabbit on #4886): when the backend returns a name that
  // differs from what was submitted (server-side normalization), the title
  // input must re-sync to the persisted value too — not just the committed
  // `name` — or the stale draft can be resubmitted verbatim on a later blur.
  // Covers F2: `handleSave` re-syncing both `name` and `titleDraft` from the
  // server response name.
  it('re-syncs titleDraft from the persisted response name on the accept-triggered save', async () => {
    getFlow.mockResolvedValue(makeFlow({ name: 'New workflow' }));
    updateFlow.mockResolvedValue(makeFlow({ name: 'Standup Reminder (normalized)' }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    await acceptProposal();

    // The visible input (`titleDraft`) must reflect the server-normalized
    // name, not just the committed `name` — otherwise a later blur would
    // resubmit the stale, pre-normalization value.
    await waitFor(() =>
      expect(screen.getByTestId('flow-canvas-title')).toHaveValue('Standup Reminder (normalized)')
    );
    expect(updateFlow).toHaveBeenCalledTimes(1);
  });

  // Regression for the CodeRabbit finding: the accepted PROPOSAL's own
  // `requireApproval` policy must reach `createFlow`, not the draft route's
  // pre-existing value — otherwise Accept would silently keep the old canvas
  // policy instead of the one the agent proposed. Route state deliberately
  // uses the OPPOSITE value from the proposal so a test that reads the
  // route's value (the pre-fix bug) fails loudly instead of passing by
  // coincidence.
  it('Accept on a draft canvas fires createFlow(name, graph, requireApproval) using the PROPOSAL policy and navigates to /flows/<id>', async () => {
    createFlow.mockResolvedValue(makeFlow({ id: 'created-id', name: 'Standup reminder' }));
    getFlow.mockResolvedValue(makeFlow({ id: 'created-id', name: 'Standup reminder' }));
    render(
      <MemoryRouter
        initialEntries={[
          {
            pathname: '/flows/draft',
            state: {
              name: 'New workflow',
              graph: {
                schema_version: 1,
                name: 'New workflow',
                nodes: [
                  {
                    id: 't',
                    kind: 'trigger',
                    name: 'Start',
                    config: {},
                    ports: [],
                    position: { x: 0, y: 0 },
                  },
                ],
                edges: [],
              },
              // Route (pre-existing draft) policy is FALSE — the opposite of
              // the proposal below — so the assertion can't pass by both
              // values coincidentally matching.
              requireApproval: false,
            },
          },
        ]}>
        <Routes>
          <Route path="/flows/draft" element={<FlowCanvasDraftPage />} />
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
        </Routes>
      </MemoryRouter>
    );
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());
    expect(screen.getByTestId('flow-canvas-title')).toHaveValue('New workflow');

    // Proposal policy is TRUE — must be what reaches `createFlow`.
    await acceptProposal(makeProposal({ requireApproval: true }));

    expect(createFlow).toHaveBeenCalledTimes(1);
    const [name, graph, requireApproval] = createFlow.mock.calls[0];
    expect(name).toBe('Standup reminder');
    expect(graph).toBeDefined();
    expect(requireApproval).toBe(true);
    expect(updateFlow).not.toHaveBeenCalled();

    // `handleSave`'s draft-create path replaces into the new flow's canonical
    // route — Accept alone drives that navigation, matching what a manual
    // Save click right after Accept used to require.
    await waitFor(() => expect(getFlow).toHaveBeenCalledWith('created-id'));
  });

  it('Accept on a saved flow fires updateFlow with the PROPOSAL requireApproval policy', async () => {
    // The loaded flow's persisted policy is FALSE; the accepted proposal's
    // is TRUE — the update payload must carry the proposal's value, not
    // silently keep the flow's current one.
    getFlow.mockResolvedValue(makeFlow({ require_approval: false }));
    updateFlow.mockResolvedValue(makeFlow({ require_approval: true }));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());

    await acceptProposal(makeProposal({ requireApproval: true }));

    expect(updateFlow).toHaveBeenCalledTimes(1);
    expect(updateFlow).toHaveBeenCalledWith(
      'test-id',
      expect.objectContaining({ requireApproval: true })
    );
  });

  // Regression test for the review finding (F1, HIGH): `handleAcceptProposal`
  // used to swallow a failed accept-triggered save (log + no rethrow), so
  // `onAccept` always resolved — the copilot panel's `clearProposal()` always
  // ran, and the proposal card silently vanished on a real save failure with
  // no way to retry (its own catch branch was dead code). This test fails
  // without the rethrow (the promise resolves instead of rejecting) and
  // passes with it.
  it('rethrows an accept-triggered save failure so the caller can leave the proposal visible for retry', async () => {
    getFlow.mockResolvedValue(makeFlow({ name: 'My flow' }));
    updateFlow.mockRejectedValue(new Error('network unreachable'));
    renderEditor();
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());
    expect(screen.queryByTestId('flow-editor-dirty')).not.toBeInTheDocument();

    // Catch INSIDE `act()` (rather than via `expect(...).rejects`, which lets
    // the rejection escape the `act()` scope unhandled) so React still
    // flushes the synchronous draft/preview updates `handleAcceptProposal`
    // makes before the failed `await handleSave(...)` — otherwise the
    // assertions below would race an incomplete render.
    let caughtErr: unknown;
    await act(async () => {
      try {
        await (copilotPanelProps.current?.onAccept as (p: WorkflowProposal) => Promise<void>)(
          makeProposal()
        );
      } catch (err) {
        caughtErr = err;
      }
    });

    // This is the fix under test: without the rethrow, `caughtErr` stays
    // `undefined` and this assertion fails.
    expect(caughtErr).toBeInstanceOf(Error);
    expect((caughtErr as Error).message).toBe('network unreachable');

    // The draft is already applied before `handleSave` is even attempted, so
    // rethrowing loses no data: the proposal's graph is still on the canvas
    // (2 nodes: the original trigger + the proposal's agent node), dirty,
    // with the header Save button enabled as the manual retry — matching
    // what `WorkflowCopilotPanel`'s own catch branch (which skips
    // `clearProposal()` on rejection) relies on to keep the card visible.
    //
    // These three assertions land on state derived from the REMOUNTED canvas
    // (`handleAcceptProposal` bumps `canvasVersion`, which changes the
    // `<FlowCanvas key=...>` and forces a fresh child mount) — its own
    // mount-time effects (`onDirtyChange`/`onSaveMetaChange`) can settle on a
    // later microtask/effect flush than the outer `act()` above guarantees,
    // so poll via `waitFor` instead of asserting immediately (this was
    // observed to occasionally race in CI).
    await waitFor(() => expect(screen.getAllByTestId('flow-node')).toHaveLength(2));
    await waitFor(() => expect(screen.getByTestId('flow-editor-dirty')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByTestId('flow-editor-save')).not.toBeDisabled());
  });
});

describe('asCopilotBuildSeed', () => {
  it('accepts a copilotBuild state with a non-empty description', () => {
    expect(asCopilotBuildSeed({ copilotBuild: { description: 'digest my Slack' } })).toEqual({
      description: 'digest my Slack',
    });
  });

  it('carries chatFirst only when explicitly true (Start building path)', () => {
    expect(
      asCopilotBuildSeed({ copilotBuild: { description: 'digest my Slack', chatFirst: true } })
    ).toEqual({ description: 'digest my Slack', chatFirst: true });
    // A falsey/absent chatFirst yields a bare seed — no drift on the Build path.
    expect(
      asCopilotBuildSeed({ copilotBuild: { description: 'digest my Slack', chatFirst: false } })
    ).toEqual({ description: 'digest my Slack' });
  });

  it('rejects missing, malformed, or blank seeds', () => {
    expect(asCopilotBuildSeed(null)).toBeNull();
    expect(asCopilotBuildSeed({})).toBeNull();
    expect(asCopilotBuildSeed({ copilotBuild: 'digest' })).toBeNull();
    expect(asCopilotBuildSeed({ copilotBuild: { description: 42 } })).toBeNull();
    expect(asCopilotBuildSeed({ copilotBuild: { description: '   ' } })).toBeNull();
  });
});

describe('FlowCanvasPage copilot build seed (prompt-bar instant create)', () => {
  beforeEach(() => {
    copilotPanelProps.current = null;
    getFlow.mockReset();
    getFlow.mockResolvedValue(makeFlow());
  });

  it('opens the copilot preloaded with the build seed from location.state', async () => {
    render(
      <MemoryRouter
        initialEntries={[
          { pathname: '/flows/test-id', state: { copilotBuild: { description: 'digest it' } } },
        ]}>
        <Routes>
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => expect(screen.getByTestId('stub-copilot-panel')).toBeInTheDocument());
    expect(copilotPanelProps.current?.buildSeed).toEqual({ description: 'digest it' });
    expect(copilotPanelProps.current?.flowId).toBe('test-id');
  });

  it('clears only the build seed on consume, preserving sibling route state (#4597)', async () => {
    render(
      <MemoryRouter
        initialEntries={[
          {
            pathname: '/flows/test-id',
            state: {
              copilotBuild: { description: 'digest it' },
              // A sibling seed (a repair context) must survive the strip: the
              // host clones state and deletes ONLY `copilotBuild` — a regression
              // that nuked the whole state object would drop this too.
              copilotRepair: { runId: 'run-1' },
            },
          },
        ]}>
        <Routes>
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() =>
      expect(copilotPanelProps.current?.buildSeed).toEqual({ description: 'digest it' })
    );
    // The sibling repair seed is present before consumption.
    expect(copilotPanelProps.current?.repairSeed).toMatchObject({ runId: 'run-1' });

    // The panel dispatched the build turn and reports it consumed; the host must
    // strip `copilotBuild` from `location.state` so a later remount (close +
    // reopen) has no seed left to re-fire.
    act(() => {
      (copilotPanelProps.current?.onBuildSeedConsumed as () => void)();
    });

    await waitFor(() => expect(copilotPanelProps.current?.buildSeed).toBeNull());
    // ...but the sibling repair seed must NOT be nuked along with it.
    expect(copilotPanelProps.current?.repairSeed).toMatchObject({ runId: 'run-1' });
  });

  it('opens the copilot by default even without a seed', async () => {
    render(
      <MemoryRouter initialEntries={['/flows/test-id']}>
        <Routes>
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
        </Routes>
      </MemoryRouter>
    );

    // The side panel now defaults to the Copilot (the header toggle switches it
    // to the Manual node palette or collapses it).
    await waitFor(() => expect(screen.getByTestId('flow-canvas')).toBeInTheDocument());
    expect(screen.getByTestId('stub-copilot-panel')).toBeInTheDocument();
  });
});

describe('asCopilotPrefillSeed', () => {
  it('accepts a copilotPrefill state with non-empty text, defaulting mode to build', () => {
    expect(asCopilotPrefillSeed({ copilotPrefill: { text: 'digest my Slack' } })).toEqual({
      text: 'digest my Slack',
      mode: 'build',
    });
  });

  it('carries an explicit mode through unchanged', () => {
    expect(
      asCopilotPrefillSeed({ copilotPrefill: { text: 'digest my Slack', mode: 'create' } })
    ).toEqual({ text: 'digest my Slack', mode: 'create' });
  });

  it('falls back to build for an unrecognized mode value', () => {
    expect(
      asCopilotPrefillSeed({ copilotPrefill: { text: 'digest my Slack', mode: 'revise' } })
    ).toEqual({ text: 'digest my Slack', mode: 'build' });
  });

  it('rejects missing, malformed, or blank seeds', () => {
    expect(asCopilotPrefillSeed(null)).toBeNull();
    expect(asCopilotPrefillSeed({})).toBeNull();
    expect(asCopilotPrefillSeed({ copilotPrefill: 'digest' })).toBeNull();
    expect(asCopilotPrefillSeed({ copilotPrefill: { text: 42 } })).toBeNull();
    expect(asCopilotPrefillSeed({ copilotPrefill: { text: '   ' } })).toBeNull();
  });
});

describe('FlowCanvasPage copilot prefill seed (Suggested Workflows "Build this")', () => {
  beforeEach(() => {
    copilotPanelProps.current = null;
    getFlow.mockReset();
    getFlow.mockResolvedValue(makeFlow());
  });

  it('opens the copilot preloaded with the prefill seed from location.state', async () => {
    render(
      <MemoryRouter
        initialEntries={[
          { pathname: '/flows/test-id', state: { copilotPrefill: { text: 'digest it' } } },
        ]}>
        <Routes>
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() => expect(screen.getByTestId('stub-copilot-panel')).toBeInTheDocument());
    // `mode` defaults to `build` when the route state omits it.
    expect(copilotPanelProps.current?.prefillSeed).toEqual({ text: 'digest it', mode: 'build' });
    expect(copilotPanelProps.current?.flowId).toBe('test-id');
  });

  it('clears only the prefill seed on consume, preserving sibling route state', async () => {
    render(
      <MemoryRouter
        initialEntries={[
          {
            pathname: '/flows/test-id',
            state: {
              copilotPrefill: { text: 'digest it' },
              // A sibling seed must survive the strip — the host clones state
              // and deletes ONLY `copilotPrefill`.
              copilotRepair: { runId: 'run-1' },
            },
          },
        ]}>
        <Routes>
          <Route path="/flows/:id" element={<FlowCanvasPage />} />
        </Routes>
      </MemoryRouter>
    );

    await waitFor(() =>
      expect(copilotPanelProps.current?.prefillSeed).toEqual({ text: 'digest it', mode: 'build' })
    );
    expect(copilotPanelProps.current?.repairSeed).toMatchObject({ runId: 'run-1' });

    // The panel consumed the prefill seed; the host must strip `copilotPrefill`
    // from `location.state` so a later remount (close + reopen) has no seed
    // left to re-apply.
    act(() => {
      (copilotPanelProps.current?.onPrefillSeedConsumed as () => void)();
    });

    await waitFor(() => expect(copilotPanelProps.current?.prefillSeed).toBeNull());
    expect(copilotPanelProps.current?.repairSeed).toMatchObject({ runId: 'run-1' });
  });
});
