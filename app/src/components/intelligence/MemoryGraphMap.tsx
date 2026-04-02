import { useCallback, useEffect, useMemo, useState } from 'react';

import type { GraphRelation } from '../../utils/tauriCommands';

interface MemoryGraphMapProps {
  relations: GraphRelation[];
  loading?: boolean;
}

interface GraphNode {
  id: string;
  label: string;
  namespace: string | null;
  connectionCount: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface GraphEdge {
  source: string;
  target: string;
  predicate: string;
}

const NAMESPACE_COLORS = [
  '#4A83DD', // ocean blue
  '#4DC46F', // sage green
  '#E8A838', // amber
  '#9B8AFB', // lavender
  '#F56565', // coral
  '#7DD3FC', // sky
  '#FDA4AF', // rose
  '#6EE7B7', // mint
];

const WIDTH = 800;
const HEIGHT = 500;
const MAX_NODES = 100;
const MAX_EDGES = 200;

function truncate(s: string, max = 15): string {
  return s.length > max ? s.slice(0, max - 1) + '…' : s;
}

function buildGraph(relations: GraphRelation[]): { nodes: GraphNode[]; edges: GraphEdge[] } {
  // Cap edges by evidence count descending
  const sorted = [...relations].sort((a, b) => b.evidenceCount - a.evidenceCount);
  const cappedRelations = sorted.slice(0, MAX_EDGES);

  // Collect unique entity ids
  const entitySet = new Map<string, { namespace: string | null; count: number }>();

  for (const r of cappedRelations) {
    const subKey = r.subject.toLowerCase();
    const objKey = r.object.toLowerCase();

    const existing = entitySet.get(subKey);
    entitySet.set(subKey, { namespace: r.namespace, count: (existing?.count ?? 0) + 1 });

    const existingObj = entitySet.get(objKey);
    entitySet.set(objKey, {
      namespace: existingObj?.namespace ?? r.namespace,
      count: (existingObj?.count ?? 0) + 1,
    });
  }

  // Sort by connection count, cap at MAX_NODES
  const sortedEntities = [...entitySet.entries()].sort((a, b) => b[1].count - a[1].count);
  const cappedEntities = sortedEntities.slice(0, MAX_NODES);
  const allowedIds = new Set(cappedEntities.map(([id]) => id));

  const nodes: GraphNode[] = cappedEntities.map(([id, info]) => ({
    id,
    label: id,
    namespace: info.namespace,
    connectionCount: info.count,
    x: 80 + Math.random() * (WIDTH - 160),
    y: 80 + Math.random() * (HEIGHT - 160),
    vx: 0,
    vy: 0,
  }));

  const edges: GraphEdge[] = cappedRelations
    .filter(r => allowedIds.has(r.subject.toLowerCase()) && allowedIds.has(r.object.toLowerCase()))
    .map(r => ({
      source: r.subject.toLowerCase(),
      target: r.object.toLowerCase(),
      predicate: r.predicate,
    }));

  return { nodes, edges };
}

function runSimulation(nodes: GraphNode[], edges: GraphEdge[], iterations = 150): GraphNode[] {
  const nodeMap = new Map(nodes.map(n => [n.id, { ...n }]));
  const nodeList = [...nodeMap.values()];

  const REPULSION = 3500;
  const ATTRACTION = 0.04;
  const CENTER_FORCE = 0.012;
  const DAMPING = 0.75;
  const cx = WIDTH / 2;
  const cy = HEIGHT / 2;

  for (let iter = 0; iter < iterations; iter++) {
    for (let i = 0; i < nodeList.length; i++) {
      for (let j = i + 1; j < nodeList.length; j++) {
        const a = nodeList[i];
        const b = nodeList[j];
        const dx = b.x - a.x || 0.01;
        const dy = b.y - a.y || 0.01;
        const dist2 = dx * dx + dy * dy;
        const force = REPULSION / (dist2 + 1);
        const fx = (dx / Math.sqrt(dist2 + 1)) * force;
        const fy = (dy / Math.sqrt(dist2 + 1)) * force;
        a.vx -= fx;
        a.vy -= fy;
        b.vx += fx;
        b.vy += fy;
      }
    }
    for (const edge of edges) {
      const a = nodeMap.get(edge.source);
      const b = nodeMap.get(edge.target);
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const delta = dist - 120;
      const fx = (dx / dist) * delta * ATTRACTION;
      const fy = (dy / dist) * delta * ATTRACTION;
      a.vx += fx;
      a.vy += fy;
      b.vx -= fx;
      b.vy -= fy;
    }
    for (const n of nodeList) {
      n.vx += (cx - n.x) * CENTER_FORCE;
      n.vy += (cy - n.y) * CENTER_FORCE;
      n.vx *= DAMPING;
      n.vy *= DAMPING;
      n.x = Math.max(40, Math.min(WIDTH - 40, n.x + n.vx));
      n.y = Math.max(40, Math.min(HEIGHT - 40, n.y + n.vy));
    }
  }

  return nodeList;
}

export function MemoryGraphMap({ relations, loading }: MemoryGraphMapProps) {
  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [edges, setEdges] = useState<GraphEdge[]>([]);
  const [hoveredEdge, setHoveredEdge] = useState<number | null>(null);
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [namespacePalette, setNamespacePalette] = useState<Map<string, string>>(new Map());
  // Build graph data from relations (synchronous, deterministic)
  const { initialNodes, initialEdges, palette } = useMemo(() => {
    if (relations.length === 0) {
      return {
        initialNodes: [] as GraphNode[],
        initialEdges: [] as GraphEdge[],
        palette: new Map<string, string>(),
      };
    }
    const { nodes: rawNodes, edges: rawEdges } = buildGraph(relations);
    const namespaces = [...new Set(rawNodes.map(n => n.namespace ?? '__none__'))];
    const p = new Map<string, string>();
    namespaces.forEach((ns, i) => {
      p.set(ns, NAMESPACE_COLORS[i % NAMESPACE_COLORS.length]);
    });
    const simulated = runSimulation(rawNodes, rawEdges);
    return { initialNodes: simulated, initialEdges: rawEdges, palette: p };
  }, [relations]);

  // Sync memo results into state (needed for interactive selection/hover)
  useEffect(() => {
    setNodes(initialNodes);
    setEdges(initialEdges);
    setNamespacePalette(palette);
  }, [initialNodes, initialEdges, palette]);

  const getNodeColor = useCallback(
    (node: GraphNode): string => {
      const ns = node.namespace ?? '__none__';
      return namespacePalette.get(ns) ?? NAMESPACE_COLORS[0];
    },
    [namespacePalette]
  );

  const nodeMap = new Map(nodes.map(n => [n.id, n]));

  const centerNodeId =
    nodes.find(n => n.id === 'user' || n.id === 'self' || n.id === 'you')?.id ??
    (nodes.length > 0 ? nodes[0].id : null);

  // Connected node ids for selected highlight
  const connectedIds = selectedNode
    ? new Set(
        edges
          .filter(e => e.source === selectedNode || e.target === selectedNode)
          .flatMap(e => [e.source, e.target])
      )
    : null;

  const namespaceEntries = [...namespacePalette.entries()].filter(([ns]) => ns !== '__none__');

  if (loading) {
    return (
      <div className="rounded-xl border border-white/10 bg-black/20 p-4">
        <p className="text-sm font-semibold text-white mb-3">Memory Graph</p>
        <div className="flex items-center justify-center" style={{ minHeight: 320 }}>
          <div className="flex gap-2 items-center text-stone-400 text-sm">
            <div className="w-4 h-4 rounded-full border-2 border-primary-500 border-t-transparent animate-spin" />
            Loading graph…
          </div>
        </div>
      </div>
    );
  }

  if (nodes.length === 0) {
    return (
      <div className="rounded-xl border border-white/10 bg-black/20 p-4">
        <p className="text-sm font-semibold text-white mb-3">Memory Graph</p>
        <div className="flex items-center justify-center" style={{ minHeight: 320 }}>
          <p className="text-stone-400 text-sm">No memory graph data yet</p>
        </div>
      </div>
    );
  }

  const maxConn = Math.max(...nodes.map(n => n.connectionCount), 1);

  return (
    <div className="rounded-xl border border-white/10 bg-black/20 p-4">
      <p className="text-sm font-semibold text-white mb-3">Memory Graph</p>

      <div
        className="w-full overflow-hidden rounded-lg border border-white/5"
        style={{ minHeight: 320 }}>
        <svg
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          width="100%"
          style={{ display: 'block', background: 'rgba(0,0,0,0.25)' }}
          onClick={() => setSelectedNode(null)}>
          <defs>
            <marker id="arrowhead" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
              <polygon points="0 0, 8 3, 0 6" fill="rgba(255,255,255,0.18)" />
            </marker>
          </defs>

          {/* Edges */}
          {edges.map((edge, i) => {
            const src = nodeMap.get(edge.source);
            const tgt = nodeMap.get(edge.target);
            if (!src || !tgt) return null;

            const isHighlighted =
              selectedNode === null || edge.source === selectedNode || edge.target === selectedNode;

            const midX = (src.x + tgt.x) / 2;
            const midY = (src.y + tgt.y) / 2;

            return (
              <g key={i}>
                <line
                  x1={src.x}
                  y1={src.y}
                  x2={tgt.x}
                  y2={tgt.y}
                  stroke={isHighlighted ? 'rgba(255,255,255,0.22)' : 'rgba(255,255,255,0.05)'}
                  strokeWidth={isHighlighted ? 1.5 : 1}
                  markerEnd="url(#arrowhead)"
                  style={{ cursor: 'pointer', transition: 'stroke 0.15s' }}
                  onMouseEnter={() => setHoveredEdge(i)}
                  onMouseLeave={() => setHoveredEdge(null)}
                />
                {/* Edge label */}
                <text
                  x={midX}
                  y={midY - 4}
                  textAnchor="middle"
                  fontSize={9}
                  fill={
                    hoveredEdge === i
                      ? 'rgba(255,255,255,0.85)'
                      : isHighlighted
                        ? 'rgba(255,255,255,0.3)'
                        : 'rgba(255,255,255,0.08)'
                  }
                  style={{ pointerEvents: 'none', userSelect: 'none', transition: 'fill 0.15s' }}>
                  {truncate(edge.predicate, 18)}
                </text>
              </g>
            );
          })}

          {/* Nodes */}
          {nodes.map(node => {
            const r = 8 + (node.connectionCount / maxConn) * 18;
            const color = getNodeColor(node);
            const isCenter = node.id === centerNodeId;
            const isSelected = selectedNode === node.id;
            const isDimmed = selectedNode !== null && !connectedIds?.has(node.id);

            return (
              <g
                key={node.id}
                transform={`translate(${node.x},${node.y})`}
                style={{ cursor: 'pointer' }}
                onClick={e => {
                  e.stopPropagation();
                  setSelectedNode(selectedNode === node.id ? null : node.id);
                }}>
                {(isCenter || isSelected) && (
                  <circle r={r + 5} fill="none" stroke={color} strokeWidth={2} opacity={0.4} />
                )}
                <circle
                  r={r}
                  fill={color}
                  opacity={isDimmed ? 0.15 : isSelected ? 1 : 0.82}
                  stroke={isSelected ? 'white' : 'rgba(255,255,255,0.18)'}
                  strokeWidth={isSelected ? 2 : 1}
                  style={{ transition: 'opacity 0.15s' }}
                />
                <text
                  y={r + 11}
                  textAnchor="middle"
                  fontSize={isCenter ? 11 : 9}
                  fontWeight={isCenter ? 600 : 400}
                  fill={isDimmed ? 'rgba(255,255,255,0.2)' : 'rgba(255,255,255,0.85)'}
                  style={{ pointerEvents: 'none', userSelect: 'none', transition: 'fill 0.15s' }}>
                  {isCenter && node.id !== 'you' ? 'You' : truncate(node.label)}
                </text>
              </g>
            );
          })}
        </svg>
      </div>

      {/* Legend */}
      {namespaceEntries.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1.5">
          {namespaceEntries.map(([ns, color]) => (
            <div key={ns} className="flex items-center gap-1.5">
              <div
                className="w-2.5 h-2.5 rounded-full flex-shrink-0"
                style={{ backgroundColor: color }}
              />
              <span className="text-xs text-stone-400 truncate max-w-[120px]">{ns}</span>
            </div>
          ))}
          {namespacePalette.has('__none__') && (
            <div className="flex items-center gap-1.5">
              <div
                className="w-2.5 h-2.5 rounded-full flex-shrink-0"
                style={{ backgroundColor: namespacePalette.get('__none__') }}
              />
              <span className="text-xs text-stone-400">uncategorized</span>
            </div>
          )}
        </div>
      )}

      <p className="mt-2 text-xs text-stone-500">
        {nodes.length} entities · {edges.length} relations · click a node to highlight connections
      </p>
    </div>
  );
}
