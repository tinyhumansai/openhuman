import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { store } from '../../../../store';
import { WorkerThreadRefCard } from '../WorkerThreadRefCard';

// Issue #1624: the worker-thread surface card must render a live
// running/completed/failed badge derived from the parent timeline
// entry's status, so users scanning a parent transcript know whether
// background work is still in flight without opening the worker. The
// status prop is optional so legacy parser-driven render paths that
// don't have a parent context still work — those just show the card
// without the badge.

const REF = {
  threadId: 't-worker-1',
  label: 'researcher',
  agentId: 'researcher',
  taskId: 'task-42',
  iterations: 3,
  elapsedMs: 1234,
};

function renderInStore(ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

describe('WorkerThreadRefCard — status badge', () => {
  it('omits the badge when no status is supplied (legacy parser path)', () => {
    renderInStore(<WorkerThreadRefCard ref={REF} />);
    expect(screen.queryByTestId('worker-thread-status-badge')).toBeNull();
  });

  it('renders the running badge with amber tone for in-flight workers', () => {
    renderInStore(<WorkerThreadRefCard ref={REF} status="running" />);
    const badge = screen.getByTestId('worker-thread-status-badge');
    expect(badge.getAttribute('data-status')).toBe('running');
    expect(badge.textContent).toContain('running');
    expect(badge.className).toContain('amber');
    expect(badge.getAttribute('aria-label')).toBe('Worker running');
  });

  it('renders the completed badge with sage tone when the worker succeeds', () => {
    renderInStore(<WorkerThreadRefCard ref={REF} status="completed" />);
    const badge = screen.getByTestId('worker-thread-status-badge');
    expect(badge.getAttribute('data-status')).toBe('completed');
    expect(badge.textContent).toContain('done');
    expect(badge.className).toContain('sage');
    expect(badge.getAttribute('aria-label')).toBe('Worker done');
  });

  it('renders the failed badge with coral tone when the worker fails', () => {
    renderInStore(<WorkerThreadRefCard ref={REF} status="failed" />);
    const badge = screen.getByTestId('worker-thread-status-badge');
    expect(badge.getAttribute('data-status')).toBe('failed');
    expect(badge.textContent).toContain('failed');
    expect(badge.className).toContain('coral');
    expect(badge.getAttribute('aria-label')).toBe('Worker failed');
  });

  it('still surfaces label, agent meta, and the open-thread affordance alongside the badge', () => {
    renderInStore(<WorkerThreadRefCard ref={REF} status="running" />);
    expect(screen.getByText('researcher')).toBeTruthy();
    expect(screen.getByText('Open worker thread')).toBeTruthy();
    // Meta line carries iterations + elapsed.
    expect(screen.getByText(/3 turns/)).toBeTruthy();
    expect(screen.getByText(/1234ms/)).toBeTruthy();
  });
});

describe('WorkerThreadRefCard — navigation', () => {
  it('dispatches setActiveThread with the worker thread id when clicked', () => {
    const dispatch = vi.spyOn(store, 'dispatch');
    renderInStore(<WorkerThreadRefCard ref={REF} status="running" />);

    fireEvent.click(screen.getByRole('button'));

    const calls = dispatch.mock.calls;
    expect(calls.length).toBeGreaterThan(0);
    const action = calls[calls.length - 1][0] as { type: string; payload?: unknown };
    // Mirrors `setActiveThread`'s slice action: payload is the worker
    // thread id, which the Conversations page uses to swap the active
    // thread (parent → worker navigation).
    expect(action.payload).toBe('t-worker-1');
    dispatch.mockRestore();
  });
});
