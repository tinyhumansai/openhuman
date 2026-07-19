/**
 * FlowCanvas (issue B5b.1) — smoke tests for the read-only Workflow Canvas
 * wrapper around `@xyflow/react`'s `<ReactFlow />`. Asserts it mounts with a
 * sample node/edge set (no crash — `@xyflow/react` needs a measurable
 * container, which jsdom doesn't provide, so nodes render at 0x0 but the DOM
 * tree, `nodeTypes` wiring, and the minimap/controls/background chrome are
 * all still verifiable) and that it renders each node via `FlowNodeComponent`
 * (by asserting the node's name text appears).
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { FlowEdge, FlowNode, FlowNodeData } from '../../../../lib/flows/graphAdapter';
import type { NodeKind } from '../../../../lib/flows/types';
import FlowCanvas from '../FlowCanvas';

function sampleNodes(): FlowNode[] {
  return [
    {
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
    },
    {
      id: 'a',
      type: 'flowNode',
      position: { x: 280, y: 0 },
      data: {
        kind: 'agent',
        name: 'Reply',
        config: {},
        ports: [],
        inputPorts: ['main'],
        outputPorts: ['main'],
      },
    },
  ];
}

function sampleEdges(): FlowEdge[] {
  return [
    { id: 't-main-a-main', source: 't', target: 'a', sourceHandle: 'main', targetHandle: 'main' },
  ];
}

describe('FlowCanvas', () => {
  it('renders without crashing given sample nodes/edges', () => {
    render(<FlowCanvas nodes={sampleNodes()} edges={sampleEdges()} />);
    expect(screen.getByTestId('flow-canvas')).toBeInTheDocument();
  });

  it('renders each node via FlowNodeComponent (shows the node names)', () => {
    render(<FlowCanvas nodes={sampleNodes()} edges={sampleEdges()} />);
    expect(screen.getByText('Start')).toBeInTheDocument();
    expect(screen.getByText('Reply')).toBeInTheDocument();
  });

  it("labels named output ports (e.g. a condition's true/false) instead of a plaintext dump", () => {
    const conditionNode: FlowNode = {
      id: 'c',
      type: 'flowNode',
      position: { x: 0, y: 0 },
      data: {
        kind: 'condition',
        name: 'Check status',
        config: {},
        ports: [],
        inputPorts: ['main'],
        outputPorts: ['true', 'false'],
      } satisfies FlowNodeData,
    };
    render(<FlowCanvas nodes={[conditionNode]} edges={[]} />);
    expect(screen.getByText('true')).toBeInTheDocument();
    expect(screen.getByText('false')).toBeInTheDocument();
  });

  it('does not label a lone implicit main port (just its handle dot)', () => {
    render(<FlowCanvas nodes={sampleNodes()} edges={sampleEdges()} />);
    // A plain agent/trigger node with a single `main` in/out shows no "main" text.
    expect(screen.queryByText('main')).not.toBeInTheDocument();
  });

  it('renders the minimap and zoom/pan controls', () => {
    const { container } = render(<FlowCanvas nodes={sampleNodes()} edges={sampleEdges()} />);
    expect(container.querySelector('.react-flow__minimap')).not.toBeNull();
    expect(container.querySelector('.react-flow__controls')).not.toBeNull();
    expect(container.querySelector('.react-flow__background')).not.toBeNull();
  });

  it('renders an empty canvas with no nodes/edges', () => {
    render(<FlowCanvas nodes={[]} edges={[]} />);
    expect(screen.getByTestId('flow-canvas')).toBeInTheDocument();
  });

  it('renders a node with an unrecognized kind as a plain node instead of crashing', () => {
    // `Flow.graph` is `unknown` on the wire — a future 13th tinyflows kind (or
    // any unexpected value) must not crash the whole canvas (no error
    // boundary wraps `<ReactFlow>`). Cast past the `NodeKind` union the same
    // way a real cast-from-`unknown` graph would.
    const unknownKindNode: FlowNode = {
      id: 'mystery',
      type: 'flowNode',
      position: { x: 0, y: 0 },
      data: {
        kind: 'time_travel' as unknown as NodeKind,
        name: 'Mystery node',
        config: {},
        ports: [],
        inputPorts: ['main'],
        outputPorts: ['main'],
      } satisfies FlowNodeData,
    };

    render(<FlowCanvas nodes={[unknownKindNode]} edges={[]} />);

    expect(screen.getByTestId('flow-canvas')).toBeInTheDocument();
    expect(screen.getByText('Mystery node')).toBeInTheDocument();
  });
});
