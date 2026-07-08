/**
 * EditableFlowCanvas — live run overlay (Phase 3e).
 *
 * Drives the canvas through the public `FlowCanvas editable` entry point with a
 * mocked `socketService`, then simulates the core's `flow:run_progress` feed and
 * asserts the target node's live-status class flips on its React Flow wrapper —
 * n8n's signature running/success/error interaction. Also proves the overlay is
 * scoped to the watched run (a different run's event is ignored) and that with
 * no socket event the node carries no run class (the 2s poller fallback, tested
 * separately, remains the source of truth).
 */
import { act, render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowNode } from '../../../../lib/flows/graphAdapter';
import FlowCanvas from '../FlowCanvas';

const validateFlow = vi.hoisted(() => vi.fn());
const listFlowConnections = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/api/flowsApi', () => ({ validateFlow, listFlowConnections }));

// A tiny in-memory socket stand-in: `emit(event, payload)` fans out to every
// handler registered via `on`, and `off` removes them (so unmount cleanup is
// observable).
const socketHandlers = vi.hoisted(() => new Map<string, Set<(data: unknown) => void>>());
const socketOn = vi.hoisted(() =>
  vi.fn((event: string, cb: (data: unknown) => void) => {
    const set = socketHandlers.get(event) ?? new Set();
    set.add(cb);
    socketHandlers.set(event, set);
  })
);
const socketOff = vi.hoisted(() =>
  vi.fn((event: string, cb: (data: unknown) => void) => {
    socketHandlers.get(event)?.delete(cb);
  })
);
vi.mock('../../../../services/socketService', () => ({
  socketService: { on: socketOn, off: socketOff },
}));

function emitProgress(payload: { run_id: string; node_id: string; status: string }) {
  act(() => {
    for (const event of ['flow:run_progress', 'flow_run_progress']) {
      for (const cb of socketHandlers.get(event) ?? []) cb(payload);
    }
  });
}

function triggerNode(): FlowNode {
  return {
    id: 't',
    type: 'flowNode',
    position: { x: 0, y: 0 },
    data: {
      kind: 'trigger',
      name: 'Start',
      config: {},
      ports: [],
      inputPorts: ['main'],
      outputPorts: ['main'],
    },
  };
}

const META = { schema_version: 1, id: 'wf_1', name: 'My flow' } as const;

function renderCanvas(props: Partial<React.ComponentProps<typeof FlowCanvas>> = {}) {
  return render(
    <FlowCanvas
      editable
      nodes={[triggerNode()]}
      edges={[]}
      meta={META}
      onSave={vi.fn()}
      {...props}
    />
  );
}

function nodeWrapper(container: HTMLElement): Element | null {
  return container.querySelector('.react-flow__node[data-id="t"]');
}

describe('EditableFlowCanvas — live run overlay', () => {
  beforeEach(() => {
    socketHandlers.clear();
    socketOn.mockClear();
    socketOff.mockClear();
    validateFlow.mockReset();
    listFlowConnections.mockReset();
    listFlowConnections.mockResolvedValue([]);
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
  });

  it('flips the node run-status class as flow:run_progress events arrive', async () => {
    const { container } = renderCanvas({ activeRunId: 'run_1' });

    // The canvas subscribed to both event aliases for the active run.
    await waitFor(() =>
      expect(socketOn).toHaveBeenCalledWith('flow:run_progress', expect.any(Function))
    );
    expect(socketOn).toHaveBeenCalledWith('flow_run_progress', expect.any(Function));

    // No event yet → no run class.
    expect(nodeWrapper(container)).not.toHaveClass('flow-node-running');

    // running → pulsing ring.
    emitProgress({ run_id: 'run_1', node_id: 't', status: 'running' });
    await waitFor(() => expect(nodeWrapper(container)).toHaveClass('flow-node-running'));

    // success → sage ring (and the running class is gone).
    emitProgress({ run_id: 'run_1', node_id: 't', status: 'success' });
    await waitFor(() => expect(nodeWrapper(container)).toHaveClass('flow-node-success'));
    expect(nodeWrapper(container)).not.toHaveClass('flow-node-running');

    // error → coral run-failed ring.
    emitProgress({ run_id: 'run_1', node_id: 't', status: 'error' });
    await waitFor(() => expect(nodeWrapper(container)).toHaveClass('flow-node-failed'));
  });

  it('ignores progress for a different run id', async () => {
    const { container } = renderCanvas({ activeRunId: 'run_1' });
    await waitFor(() => expect(socketOn).toHaveBeenCalled());

    emitProgress({ run_id: 'other_run', node_id: 't', status: 'running' });
    // Give React a chance to (not) re-render.
    await Promise.resolve();
    expect(nodeWrapper(container)).not.toHaveClass('flow-node-running');
  });

  it('does not subscribe when there is no active run (poller-only fallback)', async () => {
    const { container } = renderCanvas();
    // No run → no socket subscription, and the node never carries a run class.
    expect(socketOn).not.toHaveBeenCalledWith('flow:run_progress', expect.any(Function));
    await waitFor(() => expect(nodeWrapper(container)).toBeInTheDocument());
    expect(nodeWrapper(container)).not.toHaveClass('flow-node-running');
  });

  it('unsubscribes on unmount', async () => {
    const { unmount } = renderCanvas({ activeRunId: 'run_1' });
    await waitFor(() => expect(socketOn).toHaveBeenCalled());
    unmount();
    expect(socketOff).toHaveBeenCalledWith('flow:run_progress', expect.any(Function));
    expect(socketOff).toHaveBeenCalledWith('flow_run_progress', expect.any(Function));
  });
});
