import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

const hoisted = vi.hoisted(() => ({
  createNewThread: vi.fn(),
  updateTitle: vi.fn(),
  appendMessage: vi.fn(),
  chatSend: vi.fn(),
  navigate: vi.fn(),
  dispatch: vi.fn(),
  selectorResult: {
    agentProfiles: { activeProfileId: 'agent-profile-1' },
    locale: { current: 'en' },
  },
}));

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: hoisted.createNewThread,
    updateTitle: hoisted.updateTitle,
    appendMessage: hoisted.appendMessage,
  },
}));

vi.mock('../../../services/chatService', () => ({ chatSend: hoisted.chatSend }));

vi.mock('../../../store/hooks', () => ({
  useAppSelector: (selector: (state: typeof hoisted.selectorResult) => unknown) =>
    selector(hoisted.selectorResult),
  useAppDispatch: () => hoisted.dispatch,
}));

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => hoisted.navigate };
});

vi.mock('../AgentsLibraryPanel', () => ({
  default: ({
    onRunAgentTask,
    runningAgentId,
  }: {
    onRunAgentTask: (
      agent: {
        id: string;
        display_name: string;
        tier: string;
        model: { kind: string; value: string };
        direct_tool_count: number;
        direct_tool_names: string[];
        uses_wildcard_tools: boolean;
        subagent_ids: string[];
        includes_profile: boolean;
        includes_memory_md: boolean;
        includes_memory_context: boolean;
        can_run_as_user_facing_worker: boolean;
        write_capable: boolean;
        source: string;
      },
      task: string
    ) => Promise<void>;
    runningAgentId?: string | null;
  }) => (
    <div>
      <span data-testid="running-agent">{runningAgentId ?? 'idle'}</span>
      <button
        type="button"
        onClick={() =>
          onRunAgentTask(
            {
              id: 'researcher',
              display_name: 'Researcher',
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
            'Find the current API docs'
          )
        }>
        run researcher
      </button>
    </div>
  ),
}));

async function importTab() {
  const mod = await import('../IntelligenceAgentsTab');
  return mod.default;
}

describe('IntelligenceAgentsTab', () => {
  beforeEach(() => {
    vi.resetModules();
    hoisted.createNewThread.mockReset();
    hoisted.updateTitle.mockReset();
    hoisted.appendMessage.mockReset();
    hoisted.chatSend.mockReset();
    hoisted.navigate.mockReset();
    hoisted.dispatch.mockReset();
    hoisted.selectorResult.agentProfiles.activeProfileId = 'agent-profile-1';
    hoisted.selectorResult.locale.current = 'en';
    hoisted.createNewThread.mockResolvedValue({
      id: 'thread-agent-task',
      title: 'Agent task',
      labels: ['tasks', 'agent-library'],
      chatId: null,
      isActive: true,
      messageCount: 0,
      lastMessageAt: '2026-01-01T00:00:00Z',
      createdAt: '2026-01-01T00:00:00Z',
    });
    hoisted.updateTitle.mockResolvedValue({
      id: 'thread-agent-task',
      title: 'Agent task: Find the current API docs',
      labels: ['tasks', 'agent-library'],
      chatId: null,
      isActive: true,
      messageCount: 0,
      lastMessageAt: '2026-01-01T00:00:00Z',
      createdAt: '2026-01-01T00:00:00Z',
    });
    hoisted.appendMessage.mockResolvedValue({
      id: 'msg-1',
      content: '@agent:researcher',
      type: 'text',
      extraMetadata: {},
      sender: 'user',
      createdAt: '2026-01-01T00:00:00Z',
    });
    hoisted.chatSend.mockResolvedValue(undefined);
  });

  test('starts a labeled task thread from an explicit library agent selection', async () => {
    const Tab = await importTab();
    render(<Tab />);

    fireEvent.click(screen.getByRole('button', { name: /run researcher/i }));

    await waitFor(() =>
      expect(hoisted.createNewThread).toHaveBeenCalledWith(['tasks', 'agent-library'])
    );
    expect(hoisted.updateTitle).toHaveBeenCalledWith(
      'thread-agent-task',
      'Agent task: Find the current API docs'
    );
    expect(hoisted.appendMessage).toHaveBeenCalledWith(
      'thread-agent-task',
      expect.objectContaining({
        content: expect.stringContaining('@agent:researcher'),
        sender: 'user',
        extraMetadata: expect.objectContaining({
          source: 'agent-library',
          explicitAgentId: 'researcher',
        }),
      })
    );
    await waitFor(() =>
      expect(hoisted.chatSend).toHaveBeenCalledWith(
        expect.objectContaining({
          threadId: 'thread-agent-task',
          message: expect.stringContaining('Find the current API docs'),
          model: 'reasoning-v1',
          profileId: 'agent-profile-1',
          locale: 'en',
        })
      )
    );
    expect(hoisted.navigate).toHaveBeenCalledWith('/chat/thread-agent-task');
  });
});
