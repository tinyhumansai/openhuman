import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { hotkeyManager } from '../../../lib/commands/hotkeyManager';
import { registry } from '../../../lib/commands/registry';
import { ScopeContext } from '../../../lib/commands/ScopeContext';
import type { Thread } from '../../../types/thread';
import CommandPalette from '../CommandPalette';

const mockSearchMessages = vi.fn();

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: { searchMessages: (...args: unknown[]) => mockSearchMessages(...args) },
}));

let frame: symbol;

beforeEach(() => {
  mockSearchMessages.mockReset();
  mockSearchMessages.mockResolvedValue([]);
  hotkeyManager.teardown();
  registry.reset();
  hotkeyManager.init();
  frame = hotkeyManager.pushFrame('global', 'root');
  registry.setActiveStack([frame]);
});

afterEach(() => {
  hotkeyManager.teardown();
  registry.reset();
});

function thread(id: string, title: string, lastMessageAt: string): Thread {
  return {
    id,
    title,
    chatId: null,
    isActive: false,
    messageCount: 1,
    lastMessageAt,
    createdAt: lastMessageAt,
    labels: [],
  };
}

const THREADS = [
  thread('t1', 'Quarterly plan', '2026-04-10T12:00:00Z'),
  thread('t2', 'Gmail cleanup', '2026-04-11T12:00:00Z'),
];

function renderPalette(onOpenThread = vi.fn(), onOpenChange = vi.fn()) {
  render(
    <ScopeContext.Provider value={frame}>
      <CommandPalette
        open={true}
        onOpenChange={onOpenChange}
        threads={THREADS}
        onOpenThread={onOpenThread}
      />
    </ScopeContext.Provider>
  );
  return { onOpenThread, onOpenChange };
}

describe('CommandPalette thread search', () => {
  it('lists nothing from threads until something is typed', () => {
    renderPalette();
    expect(screen.queryByTestId('palette-thread-t1')).not.toBeInTheDocument();
    expect(mockSearchMessages).not.toHaveBeenCalled();
  });

  it('matches conversations by title and opens the picked one', async () => {
    const user = userEvent.setup();
    const { onOpenThread, onOpenChange } = renderPalette();
    await user.type(screen.getByRole('combobox'), 'quarter');
    expect(screen.getByTestId('palette-thread-t1')).toBeInTheDocument();
    expect(screen.queryByTestId('palette-thread-t2')).not.toBeInTheDocument();

    await user.click(screen.getByTestId('palette-thread-t1'));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onOpenThread).toHaveBeenCalledWith('t1');
  });

  it('searches message text across threads and opens the hit thread', async () => {
    mockSearchMessages.mockResolvedValue([
      {
        threadId: 't2',
        messageId: 'm9',
        role: 'user',
        snippet: '…archive the newsletters…',
        createdAt: '2026-04-11T12:00:00Z',
      },
    ]);
    const user = userEvent.setup();
    const { onOpenThread } = renderPalette();
    await user.type(screen.getByRole('combobox'), 'newsletters');

    const hit = await screen.findByTestId('palette-message-m9', {}, { timeout: 2000 });
    expect(mockSearchMessages).toHaveBeenLastCalledWith('newsletters', 20);
    expect(hit).toHaveTextContent('Gmail cleanup');
    expect(hit).toHaveTextContent('archive the newsletters');

    await act(async () => {
      await user.click(hit);
    });
    expect(onOpenThread).toHaveBeenCalledWith('t2');
  });

  it('keeps commands working when thread search is off', async () => {
    const user = userEvent.setup();
    render(
      <ScopeContext.Provider value={frame}>
        <CommandPalette open={true} onOpenChange={() => {}} />
      </ScopeContext.Provider>
    );
    await user.type(screen.getByRole('combobox'), 'quarter');
    expect(mockSearchMessages).not.toHaveBeenCalled();
  });
});
