import { describe, expect, it } from 'vitest';

import {
  autoLayout,
  connectionEdgeId,
  createFlowNode,
  edgeId,
  type FlowEdge,
  type FlowNode,
  isValidFlowConnection,
  normalizeWorkflowGraphForDirtyCheck,
  stepNumbers,
  stepNumbersForFlow,
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

  describe('normalizeWorkflowGraphForDirtyCheck (F-m3)', () => {
    it('backfills auto-layout positions on a graph saved without them', () => {
      const withoutPositions = graph({
        nodes: [
          node({ id: 't', kind: 'trigger', name: 'Trigger' }),
          node({ id: 'a', name: 'Agent' }),
        ],
        edges: [edge({ from_node: 't', to_node: 'a' })],
      });
      const meta = { schema_version: 1, id: 'wf_1', name: 'demo' };

      const normalized = normalizeWorkflowGraphForDirtyCheck(withoutPositions, meta);

      // Every node has a concrete position — exactly what the canvas's own
      // mount-time `onGraphChange` would report back.
      for (const n of normalized.nodes) {
        expect(n.position).toBeDefined();
        expect(typeof n.position?.x).toBe('number');
        expect(typeof n.position?.y).toBe('number');
      }
      // And it's deterministic: normalizing again is a no-op (idempotent),
      // which is what lets a REMOUNTED canvas's `editorGraph` (already
      // normalized once) compare equal to `persistedGraphRef.current`
      // normalized fresh — the two happen at different points in time but
      // must still match.
      expect(normalizeWorkflowGraphForDirtyCheck(normalized, meta)).toEqual(normalized);
    });

    it('is a no-op (beyond re-stamping meta) for a graph that already carries positions', () => {
      const withPositions = graph({
        nodes: [
          node({ id: 't', kind: 'trigger', name: 'Trigger', position: { x: 40, y: 80 } }),
          node({ id: 'a', name: 'Agent', position: { x: 320, y: 80 } }),
        ],
        edges: [edge({ from_node: 't', to_node: 'a' })],
      });
      const meta = { schema_version: 1, id: 'wf_1', name: 'demo' };

      const normalized = normalizeWorkflowGraphForDirtyCheck(withPositions, meta);

      expect(normalized.nodes[0].position).toEqual({ x: 40, y: 80 });
      expect(normalized.nodes[1].position).toEqual({ x: 320, y: 80 });
    });

    it('lets a position-less graph and its own canvas-reported (positioned) copy compare equal once both are normalized', () => {
      // This is the exact F-m3 scenario: the server graph has no positions,
      // but the canvas always reports one back (via `workflowGraphToXyflow` +
      // `xyflowToWorkflowGraph`) the moment it mounts.
      const serverGraph = graph({
        nodes: [
          node({ id: 't', kind: 'trigger', name: 'Trigger' }),
          node({ id: 'a', name: 'Agent' }),
        ],
        edges: [edge({ from_node: 't', to_node: 'a' })],
      });
      const meta = { schema_version: 1, id: 'wf_1', name: 'demo' };
      const { nodes, edges } = workflowGraphToXyflow(serverGraph);
      const canvasReported = xyflowToWorkflowGraph(nodes, edges, meta);

      expect(JSON.stringify(normalizeWorkflowGraphForDirtyCheck(serverGraph, meta))).toBe(
        JSON.stringify(normalizeWorkflowGraphForDirtyCheck(canvasReported, meta))
      );
      // Without normalizing the server side, they would NOT compare equal —
      // pinning that this test is actually exercising the fix, not a tautology.
      expect(JSON.stringify(serverGraph)).not.toBe(JSON.stringify(canvasReported));
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

  describe('connectionEdgeId', () => {
    it('matches edgeId for the same 4-tuple (editor-created edges match adapter-created ones)', () => {
      const connection = { source: 'x', sourceHandle: 'p1', target: 'y', targetHandle: 'p2' };
      expect(connectionEdgeId(connection)).toBe(
        edgeId({ from_node: 'x', from_port: 'p1', to_node: 'y', to_port: 'p2' })
      );
    });

    it('does not collide on the same "-" boundary-shift case edgeId guards against (F-m6)', () => {
      // Same colliding node/port tuples as the `edgeId` test above, but coming
      // in as onConnect's live `Connection` shape (nullable handles) rather
      // than an already-resolved WorkflowEdge.
      const first = connectionEdgeId({
        source: 'a-b',
        sourceHandle: 'c',
        target: 'd',
        targetHandle: 'e',
      });
      const second = connectionEdgeId({
        source: 'a',
        sourceHandle: 'b-c',
        target: 'd',
        targetHandle: 'e',
      });
      expect(first).not.toBe(second);
    });

    it('defaults null/undefined handles to the "main" port, matching isValidFlowConnection', () => {
      const withNullHandles = connectionEdgeId({
        source: 'a',
        sourceHandle: null,
        target: 'b',
        targetHandle: null,
      });
      const withExplicitMain = connectionEdgeId({
        source: 'a',
        sourceHandle: 'main',
        target: 'b',
        targetHandle: 'main',
      });
      expect(withNullHandles).toBe(withExplicitMain);
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

    it('seeds a loop node with declared body/done output ports (fixed runtime routing)', () => {
      const created = createFlowNode('loop', { x: 0, y: 0 }, 'loop-0', 'Repeat');
      expect(created.data.ports).toEqual([{ name: 'body' }, { name: 'done' }]);
      expect(created.data.inputPorts).toEqual(['main']);
      expect(created.data.outputPorts).toEqual(['body', 'done']);
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

describe('stepNumbers', () => {
  it('numbers from the roots outward, not in declaration order', () => {
    // Declared tail-first on purpose: the agent is listed before the trigger
    // that feeds it, so a numbering that trusted array order would label the
    // agent "1". Graphs the copilot authors routinely come back unordered.
    const nodes = [node({ id: 'b', kind: 'agent' }), node({ id: 'a', kind: 'trigger' })];
    const edges = [edge({ from_node: 'a', to_node: 'b' })];

    const steps = stepNumbers(nodes, edges);

    expect(steps.get('a')).toBe(1);
    expect(steps.get('b')).toBe(2);
  });

  it('numbers a fan-out breadth-first so sibling branches get adjacent numbers', () => {
    // trigger → (x, y) → z. Depth-first would number one branch to its end
    // before starting the other, which reads wrong beside a columnar layout.
    const nodes = [
      node({ id: 't', kind: 'trigger' }),
      node({ id: 'x', kind: 'agent' }),
      node({ id: 'y', kind: 'agent' }),
      node({ id: 'z', kind: 'agent' }),
    ];
    const edges = [
      edge({ from_node: 't', to_node: 'x' }),
      edge({ from_node: 't', to_node: 'y' }),
      edge({ from_node: 'x', to_node: 'z' }),
    ];

    const steps = stepNumbers(nodes, edges);

    // Exact values, not a sorted set: sibling order is deterministic (adjacency
    // follows edge declaration order), and a sorted assertion would still pass
    // if the traversal reversed x and y.
    expect(steps.get('t')).toBe(1);
    expect(steps.get('x')).toBe(2);
    expect(steps.get('y')).toBe(3);
    expect(steps.get('z')).toBe(4);
  });

  it('still numbers nodes the walk cannot reach', () => {
    // A disconnected node and a pure cycle both have no zero-in-degree entry.
    // Every card must show an index, so these are appended after the reachable
    // ones rather than left undefined.
    const nodes = [
      node({ id: 't', kind: 'trigger' }),
      node({ id: 'orphan', kind: 'agent' }),
      node({ id: 'c1', kind: 'agent' }),
      node({ id: 'c2', kind: 'agent' }),
    ];
    const edges = [
      edge({ from_node: 'c1', to_node: 'c2' }),
      edge({ from_node: 'c2', to_node: 'c1' }),
    ];

    const steps = stepNumbers(nodes, edges);

    // Pin the declaration-order fallback exactly — a sorted comparison would
    // pass even if the fallback emitted them in some other order.
    expect(steps.size).toBe(4);
    expect(steps.get('t')).toBe(1);
    expect(steps.get('orphan')).toBe(2);
    expect(steps.get('c1')).toBe(3);
    expect(steps.get('c2')).toBe(4);
  });

  it('numbers the xyflow shape identically to the workflow shape', () => {
    // The editable canvas only ever holds the xyflow shape, so both entry
    // points must agree or a graph would renumber itself on save/reload.
    const wfNodes = [node({ id: 'a', kind: 'trigger' }), node({ id: 'b', kind: 'agent' })];
    const wfEdges = [edge({ from_node: 'a', to_node: 'b' })];
    const graph: WorkflowGraph = {
      schema_version: 1,
      id: 'wf',
      name: 'Flow',
      nodes: wfNodes,
      edges: wfEdges,
    };

    const { nodes: flowNodes, edges: flowEdges } = workflowGraphToXyflow(graph);

    expect(stepNumbersForFlow(flowNodes, flowEdges)).toEqual(stepNumbers(wfNodes, wfEdges));
  });

  it('keeps disconnected chains contiguous instead of interleaving them', () => {
    // Regression: seeding every root at once interleaved independent chains, so
    // `a → b` plus `c → d` numbered a=1, c=2, b=3, d=4 — drawing a second chain
    // renumbered the first one's steps underneath the user.
    const nodes = [
      node({ id: 'a', kind: 'trigger' }),
      node({ id: 'b', kind: 'agent' }),
      node({ id: 'c', kind: 'trigger' }),
      node({ id: 'd', kind: 'agent' }),
    ];
    const edges = [edge({ from_node: 'a', to_node: 'b' }), edge({ from_node: 'c', to_node: 'd' })];

    expect(stepNumbers(nodes, edges)).toEqual(
      new Map([
        ['a', 1],
        ['b', 2],
        ['c', 3],
        ['d', 4],
      ])
    );
  });

  it('does not renumber the existing flow when a second chain is drawn', () => {
    // The property that matters on the canvas: numbers already on screen must
    // not shift because unrelated work appeared elsewhere.
    const a = node({ id: 'a', kind: 'trigger' });
    const b = node({ id: 'b', kind: 'agent' });
    const firstChain = [edge({ from_node: 'a', to_node: 'b' })];

    const before = stepNumbers([a, b], firstChain);

    const after = stepNumbers(
      [a, b, node({ id: 'c', kind: 'trigger' }), node({ id: 'd', kind: 'agent' })],
      [...firstChain, edge({ from_node: 'c', to_node: 'd' })]
    );

    expect(after.get('a')).toBe(before.get('a'));
    expect(after.get('b')).toBe(before.get('b'));
  });

  it('renumbers when a node is added or connected mid-edit', () => {
    // Regression: numbers used to be baked into node `data` by
    // `workflowGraphToXyflow`, which the editable canvas runs only at mount.
    // A node added afterwards (via `createFlowNode`, which sets no number) had
    // none at all, and connecting it left every other number stale until a
    // save or remount.
    const a = createFlowNode('trigger', { x: 0, y: 0 }, 'a', 'Trigger');
    const b = createFlowNode('agent', { x: 280, y: 0 }, 'b', 'Agent');
    const connected: FlowEdge[] = [{ id: 'e1', source: 'a', target: 'b' }];

    expect(stepNumbersForFlow([a, b], connected)).toEqual(
      new Map([
        ['a', 1],
        ['b', 2],
      ])
    );

    // Add a third node the way the palette does — no number of its own.
    const c = createFlowNode('tool_call', { x: 560, y: 0 }, 'c', 'Tool');
    const afterAdd = stepNumbersForFlow([a, b, c], connected);
    expect(afterAdd.get('c')).toBe(3);

    // Connect it, and the numbering follows the new topology.
    const afterConnect = stepNumbersForFlow(
      [a, b, c],
      [...connected, { id: 'e2', source: 'b', target: 'c' }]
    );
    expect(afterConnect).toEqual(
      new Map([
        ['a', 1],
        ['b', 2],
        ['c', 3],
      ])
    );

    // Rewiring so `c` comes before `b` must renumber, not keep stale values.
    const rewired = stepNumbersForFlow(
      [a, b, c],
      [
        { id: 'e3', source: 'a', target: 'c' },
        { id: 'e4', source: 'c', target: 'b' },
      ]
    );
    expect(rewired.get('c')).toBe(2);
    expect(rewired.get('b')).toBe(3);
  });
});
