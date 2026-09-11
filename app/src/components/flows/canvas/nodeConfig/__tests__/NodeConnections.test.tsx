/**
 * Behavior tests for NodeConnections: renders incoming/outgoing edges as rows
 * and fires `onRemoveEdge` when a row's remove button is clicked. `useT()`
 * falls back to the bundled English map with no provider mounted (same as
 * the sibling nodeConfig tests).
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { FlowEdge } from '../../../../../lib/flows/graphAdapter';
import { NodeConnections } from '../NodeConnections';

const edges: FlowEdge[] = [
  { id: 'e-in', source: 't', target: 'n1', sourceHandle: 'main', targetHandle: 'main' },
  { id: 'e-out', source: 'n1', target: 'a', sourceHandle: 'main', targetHandle: 'main' },
];

describe('NodeConnections', () => {
  it('shows an empty state when the node has no edges', () => {
    render(<NodeConnections nodeId="n1" edges={[]} nodeLabelById={{}} onRemoveEdge={vi.fn()} />);
    expect(screen.getByTestId('node-connections-empty')).toBeInTheDocument();
  });

  it('lists incoming and outgoing edges with the other node label', () => {
    render(
      <NodeConnections
        nodeId="n1"
        edges={edges}
        nodeLabelById={{ t: 'Start', n1: 'Fetch data', a: 'Reply' }}
        onRemoveEdge={vi.fn()}
      />
    );
    expect(screen.getByTestId('node-connections-inputs')).toHaveTextContent('Start');
    expect(screen.getByTestId('node-connections-outputs')).toHaveTextContent('Reply');
  });

  it('fires onRemoveEdge with the edge id when a row remove button is clicked', () => {
    const onRemoveEdge = vi.fn();
    render(
      <NodeConnections
        nodeId="n1"
        edges={edges}
        nodeLabelById={{ t: 'Start', n1: 'Fetch data', a: 'Reply' }}
        onRemoveEdge={onRemoveEdge}
      />
    );
    const removeButton = screen.getByTestId('node-connection-remove-e-out');
    expect(removeButton).toHaveAttribute('data-slot', 'button');
    fireEvent.click(removeButton);
    expect(onRemoveEdge).toHaveBeenCalledWith('e-out');
  });
});
