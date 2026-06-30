import { useState } from 'react';

import Button from '../../components/ui/Button';
import type { ToolTimelineEntry } from '../../store/chatRuntimeSlice';
import { AgentProcessSourcePanel } from '../conversations/components/AgentProcessSourcePanel';
import { ToolTimelineBlock } from '../conversations/components/ToolTimelineBlock';

/**
 * Dev-only visual preview of the "Agentic task insights" Chat surface.
 *
 * Renders {@link ToolTimelineBlock} and {@link AgentProcessSourcePanel} with
 * hand-built sample timeline entries so the layout, the timeline rail, the
 * name blink/done/error states, the collapsible accordion, and the source
 * panel can be eyeballed under plain `pnpm dev` — no core / model / live
 * agent run required. Reachable at `#/dev/agent-insights`.
 *
 * Not linked from any nav; throwaway harness for design review.
 */

// A live run: a mix of running (pulsing), done, errored, sub-agent, web
// sources and a code block — exercises every row variant at once.
const RUNNING_ENTRIES: ToolTimelineEntry[] = [
  {
    id: 's-slack',
    name: 'subagent:integrations_agent',
    round: 1,
    status: 'running',
    sourceToolName: 'slack',
    subagent: {
      taskId: 'sub-slack',
      agentId: 'integrations_agent',
      mode: 'typed',
      childIteration: 6,
      prompt:
        'Search for any issues or problem reports raised in Slack in the last 24 hours across all channels.',
      toolCalls: [
        { callId: 'c1', toolName: 'composio_list_tools', status: 'success', elapsedMs: 38400 },
        { callId: 'c2', toolName: 'slack_list_all_channels', status: 'success', elapsedMs: 2900 },
        { callId: 'c3', toolName: 'slack_fetch_conversation_history', status: 'running' },
      ],
    },
  },
  {
    id: 'e-search',
    name: 'web_search',
    round: 1,
    status: 'success',
    argsBuffer: JSON.stringify({ query: 'monaco gp 2026 results' }),
  },
  {
    id: 'e-fetch1',
    name: 'web_fetch',
    round: 1,
    status: 'success',
    argsBuffer: JSON.stringify({ url: 'https://news-gazette.com/sport/f1-monaco' }),
  },
  {
    id: 'e-fetch2',
    name: 'web_fetch',
    round: 1,
    status: 'success',
    argsBuffer: JSON.stringify({ url: 'https://example.org/standings' }),
  },
  {
    id: 'e-shell',
    name: 'shell',
    round: 2,
    status: 'success',
    argsBuffer: JSON.stringify({ command: 'cat report.py | head -20' }),
  },
  {
    id: 'e-err',
    name: 'file_read',
    round: 2,
    status: 'error',
    argsBuffer: JSON.stringify({ path: '/tmp/missing.txt' }),
  },
];

// A settled run (all done) — every name reads solid, no pulse.
const SETTLED_ENTRIES: ToolTimelineEntry[] = RUNNING_ENTRIES.map(e => ({
  ...e,
  status: e.status === 'error' ? 'error' : 'success',
  subagent: e.subagent
    ? {
        ...e.subagent,
        childIteration: undefined,
        iterations: 6,
        elapsedMs: 49200,
        toolCalls: e.subagent.toolCalls.map(c => ({ ...c, status: 'success', elapsedMs: 2600 })),
      }
    : undefined,
}));

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-2xl border border-line bg-[#f6f6f6] p-4 dark:border-line dark:bg-surface-canvas">
      <h2 className="mb-3 text-sm font-semibold text-content-secondary">{title}</h2>
      {children}
    </section>
  );
}

export default function AgentInsightsPreview() {
  const [panelOpen, setPanelOpen] = useState(false);

  return (
    <div className="h-full overflow-y-auto bg-surface p-8">
      <div className="mx-auto max-w-2xl space-y-6">
        <header>
          <h1 className="text-lg font-bold text-content">Agentic task insights — preview</h1>
          <p className="text-xs text-content-muted">
            Dev-only harness (#/dev/agent-insights). Sample data — not a live run.
          </p>
        </header>

        <Section title="Running — names pulse while in progress, solid when done, coral on error">
          <ToolTimelineBlock entries={RUNNING_ENTRIES} onViewSubagent={() => setPanelOpen(true)} />
        </Section>

        <Section title="Settled — all done (solid names)">
          <ToolTimelineBlock entries={SETTLED_ENTRIES} onViewSubagent={() => setPanelOpen(true)} />
        </Section>

        <Section title="Agent Process Source panel">
          <Button variant="primary" size="sm" onClick={() => setPanelOpen(true)}>
            View full agent process Source →
          </Button>
        </Section>
      </div>

      <AgentProcessSourcePanel
        open={panelOpen}
        entries={SETTLED_ENTRIES}
        onClose={() => setPanelOpen(false)}
      />
    </div>
  );
}
