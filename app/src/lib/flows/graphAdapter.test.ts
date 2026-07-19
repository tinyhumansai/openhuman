import { describe, expect, it } from 'vitest';

import {
  autoLayout,
  createFlowNode,
  edgeId,
  type FlowEdge,
  type FlowNode,
  isValidFlowConnection,
  workflowGraphToXyflow,
  xyflowToWorkflowGraph,
} from './graphAdapter';
import type { NodeKind, WorkflowEdge, WorkflowGraph, WorkflowNode } from './types';

function node(overrides: Partial<WorkflowNode> = {}): WorkflowNode {
  return { id: 'n1', kind: 'agent', name: 'Agent', config: {}, ports: [], ...overrides };
}

function edge(overrides: Partial<WorkflowEdge> = {}): WorkflowEdge {
  return { from_node: 'a', from_port: 'main', to_node: 'b', to_port: 'main', ...overrides };
}

function graph(overrides: Partial<WorkflowGraph> = {}): WorkflowGraph {
  return { schema_version: 1, id: 'wf_1', name: 'demo', nodes: [], edges: [], ...overrides };
}

describe('graphAdapter', () => {
  describe('workflowGraphToXyflow', () => {
    it('returns empty nodes/edges for an empty graph', () => {
      const { nodes, edges } = workflowGraphToXyflow(graph());
      expect(nodes).toEqual([]);
      expect(edges).toEqual([]);
    });

    it('maps a node to a flowNode with kind/name/config/ports in data', () => {
      const g = graph({
        nodes: [
          node({
            id: 't',
            kind: 'trigger',
            name: 'Start',
            config: { mode: 'manual' },
            ports: [{ name: 'main' }],
            position: { x: 10, y: 20 },
          }),
        ],
      });
      const { nodes } = workflowGraphToXyflow(g);
      expect(nodes).toHaveLength(1);
      const [flowNode] = nodes;
      expect(flowNode.id).toBe('t');
      expect(flowNode.type).toBe('flowNode');
      expect(flowNode.position).toEqual({ x: 10, y: 20 });
      expect(flowNode.data.kind).toBe('trigger');
      expect(flowNode.data.name).toBe('Start');
      expect(flowNode.data.config).toEqual({ mode: 'manual' });
      expect(flowNode.data.ports).toEqual([{ name: 'main' }]);
    });

    it('maps edge handles: id, source/target, sourceHandle/targetHandle', () => {
      const g = graph({
        nodes: [node({ id: 'a' }), node({ id: 'b' })],
        edges: [edge({ from_node: 'a', from_port: 'true', to_node: 'b', to_port: 'in' })],
      });
      const { edges } = workflowGraphToXyflow(g);
      expect(edges).toEqual([
        {
          id: edgeId({ from_node: 'a', from_port: 'true', to_node: 'b', to_port: 'in' }),
          source: 'a',
          target: 'b',
          sourceHandle: 'true',
          targetHandle: 'in',
        },
      ]);
    });

    it('uses the saved position when present, without invoking auto-layout', () => {
      const g = graph({ nodes: [node({ id: 'a', position: { x: 500, y: 600 } })] });
      const { nodes } = workflowGraphToXyflow(g);
      expect(nodes[0].position).toEqual({ x: 500, y: 600 });
    });

    it('auto-lays-out nodes missing a position', () => {
      const g = graph({
        nodes: [node({ id: 't', kind: 'trigger' }), node({ id: 'a', kind: 'agent' })],
        edges: [edge({ from_node: 't', to_node: 'a' })],
      });
      const { nodes } = workflowGraphToXyflow(g);
      const byId = Object.fromEntries(nodes.map(n => [n.id, n.position]));
      expect(byId.t).toEqual({ x: 0, y: 0 });
      expect(byId.a).toEqual({ x: 0, y: 160 });
    });

    it('derives effective input/output ports for a switch node from its edges, not just declared ports', () => {
      const g = graph({
        nodes: [
          node({ id: 't', kind: 'trigger' }),
          node({ id: 'sw', kind: 'switch', ports: [] }),
          node({ id: 'a', kind: 'agent' }),
          node({ id: 'b', kind: 'agent' }),
        ],
        edges: [
          edge({ from_node: 't', from_port: 'main', to_node: 'sw', to_port: 'main' }),
          edge({ from_node: 'sw', from_port: 'case_a', to_node: 'a', to_port: 'main' }),
          edge({ from_node: 'sw', from_port: 'case_b', to_node: 'b', to_port: 'main' }),
        ],
      });
      const { nodes } = workflowGraphToXyflow(g);
      const sw = nodes.find(n => n.id === 'sw')!;
      expect(sw.data.inputPorts).toEqual(['main']);
      expect(sw.data.outputPorts).toEqual(['case_a', 'case_b']);
    });

    it('defaults to a single "main" input/output port for an unwired node', () => {
      const g = graph({ nodes: [node({ id: 'solo', ports: [] })] });
      const { nodes } = workflowGraphToXyflow(g);
      expect(nodes[0].data.inputPorts).toEqual(['main']);
      expect(nodes[0].data.outputPorts).toEqual(['main']);
    });
  });

  describe('xyflowToWorkflowGraph', () => {
    it('round-trips a graph through workflowGraphToXyflow and back', () => {
      const original = graph({
        nodes: [
          node({
            id: 't',
            kind: 'trigger',
            name: 'Start',
            config: { mode: 'manual' },
            ports: [],
            position: { x: 0, y: 0 },
          }),
          node({
            id: 'a',
            kind: 'agent',
            name: 'Reply',
            config: { prompt: 'hi' },
            ports: [{ name: 'main' }],
            position: { x: 280, y: 0 },
          }),
        ],
        edges: [edge({ from_node: 't', from_port: 'main', to_node: 'a', to_port: 'main' })],
      });

      const { nodes, edges } = workflowGraphToXyflow(original);
      const roundTripped = xyflowToWorkflowGraph(nodes, edges, {
        schema_version: original.schema_version,
        id: original.id,
        name: original.name,
      });

      expect(roundTripped).toEqual(original);
    });

    it('round-trips node ids and port names containing "-" without edge id collisions', () => {
      // Node "a-b"/port "c" -> node "d"/port "e" and node "a"/port "b-c" -> the
      // same target/port would produce the same joined string under a naive
      // `${a}-${b}-${c}-${d}` id scheme; both must still round-trip correctly.
      const original = graph({
        nodes: [
          node({ id: 'a-b', name: 'First', ports: [{ name: 'c' }], position: { x: 0, y: 0 } }),
          node({ id: 'a', name: 'Second', ports: [{ name: 'b-c' }], position: { x: 0, y: 160 } }),
          node({ id: 'd', name: 'Target', position: { x: 280, y: 0 } }),
        ],
        edges: [
          edge({ from_node: 'a-b', from_port: 'c', to_node: 'd', to_port: 'e' }),
          edge({ from_node: 'a', from_port: 'b-c', to_node: 'd', to_port: 'e' }),
        ],
      });

      const { nodes, edges } = workflowGraphToXyflow(original);
      // The two edges must not collide on id despite the ambiguous join.
      expect(edges[0].id).not.toBe(edges[1].id);
      expect(new Set(edges.map(e => e.id)).size).toBe(2);

      const roundTripped = xyflowToWorkflowGraph(nodes, edges, {
        schema_version: original.schema_version,
        id: original.id,
        name: original.name,
      });
      expect(roundTripped).toEqual(original);
    });

    it('round-trips a non-default type_version', () => {
      const original = graph({
        nodes: [node({ id: 't', kind: 'trigger', type_version: 3, position: { x: 0, y: 0 } })],
      });

      const { nodes, edges } = workflowGraphToXyflow(original);
      expect(nodes[0].data.type_version).toBe(3);

      const roundTripped = xyflowToWorkflowGraph(nodes, edges, {
        schema_version: original.schema_version,
        id: original.id,
        name: original.name,
      });
      expect(roundTripped.nodes[0].type_version).toBe(3);
      expect(roundTripped).toEqual(original);
    });

    it('reassembles graph-level metadata (schema_version/id/name) from the passed meta, not the nodes', () => {
      const result = xyflowToWorkflowGraph([], [], {
        schema_version: 1,
        id: 'wf_2',
        name: 'renamed',
      });
      expect(result).toEqual({
        schema_version: 1,
        id: 'wf_2',
        name: 'renamed',
        nodes: [],
        edges: [],
      });
    });

    it('defaults a missing sourceHandle/targetHandle to "main"', () => {
      const flowNodes: FlowNode[] = [
        {
          id: 'a',
          type: 'flowNode',
          position: { x: 0, y: 0 },
          data: {
            kind: 'agent',
            name: 'A',
            config: {},
            ports: [],
            inputPorts: ['main'],
            outputPorts: ['main'],
          },
        },
        {
          id: 'b',
          type: 'flowNode',
          position: { x: 0, y: 160 },
          data: {
            kind: 'agent',
            name: 'B',
            config: {},
            ports: [],
            inputPorts: ['main'],
            outputPorts: ['main'],
          },
        },
      ];
      const flowEdges: FlowEdge[] = [{ id: 'a-b', source: 'a', target: 'b' }];
      const result = xyflowToWorkflowGraph(flowNodes, flowEdges, {
        schema_version: 1,
        id: null,
        name: 'g',
      });
      expect(result.edges).toEqual([
        { from_node: 'a', from_port: 'main', to_node: 'b', to_port: 'main' },
      ]);
    });

    it('returns an empty graph for empty nodes/edges', () => {
      const result = xyflowToWorkflowGraph([], [], { schema_version: 1, id: undefined, name: '' });
      expect(result.nodes).toEqual([]);
      expect(result.edges).toEqual([]);
    });
  });

  describe('autoLayout', () => {
    it('returns an empty map for no nodes', () => {
      expect(autoLayout([], []).size).toBe(0);
    });

    it('lays out a linear chain by BFS depth from the trigger', () => {
      const nodes = [node({ id: 't', kind: 'trigger' }), node({ id: 'a' }), node({ id: 'b' })];
      const edges = [
        edge({ from_node: 't', to_node: 'a' }),
        edge({ from_node: 'a', to_node: 'b' }),
      ];
      const positions = autoLayout(nodes, edges);
      expect(positions.get('t')).toEqual({ x: 0, y: 0 });
      expect(positions.get('a')).toEqual({ x: 0, y: 160 });
      expect(positions.get('b')).toEqual({ x: 0, y: 320 });
    });

    it('places parallel branches at the same depth in separate columns', () => {
      const nodes = [node({ id: 't', kind: 'trigger' }), node({ id: 'a' }), node({ id: 'b' })];
      const edges = [
        edge({ from_node: 't', to_node: 'a' }),
        edge({ from_node: 't', to_node: 'b' }),
      ];
      const positions = autoLayout(nodes, edges);
      expect(positions.get('t')).toEqual({ x: 0, y: 0 });
      expect(positions.get('a')).toEqual({ x: 0, y: 160 });
      expect(positions.get('b')).toEqual({ x: 280, y: 160 });
    });

    it('gives every node a position, even a fully disconnected graph', () => {
      const nodes = [node({ id: 'a' }), node({ id: 'b' })];
      const positions = autoLayout(nodes, []);
      expect(positions.size).toBe(2);
      expect(positions.has('a')).toBe(true);
      expect(positions.has('b')).toBe(true);
    });

    it('does not throw on an edge referencing an id outside the node set', () => {
      const nodes = [node({ id: 'a' })];
      const edges = [edge({ from_node: 'a', to_node: 'ghost' })];
      expect(() => autoLayout(nodes, edges)).not.toThrow();
      expect(autoLayout(nodes, edges).get('a')).toEqual({ x: 0, y: 0 });
    });
  });

  describe('edgeId', () => {
    it('is deterministic for the same edge', () => {
      const e = edge({ from_node: 'x', from_port: 'p1', to_node: 'y', to_port: 'p2' });
      expect(edgeId(e)).toBe(edgeId({ ...e }));
    });

    it('does not collide when a "-" in a node id/port name could ambiguously shift the boundary', () => {
      // Node "a-b"/port "c" -> node "d"/port "e" vs. node "a"/port "b-c" ->
      // node "d"/port "e": a naive `${a}-${b}-${c}-${d}` join produces
      // "a-b-c-d-e" for both. `edgeId` must tell them apart.
      const first = edgeId({ from_node: 'a-b', from_port: 'c', to_node: 'd', to_port: 'e' });
      const second = edgeId({ from_node: 'a', from_port: 'b-c', to_node: 'd', to_port: 'e' });
      expect(first).not.toBe(second);
    });

    it('produces distinct ids for otherwise-identical edges differing only in one field', () => {
      const base = { from_node: 'a', from_port: 'main', to_node: 'b', to_port: 'main' };
      const ids = new Set([
        edgeId(base),
        edgeId({ ...base, from_node: 'a2' }),
        edgeId({ ...base, from_port: 'other' }),
        edgeId({ ...base, to_node: 'b2' }),
        edgeId({ ...base, to_port: 'other' }),
      ]);
      expect(ids.size).toBe(5);
    });
  });

  describe('createFlowNode', () => {
    it('builds a flowNode with a single default main input/output and empty config/ports', () => {
      const created = createFlowNode('agent', { x: 12, y: 34 }, 'new-agent-0', 'Agent');
      expect(created.id).toBe('new-agent-0');
      expect(created.type).toBe('flowNode');
      expect(created.position).toEqual({ x: 12, y: 34 });
      expect(created.data.kind).toBe('agent');
      expect(created.data.name).toBe('Agent');
      expect(created.data.config).toEqual({});
      expect(created.data.ports).toEqual([]);
      expect(created.data.inputPorts).toEqual(['main']);
      expect(created.data.outputPorts).toEqual(['main']);
    });

    it('falls back to the kind as the name when none is given', () => {
      const created = createFlowNode('http_request', { x: 0, y: 0 }, 'id1');
      expect(created.data.name).toBe('http_request');
    });

    it('seeds a condition node with declared true/false output ports (fixed runtime routing)', () => {
      const created = createFlowNode('condition', { x: 0, y: 0 }, 'cond-0', 'Branch');
      expect(created.data.ports).toEqual([{ name: 'true' }, { name: 'false' }]);
      expect(created.data.inputPorts).toEqual(['main']);
      expect(created.data.outputPorts).toEqual(['true', 'false']);
    });
  });

  describe('isValidFlowConnection', () => {
    // A trigger → agent pair, both with the default single `main` handle, as a
    // freshly palette-built canvas would produce.
    const nodes: FlowNode[] = [
      createFlowNode('trigger', { x: 0, y: 0 }, 't', 'Start'),
      createFlowNode('agent', { x: 280, y: 0 }, 'a', 'Reply'),
    ];

    it('accepts a main→main connection between two distinct nodes', () => {
      expect(
        isValidFlowConnection(
          { source: 't', target: 'a', sourceHandle: 'main', targetHandle: 'main' },
          nodes,
          []
        )
      ).toBe(true);
    });

    it('accepts a connection with null handles (defaults to main)', () => {
      expect(
        isValidFlowConnection(
          { source: 't', target: 'a', sourceHandle: null, targetHandle: null },
          nodes,
          []
        )
      ).toBe(true);
    });

    it('rejects a self-loop', () => {
      expect(
        isValidFlowConnection(
          { source: 't', target: 't', sourceHandle: 'main', targetHandle: 'main' },
          nodes,
          []
        )
      ).toBe(false);
    });

    it('rejects a missing endpoint', () => {
      expect(
        isValidFlowConnection({ source: 't', target: null, sourceHandle: 'main' }, nodes, [])
      ).toBe(false);
    });

    it('rejects an endpoint that is not on the canvas', () => {
      expect(
        isValidFlowConnection(
          { source: 't', target: 'ghost', sourceHandle: 'main', targetHandle: 'main' },
          nodes,
          []
        )
      ).toBe(false);
    });

    it('rejects an unknown source output port', () => {
      expect(
        isValidFlowConnection(
          { source: 't', target: 'a', sourceHandle: 'nonexistent', targetHandle: 'main' },
          nodes,
          []
        )
      ).toBe(false);
    });

    it('rejects an unknown target input port', () => {
      expect(
        isValidFlowConnection(
          { source: 't', target: 'a', sourceHandle: 'main', targetHandle: 'nonexistent' },
          nodes,
          []
        )
      ).toBe(false);
    });

    it('rejects a duplicate of an edge already present', () => {
      const existing: FlowEdge[] = [
        { id: 'e1', source: 't', target: 'a', sourceHandle: 'main', targetHandle: 'main' },
      ];
      expect(
        isValidFlowConnection(
          { source: 't', target: 'a', sourceHandle: 'main', targetHandle: 'main' },
          nodes,
          existing
        )
      ).toBe(false);
    });
  });

  describe('palette-built graph round-trips through xyflowToWorkflowGraph', () => {
    it('serializes click-added nodes + a valid connection back into a WorkflowGraph', () => {
      const kinds: NodeKind[] = ['trigger', 'agent'];
      const built = kinds.map((kind, i) =>
        createFlowNode(kind, { x: i * 280, y: 0 }, `new-${kind}-${i}`, kind)
      );
      const connection = {
        source: 'new-trigger-0',
        target: 'new-agent-1',
        sourceHandle: 'main',
        targetHandle: 'main',
      };
      expect(isValidFlowConnection(connection, built, [])).toBe(true);

      const edges: FlowEdge[] = [{ id: 'e', ...connection }];
      const result = xyflowToWorkflowGraph(built, edges, {
        schema_version: 1,
        id: 'wf_new',
        name: 'Fresh flow',
      });

      expect(result.schema_version).toBe(1);
      expect(result.id).toBe('wf_new');
      expect(result.name).toBe('Fresh flow');
      expect(result.nodes.map(n => n.kind)).toEqual(['trigger', 'agent']);
      expect(result.nodes.every(n => n.config && Array.isArray(n.ports))).toBe(true);
      expect(result.edges).toEqual([
        { from_node: 'new-trigger-0', from_port: 'main', to_node: 'new-agent-1', to_port: 'main' },
      ]);
    });
  });
});
