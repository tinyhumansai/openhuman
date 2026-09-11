/**
 * FlowNodeComponent — smoke tests for the custom xyflow node renderer.
 * Mounted through a real `<ReactFlow />` (same pattern as
 * `__tests__/FlowCanvas.test.tsx`) since the component renders `@xyflow/react`
 * `<Handle>`s that need a flow context. Asserts the node's name/kind/summary
 * render, and that the in-card action row (Validate/Delete — now `ui/Button`)
 * only appears when both `CanvasActionsContext` is provided and the node is
 * selected, and that each action fires through.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { ReactFlow, ReactFlowProvider } from '@xyflow/react';
import { describe, expect, it, vi } from 'vitest';

import type { FlowNode } from '../../../lib/flows/graphAdapter';
import { CanvasActionsContext } from './canvasActions';
import FlowNodeComponent from './FlowNodeComponent';

const nodeTypes = { flowNode: FlowNodeComponent };

function sampleNode(overrides: Partial<FlowNode> = {}): FlowNode {
  return {
    id: 'n1',
    type: 'flowNode',
    position: { x: 0, y: 0 },
    data: {
      kind: 'http_request',
      name: 'Fetch data',
      config: { method: 'GET', url: 'https://x.test' },
      ports: [],
      inputPorts: ['main'],
      outputPorts: ['main'],
    },
    ...overrides,
  };
}

function renderNode(
  node: FlowNode,
  actions?: React.ComponentProps<typeof CanvasActionsContext.Provider>['value']
) {
  const tree = (
    <ReactFlowProvider>
      <ReactFlow nodes={[node]} edges={[]} nodeTypes={nodeTypes} />
    </ReactFlowProvider>
  );
  return render(
    actions !== undefined ? (
      <CanvasActionsContext.Provider value={actions}>{tree}</CanvasActionsContext.Provider>
    ) : (
      tree
    )
  );
}

describe('FlowNodeComponent', () => {
  it('renders the node name, kind label, and config-derived summary', () => {
    renderNode(sampleNode());
    expect(screen.getByTestId('flow-node')).toHaveTextContent('Fetch data');
    expect(screen.getByTestId('flow-node')).toHaveTextContent('GET https://x.test');
  });

  it('shows no action row when there is no CanvasActionsContext (read-only viewer)', () => {
    renderNode(sampleNode({ selected: true }));
    expect(screen.queryByTestId('flow-node-validate')).not.toBeInTheDocument();
    expect(screen.queryByTestId('flow-node-delete')).not.toBeInTheDocument();
  });

  it('shows no action row when the node is not selected, even with actions available', () => {
    renderNode(sampleNode(), { deleteNode: vi.fn(), validate: vi.fn(), validating: false });
    expect(screen.queryByTestId('flow-node-validate')).not.toBeInTheDocument();
  });

  it('shows the action row and fires validate/delete when selected with actions available', () => {
    const deleteNode = vi.fn();
    const validate = vi.fn();
    renderNode(sampleNode({ selected: true }), { deleteNode, validate, validating: false });

    fireEvent.click(screen.getByTestId('flow-node-validate'));
    expect(validate).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTestId('flow-node-delete'));
    expect(deleteNode).toHaveBeenCalledWith('n1');
  });
});
