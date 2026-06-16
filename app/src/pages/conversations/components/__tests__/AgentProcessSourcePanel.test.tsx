import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { store } from '../../../../store';
import type { ToolTimelineEntry } from '../../../../store/chatRuntimeSlice';
import { AgentProcessSourcePanel } from '../AgentProcessSourcePanel';

function renderPanel(ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

const fetchEntry = (id: string, url: string): ToolTimelineEntry => ({
  id,
  name: 'web_fetch',
  round: 1,
  status: 'success',
  argsBuffer: JSON.stringify({ url }),
});

describe('AgentProcessSourcePanel', () => {
  it('renders nothing while closed', () => {
    renderPanel(<AgentProcessSourcePanel open={false} entries={[]} onClose={() => {}} />);
    expect(screen.queryByTestId('agent-process-source-panel')).toBeNull();
  });

  it('shows the panel title and steps when open', () => {
    renderPanel(
      <AgentProcessSourcePanel
        open
        entries={[fetchEntry('e1', 'https://news-gazette.com/article')]}
        onClose={() => {}}
      />
    );
    expect(screen.getByTestId('agent-process-source-panel')).toBeInTheDocument();
    expect(screen.getByText('Agent Process Source')).toBeInTheDocument();
  });

  it('lists the distinct web sources the agents visited (deduped by URL)', () => {
    renderPanel(
      <AgentProcessSourcePanel
        open
        entries={[
          fetchEntry('e1', 'https://news-gazette.com/a'),
          fetchEntry('e2', 'https://news-gazette.com/a'), // duplicate URL → collapsed
          fetchEntry('e3', 'https://example.org/b'),
        ]}
        onClose={() => {}}
      />
    );
    const rows = screen.getAllByTestId('agent-source-row');
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain('news-gazette.com');
    expect(rows[1].textContent).toContain('example.org');
  });

  it('expands every step row by default (whole run visible at a glance)', () => {
    renderPanel(
      <AgentProcessSourcePanel
        open
        entries={[
          fetchEntry('e1', 'https://news-gazette.com/a'),
          fetchEntry('e2', 'https://example.org/b'),
        ]}
        onClose={() => {}}
      />
    );
    const panel = screen.getByTestId('agent-process-source-panel');
    const allDetails = panel.querySelectorAll('details');
    // Every <details> (the group + each expandable row) is open.
    expect(allDetails.length).toBeGreaterThan(1);
    allDetails.forEach(d => expect(d.hasAttribute('open')).toBe(true));
  });

  it('never shows the "view full processing" affordance (the panel IS that view)', () => {
    renderPanel(
      <AgentProcessSourcePanel
        open
        entries={[
          {
            id: 'sa',
            name: 'subagent:researcher',
            round: 1,
            status: 'success',
            subagent: {
              taskId: 'sub-1',
              agentId: 'researcher',
              toolCalls: [{ callId: 'c1', toolName: 'web_search', status: 'success' }],
            },
          },
        ]}
        onClose={() => {}}
      />
    );
    // The subagent activity renders, but with no onView → no button.
    expect(screen.getByTestId('subagent-activity')).toBeInTheDocument();
    expect(screen.queryByTestId('subagent-view-processing')).toBeNull();
  });

  it('renders no source rows when no web tools were used', () => {
    renderPanel(
      <AgentProcessSourcePanel
        open
        entries={[{ id: 'x', name: 'file_read', round: 1, status: 'success' }]}
        onClose={() => {}}
      />
    );
    expect(screen.queryByTestId('agent-source-row')).toBeNull();
  });

  it('closes via the close button', async () => {
    const onClose = vi.fn();
    renderPanel(<AgentProcessSourcePanel open entries={[]} onClose={onClose} />);
    await userEvent.click(screen.getByText('✕'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on Escape', async () => {
    const onClose = vi.fn();
    renderPanel(<AgentProcessSourcePanel open entries={[]} onClose={onClose} />);
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on backdrop click', async () => {
    const onClose = vi.fn();
    const { container } = renderPanel(
      <AgentProcessSourcePanel open entries={[]} onClose={onClose} />
    );
    // The backdrop is the first (full-bleed) Close button.
    const backdrop = container.querySelector('button[aria-label="Close"]');
    expect(backdrop).not.toBeNull();
    await userEvent.click(backdrop as HTMLElement);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
