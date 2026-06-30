import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../../../services/api/threadApi';
import type { SubagentActivity, SubagentTranscriptItem } from '../../../../store/chatRuntimeSlice';
import { SubagentDrawer } from '../SubagentDrawer';

vi.mock('../../../../services/api/threadApi', () => ({
  threadApi: { getThreadMessages: vi.fn() },
}));

function activity(overrides: Partial<SubagentActivity> = {}): SubagentActivity {
  return { taskId: 'sub-1', agentId: 'researcher', toolCalls: [], transcript: [], ...overrides };
}

const INTERLEAVED: SubagentTranscriptItem[] = [
  { kind: 'thinking', iteration: 1, text: 'comparing the two sources' },
  { kind: 'text', iteration: 1, text: 'Let me search for that.' },
  {
    kind: 'tool',
    iteration: 1,
    callId: 'c1',
    toolName: 'web_search',
    status: 'success',
    elapsedMs: 1200,
  },
  { kind: 'text', iteration: 2, text: 'The answer is **42**.' },
];

describe('SubagentDrawer', () => {
  it('renders nothing when no subagent is selected', () => {
    const { container } = render(<SubagentDrawer subagent={null} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders the transcript in chronological order (text where it occurred)', () => {
    render(
      <SubagentDrawer
        subagent={activity({ transcript: INTERLEAVED })}
        status="running"
        onClose={() => {}}
      />
    );
    const drawer = screen.getByTestId('subagent-drawer');
    expect(drawer.textContent).toContain('researcher');

    // Walk the rendered transcript items and assert their on-screen order:
    // thinking → text → tool → text — i.e. the tool sits between the two
    // text blocks, not in a separate section.
    const thinking = screen.getByTestId('subagent-transcript-thinking');
    const tool = screen.getByTestId('subagent-drawer-tool-call');
    const texts = screen.getAllByTestId('subagent-transcript-text');
    expect(texts).toHaveLength(2);

    const order = (el: Element) =>
      Array.prototype.indexOf.call(drawer.querySelectorAll('[data-testid]'), el);
    expect(order(thinking)).toBeLessThan(order(texts[0]));
    expect(order(texts[0])).toBeLessThan(order(tool));
    expect(order(tool)).toBeLessThan(order(texts[1]));

    expect(thinking.textContent).toContain('comparing the two sources');
    expect(tool.textContent).toContain('web_search');
    expect(tool.textContent).toContain('1.2s');
    expect(texts[1].textContent).toContain('The answer is');
  });

  it('opens with the parent delegation prompt as a chat bubble', () => {
    render(
      <SubagentDrawer
        subagent={activity({
          prompt: 'Research Q3 revenue drivers and summarise.',
          transcript: [{ kind: 'text', iteration: 1, text: 'On it.' }],
        })}
        status="running"
        onClose={() => {}}
      />
    );
    const parent = screen.getByTestId('subagent-parent-prompt');
    expect(parent.textContent).toContain('Research Q3 revenue drivers');
    // The parent bubble renders before the sub-agent's reply.
    const drawer = screen.getByTestId('subagent-drawer');
    const text = screen.getByTestId('subagent-transcript-text');
    const order = (el: Element) =>
      Array.prototype.indexOf.call(drawer.querySelectorAll('[data-testid]'), el);
    expect(order(parent)).toBeLessThan(order(text));
  });

  it('inserts a turn divider when the iteration advances', () => {
    render(
      <SubagentDrawer
        subagent={activity({ transcript: INTERLEAVED })}
        status="running"
        onClose={() => {}}
      />
    );
    // Two distinct iterations (1 and 2) → two turn dividers.
    expect(screen.getAllByTestId('subagent-turn-divider')).toHaveLength(2);
  });

  it('shows a working placeholder while running with an empty transcript', () => {
    render(<SubagentDrawer subagent={activity()} status="running" onClose={() => {}} />);
    expect(screen.getByTestId('subagent-drawer').textContent).toContain('Working');
  });

  it('reopens from memory: fetches the worker thread when there is no live transcript', async () => {
    vi.mocked(threadApi.getThreadMessages).mockResolvedValue({
      count: 3,
      messages: [
        {
          id: 'm0',
          content: 'Research Q3 revenue.',
          type: 'text',
          sender: 'user',
          createdAt: 't0',
          extraMetadata: { scope: 'worker_thread' },
        },
        {
          id: 'm1',
          content: 'Searched the web.',
          type: 'text',
          sender: 'agent',
          createdAt: 't1',
          extraMetadata: { tool_name: 'web_search', iteration: 1 },
        },
        {
          id: 'm2',
          content: 'Revenue grew 18%.',
          type: 'text',
          sender: 'agent',
          createdAt: 't2',
          extraMetadata: { iteration: 2, final: true },
        },
      ],
    });

    render(
      <SubagentDrawer
        subagent={activity({ workerThreadId: 'worker-abc', transcript: [] })}
        status="success"
        onClose={() => {}}
      />
    );

    await waitFor(() => expect(threadApi.getThreadMessages).toHaveBeenCalledWith('worker-abc'));
    // The persisted conversation renders: parent prompt + a tool call + the text.
    await waitFor(() =>
      expect(screen.getByTestId('subagent-parent-prompt').textContent).toContain('Research Q3')
    );
    expect(screen.getByTestId('subagent-drawer-tool-call').textContent).toContain('web_search');
    expect(screen.getByTestId('subagent-transcript-text').textContent).toContain(
      'Revenue grew 18%'
    );
  });

  it('does not fetch when a live transcript is present', () => {
    render(
      <SubagentDrawer
        subagent={activity({
          workerThreadId: 'worker-abc',
          transcript: [{ kind: 'text', iteration: 1, text: 'live' }],
        })}
        status="running"
        onClose={() => {}}
      />
    );
    expect(threadApi.getThreadMessages).not.toHaveBeenCalled();
  });

  it('invokes onClose from the close button', async () => {
    const onClose = vi.fn();
    render(<SubagentDrawer subagent={activity()} status="success" onClose={onClose} />);
    await userEvent.click(screen.getByText('✕'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('shows the Cancel task CTA only while running and with an onCancel handler', () => {
    // No handler → no CTA.
    const { rerender } = render(
      <SubagentDrawer subagent={activity()} status="running" onClose={() => {}} />
    );
    expect(screen.queryByTestId('subagent-cancel')).toBeNull();

    // Handler present but already finished → no CTA.
    rerender(
      <SubagentDrawer
        subagent={activity()}
        status="success"
        onCancel={vi.fn()}
        onClose={() => {}}
      />
    );
    expect(screen.queryByTestId('subagent-cancel')).toBeNull();

    // Running + handler → CTA shown.
    rerender(
      <SubagentDrawer
        subagent={activity()}
        status="running"
        onCancel={vi.fn()}
        onClose={() => {}}
      />
    );
    expect(screen.getByTestId('subagent-cancel')).toBeTruthy();
  });

  it('cancels via onCancel, then closes on success', async () => {
    const onCancel = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    render(
      <SubagentDrawer
        subagent={activity()}
        status="running"
        onCancel={onCancel}
        onClose={onClose}
      />
    );
    await userEvent.click(screen.getByTestId('subagent-cancel'));
    expect(onCancel).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('subagent-cancel-error')).toBeNull();
  });

  it('surfaces an error and stays open when cancel fails', async () => {
    const onCancel = vi.fn().mockRejectedValue(new Error('boom'));
    const onClose = vi.fn();
    render(
      <SubagentDrawer
        subagent={activity()}
        status="running"
        onCancel={onCancel}
        onClose={onClose}
      />
    );
    await userEvent.click(screen.getByTestId('subagent-cancel'));
    await waitFor(() => expect(screen.getByTestId('subagent-cancel-error')).toBeTruthy());
    expect(onClose).not.toHaveBeenCalled();
  });

  it('renders the cancelled status label', () => {
    render(<SubagentDrawer subagent={activity()} status="cancelled" onClose={() => {}} />);
    // Case-robust: the label may be rendered as "Cancelled" or "cancelled".
    expect(screen.getByTestId('subagent-drawer').textContent?.toLowerCase()).toContain('cancelled');
  });

  it('expands a tool call to reveal its input args and output', async () => {
    const transcript: SubagentTranscriptItem[] = [
      {
        kind: 'tool',
        iteration: 1,
        callId: 'c1',
        toolName: 'web_search',
        status: 'success',
        elapsedMs: 1200,
        args: { query: 'Q3 revenue drivers' },
        result: 'Found 3 results about revenue.',
      },
    ];
    render(
      <SubagentDrawer subagent={activity({ transcript })} status="success" onClose={() => {}} />
    );

    // Collapsed by default — neither input nor output is rendered yet.
    expect(screen.queryByTestId('subagent-tool-call-input')).toBeNull();
    expect(screen.queryByTestId('subagent-tool-call-output')).toBeNull();

    await userEvent.click(screen.getByTestId('subagent-tool-call-toggle'));

    expect(screen.getByTestId('subagent-tool-call-input').textContent).toContain(
      'Q3 revenue drivers'
    );
    expect(screen.getByTestId('subagent-tool-call-output').textContent).toContain(
      'Found 3 results about revenue.'
    );
  });

  it('shows the no-output placeholder when the tool returned an empty result', async () => {
    const transcript: SubagentTranscriptItem[] = [
      { kind: 'tool', iteration: 1, callId: 'c1', toolName: 'noop', status: 'success', result: '' },
    ];
    render(
      <SubagentDrawer subagent={activity({ transcript })} status="success" onClose={() => {}} />
    );
    await userEvent.click(screen.getByTestId('subagent-tool-call-toggle'));
    expect(screen.getByTestId('subagent-tool-call-output').textContent?.toLowerCase()).toContain(
      'no output'
    );
  });

  it('renders cancelled/awaiting_user tool-call statuses with their own label (not "failed")', () => {
    const transcript: SubagentTranscriptItem[] = [
      { kind: 'tool', iteration: 1, callId: 'c1', toolName: 'web_search', status: 'cancelled' },
      { kind: 'tool', iteration: 1, callId: 'c2', toolName: 'composio', status: 'awaiting_user' },
    ];
    render(
      <SubagentDrawer subagent={activity({ transcript })} status="cancelled" onClose={() => {}} />
    );
    const rows = screen.getAllByTestId('subagent-drawer-tool-call');
    expect(rows[0].textContent?.toLowerCase()).toContain('cancelled');
    expect(rows[0].textContent?.toLowerCase()).not.toContain('failed');
    expect(rows[1].textContent?.toLowerCase()).toContain('awaiting');
  });

  it('does not offer expansion for a tool call with no captured args or result', () => {
    const transcript: SubagentTranscriptItem[] = [
      { kind: 'tool', iteration: 1, callId: 'c1', toolName: 'web_search', status: 'success' },
    ];
    render(
      <SubagentDrawer subagent={activity({ transcript })} status="success" onClose={() => {}} />
    );
    const toggle = screen.getByTestId('subagent-tool-call-toggle') as HTMLButtonElement;
    expect(toggle.disabled).toBe(true);
    expect(screen.queryByTestId('subagent-tool-call-input')).toBeNull();
    expect(screen.queryByTestId('subagent-tool-call-output')).toBeNull();
  });
});
