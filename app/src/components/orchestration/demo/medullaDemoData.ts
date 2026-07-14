/**
 * medullaDemoData — deterministic, illustrative data for the Orchestration
 * "scale showcase" shown to users without Medulla access.
 *
 * Everything here is fake, generated from a fixed seed so the layout is stable
 * across renders (no re-simulation thrash). Labels are identifier-style tokens
 * (agent / session ids), not translatable prose — the only translated copy in
 * the demo lives in the surrounding chrome (banner, headings, task verbs).
 */
import type { GraphEdge, GraphNode } from '../../../utils/tauriCommands';

/** mulberry32 — tiny deterministic PRNG so the demo is stable per render. */
function makeRng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const OFFLINE_COLOR = '#6B7280';

/** Vibrant palette for "active" sub-agents — the noise/chaos of live work. */
const ACTIVE_COLORS = [
  '#4A83DD', // ocean (primary)
  '#22C55E', // sage
  '#F59E0B', // amber
  '#EF4444', // coral
  '#8B5CF6', // violet
  '#06B6D4', // cyan
  '#EC4899', // pink
];

function hex4(rng: () => number): string {
  return Math.floor(rng() * 0xffff)
    .toString(16)
    .padStart(4, '0');
}

// ── Demo agent graph ───────────────────────────────────────────────────────
//
// A central **agent core** with two **devices** hanging off it, and a busy
// fan-out of agents under each device (100 on the first, 20 on the second).
// Devices are `source` nodes (auto-linked to the synthetic core by the layout
// engine); agents are `chunk` nodes linked to their device by an explicit edge.
// The live MemoryGraph force engine lays it out — the orchestration overview
// passes a high-repulsion tuning so the 120 nodes fan out wide from the core.

/** How many agents fan out from each device (device 1 = 100, device 2 = 20). */
const DEVICE_AGENT_COUNTS = [100, 20] as const;

/**
 * Build the demo agent graph as force-engine nodes/edges.
 *
 * @param deviceLabel Localised word for "Device" — devices are labelled
 *   `${deviceLabel} 1` / `${deviceLabel} 2`.
 */
export function buildDemoGraph(deviceLabel: string): { nodes: GraphNode[]; edges: GraphEdge[] } {
  const rng = makeRng(0xc0ffee); // fixed seed → stable layout
  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];

  DEVICE_AGENT_COUNTS.forEach((agentCount, d) => {
    const deviceId = `device:${d}`;
    nodes.push({ kind: 'source', id: deviceId, label: `${deviceLabel} ${d + 1}` });

    for (let a = 0; a < agentCount; a++) {
      const online = rng() > 0.22;
      const color = online
        ? ACTIVE_COLORS[Math.floor(rng() * ACTIVE_COLORS.length)]
        : OFFLINE_COLOR;
      const agentId = `agent:${d}:${a}`;
      nodes.push({ kind: 'chunk', id: agentId, label: `0x${hex4(rng)}`, color });
      edges.push({ from: agentId, to: deviceId });
    }
  });

  return { nodes, edges };
}

// ── Demo peer network ──────────────────────────────────────────────────────

export type DemoPeerStatus = 'connected' | 'connecting' | 'idle';

export interface DemoPeer {
  id: string;
  address: string;
  status: DemoPeerStatus;
  /** Number of live sub-agents under this peer. */
  sessions: number;
}

/** ~36 peer agents with mixed connection states for the network showcase. */
export function buildDemoPeers(): DemoPeer[] {
  const rng = makeRng(0x1d3a);
  const peers: DemoPeer[] = [];
  for (let i = 0; i < 36; i++) {
    const r = rng();
    const status: DemoPeerStatus = r < 0.62 ? 'connected' : r < 0.85 ? 'connecting' : 'idle';
    peers.push({
      id: `peer-${i}`,
      address: `0x${hex4(rng)}…${hex4(rng)}`,
      status,
      sessions: status === 'connected' ? 1 + Math.floor(rng() * 12) : 0,
    });
  }
  return peers;
}

// ── Demo chat transcript ───────────────────────────────────────────────────

export type DemoChatRole = 'user' | 'assistant' | 'activity';

export interface DemoChatMessage {
  id: string;
  role: DemoChatRole;
  /** i18n key for the message body. */
  textKey: string;
}

/** A short orchestration conversation illustrating fan-out at scale. */
export const DEMO_CHAT: DemoChatMessage[] = [
  { id: 'm1', role: 'user', textKey: 'orchPage.demo.chat.user1' },
  { id: 'm2', role: 'assistant', textKey: 'orchPage.demo.chat.assistant1' },
  { id: 'm3', role: 'activity', textKey: 'orchPage.demo.chat.activity1' },
  { id: 'm4', role: 'activity', textKey: 'orchPage.demo.chat.activity2' },
  { id: 'm5', role: 'activity', textKey: 'orchPage.demo.chat.activity3' },
  { id: 'm6', role: 'assistant', textKey: 'orchPage.demo.chat.assistant2' },
];
