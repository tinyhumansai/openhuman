/**
 * Composer slot identity on the assistant-ui chat surface.
 *
 * `thread.tsx` renders each host slot **by type**, so a slot defined as a
 * callback with a changing dep list hands React a new element type and the
 * subtree unmounts and remounts on every host render (#6246). The slots now
 * hold a constant identity and read their props through a ref instead.
 *
 * That trade has a failure mode of its own: a frozen slot that never re-renders
 * would show stale props forever. It does not happen here because nothing in
 * `thread.tsx` is memoized — the whole subtree re-renders with its host — and
 * these tests pin that down, because it is the only thing keeping the stable
 * slots correct.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import mascotReducer from '../../../store/mascotSlice';
import runModeReducer from '../../../store/runModeSlice';
import threadGoalReducer from '../../../store/threadGoalSlice';
import threadReducer from '../../../store/threadSlice';
import threadTodosReducer from '../../../store/threadTodosSlice';
import { AssistantUiChat } from './AssistantUiChat';

const THREAD_ID = 't-slots';

function buildStore() {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      threadGoal: threadGoalReducer,
      threadTodos: threadTodosReducer,
      chatRuntime: chatRuntimeReducer,
      mascot: mascotReducer,
      // The composer's `/plan` / `/build` commands read it (`useRunMode`).
      runMode: runModeReducer,
    }),
    preloadedState: {
      thread: {
        threads: [
          {
            id: THREAD_ID,
            title: 'Slots thread',
            chatId: null,
            isActive: false,
            messageCount: 0,
            lastMessageAt: '2026-01-01T00:00:00.000Z',
            createdAt: '2026-01-01T00:00:00.000Z',
            labels: [],
          },
        ],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: [] },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

function chat(
  onOpenHumanMode?: () => void,
  overrides: {
    attachmentsEnabled?: boolean;
    attachmentInteractionBlocked?: boolean;
    composerReplacement?: React.ReactNode;
    onAttachFiles?: (files: FileList | File[] | null) => Promise<void>;
  } = {}
) {
  return (
    <AssistantUiChat
      model={null}
      onModelChange={vi.fn()}
      inputValue=""
      onInputValueChange={vi.fn()}
      attachments={[]}
      onAttachFiles={overrides.onAttachFiles ?? vi.fn()}
      onRemoveAttachment={vi.fn()}
      maxAttachments={5}
      attachmentsEnabled={overrides.attachmentsEnabled ?? false}
      attachmentInteractionBlocked={overrides.attachmentInteractionBlocked ?? false}
      onAttachmentOnlySend={vi.fn()}
      onOpenHumanMode={onOpenHumanMode}
      composerReplacement={overrides.composerReplacement}
    />
  );
}

function threadViewport(): HTMLElement {
  return document.querySelector('[data-slot="aui_thread-viewport"]') as HTMLElement;
}

function composerShell(): HTMLElement {
  return document.querySelector('[data-slot="aui_composer-shell"]') as HTMLElement;
}

describe('assistant-ui composer slots', () => {
  it('keeps the idle mascot button mounted across a host re-render', () => {
    const store = buildStore();
    const navigate = vi.fn();
    // A fresh arrow per render, exactly as `Conversations` passes it
    // (`onOpenHumanMode={() => navigate('/human')}`). That changing identity is
    // what used to remount the slot.
    const { rerender } = render(
      <Provider store={store}>{chat(() => navigate('/human'))}</Provider>
    );

    const button = screen.getByTestId('composer-human-mode');
    const icon = screen.getByTestId('composer-human-mascot-icon');
    expect(icon).toHaveAttribute('width', '24');
    expect(icon).toHaveAttribute('height', '24');
    expect(icon.tagName).toBe('IMG');
    expect(icon.getAttribute('src')).toMatch(/^data:image\/png;base64,/);

    rerender(<Provider store={store}>{chat(() => navigate('/human'))}</Provider>);

    expect(screen.getByTestId('composer-human-mode')).toBe(button);
  });

  it('refuses a file drag while the composer is locked', () => {
    const store = buildStore();
    const onAttachFiles = vi.fn(() => Promise.resolve());
    render(
      <Provider store={store}>
        {chat(undefined, {
          attachmentsEnabled: true,
          attachmentInteractionBlocked: true,
          onAttachFiles,
        })}
      </Provider>
    );

    const dataTransfer = { types: ['Files'], dropEffect: 'copy' };
    fireEvent.dragOver(composerShell(), { dataTransfer });

    // `preventDefault` still ran — otherwise the webview navigates away to the
    // dropped file — but the drop is refused and no affordance is shown.
    expect(dataTransfer.dropEffect).toBe('none');
    expect(composerShell().getAttribute('data-dragging')).toBeNull();

    const drop = fireEvent.drop(threadViewport(), {
      dataTransfer: { types: ['Files'], files: [new File(['blocked'], 'blocked.txt')], items: [] },
    });

    expect(drop).toBe(false); // default navigation is still cancelled
    expect(onAttachFiles).not.toHaveBeenCalled();
    expect(composerShell().getAttribute('data-dragging')).toBeNull();
  });

  it('leaves the assistant-ui dropzone in charge when the host takes no files', () => {
    const store = buildStore();
    render(<Provider store={store}>{chat()}</Provider>);

    const dataTransfer = { types: ['Files'], dropEffect: 'copy' };
    fireEvent.dragOver(composerShell(), { dataTransfer });

    // `attachmentsEnabled` is false here, so no host file sink is published and
    // the primitive's own (capability-gated) handling is what remains.
    expect(composerShell().getAttribute('data-dragging')).toBeNull();
    expect(dataTransfer.dropEffect).toBe('none');
  });

  it('takes a file dropped anywhere over the open thread, not just the composer', async () => {
    const store = buildStore();
    const onAttachFiles = vi.fn(() => Promise.resolve());
    render(
      <Provider store={store}>
        {chat(undefined, { attachmentsEnabled: true, onAttachFiles })}
      </Provider>
    );

    const file = new File(['png'], 'shot.png', { type: 'image/png' });
    const dragOver = { types: ['Files'], dropEffect: 'none' };
    fireEvent.dragOver(threadViewport(), { dataTransfer: dragOver });

    // The drag is claimed over the transcript and the composer lights up as
    // the place the file will land.
    expect(dragOver.dropEffect).toBe('copy');
    expect(composerShell().getAttribute('data-dragging')).toBe('true');

    const drop = fireEvent.drop(threadViewport(), {
      dataTransfer: { types: ['Files'], files: [file], items: [] },
    });

    expect(drop).toBe(false); // default (navigate to the file) cancelled
    await vi.waitFor(() => expect(onAttachFiles).toHaveBeenCalledWith([file]));
    expect(composerShell().getAttribute('data-dragging')).toBeNull();
  });

  it('serializes rapid thread drops while attachment ingestion is pending', async () => {
    const store = buildStore();
    let finishFirst!: () => void;
    const firstFinished = new Promise<void>(resolve => {
      finishFirst = resolve;
    });
    const onAttachFiles = vi.fn(() => firstFinished);
    render(
      <Provider store={store}>
        {chat(undefined, { attachmentsEnabled: true, onAttachFiles })}
      </Provider>
    );

    const drop = (name: string) =>
      fireEvent.drop(threadViewport(), {
        dataTransfer: { types: ['Files'], files: [new File(['file'], name)], items: [] },
      });

    drop('first.txt');
    await vi.waitFor(() => expect(onAttachFiles).toHaveBeenCalledTimes(1));
    drop('second.txt');
    await Promise.resolve();
    expect(onAttachFiles).toHaveBeenCalledTimes(1);

    finishFirst();
    await firstFinished;
    await vi.waitFor(() => expect(onAttachFiles).toHaveBeenCalledTimes(2));
  });

  it('swaps only the composer when the host supplies a replacement (mic-cloud)', () => {
    const store = buildStore();
    const { rerender } = render(
      <Provider store={store}>
        {chat(undefined, { composerReplacement: <div data-testid="voice-probe">one</div> })}
      </Provider>
    );

    // The assistant-ui transcript is still the one mounted...
    expect(document.querySelector('[data-slot="aui_thread-viewport"]')).not.toBeNull();
    // ...but the text composer is gone, replaced by the host's.
    expect(composerShell()).toBeNull();
    const probe = screen.getByTestId('voice-probe');

    // Stable slot identity: a host re-render updates the replacement in place
    // rather than remounting it (MicComposer holds recording state).
    rerender(
      <Provider store={store}>
        {chat(undefined, { composerReplacement: <div data-testid="voice-probe">two</div> })}
      </Provider>
    );
    expect(screen.getByTestId('voice-probe')).toBe(probe);
    expect(probe).toHaveTextContent('two');
  });

  it('keeps the built-in composer when no replacement is supplied', () => {
    const store = buildStore();
    render(<Provider store={store}>{chat()}</Provider>);
    expect(composerShell()).not.toBeNull();
  });
});
