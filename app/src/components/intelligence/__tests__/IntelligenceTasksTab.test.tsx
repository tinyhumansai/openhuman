/**
 * Vitest for IntelligenceTasksTab.
 *
 * Covers:
 *  - Loading state while listTurnStates is in-flight.
 *  - Error state when listTurnStates rejects.
 *  - Empty state when no boards have any cards.
 *  - Board aggregation: persisted boards from turn-state list are shown.
 *  - Live boards from Redux take priority and render a "live" badge.
 *  - Thread title resolution: threads with a title use it; unknown threads
 *    fall back to a shortened thread id.
 */
import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

const hoisted = vi.hoisted(() => ({
  listTurnStates: vi.fn(),
  selectorResult: {
    chatRuntime: { taskBoardByThread: {} as Record<string, unknown> },
    thread: { threads: [] as unknown[] },
  },
}));

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: { listTurnStates: hoisted.listTurnStates },
}));

vi.mock('../../../store/hooks', () => ({
  useAppSelector: (selector: (state: typeof hoisted.selectorResult) => unknown) =>
    selector(hoisted.selectorResult),
  useAppDispatch: () => vi.fn(),
}));

// TaskKanbanBoard is exercised by its own test; stub it to a simple
// table so we can assert on title/card-count without rendering the
// full kanban grid.
vi.mock('../../../pages/conversations/components/TaskKanbanBoard', () => ({
  TaskKanbanBoard: ({ board }: { board: { cards: { title: string }[] } }) => (
    <div data-testid="kanban-stub">
      {board.cards.map(c => (
        <span key={c.title}>{c.title}</span>
      ))}
    </div>
  ),
}));

async function importTab() {
  const mod = await import('../IntelligenceTasksTab');
  return mod.default;
}

function makeBoard(threadId: string, cardTitles: string[]) {
  return {
    threadId,
    cards: cardTitles.map((title, i) => ({
      id: `card-${i}`,
      title,
      status: 'todo' as const,
      order: i,
      updatedAt: '2026-01-01T00:00:00Z',
    })),
    updatedAt: '2026-01-01T00:00:00Z',
  };
}

function renderTab(Tab: React.ComponentType) {
  const { render } = require('@testing-library/react');
  render(<Tab />);
}

describe('IntelligenceTasksTab', () => {
  beforeEach(() => {
    vi.resetModules();
    hoisted.listTurnStates.mockReset();
    hoisted.selectorResult.chatRuntime.taskBoardByThread = {};
    hoisted.selectorResult.thread.threads = [];
  });

  test('shows loading spinner while fetching', async () => {
    // Never resolves during this test
    hoisted.listTurnStates.mockReturnValue(new Promise(() => {}));
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    expect(screen.getByText(/loading task boards/i)).toBeInTheDocument();
  });

  test('shows error message when listTurnStates rejects', async () => {
    hoisted.listTurnStates.mockRejectedValue(new Error('rpc failed'));
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText(/rpc failed/i)).toBeInTheDocument();
    });
  });

  test('shows empty-state when no boards have cards', async () => {
    hoisted.listTurnStates.mockResolvedValue([
      { threadId: 'thread-a', taskBoard: null },
      { threadId: 'thread-b', taskBoard: makeBoard('thread-b', []) },
    ]);
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText(/no agent task boards yet/i)).toBeInTheDocument();
    });
  });

  test('renders persisted boards from turn-state list', async () => {
    hoisted.listTurnStates.mockResolvedValue([
      { threadId: 'thread-x', taskBoard: makeBoard('thread-x', ['Write docs', 'Fix bug']) },
    ]);
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByTestId('kanban-stub')).toBeInTheDocument();
    });
    expect(screen.getByText('Write docs')).toBeInTheDocument();
    expect(screen.getByText('Fix bug')).toBeInTheDocument();
  });

  test('resolves thread title from thread list', async () => {
    hoisted.listTurnStates.mockResolvedValue([
      { threadId: 'thread-y', taskBoard: makeBoard('thread-y', ['Task A']) },
    ]);
    hoisted.selectorResult.thread.threads = [
      { id: 'thread-y', title: 'Research sprint', labels: [] },
    ];
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('Research sprint')).toBeInTheDocument();
    });
  });

  test('falls back to shortened thread id when title is missing', async () => {
    hoisted.listTurnStates.mockResolvedValue([
      { threadId: 'abcdef1234567890', taskBoard: makeBoard('abcdef1234567890', ['Plan']) },
    ]);
    // No entry in thread list
    hoisted.selectorResult.thread.threads = [];
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      // Shortened id — last 8 chars are "34567890"
      expect(screen.getByText(/34567890/)).toBeInTheDocument();
    });
  });

  test('live boards from Redux take priority and show "live" badge', async () => {
    hoisted.listTurnStates.mockResolvedValue([
      { threadId: 'thread-live', taskBoard: makeBoard('thread-live', ['Old card']) },
    ]);
    hoisted.selectorResult.chatRuntime.taskBoardByThread = {
      'thread-live': makeBoard('thread-live', ['Live card']),
    };
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('Live card')).toBeInTheDocument();
    });
    // The live badge is present
    expect(screen.getByText('live')).toBeInTheDocument();
  });

  test('shows count of active boards', async () => {
    hoisted.listTurnStates.mockResolvedValue([
      { threadId: 'ta', taskBoard: makeBoard('ta', ['A']) },
      { threadId: 'tb', taskBoard: makeBoard('tb', ['B']) },
    ]);
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText(/2 active boards/i)).toBeInTheDocument();
    });
  });
});
