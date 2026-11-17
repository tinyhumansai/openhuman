import { act, fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  selectChatMascotExpanded,
  selectChatMascotLiveVoicePhase,
  selectSpeakReplies,
} from '../../../store/mascotSlice';
import { renderWithProviders } from '../../../test/test-utils';
import {
  type ChatMascotContextValue,
  ChatMascotProvider,
  useChatMascot,
} from './ChatMascotContext';
import ChatMascotStage from './ChatMascotStage';

// The real control opens a socket and the mic; this stub exposes the seams the
// stage owns — the thread it binds to, the auto-start request and the phase
// report the overlay reads.
const liveProps = vi.hoisted(() => ({ current: null as null | Record<string, unknown> }));
vi.mock('../LiveVoiceControls', () => ({
  default: (props: {
    threadId: string | null;
    consumeAutoStart: () => boolean;
    onPhaseChange: (phase: string) => void;
  }) => {
    liveProps.current = props;
    return (
      <div data-testid="live-voice-stub" data-thread={props.threadId ?? ''}>
        <button data-testid="phase-speaking" onClick={() => props.onPhaseChange('speaking')}>
          speak
        </button>
      </div>
    );
  },
}));

const RequestVoice = ({ onReady }: { onReady: (ctx: ChatMascotContextValue) => void }) => {
  onReady(useChatMascot());
  return null;
};

const renderStage = (opts: { threadId?: string | null; requestVoice?: boolean } = {}) => {
  let ctx: ChatMascotContextValue | null = null;
  const utils = renderWithProviders(
    <ChatMascotProvider>
      <RequestVoice onReady={c => (ctx = c)} />
      <ChatMascotStage />
    </ChatMascotProvider>,
    {
      preloadedState: {
        mascot: { chatMascotExpanded: true, speakReplies: true },
        thread: { selectedThreadId: opts.threadId ?? null },
      },
    }
  );
  return { ...utils, ctx: () => ctx! };
};

describe('ChatMascotStage', () => {
  beforeEach(() => vi.clearAllMocks());

  it('hosts the live voice agent bound to the open thread', () => {
    renderStage({ threadId: 'thread-42' });
    expect(screen.getByTestId('live-voice-stub')).toHaveAttribute('data-thread', 'thread-42');
  });

  it('hands the dock click request to the live control exactly once', () => {
    const { ctx } = renderStage();
    const consume = liveProps.current!.consumeAutoStart as () => boolean;
    expect(consume()).toBe(false);
    act(() => ctx().expandWithVoice());
    expect(consume()).toBe(true);
    expect(consume()).toBe(false);
  });

  it('collapsing drops a pending voice request', () => {
    const { ctx } = renderStage();
    act(() => ctx().expandWithVoice());
    fireEvent.click(screen.getByTestId('chat-mascot-collapse'));
    expect((liveProps.current!.consumeAutoStart as () => boolean)()).toBe(false);
  });

  it('publishes the live phase for the overlay', () => {
    const { store } = renderStage();
    fireEvent.click(screen.getByTestId('phase-speaking'));
    expect(selectChatMascotLiveVoicePhase(store.getState())).toBe('speaking');
  });

  it('toggles the speak-replies preference', () => {
    const { store } = renderStage();

    fireEvent.click(screen.getByTestId('chat-mascot-speak-replies'));

    expect(selectSpeakReplies(store.getState())).toBe(false);
  });

  it('collapses back to the dock', () => {
    const { store } = renderStage();

    fireEvent.click(screen.getByTestId('chat-mascot-collapse'));

    expect(selectChatMascotExpanded(store.getState())).toBe(false);
  });

  it('leaves the mascot anchor empty — the shared overlay paints it', () => {
    renderStage();

    expect(screen.getByTestId('chat-mascot-stage-anchor')).toBeEmptyDOMElement();
  });

  it('collapses when the mascot itself is clicked', () => {
    // Symmetry with the dock: the mascot is the toggle in both directions.
    const { store } = renderStage();

    fireEvent.click(screen.getByTestId('chat-mascot-stage-anchor'));

    expect(selectChatMascotExpanded(store.getState())).toBe(false);
  });

  it('keeps the mascot target out of the a11y tree — the button is the one control', () => {
    renderStage();

    const anchor = screen.getByTestId('chat-mascot-stage-anchor');
    expect(anchor).toHaveAttribute('aria-hidden', 'true');
    expect(anchor).toHaveAttribute('tabindex', '-1');
    expect(screen.getAllByRole('button', { name: 'Back to the chat' })).toHaveLength(1);
  });
});
