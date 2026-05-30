import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  deleteArtifact,
  downloadArtifact,
  revealArtifactInFileManager,
} from '../../../services/artifactDownloadService';
import chatRuntimeReducer, {
  type ArtifactSnapshot,
  upsertArtifactReadyForThread,
} from '../../../store/chatRuntimeSlice';
import ChatFilesPanel from '../ChatFilesPanel';

vi.mock('../../../services/artifactDownloadService', () => ({
  downloadArtifact: vi.fn(),
  deleteArtifact: vi.fn(),
  revealArtifactInFileManager: vi.fn(),
}));

const THREAD = 't-panel-1';

function mkStore(artifactPayloads: Array<Parameters<typeof upsertArtifactReadyForThread>[0]>) {
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  for (const p of artifactPayloads) {
    store.dispatch(upsertArtifactReadyForThread(p));
  }
  return store;
}

function readyArtifact(id: string, title: string): ArtifactSnapshot {
  return {
    artifactId: id,
    kind: 'presentation',
    title,
    status: 'ready',
    path: `artifacts/${id}.pptx`,
    sizeBytes: 4096,
    updatedAt: Date.now(),
  };
}

describe('ChatFilesPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders an empty-state message when the panel is opened with no artifacts', () => {
    const store = mkStore([]);
    render(
      <Provider store={store}>
        <ChatFilesPanel threadId={THREAD} artifacts={[]} onClose={() => {}} />
      </Provider>
    );
    expect(screen.getByText('No files yet. Ask the agent to generate one.')).toBeInTheDocument();
  });

  it('lists rows + per-row actions when populated', () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    const b = readyArtifact('art-2', 'Q2 Report');
    const store = mkStore([
      {
        threadId: THREAD,
        artifactId: a.artifactId,
        kind: a.kind,
        title: a.title,
        path: a.path!,
        sizeBytes: a.sizeBytes!,
      },
      {
        threadId: THREAD,
        artifactId: b.artifactId,
        kind: b.kind,
        title: b.title,
        path: b.path!,
        sizeBytes: b.sizeBytes!,
      },
    ]);
    render(
      <Provider store={store}>
        <ChatFilesPanel threadId={THREAD} artifacts={[a, b]} onClose={() => {}} />
      </Provider>
    );
    expect(screen.getByText('Climate Deck')).toBeInTheDocument();
    expect(screen.getByText('Q2 Report')).toBeInTheDocument();
    expect(screen.getByTestId('chat-files-download-art-1')).toBeInTheDocument();
    expect(screen.getByTestId('chat-files-delete-art-2')).toBeInTheDocument();
  });

  it('on Download click → calls downloadArtifact + surfaces a Show-in-folder button on success', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    vi.mocked(downloadArtifact).mockResolvedValueOnce({
      ok: true,
      path: '/Users/me/Downloads/Climate Deck.pptx',
    });
    const store = mkStore([
      {
        threadId: THREAD,
        artifactId: a.artifactId,
        kind: a.kind,
        title: a.title,
        path: a.path!,
        sizeBytes: a.sizeBytes!,
      },
    ]);
    render(
      <Provider store={store}>
        <ChatFilesPanel threadId={THREAD} artifacts={[a]} onClose={() => {}} />
      </Provider>
    );
    fireEvent.click(screen.getByTestId('chat-files-download-art-1'));
    await waitFor(() => {
      expect(screen.getByTestId('chat-files-reveal-art-1')).toBeInTheDocument();
    });
    expect(downloadArtifact).toHaveBeenCalledWith('art-1', 'Climate Deck', 'pptx');

    fireEvent.click(screen.getByTestId('chat-files-reveal-art-1'));
    await waitFor(() => {
      expect(revealArtifactInFileManager).toHaveBeenCalledWith(
        '/Users/me/Downloads/Climate Deck.pptx'
      );
    });
  });

  it('Delete → Cancel keeps the artifact and does NOT call the RPC', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    const store = mkStore([
      {
        threadId: THREAD,
        artifactId: a.artifactId,
        kind: a.kind,
        title: a.title,
        path: a.path!,
        sizeBytes: a.sizeBytes!,
      },
    ]);
    render(
      <Provider store={store}>
        <ChatFilesPanel threadId={THREAD} artifacts={[a]} onClose={() => {}} />
      </Provider>
    );
    fireEvent.click(screen.getByTestId('chat-files-delete-art-1'));
    expect(screen.getByText('Delete this file?')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Cancel'));
    expect(deleteArtifact).not.toHaveBeenCalled();
    expect(store.getState().chatRuntime.artifactsByThread[THREAD]).toHaveLength(1);
  });

  it('Delete → Confirm → RPC ok → row removed from slice', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    vi.mocked(deleteArtifact).mockResolvedValueOnce({ ok: true });
    const store = mkStore([
      {
        threadId: THREAD,
        artifactId: a.artifactId,
        kind: a.kind,
        title: a.title,
        path: a.path!,
        sizeBytes: a.sizeBytes!,
      },
    ]);
    render(
      <Provider store={store}>
        <ChatFilesPanel threadId={THREAD} artifacts={[a]} onClose={() => {}} />
      </Provider>
    );
    fireEvent.click(screen.getByTestId('chat-files-delete-art-1'));
    fireEvent.click(screen.getByTestId('chat-files-confirm-art-1'));
    await waitFor(() => {
      expect(deleteArtifact).toHaveBeenCalledWith('art-1');
    });
    // Bucket should be empty (last row removed → key deleted in reducer).
    expect(store.getState().chatRuntime.artifactsByThread[THREAD]).toBeUndefined();
  });

  it('Delete → Confirm → RPC fails → row re-inserted + error surfaced', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    vi.mocked(deleteArtifact).mockResolvedValueOnce({ ok: false, error: 'core dropped' });
    const store = mkStore([
      {
        threadId: THREAD,
        artifactId: a.artifactId,
        kind: a.kind,
        title: a.title,
        path: a.path!,
        sizeBytes: a.sizeBytes!,
      },
    ]);
    render(
      <Provider store={store}>
        <ChatFilesPanel threadId={THREAD} artifacts={[a]} onClose={() => {}} />
      </Provider>
    );
    fireEvent.click(screen.getByTestId('chat-files-delete-art-1'));
    fireEvent.click(screen.getByTestId('chat-files-confirm-art-1'));
    await waitFor(() => {
      expect(screen.getByText('core dropped')).toBeInTheDocument();
    });
    // Re-inserted: bucket still has the entry.
    expect(store.getState().chatRuntime.artifactsByThread[THREAD]).toHaveLength(1);
    expect(store.getState().chatRuntime.artifactsByThread[THREAD][0].artifactId).toBe('art-1');
  });
});
