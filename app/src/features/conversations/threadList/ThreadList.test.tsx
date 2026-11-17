import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Thread } from '../../../types/thread';
import { PINNED_THREAD_LABEL } from './groupThreads';
import { ThreadList } from './ThreadList';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
// The real module pulls in the whole chat surface; the list only needs the
// IME guard.
vi.mock('../Conversations', () => ({ isImeCompositionKeyEvent: () => false }));

const NOW = new Date(2026, 9, 6, 15, 0, 0);

function thread(id: string, title: string, daysAgo: number, extra: Partial<Thread> = {}): Thread {
  const at = new Date(NOW.getFullYear(), NOW.getMonth(), NOW.getDate() - daysAgo, 12);
  return {
    id,
    title,
    chatId: null,
    isActive: true,
    messageCount: 1,
    lastMessageAt: at.toISOString(),
    createdAt: at.toISOString(),
    labels: [],
    ...extra,
  };
}

const THREADS = [
  thread('t1', 'Fix Gmail OAuth', 0),
  thread('t2', 'Plan trip', 1),
  thread('t3', 'Old notes', 60, { labels: [PINNED_THREAD_LABEL] }),
  thread('t4', 'Refactor sidebar', 3, { actionDir: '/home/me/projects/site/' }),
];

function renderList(props: Partial<Parameters<typeof ThreadList>[0]> = {}) {
  const titles = new Map(THREADS.map(t => [t.id, t.title]));
  return render(
    <ThreadList
      threads={THREADS}
      selectedThreadId={null}
      onCreateThread={vi.fn()}
      onSelectThread={vi.fn()}
      resolveTitle={id => titles.get(id) ?? id}
      onRequestDelete={vi.fn()}
      onRenameThread={vi.fn(async () => {})}
      {...props}
    />
  );
}

describe('ThreadList', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(NOW);
  });
  afterEach(() => vi.useRealTimers());

  it('groups threads into pinned and recency sections in order', () => {
    renderList();
    const sections = screen.getAllByRole('region');
    expect(sections.map(s => s.getAttribute('data-testid'))).toEqual([
      'thread-group-pinned',
      'thread-group-today',
      'thread-group-yesterday',
      'thread-group-previous7Days',
    ]);
    expect(within(sections[0]).getByTestId('thread-row-t3')).toBeInTheDocument();
  });

  it('has no search box in the conversation sidebar', () => {
    renderList();
    expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
  });

  it('toggles a pin through the registry menu without selecting the row', async () => {
    const onTogglePin = vi.fn();
    const onSelectThread = vi.fn();
    renderList({ onTogglePin, onSelectThread });
    await userEvent.click(
      within(screen.getByTestId('thread-row-t1').parentElement!).getByRole('button', {
        name: 'assistantUi.threadList.moreOptions',
      })
    );
    await userEvent.click(await screen.findByTestId('thread-pin-t1'));
    expect(onTogglePin).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }), true);
    await userEvent.click(
      within(screen.getByTestId('thread-row-t3').parentElement!).getByRole('button', {
        name: 'assistantUi.threadList.moreOptions',
      })
    );
    await userEvent.click(await screen.findByTestId('thread-pin-t3'));
    expect(onTogglePin).toHaveBeenCalledWith(expect.objectContaining({ id: 't3' }), false);
    expect(onSelectThread).not.toHaveBeenCalled();
  });

  it('hides the pin action when no handler is given', () => {
    renderList();
    expect(screen.queryByTestId('thread-pin-t1')).not.toBeInTheDocument();
  });

  it('shows unread only for idle threads and an explicit loader for running threads', () => {
    renderList({ unreadThreadIds: new Set(['t1', 't2']), isThreadRunning: id => id === 't2' });
    expect(screen.getByTestId('thread-unread-t1')).toBeInTheDocument();
    expect(screen.queryByTestId('thread-unread-t2')).not.toBeInTheDocument();
    const running = within(screen.getByTestId('thread-row-t2')).getByText('Plan trip');
    expect(running).toHaveAttribute('data-running', 'true');
    expect(running).not.toHaveClass('shimmer');
    expect(
      screen
        .getByTestId('thread-row-t2')
        .querySelector('[data-slot="aui_thread-list-item-running"]')
    ).toBeInTheDocument();
  });

  it('creates and switches conversations through the runtime adapter', async () => {
    const onCreateThread = vi.fn();
    const onSelectThread = vi.fn();
    renderList({ onCreateThread, onSelectThread });
    await userEvent.click(screen.getByTestId('new-thread-button'));
    await userEvent.click(screen.getByTestId('thread-row-t2'));
    await waitFor(() => expect(onCreateThread).toHaveBeenCalledTimes(1));
    expect(onSelectThread).toHaveBeenCalledWith('t2');
  });

  it('renames in place with the registry editor and keeps deletion behind the host confirmation', async () => {
    const onRenameThread = vi.fn(async () => {});
    const onRequestDelete = vi.fn();
    renderList({ onRenameThread, onRequestDelete });
    await userEvent.click(
      within(screen.getByTestId('thread-row-t1').parentElement!).getByRole('button', {
        name: 'assistantUi.threadList.moreOptions',
      })
    );
    await userEvent.click(
      await screen.findByRole('menuitem', { name: 'assistantUi.threadList.rename' })
    );
    const input = screen.getByTestId('thread-title-input-t1');
    fireEvent.change(input, { target: { value: 'Updated title' } });
    await userEvent.keyboard('{Enter}');
    await waitFor(() => expect(onRenameThread).toHaveBeenCalledWith('t1', 'Updated title'));
    await userEvent.click(
      within(screen.getByTestId('thread-row-t1').parentElement!).getByRole('button', {
        name: 'assistantUi.threadList.moreOptions',
      })
    );
    await userEvent.click(await screen.findByRole('menuitem', { name: 'common.delete' }));
    expect(onRequestDelete).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }));
  });

  it('cancels a rename on Escape without committing the subsequent blur', async () => {
    const onRenameThread = vi.fn(async () => {});
    renderList({ onRenameThread });
    await userEvent.click(
      within(screen.getByTestId('thread-row-t1').parentElement!).getByRole('button', {
        name: 'assistantUi.threadList.moreOptions',
      })
    );
    await userEvent.click(
      await screen.findByRole('menuitem', { name: 'assistantUi.threadList.rename' })
    );
    const input = screen.getByTestId('thread-title-input-t1');
    await userEvent.clear(input);
    await userEvent.type(input, 'Must not be committed');
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByTestId('thread-title-input-t1')).toBeNull());
    expect(onRenameThread).not.toHaveBeenCalled();
    expect(screen.getByTestId('thread-row-t1')).toHaveTextContent('Fix Gmail OAuth');
    expect(screen.getByTestId('thread-row-t1')).toHaveFocus();
  });

  it('cancels a rename while the closed menu is still animating out', async () => {
    // JSDOM has no CSS animations. Keep the real Radix menu mounted during
    // its closing animation, as it is in the browser when Escape is pressed.
    const getComputedStyle = window.getComputedStyle.bind(window);
    const styleSpy = vi.spyOn(window, 'getComputedStyle').mockImplementation(element => {
      const style = getComputedStyle(element);
      if (element.getAttribute('data-slot') === 'aui_thread-list-item-more-content') {
        Object.defineProperty(style, 'animationName', {
          get: () => (element.getAttribute('data-state') === 'closed' ? 'menu-out' : 'menu-in'),
        });
      }
      return style;
    });
    const onRenameThread = vi.fn(async () => {});
    const view = renderList({ onRenameThread });
    try {
      await userEvent.click(
        within(screen.getByTestId('thread-row-t1').parentElement!).getByRole('button', {
          name: 'assistantUi.threadList.moreOptions',
        })
      );
      const menu = await screen.findByRole('menu');
      await userEvent.click(
        within(menu).getByRole('menuitem', { name: 'assistantUi.threadList.rename' })
      );
      const input = screen.getByTestId('thread-title-input-t1');
      await userEvent.clear(input);
      await userEvent.type(input, 'Must not be committed during menu close');
      expect(menu).toBeInTheDocument();
      expect(menu).toHaveAttribute('data-state', 'closed');
      await userEvent.keyboard('{Escape}');
      await waitFor(() => expect(screen.queryByTestId('thread-title-input-t1')).toBeNull());
      expect(onRenameThread).not.toHaveBeenCalled();
      expect(screen.getByTestId('thread-row-t1')).toHaveTextContent('Fix Gmail OAuth');
    } finally {
      view.unmount();
      styleSpy.mockRestore();
    }
  });

  it('puts the working folder name in the row tooltip', () => {
    renderList();
    expect(screen.getByTestId('thread-row-t4')).toHaveAttribute(
      'title',
      'chat.sidebar.workingFolder'.replace('{folder}', 'site')
    );
    expect(screen.getByTestId('thread-row-t1')).not.toHaveAttribute('title');
  });
});
