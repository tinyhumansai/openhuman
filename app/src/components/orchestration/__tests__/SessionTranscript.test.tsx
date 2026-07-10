import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ChatMessage } from '../../../lib/orchestration/useOrchestrationChats';
import SessionTranscript from '../SessionTranscript';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const msg = (over: Partial<ChatMessage>): ChatMessage => ({
  id: 'x',
  from: 'agent',
  body: '',
  timestamp: '2026-07-08T17:00:00Z',
  encrypted: false,
  ...over,
});

describe('SessionTranscript', () => {
  it('renders user vs agent bubbles by sender', () => {
    render(
      <SessionTranscript
        messages={[
          msg({ id: 'u', from: 'you', eventKind: 'user_prompt', body: 'hello' }),
          msg({ id: 'a', from: 'agent', eventKind: 'agent_message', body: 'hi back' }),
        ]}
      />
    );
    expect(
      screen.getByText('hello').closest('[data-event-kind="user_prompt"]')
    ).toBeInTheDocument();
    expect(
      screen.getByText('hi back').closest('[data-event-kind="agent_message"]')
    ).toBeInTheDocument();
  });

  it('renders an owner-authored reply (role "owner") as a user bubble', () => {
    // A composer reply is mirrored back with role "owner" and no eventKind;
    // it must sit on the right (primary bubble), not as a left agent bubble.
    render(<SessionTranscript messages={[msg({ id: 'o', from: 'owner', body: 'my reply' })]} />);
    expect(screen.getByText('my reply').closest('.bg-primary-500')).toBeInTheDocument();
  });

  it('merges a tool_call+result and marks failure', () => {
    render(
      <SessionTranscript
        messages={[
          msg({ id: 'tc', eventKind: 'tool_call', toolName: 'Bash', callId: 'c1', body: 'ls' }),
          msg({
            id: 'tr',
            eventKind: 'tool_result',
            callId: 'c1',
            body: 'boom',
            isError: true,
            exitCode: 1,
          }),
        ]}
      />
    );
    const tool = screen.getByText('ls').closest('[data-event-kind="tool_call"]')!;
    expect(tool).toHaveAttribute('data-failed', 'true');
    expect(screen.getByText('boom')).toBeInTheDocument();
  });

  it('renders an approval read-only without onDecide', () => {
    render(
      <SessionTranscript
        messages={[
          msg({ id: 'ap', eventKind: 'approval_request', toolName: 'gh', body: 'gh pr create' }),
        ]}
      />
    );
    expect(screen.getByText('chat.approval.title')).toBeInTheDocument();
    expect(screen.queryByText('chat.approval.approve')).not.toBeInTheDocument();
  });

  it('wires approval buttons to onDecide', () => {
    const onDecide = vi.fn();
    const approval = msg({ id: 'ap', eventKind: 'approval_request', body: 'run it' });
    render(<SessionTranscript messages={[approval]} onDecide={onDecide} />);
    fireEvent.click(screen.getByText('chat.approval.approve'));
    expect(onDecide).toHaveBeenCalledWith(approval, 'approve');
    fireEvent.click(screen.getByText('chat.approval.deny'));
    expect(onDecide).toHaveBeenCalledWith(approval, 'deny');
  });
});
