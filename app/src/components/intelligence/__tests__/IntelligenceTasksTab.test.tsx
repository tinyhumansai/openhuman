/**
 * Vitest for IntelligenceTasksTab.
 *
 * Covers:
 *  - Loading state while the boards are in-flight.
 *  - Error state when listTurnStates rejects.
 *  - The personal board ({@link USER_TASKS_THREAD_ID}) is always shown, with
 *    an empty-state CTA when it has no cards, and is editable (move/delete)
 *    and refreshable from the create composer.
 *  - Agent board aggregation: persisted boards from the turn-state list are
 *    shown read-only; live boards from Redux take priority + a "live" badge.
 *  - Thread title resolution for agent boards.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

const hoisted = vi.hoisted(() => ({
  listTurnStates: vi.fn(),
  todosList: vi.fn(),
  todosAdd: vi.fn(),
  todosEdit: vi.fn(),
  todosUpdateStatus: vi.fn(),
  todosRemove: vi.fn(),
  selectorResult: {
    chatRuntime: { taskBoardByThread: {} as Record<string, unknown> },
    thread: { threads: [] as unknown[] },
  },
}));

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: { listTurnStates: hoisted.listTurnStates },
}));

vi.mock('../../../services/api/todosApi', () => ({
  USER_TASKS_THREAD_ID: 'user-tasks',
  todosApi: {
    list: hoisted.todosList,
    add: hoisted.todosAdd,
    edit: hoisted.todosEdit,
    updateStatus: hoisted.todosUpdateStatus,
    remove: hoisted.todosRemove,
  },
}));

vi.mock('../../../store/hooks', () => ({
  useAppSelector: (selector: (state: typeof hoisted.selectorResult) => unknown) =>
    selector(hoisted.selectorResult),
  useAppDispatch: () => vi.fn(),
}));

// Stub the composer so we can drive its `onCreated` callback without
// exercising its internals.
vi.mock('../UserTaskComposer', () => ({
  UserTaskComposer: ({ onCreated }: { onCreated: (threadId: string, board: unknown) => void }) => (
    <div data-testid="composer">
      <button
        type="button"
        onClick={() =>
          onCreated('user-tasks', {
            threadId: 'user-tasks',
            cards: [
              {
                id: 'created-0',
                title: 'Created card',
                status: 'todo',
                order: 0,
                updatedAt: '2026-01-01T00:00:00Z',
              },
            ],
            updatedAt: '2026-01-01T00:00:00Z',
          })
        }>
        stub-create
      </button>
    </div>
  ),
}));

// Stub the kanban to a simple list that still surfaces the write callbacks
// the personal board wires up, so we can assert the todos RPC is called.
vi.mock('../../../pages/conversations/components/TaskKanbanBoard', () => ({
  TaskKanbanBoard: ({
    board,
    onMove,
    onDeleteCard,
  }: {
    board: { cards: { id: string; title: string; status: string }[] };
    onMove?: (card: unknown, status: string) => void;
    onDeleteCard?: (card: unknown) => void;
  }) => (
    <div data-testid="kanban-stub">
      {board.cards.map(c => (
        <span key={c.id}>{c.title}</span>
      ))}
      {onMove && (
        <button type="button" onClick={() => onMove(board.cards[0], 'in_progress')}>
          stub-move
        </button>
      )}
      {onDeleteCard && (
        <button type="button" onClick={() => onDeleteCard(board.cards[0])}>
          stub-delete
        </button>
      )}
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
    hoisted.todosList.mockReset();
    hoisted.todosAdd.mockReset();
    hoisted.todosEdit.mockReset();
    hoisted.todosUpdateStatus.mockReset();
    hoisted.todosRemove.mockReset();
    hoisted.selectorResult.chatRuntime.taskBoardByThread = {};
    hoisted.selectorResult.thread.threads = [];
    // Sensible defaults: empty personal board, no agent boards.
    hoisted.listTurnStates.mockResolvedValue([]);
    hoisted.todosList.mockResolvedValue(makeBoard('user-tasks', []));
  });

  test('shows loading spinner while fetching', async () => {
    hoisted.listTurnStates.mockReturnValue(new Promise(() => {})); // never resolves
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

  test('always shows the personal board with an empty-state CTA', async () => {
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('No personal tasks yet')).toBeInTheDocument();
    });
    expect(screen.getByText('Agent Tasks')).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: /New task/ }).length).toBeGreaterThan(0);
  });

  test('renders persisted agent boards from the turn-state list', async () => {
    hoisted.listTurnStates.mockResolvedValue([
      { threadId: 'thread-x', taskBoard: makeBoard('thread-x', ['Write docs', 'Fix bug']) },
    ]);
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('Write docs')).toBeInTheDocument();
    });
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
    expect(screen.getByText('live')).toBeInTheDocument();
  });

  test('renders personal cards and moves one via the todos RPC', async () => {
    hoisted.todosList.mockResolvedValue(makeBoard('user-tasks', ['My personal task']));
    hoisted.todosUpdateStatus.mockResolvedValue(makeBoard('user-tasks', ['My personal task']));
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('My personal task')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText('stub-move'));
    await waitFor(() => expect(hoisted.todosUpdateStatus).toHaveBeenCalledTimes(1));
    expect(hoisted.todosUpdateStatus).toHaveBeenCalledWith('user-tasks', 'card-0', 'in_progress');
  });

  test('deletes a personal card via the todos RPC', async () => {
    hoisted.todosList.mockResolvedValue(makeBoard('user-tasks', ['Disposable']));
    hoisted.todosRemove.mockResolvedValue(makeBoard('user-tasks', []));
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('Disposable')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText('stub-delete'));
    await waitFor(() => expect(hoisted.todosRemove).toHaveBeenCalledTimes(1));
    expect(hoisted.todosRemove).toHaveBeenCalledWith('user-tasks', 'card-0');
  });

  test('opens the composer and applies the created personal board', async () => {
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => expect(screen.getByText('Agent Tasks')).toBeInTheDocument());

    fireEvent.click(screen.getAllByRole('button', { name: /New task/ })[0]);
    expect(screen.getByTestId('composer')).toBeInTheDocument();

    fireEvent.click(screen.getByText('stub-create'));
    await waitFor(() => {
      expect(screen.getByText('Created card')).toBeInTheDocument();
    });
  });
});
