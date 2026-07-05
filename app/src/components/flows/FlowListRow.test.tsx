/**
 * FlowListRow (issue B5a / B5a.1 / B5b.1) — one saved-flow row on the
 * Workflows list page. Asserts the name/status rendering, the
 * last-run/never-run text (including the localized relative-time strings),
 * that the toggle/Run/View runs controls call back with the row's `Flow`,
 * and that the flow name itself is the "View" affordance that opens the
 * read-only Workflow Canvas (issue B5b.1).
 */
import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { Flow } from '../../services/api/flowsApi';
import { renderWithProviders } from '../../test/test-utils';
import FlowListRow from './FlowListRow';

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

describe('FlowListRow', () => {
  it('renders the flow name and an Enabled badge when enabled', () => {
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    expect(screen.getByText('Daily digest')).toBeInTheDocument();
    expect(screen.getByTestId('flow-status-flow-1')).toHaveTextContent('Enabled');
  });

  it('renders a Paused badge when disabled', () => {
    renderWithProviders(
      <FlowListRow
        flow={makeFlow({ enabled: false })}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    expect(screen.getByTestId('flow-status-flow-1')).toHaveTextContent('Paused');
  });

  it('shows "Never run" when the flow has no last_run_at', () => {
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    expect(screen.getByText('Never run')).toBeInTheDocument();
  });

  it('shows the capitalized status and "Just now" for a run seconds ago', () => {
    renderWithProviders(
      <FlowListRow
        flow={makeFlow({ last_run_at: new Date().toISOString(), last_status: 'completed' })}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    expect(screen.getByText('Completed · Just now')).toBeInTheDocument();
  });

  it('shows a minutes-ago relative time', () => {
    const fiveMinAgo = new Date(Date.now() - 5 * 60_000).toISOString();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow({ last_run_at: fiveMinAgo, last_status: 'completed' })}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    expect(screen.getByText('Completed · 5m ago')).toBeInTheDocument();
  });

  it('shows an hours-ago relative time', () => {
    const threeHoursAgo = new Date(Date.now() - 3 * 60 * 60_000).toISOString();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow({ last_run_at: threeHoursAgo, last_status: 'failed' })}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    expect(screen.getByText('Failed · 3h ago')).toBeInTheDocument();
  });

  it('shows a days-ago relative time', () => {
    const twoDaysAgo = new Date(Date.now() - 2 * 24 * 60 * 60_000).toISOString();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow({ last_run_at: twoDaysAgo, last_status: 'pending_approval' })}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    expect(screen.getByText('Pending_approval · 2d ago')).toBeInTheDocument();
  });

  it('calls onToggle with the flow when the switch is clicked', () => {
    const onToggle = vi.fn();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={onToggle}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    fireEvent.click(screen.getByTestId('flow-toggle-flow-1'));

    expect(onToggle).toHaveBeenCalledWith(makeFlow());
  });

  it('calls onRun with the flow when the Run button is clicked', () => {
    const onRun = vi.fn();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={onRun}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    fireEvent.click(screen.getByTestId('flow-run-flow-1'));

    expect(onRun).toHaveBeenCalledWith(makeFlow());
  });

  it('renders a "View runs" control and calls onViewRuns with the flow when clicked', () => {
    const onViewRuns = vi.fn();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={onViewRuns}
        onView={vi.fn()}
        onExport={vi.fn()}
      />
    );

    const viewRunsButton = screen.getByTestId('flow-view-runs-flow-1');
    expect(viewRunsButton).toHaveTextContent('View runs');
    fireEvent.click(viewRunsButton);

    expect(onViewRuns).toHaveBeenCalledWith(makeFlow());
  });

  it('renders the flow name as a "View" affordance and calls onView with the flow when clicked', () => {
    const onView = vi.fn();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={onView}
        onExport={vi.fn()}
      />
    );

    const viewButton = screen.getByTestId('flow-view-flow-1');
    expect(viewButton).toHaveTextContent('Daily digest');
    fireEvent.click(viewButton);

    expect(onView).toHaveBeenCalledWith(makeFlow());
  });

  it('shows the running label and disables Run while busy', () => {
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
        busy="run"
      />
    );

    const runButton = screen.getByTestId('flow-run-flow-1');
    expect(runButton).toHaveTextContent('Running…');
    expect(runButton).toBeDisabled();
  });

  it('disables the toggle while busy=toggle', () => {
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={vi.fn()}
        busy="toggle"
      />
    );

    expect(screen.getByTestId('flow-toggle-flow-1')).toBeDisabled();
  });

  it('renders an Export control and calls onExport with the flow when clicked', () => {
    const onExport = vi.fn();
    renderWithProviders(
      <FlowListRow
        flow={makeFlow()}
        onToggle={vi.fn()}
        onRun={vi.fn()}
        onViewRuns={vi.fn()}
        onView={vi.fn()}
        onExport={onExport}
      />
    );

    const exportButton = screen.getByTestId('flow-export-flow-1');
    expect(exportButton).toHaveTextContent('Export');
    fireEvent.click(exportButton);

    expect(onExport).toHaveBeenCalledWith(makeFlow());
  });
});
