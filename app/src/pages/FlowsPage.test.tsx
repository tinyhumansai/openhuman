/**
 * FlowsPage (issue B5a / B5a.1 / B5b.1) — the Workflows list page. Asserts
 * the loading/empty/error/list states, that toggling a flow calls
 * `setFlowEnabled` and refreshes the row, that Run fires `runFlowDetached`
 * (F-M1/F-M2 — the non-blocking `flows_run_detached` RPC, not the old
 * blocking `flows_run`), shows a "Workflow started" toast immediately, that
 * a run in flight on one row does NOT disable any other row's actions
 * (F-M2's core regression), that "View runs" opens `FlowRunsDrawer` for the
 * clicked flow, that clicking a flow's name navigates to its read-only
 * Workflow Canvas (`/flows/:id`, issue B5b.1), and that "New workflow"
 * (header + empty state) opens the Phase 4a chooser (start from scratch /
 * template / describe), with the empty state also surfacing the Phase 4c
 * template gallery inline.
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { FLOW_TEMPLATES } from '../lib/flows/templates';
import type { Flow } from '../services/api/flowsApi';
import { renderWithProviders } from '../test/test-utils';
import FlowsPage from './FlowsPage';

const listFlows = vi.hoisted(() => vi.fn());
const setFlowEnabled = vi.hoisted(() => vi.fn());
const runFlowDetached = vi.hoisted(() => vi.fn());
const listFlowRuns = vi.hoisted(() => vi.fn());
const createFlow = vi.hoisted(() => vi.fn());
const importFlow = vi.hoisted(() => vi.fn());
const deleteFlow = vi.hoisted(() => vi.fn());
const duplicateFlow = vi.hoisted(() => vi.fn());
// Flow Scout discovery clients — rendered via the SuggestedWorkflows section.
const discoverWorkflows = vi.hoisted(() => vi.fn());
const listSuggestions = vi.hoisted(() => vi.fn());
const dismissSuggestion = vi.hoisted(() => vi.fn());
const markSuggestionBuilt = vi.hoisted(() => vi.fn());
vi.mock('../services/api/flowsApi', () => ({
  listFlows,
  setFlowEnabled,
  runFlowDetached,
  listFlowRuns,
  createFlow,
  importFlow,
  deleteFlow,
  duplicateFlow,
  discoverWorkflows,
  listSuggestions,
  dismissSuggestion,
  markSuggestionBuilt,
}));

const downloadFlowGraph = vi.hoisted(() => vi.fn(() => true));
vi.mock('../lib/flows/exportFlow', () => ({ downloadFlowGraph }));

const mockNavigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

function makeFlow(overrides: Partial<Flow> = {}): Flow {
  return {
    id: 'flow-1',
    name: 'Daily digest',
    enabled: true,
    graph: { nodes: [], edges: [] },
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    last_run_at: null,
    last_status: null,
    require_approval: false,
    ...overrides,
  };
}

/**
 * Opens a row's "⋯" overflow menu. `FlowRowMenu` is now Radix `DropdownMenu`
 * (issue: UI library adoption), whose trigger opens on `pointerdown` rather
 * than `click` (so a press-and-drag select onto an item keeps working) — a
 * plain `fireEvent.click` never dispatches that event.
 */
function openRowMenu(rowId = 'flow-1') {
  const trigger = screen.getByTestId(`flow-menu-${rowId}`);
  fireEvent.pointerDown(trigger);
  fireEvent.click(trigger);
}

describe('FlowsPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // SuggestedWorkflows loads persisted suggestions on mount; default to none
    // so the section renders its (harmless) empty state in these flow-list tests.
    listSuggestions.mockResolvedValue([]);
    discoverWorkflows.mockResolvedValue([]);
    dismissSuggestion.mockResolvedValue(true);
    markSuggestionBuilt.mockResolvedValue(true);
  });

  it('shows the beta banner at the top of the page', async () => {
    listFlows.mockResolvedValue([]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByTestId('flows-beta-banner')).toBeInTheDocument());
    expect(screen.getByTestId('flows-beta-banner')).toHaveTextContent('Beta');
  });

  it('shows a loading state while flows are being fetched', () => {
    listFlows.mockReturnValue(new Promise(() => {})); // never resolves
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    expect(screen.getByText('Loading workflows…')).toBeInTheDocument();
  });

  it('shows the empty state when there are no saved flows, with a "New workflow" action', async () => {
    listFlows.mockResolvedValue([]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByText('No workflows yet')).toBeInTheDocument());
    // There's no canvas builder yet (B5b) — the empty state's action bridges
    // to Chat/B4 instead, same as the header button.
    expect(screen.getByTestId('flows-empty-new-workflow')).toHaveTextContent('New workflow');
  });

  it('shows an error banner when the fetch fails', async () => {
    listFlows.mockRejectedValue(new Error('core unreachable'));
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() =>
      expect(screen.getByText('Could not load workflows. Please try again.')).toBeInTheDocument()
    );
  });

  it('renders one row per saved flow', async () => {
    listFlows.mockResolvedValue([makeFlow(), makeFlow({ id: 'flow-2', name: 'Weekly report' })]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByText('Daily digest')).toBeInTheDocument());
    expect(screen.getByText('Weekly report')).toBeInTheDocument();
  });

  it('toggles a flow via setFlowEnabled and reflects the updated state', async () => {
    listFlows.mockResolvedValue([makeFlow({ enabled: true })]);
    setFlowEnabled.mockResolvedValue(makeFlow({ enabled: false }));
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByTestId('flow-toggle-flow-1')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('flow-toggle-flow-1'));

    expect(setFlowEnabled).toHaveBeenCalledWith('flow-1', false);
    // The toggle is a SettingsSwitch (role=switch) now; state is conveyed via aria-checked.
    await waitFor(() =>
      expect(screen.getByTestId('flow-toggle-flow-1')).toHaveAttribute('aria-checked', 'false')
    );
  });

  it('runs a flow via the non-blocking flows_run_detached RPC and shows a "Workflow started" toast', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    runFlowDetached.mockResolvedValue({
      run_id: 'flow:flow-1:t1',
      flow_id: 'flow-1',
      status: 'running',
      detached: true,
    });
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByTestId('flow-run-flow-1')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('flow-run-flow-1'));

    expect(runFlowDetached).toHaveBeenCalledWith('flow-1');
    await waitFor(() => expect(screen.getByText('Workflow started')).toBeInTheDocument());
  });

  it('shows an error banner (without a toast) when runFlowDetached rejects', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    runFlowDetached.mockRejectedValue(new Error('flow disabled'));
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByTestId('flow-run-flow-1')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('flow-run-flow-1'));

    await waitFor(() => expect(screen.getByText('flow disabled')).toBeInTheDocument());
    expect(screen.queryByText('Workflow started')).not.toBeInTheDocument();
  });

  // F-M2 regression: the old page-global `busyKey` disabled EVERY row's
  // Run/Toggle for the whole duration of any one row's action. Now that Run
  // starts a detached, possibly minutes-long flow (F-M1), that would have
  // frozen the entire list. Assert the busy state stays scoped to the row
  // whose action is actually in flight.
  it('keeps other rows interactive while one row has a run in flight', async () => {
    listFlows.mockResolvedValue([makeFlow(), makeFlow({ id: 'flow-2', name: 'Weekly report' })]);
    // Never resolves — simulates the RPC round-trip still being in flight so
    // the row-1 Run button stays in its busy state for the assertion window.
    runFlowDetached.mockReturnValue(new Promise(() => {}));
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByTestId('flow-run-flow-1')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('flow-run-flow-1'));

    // Row 1's own Run button reflects its in-flight state...
    await waitFor(() => expect(screen.getByTestId('flow-run-flow-1')).toBeDisabled());
    // ...but row 2's Run and Toggle stay fully interactive — nothing
    // page-global gates them anymore.
    expect(screen.getByTestId('flow-run-flow-2')).not.toBeDisabled();
    expect(screen.getByTestId('flow-toggle-flow-2')).not.toBeDisabled();

    // Row 2's own actions still work while row 1 is busy.
    setFlowEnabled.mockResolvedValue(
      makeFlow({ id: 'flow-2', name: 'Weekly report', enabled: false })
    );
    fireEvent.click(screen.getByTestId('flow-toggle-flow-2'));
    await waitFor(() => expect(setFlowEnabled).toHaveBeenCalledWith('flow-2', false));
  });

  it('opens the run-history drawer for the clicked flow when "View runs" is clicked', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    listFlowRuns.mockResolvedValue([]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    // "View runs" is a secondary action behind the row's overflow menu now.
    await waitFor(() => expect(screen.getByTestId('flow-menu-flow-1')).toBeInTheDocument());
    openRowMenu();
    fireEvent.click(screen.getByTestId('flow-view-runs-flow-1'));

    expect(await screen.findByTestId('flow-runs-drawer')).toBeInTheDocument();
    expect(screen.getByText('Runs for Daily digest')).toBeInTheDocument();
    expect(listFlowRuns).toHaveBeenCalledWith('flow-1');

    fireEvent.click(screen.getByTestId('flow-runs-close'));
    expect(screen.queryByTestId('flow-runs-drawer')).not.toBeInTheDocument();
  });

  it('navigates to the Workflow Canvas when a flow name is clicked', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await waitFor(() => expect(screen.getByTestId('flow-view-flow-1')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('flow-view-flow-1'));

    expect(mockNavigate).toHaveBeenCalledWith('/flows/flow-1');
  });

  it('renders a "New workflow" header button that opens the chooser modal', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    const newWorkflowButton = await screen.findByTestId('flows-new-workflow');
    expect(newWorkflowButton).toHaveTextContent('New workflow');
    fireEvent.click(newWorkflowButton);

    expect(screen.getByTestId('new-workflow-modal')).toBeInTheDocument();
    expect(screen.getByTestId('new-workflow-scratch')).toBeInTheDocument();
  });

  it('opens the chooser from the welcome landing "New workflow" action', async () => {
    listFlows.mockResolvedValue([]);
    renderWithProviders(<FlowsPage />);

    fireEvent.click(await screen.findByTestId('flows-welcome-cta-new'));

    expect(screen.getByTestId('new-workflow-modal')).toBeInTheDocument();
    expect(screen.getByTestId('new-workflow-scratch')).toBeInTheDocument();
  });

  it('opens the chooser from the empty-state "New workflow" action', async () => {
    listFlows.mockResolvedValue([]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    const emptyStateButton = await screen.findByTestId('flows-empty-new-workflow');
    fireEvent.click(emptyStateButton);

    expect(screen.getByTestId('new-workflow-modal')).toBeInTheDocument();
  });

  it('no longer shows the in-place copilot composer on the list page', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    // The list-page composer was removed — building now happens in the canvas.
    await screen.findByTestId('flows-new-workflow');
    expect(screen.queryByTestId('workflow-prompt-bar')).not.toBeInTheDocument();

    // The chooser modal offers scratch + template only — no describe.
    fireEvent.click(screen.getByTestId('flows-new-workflow'));
    expect(screen.getByTestId('new-workflow-scratch')).toBeInTheDocument();
    expect(screen.queryByTestId('new-workflow-describe')).not.toBeInTheDocument();
  });

  it('empty-state template gallery creates a flow and opens its canvas', async () => {
    listFlows.mockResolvedValue([]);
    createFlow.mockResolvedValue({ id: 'flow-created' });
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await screen.findByTestId('flows-empty-templates');
    const template = FLOW_TEMPLATES[0];
    fireEvent.click(screen.getByTestId(`flow-template-${template.id}`));

    await waitFor(() => expect(createFlow).toHaveBeenCalledTimes(1));
    expect(createFlow.mock.calls[0][1]).toBe(template.graph);
    await waitFor(() => expect(mockNavigate).toHaveBeenCalledWith('/flows/flow-created'));
  });

  it('renders an Import button in the header', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    const importButton = await screen.findByTestId('flows-import');
    expect(importButton).toHaveTextContent('Import');
  });

  it('exports a flow row as JSON via downloadFlowGraph', async () => {
    listFlows.mockResolvedValue([makeFlow({ graph: { nodes: [], edges: [] } })]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    // Export now lives behind the row's "⋯" overflow menu.
    await screen.findByTestId('flow-menu-flow-1');
    openRowMenu();
    fireEvent.click(await screen.findByTestId('flow-export-flow-1'));

    expect(downloadFlowGraph).toHaveBeenCalledWith('Daily digest', { nodes: [], edges: [] });
  });

  it('deletes a flow via the overflow menu + confirm dialog', async () => {
    listFlows.mockResolvedValueOnce([makeFlow()]).mockResolvedValueOnce([]);
    deleteFlow.mockResolvedValue('flow-1');
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    // Delete now lives behind the row's "⋯" overflow menu, alongside
    // Export/Duplicate, rather than a standalone icon button.
    await screen.findByTestId('flow-menu-flow-1');
    openRowMenu();
    fireEvent.click(await screen.findByTestId('flow-delete-flow-1'));

    // Confirm dialog gates the destructive call.
    expect(deleteFlow).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByTestId('flow-delete-confirm-button'));

    await waitFor(() => expect(deleteFlow).toHaveBeenCalledWith('flow-1'));
  });

  it('duplicates a flow via the overflow menu', async () => {
    listFlows.mockResolvedValue([makeFlow()]);
    duplicateFlow.mockResolvedValue(makeFlow({ id: 'flow-2', name: 'Daily digest copy' }));
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    await screen.findByTestId('flow-menu-flow-1');
    openRowMenu();
    fireEvent.click(await screen.findByTestId('flow-duplicate-flow-1'));

    await waitFor(() => expect(duplicateFlow).toHaveBeenCalledWith('flow-1'));
  });

  it('imports a picked JSON file and opens the result as a draft canvas', async () => {
    listFlows.mockResolvedValue([]);
    const graph = { schema_version: 1, name: 'Imported', nodes: [], edges: [] };
    importFlow.mockResolvedValue({ graph, warnings: ['heads up'] });
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    const input = await screen.findByTestId('flows-import-input');
    const file = new File([JSON.stringify({ nodes: [] })], 'wf.json', { type: 'application/json' });
    fireEvent.change(input, { target: { files: [file] } });

    await waitFor(() => expect(importFlow).toHaveBeenCalledWith({ nodes: [] }, 'auto'));
    await waitFor(() =>
      expect(mockNavigate).toHaveBeenCalledWith('/flows/draft', {
        state: { name: 'Imported', graph, requireApproval: true, importWarnings: ['heads up'] },
      })
    );
  });

  it('shows an error when the picked file is not valid JSON', async () => {
    listFlows.mockResolvedValue([]);
    renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

    const input = await screen.findByTestId('flows-import-input');
    const file = new File(['not json{'], 'wf.json', { type: 'application/json' });
    fireEvent.change(input, { target: { files: [file] } });

    expect(await screen.findByTestId('flows-error')).toHaveTextContent(
      'That file is not valid workflow JSON.'
    );
    expect(importFlow).not.toHaveBeenCalled();
  });
  it('refetches the list on a poll backstop when the run-finished broadcast never arrives', async () => {
    // `flows_run_detached` returns immediately, so a row's last-run outcome is
    // refreshed by the `FlowRunFinished` broadcast — a plain socket emit with
    // no server-side replay. If that is missed (reconnect gap, sleeping
    // laptop), the row would show a stale outcome until the user navigates
    // away and back. Every other run-outcome surface pairs the event with a
    // poll backstop; this pins that this page does too.
    //
    // `waitFor` is avoided throughout: it deadlocks under fake timers, so the
    // clock is advanced explicitly with `advanceTimersByTimeAsync`, which also
    // flushes the pending promises.
    vi.useFakeTimers();
    try {
      listFlows.mockResolvedValue([makeFlow()]);
      runFlowDetached.mockResolvedValue({
        run_id: 'flow:flow-1:t1',
        flow_id: 'flow-1',
        status: 'running',
        detached: true,
      });
      renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });

      // Flush the mount fetch without moving the clock into the poll window.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      const callsBeforeRun = listFlows.mock.calls.length;

      fireEvent.click(screen.getByTestId('flow-run-flow-1'));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(runFlowDetached).toHaveBeenCalledWith('flow-1');

      // No FlowRunFinished is ever delivered — the backstop must still fire.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(31_000);
      });
      expect(listFlows.mock.calls.length).toBeGreaterThan(callsBeforeRun);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not poll when no run from this page is outstanding', async () => {
    vi.useFakeTimers();
    try {
      listFlows.mockResolvedValue([makeFlow()]);
      renderWithProviders(<FlowsPage />, { initialEntries: ['/?view=main'] });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      const callsAfterLoad = listFlows.mock.calls.length;

      // Several poll windows pass with nothing outstanding — no refetch.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(120_000);
      });
      expect(listFlows.mock.calls.length).toBe(callsAfterLoad);
    } finally {
      vi.useRealTimers();
    }
  });
});
