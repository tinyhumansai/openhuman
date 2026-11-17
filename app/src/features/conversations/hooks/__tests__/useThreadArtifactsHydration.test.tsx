import { configureStore } from '@reduxjs/toolkit';
import { act, renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ARTIFACT_IDS_METADATA_KEY } from '../../../../services/api/artifactRows';
import { listArtifactsForThread } from '../../../../services/artifactDownloadService';
import chatRuntimeReducer from '../../../../store/chatRuntimeSlice';
import threadReducer, { loadThreadMessages } from '../../../../store/threadSlice';
import type { ThreadMessage } from '../../../../types/thread';
import { useThreadArtifactsHydration } from '../useThreadArtifactsHydration';

vi.mock('../../../../services/artifactDownloadService', () => ({
  listArtifactsForThread: vi.fn(),
}));

const list = vi.mocked(listArtifactsForThread);
const listed = (artifactId: string) => ({
  artifactId,
  kind: 'document' as const,
  title: artifactId,
  path: `/tmp/${artifactId}.md`,
  sizeBytes: 4,
});
const reply = (ids: string[]): ThreadMessage => ({
  id: 'agent:req-1',
  content: 'Made it.',
  type: 'text',
  extraMetadata: { requestId: 'req-1', [ARTIFACT_IDS_METADATA_KEY]: ids },
  sender: 'agent',
  createdAt: '2026-10-16T06:34:47Z',
});

function setup() {
  const store = configureStore({
    reducer: { chatRuntime: chatRuntimeReducer, thread: threadReducer },
  });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  renderHook(() => useThreadArtifactsHydration('t-1'), { wrapper });
  return store;
}

describe('useThreadArtifactsHydration', () => {
  beforeEach(() => list.mockReset());

  it('loads again when a reply lists an artifact the store does not have', async () => {
    // Opened before the artifact existed.
    list.mockResolvedValueOnce({ ok: true, artifacts: [] });
    const store = setup();
    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));

    // Its live event was missed (the socket reconnected); the re-read reply
    // lists it.
    list.mockResolvedValue({ ok: true, artifacts: [listed('a-new')] });
    act(() => {
      store.dispatch(
        loadThreadMessages.fulfilled({ threadId: 't-1', messages: [reply(['a-new'])] }, 'r', 't-1')
      );
    });

    await waitFor(() =>
      expect(store.getState().chatRuntime.artifactsByThread['t-1']?.map(a => a.artifactId)).toEqual(
        ['a-new']
      )
    );
    expect(list).toHaveBeenCalledWith('t-1');
  });

  it('does not keep loading for an id the core does not list', async () => {
    list.mockResolvedValue({ ok: true, artifacts: [] });
    const store = setup();
    act(() => {
      store.dispatch(
        loadThreadMessages.fulfilled({ threadId: 't-1', messages: [reply(['gone'])] }, 'r', 't-1')
      );
    });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    await new Promise(resolve => setTimeout(resolve, 50));
    expect(list).toHaveBeenCalledTimes(2);
  });
});
