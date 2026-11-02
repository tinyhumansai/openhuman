import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { activityGroupLabel } from './activity-group';
import { Thread } from './thread';

/**
 * A turn that interleaves reasoning and tool calls must read input → work →
 * answer: ONE disclosure holding the reasoning and the tool calls in the order
 * they happened, with the answer outside it. The previous grouping split the
 * same run into alternating reasoning and tool sub-groups.
 */
type Part = Exclude<ThreadMessageLike['content'], string>[number];

const tool = (id: string, name: string, over: Record<string, unknown> = {}): Part =>
  ({
    type: 'tool-call',
    toolCallId: id,
    toolName: name,
    args: {},
    argsText: '{}',
    result: 'ok',
    ...over,
  }) as never;

const interleaved = (status: ThreadMessageLike['status']): ThreadMessageLike[] => [
  { role: 'user', content: [{ type: 'text', text: 'find it' }] },
  {
    role: 'assistant',
    status,
    content: [
      { type: 'reasoning', text: 'first thought' },
      tool('t1', 'search_one'),
      { type: 'reasoning', text: 'second thought' },
      tool('t2', 'search_two'),
      { type: 'text', text: 'final answer' },
    ],
  },
];

function Harness({ messages, isRunning }: { messages: ThreadMessageLike[]; isRunning: boolean }) {
  const runtime = useExternalStoreRuntime({
    messages,
    isRunning,
    convertMessage: (m: ThreadMessageLike) => m,
    onNew: async () => {},
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Thread />
    </AssistantRuntimeProvider>
  );
}

describe('activity group', () => {
  it('renders persisted channel reactions and read receipts on the source message', () => {
    render(
      <Harness
        messages={[
          {
            role: 'user',
            content: [{ type: 'text', text: 'incoming channel message' }],
            metadata: {
              custom: {
                extraMetadata: {
                  channelActivity: {
                    reactions: { 'reaction-key': { actorId: 'alice', value: '🔥' } },
                    readReceipts: { 'receipt-key': { actorId: 'bob', timestamp: 123 } },
                  },
                },
              },
            },
          },
        ]}
        isRunning={false}
      />
    );

    expect(screen.getByTestId('channel-message-reaction')).toHaveTextContent('🔥 1');
    expect(screen.getByTestId('channel-message-read-receipts')).toHaveTextContent('✓✓ 1');
  });

  it('puts interleaved reasoning and tool calls under one trigger, answer outside', () => {
    render(
      <Harness messages={interleaved({ type: 'complete', reason: 'stop' })} isRunning={false} />
    );

    const triggers = screen.getAllByRole('button', { name: /Reasoning · 2 tool calls/ });
    expect(triggers).toHaveLength(1);
    expect(screen.queryByText(/^Reasoning$/)).toBeNull();

    const group = triggers[0]!.closest('[data-slot=tool-group-root]') as HTMLElement;
    expect(within(group).queryByText('final answer')).toBeNull();
    expect(screen.getByText('final answer')).toBeInTheDocument();

    // Settled work starts collapsed; opening it shows the steps in order.
    fireEvent.click(triggers[0]!);
    const first = within(group).getByText('first thought');
    const firstTool = within(group).getByText('search_one');
    const second = within(group).getByText('second thought');
    const secondTool = within(group).getByText('search_two');
    const steps = [first, firstTool, second, secondTool];
    for (const [current, next] of steps.map((step, index) => [step, steps[index + 1]] as const)) {
      if (next) {
        expect(
          current.compareDocumentPosition(next) & Node.DOCUMENT_POSITION_FOLLOWING
        ).toBeTruthy();
      }
    }
  });

  it('is open while the turn is still running', () => {
    render(
      <Harness
        messages={interleaved({ type: 'running' }).map(
          (m, i): ThreadMessageLike =>
            i === 1
              ? {
                  ...m,
                  content: [{ type: 'reasoning', text: 'live thought' }, tool('t1', 'search_one')],
                }
              : m
        )}
        isRunning
      />
    );

    expect(screen.getByText('live thought')).toBeVisible();
  });

  it('is open when a parked approval precedes a completed tool', () => {
    render(
      <Harness
        messages={[
          { role: 'user', content: [{ type: 'text', text: 'do it' }] },
          {
            role: 'assistant',
            status: { type: 'requires-action', reason: 'interrupt' },
            content: [tool('t1', 'shell', { result: undefined }), tool('t2', 'search_two')],
          },
        ]}
        isRunning={false}
      />
    );

    expect(screen.getByRole('button', { name: '2 tool calls' })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
  });

  it('is open when an earlier part is parked even though the message is running', () => {
    render(
      <Harness
        messages={[
          { role: 'user', content: [{ type: 'text', text: 'do it' }] },
          {
            role: 'assistant',
            status: { type: 'running' },
            content: [
              tool('t1', 'shell', { result: undefined, status: { type: 'requires-action' } }),
              tool('t2', 'search_two'),
            ],
          },
        ]}
        isRunning={false}
      />
    );

    expect(screen.getByRole('button', { name: '2 tool calls' })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
  });

  it('closes a completed group once streaming has moved to following text', () => {
    render(
      <Harness
        messages={[
          { role: 'user', content: [{ type: 'text', text: 'do it' }] },
          {
            role: 'assistant',
            status: { type: 'running' },
            content: [tool('t1', 'search_one'), { type: 'text', text: 'streaming answer' }],
          },
        ]}
        isRunning
      />
    );

    expect(screen.getByRole('button', { name: '1 tool call' })).toHaveAttribute(
      'aria-expanded',
      'false'
    );
  });
});

describe('activityGroupLabel', () => {
  const EN: Record<string, string> = {
    'chat.tools.callOne': '{count} tool call',
    'chat.tools.callOther': '{count} tool calls',
    'chat.tools.reasoningWithCalls': 'Reasoning · {calls}',
    'chat.tools.reasoning': 'Reasoning',
    'chat.tools.activity': 'Activity',
    'chat.tools.workedFor': 'Worked for {duration} · {calls}',
  };
  const t = (key: string) => EN[key] ?? key;

  it('names reasoning and counts tool calls', () => {
    expect(activityGroupLabel(2, 1, t)).toBe('Reasoning · 1 tool call');
    expect(activityGroupLabel(1, 0, t)).toBe('Reasoning');
    expect(activityGroupLabel(0, 3, t)).toBe('3 tool calls');
    expect(activityGroupLabel(0, 0, t)).toBe('Activity');
  });

  it('reads as a receipt once the turn time is known', () => {
    expect(activityGroupLabel(1, 3, t, 42_000)).toBe('Worked for 42s · 3 tool calls');
    // Without a tool call there is nothing to credit the time to.
    expect(activityGroupLabel(1, 0, t, 42_000)).toBe('Reasoning');
  });
});
