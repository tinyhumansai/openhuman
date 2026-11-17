import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../../services/api/threadApi';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import type { Thread } from '../../../types/thread';
import { ThreadWorkspaceChip } from './ThreadWorkspaceChip';

vi.mock('../../../lib/i18n/I18nContext', async importOriginal => ({
  ...(await importOriginal<object>()),
  useT: () => ({ t: (key: string) => key }),
}));
vi.mock('../../../utils/tauriCommands/config', async importOriginal => ({
  ...(await importOriginal<object>()),
  openhumanGetAgentPaths: vi
    .fn()
    .mockResolvedValue({ result: { action_dir: '/home/me/projects' } }),
}));
vi.mock('../../../utils/tauriCommands/common', async importOriginal => ({
  ...(await importOriginal<object>()),
  isTauri: () => true,
}));
const pickDirectoryNatively = vi.fn();
vi.mock('../../../utils/tauriCommands/directoryPicker', () => ({
  pickDirectoryNatively: () => pickDirectoryNatively(),
}));

function thread(id: string, extra: Partial<Thread> = {}): Thread {
  return {
    id,
    title: 'Chat',
    chatId: null,
    isActive: true,
    messageCount: 0,
    lastMessageAt: new Date(2026, 9, 6).toISOString(),
    createdAt: new Date(2026, 9, 6).toISOString(),
    labels: [],
    ...extra,
  };
}

function renderChip(threads: Thread[], threadId: string | null) {
  const base = createTestStore().getState() as unknown as { thread: Record<string, unknown> };
  const store = createTestStore({
    thread: { ...base.thread, threads, selectedThreadId: threadId },
  });
  renderWithProviders(<ThreadWorkspaceChip threadId={threadId} />, { store });
  return store;
}

function openMenu() {
  fireEvent.pointerDown(screen.getByRole('button', { name: 'composer.workspace.label' }), {
    button: 0,
    ctrlKey: false,
  });
}

describe('ThreadWorkspaceChip', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    pickDirectoryNatively.mockReset();
  });

  it('renders nothing once the thread has messages', () => {
    renderChip([thread('t1', { messageCount: 2 })], 't1');
    expect(screen.queryByTestId('composer-workspace')).toBeNull();
  });

  it('binds a chosen folder on the empty thread', async () => {
    pickDirectoryNatively.mockResolvedValue({ ok: true, path: '/home/me/work/api' });
    const update = vi
      .spyOn(threadApi, 'updateWorkingDir')
      .mockResolvedValue(thread('t1', { actionDir: '/home/me/work/api' }));
    renderChip([thread('t1')], 't1');

    openMenu();
    fireEvent.click(screen.getByTestId('composer-workspace-choose'));

    await waitFor(() => expect(update).toHaveBeenCalledWith('t1', '/home/me/work/api'));
    await waitFor(() =>
      expect(screen.getByTestId('composer-workspace-label')).toHaveTextContent('api')
    );
  });

  it('shows the core refusal when a folder cannot be bound', async () => {
    vi.spyOn(threadApi, 'updateWorkingDir').mockRejectedValue(
      new Error('working folder is in a protected location')
    );
    renderChip([thread('t1'), thread('t0', { actionDir: '/home/me/.ssh-ish' })], 't1');

    openMenu();
    fireEvent.click(screen.getByTestId('composer-workspace-recent'));

    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent(
        'working folder is in a protected location'
      )
    );
  });
});
