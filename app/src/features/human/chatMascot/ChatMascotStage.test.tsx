import { fireEvent, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { selectChatMascotExpanded, selectSpeakReplies } from '../../../store/mascotSlice';
import { renderWithProviders } from '../../../test/test-utils';
import {
  type ChatMascotContextValue,
  ChatMascotProvider,
  useChatMascot,
} from './ChatMascotContext';
import ChatMascotStage from './ChatMascotStage';

// The real MicComposer wants `navigator.mediaDevices` and the STT client; this
// stub exposes just the seams the stage owns — the submit path, the disabled
// flag, and the recording report.
vi.mock('../MicComposer', () => ({
  default: ({
    disabled,
    onSubmit,
    onError,
    onRecordingChange,
  }: {
    disabled: boolean;
    onSubmit: (text: string) => void;
    onError?: (message: string) => void;
    onRecordingChange?: (recording: boolean) => void;
  }) => (
    <div>
      <button data-testid="mic-submit" disabled={disabled} onClick={() => onSubmit('hello there')}>
        submit
      </button>
      <button data-testid="mic-error" onClick={() => onError?.('mic exploded')}>
        error
      </button>
      <button data-testid="mic-record-on" onClick={() => onRecordingChange?.(true)}>
        record
      </button>
    </div>
  ),
}));

/** Publishes a send binding the way `Conversations` does. */
const BindSend = ({
  submit,
  onError,
  disabled = false,
  onReady,
}: {
  submit: (text: string) => void;
  onError: (message: string) => void;
  disabled?: boolean;
  onReady?: (ctx: ChatMascotContextValue) => void;
}) => {
  const ctx = useChatMascot();
  ctx.sendStore.set({ submit, onError, disabled });
  onReady?.(ctx);
  return null;
};

const renderStage = (opts: { disabled?: boolean; bind?: boolean } = {}) => {
  const submit = vi.fn();
  const onError = vi.fn();
  const utils = renderWithProviders(
    <ChatMascotProvider>
      {opts.bind === false ? null : (
        <BindSend submit={submit} onError={onError} disabled={opts.disabled ?? false} />
      )}
      <ChatMascotStage />
    </ChatMascotProvider>,
    { preloadedState: { mascot: { chatMascotExpanded: true, speakReplies: true } } }
  );
  return { ...utils, submit, onError };
};

describe('ChatMascotStage', () => {
  beforeEach(() => vi.clearAllMocks());

  it('routes a transcript through the chat send path', () => {
    const { submit } = renderStage();

    fireEvent.click(screen.getByTestId('mic-submit'));

    expect(submit).toHaveBeenCalledWith('hello there');
  });

  it('surfaces mic failures through the chat error path', () => {
    const { onError } = renderStage();

    fireEvent.click(screen.getByTestId('mic-error'));

    expect(onError).toHaveBeenCalledWith('mic exploded');
  });

  it('disables the mic while the chat says sending is blocked', () => {
    renderStage({ disabled: true });

    expect(screen.getByTestId('mic-submit')).toBeDisabled();
  });

  it('disables the mic when no chat is bound at all', () => {
    // Without this, a transcript spoken before the chat mounts hits
    // handleSendMessage's early return and is silently dropped.
    renderStage({ bind: false });

    expect(screen.getByTestId('mic-submit')).toBeDisabled();
  });

  it('reports a hot mic so the mascot can hold its listening pose', () => {
    const { store } = renderStage();

    fireEvent.click(screen.getByTestId('mic-record-on'));

    expect(store.getState().mascot.chatMascotListening).toBe(true);
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
