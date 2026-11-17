import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import chatRuntimeReducer, {
  openArtifact,
  upsertArtifactReadyForThread,
} from '../../../store/chatRuntimeSlice';
import { ArtifactPreviewPanel, AUTOSAVE_DELAY_MS } from './ArtifactPreviewPanel';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));

const rpc = vi.mocked(callCoreRpc);
const body = (text: string, version: number) => ({
  kind: 'markdown',
  title: 'Notes',
  version,
  body: text,
  is_text: true,
  converted: false,
});
const ready = () =>
  upsertArtifactReadyForThread({
    threadId: 't-1',
    artifactId: 'a-1',
    kind: 'document',
    title: 'Notes',
    path: '/tmp/n.md',
    sizeBytes: 4,
  });

function setup() {
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  store.dispatch(ready());
  store.dispatch(openArtifact('a-1'));
  render(
    <Provider store={store}>
      <ArtifactPreviewPanel />
    </Provider>
  );
  return store;
}

describe('ArtifactPreviewPanel — versions saved elsewhere', () => {
  beforeEach(() => rpc.mockReset());

  it('shows an agent revision of the open artifact', async () => {
    rpc.mockResolvedValueOnce(body('# first', 1));
    const store = setup();
    expect((await screen.findByRole('heading', { level: 1 })).textContent).toBe('first');

    rpc.mockResolvedValueOnce(body('# revised by the agent', 2));
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 2));
      store.dispatch(ready());
    });
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('revised by the agent')
    );
  });

  it('keeps an unsaved edit when a revision arrives', async () => {
    rpc.mockResolvedValueOnce(body('# first', 1));
    const store = setup();
    await screen.findByRole('heading', { level: 1 });
    fireEvent.mouseDown(screen.getByRole('tab', { name: /chat.artifact.view.code/ }));
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '# my edit' } });

    rpc.mockResolvedValueOnce(body('# revised by the agent', 2));
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 2));
      store.dispatch(ready());
    });
    await waitFor(() => expect(rpc).toHaveBeenCalledTimes(2));
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('# my edit');
  });
});

describe('ArtifactPreviewPanel — download', () => {
  beforeEach(() => rpc.mockReset());

  it('says why a download is not possible instead of failing silently', async () => {
    rpc.mockResolvedValueOnce(body('# first', 1));
    setup();
    await screen.findByRole('heading', { level: 1 });
    // Not the desktop app: the service refuses before any dialog.
    fireEvent.click(screen.getByRole('button', { name: 'chat.artifact.download' }));
    expect(await screen.findByText('chat.files.error.not_desktop')).toBeTruthy();
  });
});

describe('ArtifactPreviewPanel — resize handle', () => {
  beforeEach(() => rpc.mockReset());

  it('resizes with the arrow keys, within the bounds', async () => {
    rpc.mockResolvedValue(body('# first', 1));
    setup();
    const handle = screen.getByRole('separator', { name: 'chat.artifact.panel.resize' });
    const width = () => Number(handle.getAttribute('aria-valuenow'));
    const start = width();

    fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    expect(width()).toBe(start + 24);
    fireEvent.keyDown(handle, { key: 'ArrowRight' });
    fireEvent.keyDown(handle, { key: 'ArrowRight' });
    expect(width()).toBe(start - 24);
    for (let i = 0; i < 100; i++) fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    expect(width()).toBe(Number(handle.getAttribute('aria-valuemax')));
  });
});

describe('ArtifactPreviewPanel — a failed save, then a revision', () => {
  beforeEach(() => rpc.mockReset());
  afterEach(() => vi.useRealTimers());

  it('keeps the revision and never re-sends the failed edit', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const saves = () =>
      rpc.mock.calls.filter(([req]) => req?.method === 'openhuman.ai_save_artifact_version');
    rpc.mockImplementation(async req => {
      if (req?.method === 'openhuman.ai_save_artifact_version') throw new Error('core restarting');
      return body('# first', 1);
    });
    const store = setup();
    await screen.findByRole('heading', { level: 1 });
    fireEvent.mouseDown(screen.getByRole('tab', { name: /chat.artifact.view.code/ }));
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '# my edit' } });

    // The save fails; a retry is pending.
    await act(async () => vi.advanceTimersByTime(AUTOSAVE_DELAY_MS));
    await waitFor(() => expect(saves()).toHaveLength(1));

    // The agent saves a revision.
    rpc.mockImplementation(async req => {
      if (req?.method === 'openhuman.ai_save_artifact_version') throw new Error('core restarting');
      return body('# revised by the agent', 2);
    });
    await act(async () => {
      vi.advanceTimersByTime(2);
      store.dispatch(ready());
    });
    await waitFor(() =>
      expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe(
        '# revised by the agent'
      )
    );

    // The old edit's retry is gone.
    await act(async () => vi.advanceTimersByTime(AUTOSAVE_DELAY_MS * 5));
    expect(saves()).toHaveLength(1);
  });
});
