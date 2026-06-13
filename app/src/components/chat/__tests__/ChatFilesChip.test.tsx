import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import chatRuntimeReducer, {
  type ArtifactSnapshot,
  upsertArtifactInProgressForThread,
  upsertArtifactReadyForThread,
} from '../../../store/chatRuntimeSlice';
import ChatFilesChip from '../ChatFilesChip';

const THREAD = 't-chip-1';

function mkStore() {
  return configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
}

function readyArtifact(
  idx: number
): Omit<ArtifactSnapshot, 'updatedAt' | 'status'> & { path: string; sizeBytes: number } {
  return {
    artifactId: `art-${idx}`,
    kind: 'presentation' as const,
    title: `Deck ${idx}`,
    path: `artifacts/art-${idx}.pptx`,
    sizeBytes: 1024 * idx,
  };
}

describe('ChatFilesChip', () => {
  it('renders nothing when the thread has zero ready artifacts', () => {
    const store = mkStore();
    const { container } = render(
      <Provider store={store}>
        <ChatFilesChip threadId={THREAD} />
      </Provider>
    );
    // Empty render, no chip in DOM.
    expect(container.firstChild).toBeNull();
    expect(screen.queryByTestId('chat-files-chip')).toBeNull();
  });

  it('hides itself when only in_progress artifacts exist (those live above composer)', () => {
    const store = mkStore();
    store.dispatch(
      upsertArtifactInProgressForThread({
        threadId: THREAD,
        artifactId: 'in-flight',
        kind: 'presentation',
        title: 'In flight',
      })
    );
    render(
      <Provider store={store}>
        <ChatFilesChip threadId={THREAD} />
      </Provider>
    );
    expect(screen.queryByTestId('chat-files-chip')).toBeNull();
  });

  it('shows the chip + numeric count when the thread has ready artifacts', () => {
    const store = mkStore();
    store.dispatch(upsertArtifactReadyForThread({ threadId: THREAD, ...readyArtifact(1) }));
    store.dispatch(upsertArtifactReadyForThread({ threadId: THREAD, ...readyArtifact(2) }));
    store.dispatch(upsertArtifactReadyForThread({ threadId: THREAD, ...readyArtifact(3) }));
    render(
      <Provider store={store}>
        <ChatFilesChip threadId={THREAD} />
      </Provider>
    );
    expect(screen.getByTestId('chat-files-chip')).toBeInTheDocument();
    expect(screen.getByTestId('chat-files-chip-count')).toHaveTextContent('3');
  });

  it('renders the singular aria-label when exactly one ready artifact exists', () => {
    const store = mkStore();
    store.dispatch(upsertArtifactReadyForThread({ threadId: THREAD, ...readyArtifact(1) }));
    render(
      <Provider store={store}>
        <ChatFilesChip threadId={THREAD} />
      </Provider>
    );
    // Singular form — no trailing "s" on "file".
    expect(screen.getByTestId('chat-files-chip')).toHaveAttribute(
      'aria-label',
      '1 file in this chat'
    );
  });

  it('renders the plural aria-label when the count is greater than one', () => {
    const store = mkStore();
    store.dispatch(upsertArtifactReadyForThread({ threadId: THREAD, ...readyArtifact(1) }));
    store.dispatch(upsertArtifactReadyForThread({ threadId: THREAD, ...readyArtifact(2) }));
    render(
      <Provider store={store}>
        <ChatFilesChip threadId={THREAD} />
      </Provider>
    );
    expect(screen.getByTestId('chat-files-chip')).toHaveAttribute(
      'aria-label',
      '2 files in this chat'
    );
  });

  it('opens the panel on chip click and exposes per-row download/delete actions', () => {
    const store = mkStore();
    store.dispatch(upsertArtifactReadyForThread({ threadId: THREAD, ...readyArtifact(1) }));
    render(
      <Provider store={store}>
        <ChatFilesChip threadId={THREAD} />
      </Provider>
    );
    expect(screen.queryByTestId('chat-files-panel')).toBeNull();
    fireEvent.click(screen.getByTestId('chat-files-chip'));
    expect(screen.getByTestId('chat-files-panel')).toBeInTheDocument();
    expect(screen.getByTestId('chat-files-download-art-1')).toBeInTheDocument();
    expect(screen.getByTestId('chat-files-delete-art-1')).toBeInTheDocument();
  });
});
