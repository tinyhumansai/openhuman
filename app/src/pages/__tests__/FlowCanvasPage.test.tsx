/**
 * FlowCanvasPage (issue B5b / Phase 3) — the editable Workflow Canvas builder
 * at `/flows/:id`. Asserts the loading → canvas happy path, the not-found state
 * (mirrors the Rust `flows_get` "not found" error), the generic error state,
 * and the Phase 3d host wiring: Save persists via `flows_update`, and the
 * unsaved-changes guard intercepts the Back button while dirty.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { createMemoryRouter, MemoryRouter, Route, RouterProvider, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Flow } from '../../services/api/flowsApi';
import FlowCanvasPage, { FlowCanvasDraftPage } from '../FlowCanvasPage';

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
    expect(screen.getByText('Daily digest')).toBeInTheDocument();
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
    await waitFor(() => expect(screen.getByText('New flow')).toBeInTheDocument());

    // Now let the stale old-id fetch resolve — it must not clobber the
    // already-rendered new-id state.
    resolveFirst(makeFlow({ id: 'old-id', name: 'Old flow (stale)' }));
    await Promise.resolve();
    await Promise.resolve();

    expect(screen.getByText('New flow')).toBeInTheDocument();
    expect(screen.queryByText('Old flow (stale)')).not.toBeInTheDocument();
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
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    fireEvent.click(screen.getByTestId('flow-editor-save'));

    await waitFor(() => expect(updateFlow).toHaveBeenCalledTimes(1));
    const [calledId, update] = updateFlow.mock.calls[0];
    expect(calledId).toBe('test-id');
    expect(update.graph.nodes.map((n: { kind: string }) => n.kind).sort()).toEqual([
      'agent',
      'trigger',
    ]);
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
    expect(screen.getByText('Proposed flow')).toBeInTheDocument();
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
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    fireEvent.click(screen.getByTestId('flow-editor-save'));

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
