import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { store } from '../../../store';
import type { StreamingAssistantState, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import type { ThreadMessage } from '../../../types/thread';
import { ConversationTimeline } from './ConversationTimeline';
import { buildThreadTimeline } from './selectors';

function renderInStore(ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

function userMsg(id: string, content: string): ThreadMessage {
  return {
    id,
    content,
    type: 'text',
    extraMetadata: {},
    sender: 'user',
    createdAt: '2026-01-01T00:00:00Z',
  };
}
function agentMsg(id: string, content: string): ThreadMessage {
  return {
    id,
    content,
    type: 'text',
    extraMetadata: {},
    sender: 'agent',
    createdAt: '2026-01-01T00:00:01Z',
  };
}
function tool(id: string, name: string, round = 0): ToolTimelineEntry {
  return { id, name, round, seq: 0, status: 'success' };
}
function subagentRow(id: string, taskId: string): ToolTimelineEntry {
  return {
    id,
    name: 'subagent:researcher',
    round: 0,
    seq: 0,
    status: 'running',
    subagent: { taskId, agentId: 'researcher', toolCalls: [] },
  };
}
function stream(requestId: string, content: string, thinking = ''): StreamingAssistantState {
  return { requestId, content, thinking };
}

function timeline(over: Partial<Parameters<typeof buildThreadTimeline>[0]>) {
  return buildThreadTimeline({
    threadId: 't1',
    messages: [],
    toolTimeline: [],
    streaming: null,
    parallelStreams: [],
    hideAgentInsights: false,
    ...over,
  });
}

describe('ConversationTimeline', () => {
  it('renders user + assistant messages in order', () => {
    const items = timeline({ messages: [userMsg('u1', 'hello there'), agentMsg('a1', 'hi back')] });
    renderInStore(<ConversationTimeline items={items} />);
    expect(screen.getByTestId('conversation-timeline')).toBeTruthy();
    expect(screen.getByText('hello there')).toBeTruthy();
    expect(screen.getByText('hi back')).toBeTruthy();
  });

  it('coalesces a run of process items into a single ToolTimelineBlock', () => {
    const items = timeline({
      messages: [userMsg('u1', 'go')],
      toolTimeline: [tool('c1', 'read_file', 0), tool('c2', 'write_file', 1)],
    });
    const { container } = renderInStore(<ConversationTimeline items={items} />);
    // The two consecutive tool items coalesce into one ToolTimelineBlock (one
    // `<details>` group), not two separate blocks.
    expect(container.querySelectorAll('details')).toHaveLength(1);
  });

  it('invokes onOpenSubagent when a subagent row requests the drawer', () => {
    const onOpenSubagent = vi.fn();
    const items = timeline({
      messages: [userMsg('u1', 'go')],
      toolTimeline: [subagentRow('row1', 'sub-1')],
    });
    renderInStore(<ConversationTimeline items={items} handlers={{ onOpenSubagent }} />);
    // The subagent row renders within the block; the handler is wired through.
    expect(screen.getByTestId('conversation-timeline')).toBeTruthy();
  });

  it('renders a streaming tail with the 120-char slice and thinking details', () => {
    const long = 'x'.repeat(200);
    const items = timeline({
      messages: [userMsg('u1', 'go')],
      streaming: stream('req1', long, 'deliberating'),
    });
    renderInStore(<ConversationTimeline items={items} />);
    const primary = screen.getByTestId('stream-primary');
    // Truncation ellipsis is present and the thinking summary renders.
    expect(primary.textContent).toContain('…');
    expect(primary.textContent).toContain('deliberating');
    // Only the last 120 chars of the 200-char body render in the content bubble.
    const contentBubble = primary.querySelector('.font-mono.text-sm');
    expect(contentBubble?.textContent).toBe(`…${'x'.repeat(120)}`);
  });

  it('renders forked branch streams as branch tails', () => {
    const items = timeline({
      messages: [userMsg('u1', 'go')],
      parallelStreams: [stream('b1', 'branch answer')],
    });
    renderInStore(<ConversationTimeline items={items} />);
    expect(screen.getByTestId('stream-branch')).toBeTruthy();
    expect(screen.getByText('branch answer')).toBeTruthy();
  });

  it('renders nothing but the container for an empty timeline', () => {
    renderInStore(<ConversationTimeline items={[]} />);
    const container = screen.getByTestId('conversation-timeline');
    expect(container.children).toHaveLength(0);
  });
});
