import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { ThreadMessage } from '../../../types/thread';
import { sameThreadMessage, TranscriptRow, type TranscriptRowProps } from './TranscriptRow';

// The share affordance opens a modal and talks to the backend; the row only
// has to place it, so stub it down to a marker.
vi.mock('../../share/ShareMessageButton', () => ({
  ShareMessageButton: ({ content }: { content: string }) => (
    <button type="button" data-testid="share-button" data-content={content} />
  ),
}));

function makeMessage(overrides: Partial<ThreadMessage> = {}): ThreadMessage {
  return {
    id: 'm-1',
    sender: 'agent',
    type: 'text',
    content: 'Hello there',
    extraMetadata: {},
    createdAt: '2026-01-01T00:00:00.000Z',
    ...overrides,
  } as ThreadMessage;
}

function renderRow(overrides: Partial<TranscriptRowProps> = {}) {
  const props: TranscriptRowProps = {
    msg: makeMessage(),
    threadId: 't-1',
    agentMessageViewMode: 'text',
    isLatestVisible: false,
    isCopied: false,
    isReactionPickerOpen: false,
    shareAgentName: 'OpenHuman',
    onCopy: vi.fn(),
    onReact: vi.fn(),
    onOpenReactionPicker: vi.fn(),
    ...overrides,
  };
  return { ...render(<TranscriptRow {...props} />), props };
}

describe('TranscriptRow', () => {
  it('renders an agent turn with the transcript test hooks the E2E specs read', () => {
    renderRow();

    const row = screen.getByTestId('chat-message-row');
    expect(row).toHaveAttribute('data-sender', 'agent');
    expect(row).toHaveAttribute('data-from', 'assistant');
    expect(screen.getByTestId('agent-message')).toBeInTheDocument();
    expect(screen.getByText('Hello there')).toBeInTheDocument();
  });

  it('renders a user turn on the trailing edge and offers no share affordance', () => {
    renderRow({ msg: makeMessage({ id: 'm-2', sender: 'user', content: 'A question?' }) });

    expect(screen.getByTestId('chat-message-row')).toHaveAttribute('data-from', 'user');
    expect(screen.getByText('A question?')).toBeInTheDocument();
    expect(screen.queryByTestId('share-button')).toBeNull();
  });

  it('unwraps a raw tool-call envelope to its human text, never raw JSON', () => {
    renderRow({
      msg: makeMessage({
        content: JSON.stringify({
          content: 'Pulling that up now.',
          tool_calls: [{ id: 'call_1', name: 'memory_search', arguments: '{}' }],
        }),
      }),
    });

    expect(screen.getByText('Pulling that up now.')).toBeInTheDocument();
    expect(screen.queryByText(/tool_calls/)).toBeNull();
  });

  it('copies the turn text through the supplied handler', async () => {
    const user = userEvent.setup();
    const onCopy = vi.fn();
    renderRow({ onCopy });

    await user.click(screen.getByRole('button', { name: /copy/i }));

    expect(onCopy).toHaveBeenCalledWith('m-1', 'Hello there');
  });

  it('shows the reaction affordance only on the latest visible turn', () => {
    const { unmount } = renderRow({ isLatestVisible: false });
    expect(screen.queryByRole('button', { name: /reaction/i })).toBeNull();
    unmount();

    renderRow({ isLatestVisible: true });
    expect(screen.getByRole('button', { name: /reaction/i })).toBeInTheDocument();
  });

  it('reacts and closes the picker in one click', async () => {
    const user = userEvent.setup();
    const onReact = vi.fn();
    const onOpenReactionPicker = vi.fn();
    renderRow({ isLatestVisible: true, isReactionPickerOpen: true, onReact, onOpenReactionPicker });

    await user.click(screen.getByTitle('👍'));

    expect(onReact).toHaveBeenCalledWith('m-1', '👍');
    expect(onOpenReactionPicker).toHaveBeenCalledWith(null);
  });

  it('marks a turn the user stopped mid-stream', () => {
    renderRow({ msg: makeMessage({ extraMetadata: { stopped: true } }) });

    expect(screen.getByTestId('stopped-marker')).toBeInTheDocument();
  });

  it('renders a restored past-turn process trail above the answer', () => {
    renderRow({
      pastTurn: {
        entries: [
          { id: 'e-1', name: 'memory_search', round: 0, seq: 0, status: 'success' as const },
        ],
        transcript: [],
      },
    });

    expect(screen.getByTestId('past-turn-insights')).toBeInTheDocument();
  });
});

describe('sameThreadMessage', () => {
  it('treats a structurally identical rehydrated message as unchanged', () => {
    expect(sameThreadMessage(makeMessage(), makeMessage())).toBe(true);
  });

  it('separates messages whose rendered content differs', () => {
    expect(sameThreadMessage(makeMessage(), makeMessage({ content: 'Different' }))).toBe(false);
  });

  it('separates messages whose metadata differs', () => {
    expect(
      sameThreadMessage(makeMessage(), makeMessage({ extraMetadata: { stopped: true } }))
    ).toBe(false);
  });

  it('separates messages whose metadata holds a different array instance', () => {
    // Arrays are compared by identity: Redux keeps the instance stable for an
    // untouched message, so a new instance means something rewrote it.
    expect(
      sameThreadMessage(
        makeMessage({ extraMetadata: { myReactions: ['👍'] } }),
        makeMessage({ extraMetadata: { myReactions: ['👍'] } })
      )
    ).toBe(false);
  });
});
