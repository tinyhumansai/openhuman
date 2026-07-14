import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { TaskBoard, TaskBoardCard } from '../../../types/turnState';
import {
  isTauri,
  openhumanTaskSourcesFetch,
  openhumanTaskSourcesList,
  openhumanTaskSourcesStatus,
  openhumanTaskSourcesSync,
  openhumanTaskSourcesUpdate,
} from '../../../utils/tauriCommands';
import { TaskKanbanBoard } from './TaskKanbanBoard';

// Echo i18n keys so we can query by the stable key strings.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(),
  openhumanTaskSourcesFetch: vi.fn(),
  openhumanTaskSourcesList: vi.fn(),
  openhumanTaskSourcesStatus: vi.fn(),
  openhumanTaskSourcesSync: vi.fn(),
  openhumanTaskSourcesUpdate: vi.fn(),
}));

// TaskSourceControls navigates to the settings page via useNavigate(); these
// tests render the board without a <Router>, so capture navigation via a spy.
const navigateSpy = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return {
    ...actual,
    useNavigate: () => navigateSpy,
    useLocation: () => ({ pathname: '/chat', search: '', hash: '', state: null, key: 'test' }),
  };
});

function card(partial: Partial<TaskBoardCard>): TaskBoardCard {
  return {
    id: 'c1',
    title: 'Do thing',
    status: 'todo',
    order: 0,
    updatedAt: '',
    ...partial,
  } as TaskBoardCard;
}

function board(cards: TaskBoardCard[]): TaskBoard {
  return { threadId: 't1', cards, updatedAt: '' };
}

describe('TaskKanbanBoard approval surface', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(openhumanTaskSourcesList).mockResolvedValue([
      {
        id: 'src-1',
        provider: 'github',
        name: 'Open issues',
        enabled: true,
        filter: { provider: 'github', repo: 'tinyhumans/openhuman', assignee_is_me: true },
        intervalSecs: 600,
        target: 'agent_todo_proactive',
        maxTasksPerFetch: 10,
        createdAt: '2026-06-02T00:00:00Z',
      },
    ]);
    vi.mocked(openhumanTaskSourcesStatus).mockResolvedValue({
      enabled: true,
      defaultIntervalSecs: 600,
      sourceCount: 1,
      enabledSourceCount: 1,
    });
    vi.mocked(openhumanTaskSourcesFetch).mockResolvedValue({
      sourceId: 'src-1',
      provider: 'github',
      fetched: 3,
      routed: 2,
      skippedDupe: 1,
      pruned: 0,
    });
    vi.mocked(openhumanTaskSourcesSync).mockResolvedValue([
      { sourceId: 'src-1', provider: 'github', fetched: 3, routed: 2, skippedDupe: 1, pruned: 1 },
    ]);
    vi.mocked(openhumanTaskSourcesUpdate).mockResolvedValue({
      id: 'src-1',
      provider: 'github',
      name: 'Open issues',
      enabled: false,
      filter: { provider: 'github', repo: 'tinyhumans/openhuman', assignee_is_me: true },
      intervalSecs: 600,
      target: 'agent_todo_proactive',
      maxTasksPerFetch: 10,
      createdAt: '2026-06-02T00:00:00Z',
    });
  });

  it('renders Approve/Reject on an awaiting_approval card and calls onDecidePlan', () => {
    const onDecidePlan = vi.fn();
    render(
      <TaskKanbanBoard
        board={board([card({ id: 'a', status: 'awaiting_approval', title: 'Needs approval' })])}
        onDecidePlan={onDecidePlan}
      />
    );

    fireEvent.click(screen.getByTitle('chat.approval.approve'));
    expect(onDecidePlan).toHaveBeenCalledWith(expect.objectContaining({ id: 'a' }), true);

    fireEvent.click(screen.getByTitle('chat.approval.deny'));
    expect(onDecidePlan).toHaveBeenCalledWith(expect.objectContaining({ id: 'a' }), false);
  });

  it('buckets ready→in_progress column and rejected→done column so the cards still render', () => {
    render(
      <TaskKanbanBoard
        board={board([
          card({ id: 'r', status: 'ready', title: 'Ready card' }),
          card({ id: 'x', status: 'rejected', title: 'Rejected card' }),
        ])}
      />
    );

    expect(screen.getByText('Ready card')).toBeInTheDocument();
    expect(screen.getByText('Rejected card')).toBeInTheDocument();
    // An approval-flow card without onDecidePlan shows no approve/reject controls.
    expect(screen.queryByTitle('chat.approval.approve')).toBeNull();
  });

  it('edit dialog status select has a matching option for approval-flow statuses', () => {
    // Regression: the dialog <select> must carry an <option> for every status,
    // not just the four column statuses — otherwise an awaiting_approval card
    // renders a controlled select with no matching option (React warns and the
    // value silently shows as the first option, hiding the real status).
    render(
      <TaskKanbanBoard
        board={board([card({ id: 'a', status: 'awaiting_approval', title: 'Needs approval' })])}
        onUpdateCard={vi.fn()}
      />
    );

    fireEvent.click(screen.getByText('conversations.taskKanban.briefButton'));

    // The status select shows the awaiting_approval label as its selected
    // value, proving a matching option exists (no fallback to 'todo').
    expect(
      screen.getByDisplayValue('conversations.taskKanban.awaitingApproval')
    ).toBeInTheDocument();
  });

  it('renders task-source metadata on cards and in the brief dialog', () => {
    render(
      <TaskKanbanBoard
        board={board([
          card({
            id: 'sourced',
            title: '[github] Fix issue',
            sourceMetadata: {
              provider: 'github',
              source_id: 'src-1',
              external_id: '42',
              repo: 'tinyhumans/openhuman',
              url: 'https://github.com/tinyhumans/openhuman/issues/42',
              urgency: 0.82,
            },
          }),
        ])}
      />
    );

    expect(
      screen.getByText('settings.taskSources.providers.github · tinyhumans/openhuman#42')
    ).toBeInTheDocument();
    expect(screen.getByText('conversations.taskKanban.source.openExternalShort')).toHaveAttribute(
      'href',
      'https://github.com/tinyhumans/openhuman/issues/42'
    );

    fireEvent.click(screen.getByText('conversations.taskKanban.briefButton'));

    expect(screen.getByText('conversations.taskKanban.source.title')).toBeInTheDocument();
    expect(screen.getByText('src-1')).toBeInTheDocument();
    expect(screen.getByText('42')).toBeInTheDocument();
    expect(screen.getByText('conversations.taskKanban.source.urgencyValue')).toBeInTheDocument();
  });

  it('opens task-source controls and calls fetch/toggle actions', async () => {
    render(<TaskKanbanBoard board={{ threadId: 'task-sources', updatedAt: '', cards: [] }} />);

    fireEvent.click(screen.getByText('conversations.taskKanban.sourcesButton'));

    expect(await screen.findByText('Open issues')).toBeInTheDocument();

    fireEvent.click(screen.getByText('settings.taskSources.fetchNow'));
    await waitFor(() => expect(openhumanTaskSourcesFetch).toHaveBeenCalledWith('src-1'));

    fireEvent.click(screen.getByText('settings.taskSources.syncAll'));
    await waitFor(() => expect(openhumanTaskSourcesSync).toHaveBeenCalled());

    fireEvent.click(screen.getByText('settings.taskSources.disable'));
    await waitFor(() =>
      expect(openhumanTaskSourcesUpdate).toHaveBeenCalledWith('src-1', { enabled: false })
    );

    // "Manage sources" jumps to the merged Integrations settings page
    // (task-sources was folded into /settings/integrations).
    fireEvent.click(screen.getByText('conversations.taskKanban.sources.manage'));
    expect(navigateSpy).toHaveBeenCalledWith('/settings/integrations', {
      state: { backgroundLocation: expect.objectContaining({ pathname: '/chat' }) },
    });
  });

  it('shows a "View work" button on a card with a session thread and calls onViewSession', () => {
    const onViewSession = vi.fn();
    render(
      <TaskKanbanBoard
        board={board([
          card({
            id: 'c1',
            title: 'Worked card',
            status: 'in_progress',
            sessionThreadId: 'task-xyz',
          }),
        ])}
        onViewSession={onViewSession}
      />
    );

    fireEvent.click(screen.getByTitle('conversations.taskKanban.viewWork'));
    expect(onViewSession).toHaveBeenCalledWith(expect.objectContaining({ id: 'c1' }));
  });

  it('omits the "View work" button when the card has no session thread', () => {
    render(
      <TaskKanbanBoard
        board={board([card({ id: 'c2', title: 'Unworked card', status: 'todo' })])}
        onViewSession={vi.fn()}
      />
    );

    expect(screen.queryByTitle('conversations.taskKanban.viewWork')).toBeNull();
  });
});
