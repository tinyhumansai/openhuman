/**
 * OverviewPanel — an interactive "Agent graph" of the agent / sub-agent system,
 * reusing the Brain's {@link MemoryGraph} (drag / pan / zoom, WebGL with SVG
 * fallback), full-screen and fit-to-bounds.
 *
 * Three tiers fan out from a central **agent core** hub:
 *   - core (the synthetic root)
 *   - **agents** — every accepted peer contact (`source` nodes). Offline agents
 *     (no live session) render in a dead/grey colour.
 *   - **sub-agents** — the sessions running under each agent (`chunk` nodes),
 *     linked to their agent by explicit edges; disconnected ones also greyed.
 *
 * Contacts come from {@link usePairing} (so offline agents with no sessions
 * still appear); sessions come from {@link useContactSessions}.
 */
import { useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { SessionSummary } from '../../lib/orchestration/orchestrationClient';
import { useContactSessions } from '../../lib/orchestration/useOrchestrationSessions';
import { usePairing } from '../../lib/orchestration/usePairing';
import type { GraphEdge, GraphNode } from '../../utils/tauriCommands';
import { MemoryGraph } from '../intelligence/MemoryGraph';
import { contactAddress } from '../intelligence/orchestrationTabHelpers';

/** Muted grey for offline agents / disconnected sub-agents. */
const OFFLINE_COLOR = '#6B7280';

function shortAddress(address: string): string {
  return address.length <= 14 ? address : `${address.slice(0, 6)}…${address.slice(-4)}`;
}

function sessionLabel(session: SessionSummary): string {
  return session.label?.trim() || session.sessionId;
}

/**
 * A session/agent is "connected" when the core reports live peer presence; when
 * presence is unknown, fall back to the recency heuristic (active / running /
 * awaiting input).
 */
function isConnected(session: SessionSummary): boolean {
  if (session.peerOnline === true) return true;
  if (session.peerOnline === false) return false;
  return session.active || session.status === 'running' || session.status === 'waiting-approval';
}

export default function OverviewPanel() {
  const { t } = useT();
  const { byContact } = useContactSessions();
  const pairing = usePairing();

  const { nodes, edges } = useMemo(() => {
    const nodes: GraphNode[] = [];
    const edges: GraphEdge[] = [];

    // Every accepted contact is an agent — including offline ones with no
    // sessions — unioned with any address that has live sessions.
    const accepted =
      pairing.state.status === 'ok'
        ? pairing.state.snapshot.contacts.contacts.filter(c => c.status === 'accepted')
        : [];
    const addresses = new Set<string>();
    for (const c of accepted) {
      const addr = contactAddress(c);
      if (addr) addresses.add(addr);
    }
    for (const addr of byContact.keys()) addresses.add(addr);

    for (const address of addresses) {
      const sessions = byContact.get(address) ?? [];
      const online = sessions.some(isConnected);
      const instanceId = `inst:${address}`;
      nodes.push({
        kind: 'source',
        id: instanceId,
        label: shortAddress(address),
        ...(online ? {} : { color: OFFLINE_COLOR }),
      });
      for (const session of sessions) {
        const sessionNodeId = `sess:${session.sessionId}`;
        nodes.push({
          kind: 'chunk',
          id: sessionNodeId,
          label: sessionLabel(session),
          ...(isConnected(session) ? {} : { color: OFFLINE_COLOR }),
        });
        edges.push({ from: sessionNodeId, to: instanceId });
      }
    }
    return { nodes, edges };
  }, [byContact, pairing.state]);

  return (
    // Full-screen: the graph fills the whole pane (no card gutter) and fits the
    // whole agent/sub-agent cloud tightly to the viewport.
    <div className="h-full p-2" data-testid="orch-overview">
      <MemoryGraph
        nodes={nodes}
        edges={edges}
        mode="contacts"
        rootLabel={t('orchPage.overview.core')}
        emptyHint={t('orchPage.overview.empty')}
        fill
        fitToBounds
        showLabels
      />
    </div>
  );
}
