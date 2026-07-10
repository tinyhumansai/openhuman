/**
 * Behavior tests for NodeConfigDrawer (Phase 3b): renders the selected node's
 * name + per-kind form, edits flow back through `onChange` (controlled), the
 * raw-JSON escape hatch toggles/parses, and close fires `onClose`. `useT()`
 * falls back to the bundled English map with no provider mounted.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { FlowNode } from '../../../../../lib/flows/graphAdapter';
import NodeConfigDrawer from '../NodeConfigDrawer';

function makeNode(overrides: Partial<FlowNode['data']> = {}): FlowNode {
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
      ...overrides,
    },
  };
}

describe('NodeConfigDrawer', () => {
  it('renders nothing when no node is selected', () => {
    const { container } = render(
      <NodeConfigDrawer node={null} onClose={vi.fn()} onChange={vi.fn()} connections={[]} />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('renders the node name and its per-kind form', () => {
    render(
      <NodeConfigDrawer node={makeNode()} onClose={vi.fn()} onChange={vi.fn()} connections={[]} />
    );
    expect(screen.getByTestId('node-config-drawer')).toBeInTheDocument();
    expect(screen.getByTestId('node-config-name')).toHaveValue('Fetch data');
    // http_request form fields are present.
    expect(screen.getByTestId('node-config-http-method')).toBeInTheDocument();
  });

  it("lists the node's input/output connections and removes one", () => {
    const onRemoveEdge = vi.fn();
    render(
      <NodeConfigDrawer
        node={makeNode()}
        onClose={vi.fn()}
        onChange={vi.fn()}
        connections={[]}
        edges={[
          { id: 'e-in', source: 't', target: 'n1', sourceHandle: 'main', targetHandle: 'main' },
          { id: 'e-out', source: 'n1', target: 'a', sourceHandle: 'main', targetHandle: 'main' },
        ]}
        nodeLabelById={{ t: 'Start', n1: 'Fetch data', a: 'Reply' }}
        onRemoveEdge={onRemoveEdge}
      />
    );
    // Incoming edge shows the source node; outgoing shows the target node.
    expect(screen.getByTestId('node-connections-inputs')).toHaveTextContent('Start');
    expect(screen.getByTestId('node-connections-outputs')).toHaveTextContent('Reply');

    fireEvent.click(screen.getByTestId('node-connection-remove-e-out'));
    expect(onRemoveEdge).toHaveBeenCalledWith('e-out');
  });

  it('shows an empty connections state for an unconnected node', () => {
    render(
      <NodeConfigDrawer node={makeNode()} onClose={vi.fn()} onChange={vi.fn()} connections={[]} />
    );
    expect(screen.getByTestId('node-connections-empty')).toBeInTheDocument();
  });

  it('shows a dynamic, config-derived description of the node', () => {
    render(
      <NodeConfigDrawer
        node={makeNode({ config: { method: 'POST', url: 'https://x.test/hook' } })}
        onClose={vi.fn()}
        onChange={vi.fn()}
        connections={[]}
      />
    );
    expect(screen.getByTestId('node-config-summary')).toHaveTextContent('POST https://x.test/hook');
  });

  it('emits a name patch when the name is edited', () => {
    const onChange = vi.fn();
    render(
      <NodeConfigDrawer node={makeNode()} onClose={onChange} onChange={onChange} connections={[]} />
    );
    fireEvent.change(screen.getByTestId('node-config-name'), { target: { value: 'Renamed' } });
    expect(onChange).toHaveBeenCalledWith('n1', { name: 'Renamed' });
  });

  it('merges a form edit into the existing config', () => {
    const onChange = vi.fn();
    render(
      <NodeConfigDrawer node={makeNode()} onClose={vi.fn()} onChange={onChange} connections={[]} />
    );
    fireEvent.change(screen.getByTestId('node-config-http-method'), { target: { value: 'POST' } });
    // Existing url is preserved; only method changes.
    expect(onChange).toHaveBeenCalledWith('n1', {
      config: { method: 'POST', url: 'https://x.test' },
    });
  });

  it('toggles to the raw-JSON editor and replaces config on valid JSON', () => {
    const onChange = vi.fn();
    render(
      <NodeConfigDrawer node={makeNode()} onClose={vi.fn()} onChange={onChange} connections={[]} />
    );
    fireEvent.click(screen.getByTestId('node-config-raw-toggle'));
    const editor = screen.getByTestId('node-config-raw-json');
    fireEvent.change(editor, { target: { value: '{"method":"DELETE"}' } });
    expect(onChange).toHaveBeenLastCalledWith('n1', { config: { method: 'DELETE' } });
  });

  it('uses the raw-JSON editor by default for kinds without a dedicated form', () => {
    render(
      <NodeConfigDrawer
        node={makeNode({ kind: 'merge', name: 'Join', config: {} })}
        onClose={vi.fn()}
        onChange={vi.fn()}
        connections={[]}
      />
    );
    expect(screen.getByTestId('node-config-raw-json')).toBeInTheDocument();
    // No form/raw toggle for kinds that only have the raw editor.
    expect(screen.queryByTestId('node-config-raw-toggle')).not.toBeInTheDocument();
  });

  it('fires onClose from the close button', () => {
    const onClose = vi.fn();
    render(
      <NodeConfigDrawer node={makeNode()} onClose={onClose} onChange={vi.fn()} connections={[]} />
    );
    fireEvent.click(screen.getByTestId('node-config-close'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
