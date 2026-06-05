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
  createNewThread: vi.fn(),
  updateTitle: vi.fn(),
  appendMessage: vi.fn(),
  chatSend: vi.fn(),
  listAgentDefinitions: vi.fn(),
  todosList: vi.fn(),
  todosAdd: vi.fn(),
  todosEdit: vi.fn(),
  todosUpdateStatus: vi.fn(),
  todosSetSessionThread: vi.fn(),
  todosRemove: vi.fn(),
  navigate: vi.fn(),
  selectorResult: {
    chatRuntime: { taskBoardByThread: {} as Record<string, unknown> },
    thread: { threads: [] as unknown[] },
    agentProfiles: { activeProfileId: 'agent-profile-1' },
    locale: { current: 'en' },
  },
}));

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: {
    listTurnStates: hoisted.listTurnStates,
    createNewThread: hoisted.createNewThread,
    updateTitle: hoisted.updateTitle,
    appendMessage: hoisted.appendMessage,
  },
}));

vi.mock('../../../services/chatService', () => ({ chatSend: hoisted.chatSend }));

vi.mock('../../../services/api/agentLibraryApi', () => ({
  agentLibraryApi: { listDefinitions: hoisted.listAgentDefinitions },
}));

vi.mock('../../../services/api/todosApi', () => ({
  TASK_SOURCES_THREAD_ID: 'task-sources',
  USER_TASKS_THREAD_ID: 'user-tasks',
  todosApi: {
    list: hoisted.todosList,
    add: hoisted.todosAdd,
    edit: hoisted.todosEdit,
    updateStatus: hoisted.todosUpdateStatus,
    setSessionThread: hoisted.todosSetSessionThread,
    remove: hoisted.todosRemove,
  },
}));

vi.mock('../../../store/hooks', () => ({
  useAppSelector: (selector: (state: typeof hoisted.selectorResult) => unknown) =>
    selector(hoisted.selectorResult),
  useAppDispatch: () => vi.fn(),
}));

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => hoisted.navigate };
});

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
    headerTitleKey,
    onMove,
    onDeleteCard,
    onWorkTask,
    onViewSession,
  }: {
    board: {
      threadId: string;
      cards: { id: string; title: string; status: string; sessionThreadId?: string }[];
    };
    headerTitleKey?: string;
    onMove?: (card: unknown, status: string) => void;
    onDeleteCard?: (card: unknown) => void;
    onWorkTask?: (card: unknown) => void;
    onViewSession?: (card: unknown) => void;
  }) => (
    <div data-testid="kanban-stub">
      <span>{board.threadId}</span>
      {headerTitleKey && <span>{headerTitleKey}</span>}
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
      {onWorkTask && (
        <button type="button" onClick={() => onWorkTask(board.cards[0])}>
          stub-work-task
        </button>
      )}
      {onViewSession && (
        <button type="button" onClick={() => onViewSession(board.cards[0])}>
          stub-view-session
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
    hoisted.createNewThread.mockReset();
    hoisted.updateTitle.mockReset();
    hoisted.appendMessage.mockReset();
    hoisted.chatSend.mockReset();
    hoisted.listAgentDefinitions.mockReset();
    hoisted.todosList.mockReset();
    hoisted.todosAdd.mockReset();
    hoisted.todosEdit.mockReset();
    hoisted.todosUpdateStatus.mockReset();
    hoisted.todosSetSessionThread.mockReset();
    hoisted.todosRemove.mockReset();
    hoisted.navigate.mockReset();
    hoisted.selectorResult.chatRuntime.taskBoardByThread = {};
    hoisted.selectorResult.thread.threads = [];
    hoisted.selectorResult.agentProfiles.activeProfileId = 'agent-profile-1';
    hoisted.selectorResult.locale.current = 'en';
    // Sensible defaults: empty personal board, no agent boards.
    hoisted.listTurnStates.mockResolvedValue([]);
    hoisted.createNewThread.mockResolvedValue({
      id: 'thread-agent-task',
      title: 'Agent task',
      labels: ['tasks'],
      chatId: null,
      isActive: true,
      messageCount: 0,
      lastMessageAt: '2026-01-01T00:00:00Z',
      createdAt: '2026-01-01T00:00:00Z',
    });
    hoisted.updateTitle.mockResolvedValue({
      id: 'thread-agent-task',
      title: 'Agent task: My personal task',
      labels: ['tasks'],
      chatId: null,
      isActive: true,
      messageCount: 0,
      lastMessageAt: '2026-01-01T00:00:00Z',
      createdAt: '2026-01-01T00:00:00Z',
    });
    hoisted.appendMessage.mockResolvedValue({
      id: 'msg-1',
      content: 'Work this approved agent task from the task board.',
      type: 'text',
      extraMetadata: {},
      sender: 'user',
      createdAt: '2026-01-01T00:00:00Z',
    });
    hoisted.chatSend.mockResolvedValue(undefined);
    // The "Work" flow links the card to its session thread after starting it
    // (`todosApi.setSessionThread(...).catch(...)`), so resolve it to a board
    // by default — otherwise the awaited `.catch()` chain stalls the handler
    // before it reaches chatSend.
    hoisted.todosSetSessionThread.mockResolvedValue(makeBoard('user-tasks', []));
    hoisted.listAgentDefinitions.mockResolvedValue([
      {
        id: 'researcher',
        display_name: 'Researcher',
        when_to_use: 'Use for focused research.',
        tier: 'worker',
        model: { kind: 'hint', value: 'reasoning' },
        direct_tool_count: 1,
        direct_tool_names: ['web_search'],
        uses_wildcard_tools: false,
        subagent_ids: [],
        includes_profile: false,
        includes_memory_md: false,
        includes_memory_context: false,
        can_run_as_user_facing_worker: true,
        write_capable: false,
        source: 'builtin',
      },
    ]);
    hoisted.todosList.mockImplementation((threadId: string) =>
      Promise.resolve(makeBoard(threadId, []))
    );
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

  test('renders the task source list even when it is empty', async () => {
    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('Task Sources')).toBeInTheDocument();
    });
    expect(screen.getByText('No source tasks waiting.')).toBeInTheDocument();
    expect(hoisted.todosList).toHaveBeenCalledWith('task-sources');

    // "Manage sources" jumps to the dedicated settings page.
    fireEvent.click(screen.getByText('Manage sources'));
    expect(hoisted.navigate).toHaveBeenCalledWith('/settings/task-sources');
  });

  test('refines a source task and approves it into the personal agent board', async () => {
    hoisted.todosList.mockImplementation((threadId: string) =>
      Promise.resolve(
        threadId === 'task-sources'
          ? {
              threadId,
              cards: [
                {
                  id: 'source-1',
                  title: 'GitHub: tinyhumansai/openhuman#42: Fix source task',
                  status: 'todo',
                  objective: 'Fix the source task flow',
                  notes: 'Original notes',
                  sourceMetadata: {
                    provider: 'github',
                    repo: 'tinyhumansai/openhuman',
                    external_id: '42',
                    url: 'https://github.com/tinyhumansai/openhuman/issues/42',
                  },
                  order: 0,
                  updatedAt: '2026-01-01T00:00:00Z',
                },
              ],
              updatedAt: '2026-01-01T00:00:00Z',
            }
          : makeBoard(threadId, [])
      )
    );
    hoisted.todosAdd.mockResolvedValue({
      threadId: 'user-tasks',
      cards: [
        {
          id: 'agent-task-1',
          title: 'tinyhumansai/openhuman#42: Fix source task',
          status: 'todo',
          order: 0,
          updatedAt: '2026-06-03T00:00:00Z',
        },
      ],
      updatedAt: '2026-06-03T00:00:00Z',
    });
    hoisted.todosEdit.mockResolvedValue(makeBoard('user-tasks', ['Queued agent task']));
    hoisted.todosUpdateStatus.mockResolvedValue({
      threadId: 'task-sources',
      cards: [
        {
          id: 'source-1',
          title: 'GitHub: tinyhumansai/openhuman#42: Fix source task',
          status: 'done',
          order: 0,
          updatedAt: '2026-06-03T00:00:00Z',
        },
      ],
      updatedAt: '2026-06-03T00:00:00Z',
    });

    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);

    await waitFor(() => {
      expect(screen.getByText(/Fix source task/)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Work on task' }));

    expect(screen.getByText('Refine source task')).toBeInTheDocument();
    expect(screen.getByText('Research agent draft')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Approve Plan' }));

    await waitFor(() => expect(hoisted.todosAdd).toHaveBeenCalledTimes(1));
    expect(hoisted.todosAdd).toHaveBeenCalledWith(
      expect.objectContaining({
        threadId: 'user-tasks',
        content: 'tinyhumansai/openhuman#42: Fix source task',
        status: 'todo',
      })
    );
    await waitFor(() => expect(hoisted.todosEdit).toHaveBeenCalledTimes(1));
    expect(hoisted.todosEdit).toHaveBeenCalledWith(
      expect.objectContaining({
        threadId: 'user-tasks',
        id: 'agent-task-1',
        assignedAgent: 'orchestrator',
        approvalMode: 'not_required',
      })
    );
    expect(hoisted.todosUpdateStatus).toHaveBeenCalledWith('task-sources', 'source-1', 'done');
  });

  test('work flow still completes when linking the session thread fails', async () => {
    hoisted.todosList.mockImplementation((threadId: string) =>
      Promise.resolve(
        threadId === 'user-tasks'
          ? {
              threadId,
              cards: [
                {
                  id: 'personal-1',
                  title: 'Linkless task',
                  status: 'todo',
                  order: 0,
                  updatedAt: '2026-01-01T00:00:00Z',
                },
              ],
              updatedAt: '2026-01-01T00:00:00Z',
            }
          : makeBoard(threadId, [])
      )
    );
    // The session-thread link rejects; the work flow must fall back and still
    // dispatch the agent turn.
    hoisted.todosSetSessionThread.mockRejectedValue(new Error('link offline'));

    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);

    await waitFor(() => expect(screen.getByText('Linkless task')).toBeInTheDocument());
    fireEvent.click(screen.getByText('stub-work-task'));

    await waitFor(() => expect(hoisted.chatSend).toHaveBeenCalledTimes(1));
  });

  test('View work on a personal card opens its exact session thread', async () => {
    hoisted.todosList.mockImplementation((threadId: string) =>
      Promise.resolve(
        threadId === 'user-tasks'
          ? {
              threadId,
              cards: [
                {
                  id: 'personal-1',
                  title: 'Worked card',
                  status: 'in_progress',
                  sessionThreadId: 'task-session-99',
                  order: 0,
                  updatedAt: '2026-01-01T00:00:00Z',
                },
              ],
              updatedAt: '2026-01-01T00:00:00Z',
            }
          : makeBoard(threadId, [])
      )
    );

    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);

    await waitFor(() => expect(screen.getByText('Worked card')).toBeInTheDocument());
    fireEvent.click(screen.getByText('stub-view-session'));

    expect(hoisted.navigate).toHaveBeenCalledWith('/chat', {
      state: { openThreadId: 'task-session-99' },
    });
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
    hoisted.todosList.mockImplementation((threadId: string) =>
      Promise.resolve(
        threadId === 'user-tasks'
          ? makeBoard('user-tasks', ['My personal task'])
          : makeBoard(threadId, [])
      )
    );
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

  test('starts a labeled Tasks thread from a personal task', async () => {
    hoisted.todosList.mockImplementation((threadId: string) =>
      Promise.resolve(
        threadId === 'user-tasks'
          ? {
              threadId: 'user-tasks',
              cards: [
                {
                  id: 'personal-1',
                  title: 'Implement task source worker',
                  status: 'todo',
                  objective: 'Ship the task source worker flow',
                  notes: 'Use the approved plan.',
                  plan: ['Read the source issue', 'Implement the flow'],
                  acceptanceCriteria: ['Agent thread is labeled'],
                  allowedTools: ['shell', 'edit'],
                  order: 0,
                  updatedAt: '2026-01-01T00:00:00Z',
                },
              ],
              updatedAt: '2026-01-01T00:00:00Z',
            }
          : makeBoard(threadId, [])
      )
    );
    hoisted.todosUpdateStatus.mockResolvedValue(
      makeBoard('user-tasks', ['Implement task source worker'])
    );

    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('Implement task source worker')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText('stub-work-task'));

    await waitFor(() => expect(hoisted.createNewThread).toHaveBeenCalledWith(['tasks']));
    expect(hoisted.updateTitle).toHaveBeenCalledWith(
      'thread-agent-task',
      'Agent task: Implement task source worker'
    );
    expect(hoisted.appendMessage).toHaveBeenCalledWith(
      'thread-agent-task',
      expect.objectContaining({
        content: expect.stringContaining('Task: Implement task source worker'),
        sender: 'user',
        extraMetadata: expect.objectContaining({
          source: 'agent-task-board',
          taskCardId: 'personal-1',
        }),
      })
    );
    expect(hoisted.todosUpdateStatus).toHaveBeenCalledWith(
      'user-tasks',
      'personal-1',
      'in_progress'
    );
    // chatSend is the last call in the work flow, after an extra `await`
    // (session-thread link), so wait for it rather than asserting synchronously.
    await waitFor(() =>
      expect(hoisted.chatSend).toHaveBeenCalledWith(
        expect.objectContaining({
          threadId: 'thread-agent-task',
          message: expect.stringContaining('Acceptance criteria:'),
          model: 'reasoning-v1',
          profileId: 'agent-profile-1',
          locale: 'en',
        })
      )
    );
  });

  test('starts a labeled task thread from an explicit library agent selection', async () => {
    hoisted.todosUpdateStatus.mockResolvedValue(makeBoard('user-tasks', []));

    vi.resetModules();
    const Tab = await importTab();
    renderTab(Tab);
    await waitFor(() => {
      expect(screen.getByText('Researcher')).toBeInTheDocument();
    });

    fireEvent.change(screen.getByPlaceholderText(/task for this agent/i), {
      target: { value: 'Find the current API docs' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Run task/i }));

    await waitFor(() =>
      expect(hoisted.createNewThread).toHaveBeenCalledWith(['tasks', 'agent-library'])
    );
    expect(hoisted.appendMessage).toHaveBeenCalledWith(
      'thread-agent-task',
      expect.objectContaining({
        content: expect.stringContaining('@agent:researcher'),
        extraMetadata: expect.objectContaining({
          source: 'agent-library',
          explicitAgentId: 'researcher',
        }),
      })
    );
    expect(hoisted.chatSend).toHaveBeenCalledWith(
      expect.objectContaining({
        threadId: 'thread-agent-task',
        message: expect.stringContaining('Find the current API docs'),
        model: 'reasoning-v1',
        profileId: 'agent-profile-1',
      })
    );
  });

  test('deletes a personal card via the todos RPC', async () => {
    hoisted.todosList.mockImplementation((threadId: string) =>
      Promise.resolve(
        threadId === 'user-tasks'
          ? makeBoard('user-tasks', ['Disposable'])
          : makeBoard(threadId, [])
      )
    );
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

  test('re-polls the personal + task-source boards on an interval (background runs show live)', async () => {
    vi.useFakeTimers();
    try {
      hoisted.todosList.mockImplementation((threadId: string) =>
        Promise.resolve(makeBoard(threadId, []))
      );
      vi.resetModules();
      const Tab = await importTab();
      renderTab(Tab);

      // Flush the mount effect's setTimeout(0) + the initial loadAll fetches.
      await vi.advanceTimersByTimeAsync(0);
      const userBefore = hoisted.todosList.mock.calls.filter(c => c[0] === 'user-tasks').length;
      const sourceBefore = hoisted.todosList.mock.calls.filter(c => c[0] === 'task-sources').length;
      expect(userBefore).toBeGreaterThan(0);
      expect(sourceBefore).toBeGreaterThan(0);

      // One poll interval later, both boards are re-read (so a background poller
      // run's board changes surface without a manual refresh).
      await vi.advanceTimersByTimeAsync(4000);
      const userAfter = hoisted.todosList.mock.calls.filter(c => c[0] === 'user-tasks').length;
      const sourceAfter = hoisted.todosList.mock.calls.filter(c => c[0] === 'task-sources').length;
      expect(userAfter).toBeGreaterThan(userBefore);
      expect(sourceAfter).toBeGreaterThan(sourceBefore);
    } finally {
      vi.useRealTimers();
    }
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
