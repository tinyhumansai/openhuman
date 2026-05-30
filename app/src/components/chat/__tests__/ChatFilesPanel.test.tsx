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

  it('Delete → typed code surfaces the localized headline, not the raw detail', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    // Service returns the new shape: structured code + raw RPC string.
    // UI must prefer the localized headline mapped from `code`.
    vi.mocked(deleteArtifact).mockResolvedValueOnce({
      ok: false,
      code: 'DELETE_FAILED',
      error: 'rpc transport closed',
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
    fireEvent.click(screen.getByTestId('chat-files-delete-art-1'));
    fireEvent.click(screen.getByTestId('chat-files-confirm-art-1'));
    await waitFor(() => {
      // Localized headline from chat.files.error.delete_failed (en).
      expect(screen.getByText('Couldn’t delete the file. Please try again.')).toBeInTheDocument();
    });
    // Raw detail MUST NOT leak into the user-facing surface.
    expect(screen.queryByText('rpc transport closed')).toBeNull();
  });

  it('passes the title-derived extension to downloadArtifact when title carries one', async () => {
    const a = readyArtifact('art-1', 'climate-deck.pptx');
    vi.mocked(downloadArtifact).mockResolvedValueOnce({ ok: true, path: '/d/x.pptx' });
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
      expect(downloadArtifact).toHaveBeenCalledWith('art-1', 'climate-deck.pptx', 'pptx');
    });
  });

  it.each([
    ['document' as const, 'pdf'],
    ['image' as const, 'png'],
    ['other' as const, 'bin'],
  ])(
    'falls back to the per-kind extension default when the title has none (kind=%s → ext=%s)',
    async (kind, expectedExt) => {
      const a: ArtifactSnapshot = {
        artifactId: 'art-1',
        kind,
        title: 'no-extension-title',
        status: 'ready',
        path: 'artifacts/x',
        sizeBytes: 1024,
        updatedAt: Date.now(),
      };
      vi.mocked(downloadArtifact).mockResolvedValueOnce({ ok: true, path: '/d/x' });
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
        expect(downloadArtifact).toHaveBeenCalledWith('art-1', 'no-extension-title', expectedExt);
      });
    }
  );

  it('Esc closes the panel via the keydown handler', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    const onClose = vi.fn();
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
        <ChatFilesPanel threadId={THREAD} artifacts={[a]} onClose={onClose} />
      </Provider>
    );
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Esc while a row is in confirm-delete mode dismisses the confirm, NOT the panel', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    const onClose = vi.fn();
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
        <ChatFilesPanel threadId={THREAD} artifacts={[a]} onClose={onClose} />
      </Provider>
    );
    fireEvent.click(screen.getByTestId('chat-files-delete-art-1'));
    expect(screen.getByText('Delete this file?')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByText('Delete this file?')).toBeNull();
  });

  it('pointerdown outside the panel closes it (click-outside)', () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    const onClose = vi.fn();
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
        <ChatFilesPanel threadId={THREAD} artifacts={[a]} onClose={onClose} />
      </Provider>
    );
    // Fire a pointerdown event whose target is outside the panel.
    const outside = document.createElement('div');
    document.body.appendChild(outside);
    const evt = new PointerEvent('pointerdown', { bubbles: true });
    Object.defineProperty(evt, 'target', { value: outside });
    document.dispatchEvent(evt);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Download → typed code surfaces the localized headline in the row error', async () => {
    const a = readyArtifact('art-1', 'Climate Deck');
    // Use a deliberately distinct raw `error` so this test only passes when
    // the UI renders the localized copy keyed off `code`, not the raw
    // backend detail. If the row ever regressed to echoing `error` verbatim,
    // the `queryByText(rawError)` assertion below would catch it.
    const rawError = 'transport socket closed mid-resolve';
    vi.mocked(downloadArtifact).mockResolvedValueOnce({
      ok: false,
      code: 'NOT_DESKTOP',
      error: rawError,
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
      // The download_failed copy is wrapped by the chat.artifact.download_failed
      // template (`{reason}`), which renders the localized inner text.
      expect(
        screen.getByText(/Downloads are only available in the desktop app/)
      ).toBeInTheDocument();
    });
    // Raw backend detail must NOT leak into the rendered row.
    expect(screen.queryByText(rawError)).toBeNull();
  });
});
