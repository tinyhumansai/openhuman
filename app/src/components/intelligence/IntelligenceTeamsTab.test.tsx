import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { type AgentTeam, agentTeamApi, type TeamView } from '../../services/api/agentTeamApi';
import IntelligenceTeamsTab from './IntelligenceTeamsTab';

vi.mock('../../services/api/agentTeamApi', () => ({
  agentTeamApi: { list: vi.fn(), get: vi.fn(), listMessages: vi.fn() },
}));
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const mockList = vi.mocked(agentTeamApi.list);
const mockGet = vi.mocked(agentTeamApi.get);
const mockMessages = vi.mocked(agentTeamApi.listMessages);

function team(id: string, summary: string): AgentTeam {
  return {
    id,
    leadAgentId: `lead-${id}`,
    status: 'active',
    summary,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  };
}

function view(t: AgentTeam): TeamView {
  return {
    team: t,
    members: [
      {
        id: 'm1',
        teamId: t.id,
        name: 'planner',
        memberStatus: 'active',
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
      },
    ],
    tasks: [
      {
        id: 'task-1',
        teamId: t.id,
        title: 'Audit flow',
        status: 'todo',
        dependsOn: [],
        gateStatus: 'pending',
        evidence: [],
        orderIndex: 0,
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
      },
    ],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockMessages.mockResolvedValue([]);
});

describe('IntelligenceTeamsTab', () => {
  it('shows the empty state when there are no teams', async () => {
    mockList.mockResolvedValue([]);
    render(<IntelligenceTeamsTab />);
    expect(await screen.findByText('intelligence.teams.empty')).toBeInTheDocument();
    expect(screen.getByText('intelligence.teams.emptyHint')).toBeInTheDocument();
  });

  it('requests active teams only (closed teams are not surfaced)', async () => {
    mockList.mockResolvedValue([]);
    render(<IntelligenceTeamsTab />);
    await screen.findByText('intelligence.teams.empty');
    expect(mockList).toHaveBeenCalledWith({ status: 'active' });
  });

  it('surfaces a load error', async () => {
    mockList.mockRejectedValue(new Error('core down'));
    render(<IntelligenceTeamsTab />);
    expect(await screen.findByText(/intelligence.teams.failedToLoad/)).toBeInTheDocument();
    expect(screen.getByText(/core down/)).toBeInTheDocument();
  });

  it('recovers from a load error via the retry button', async () => {
    // First list rejects (error state), the retry succeeds. Counter-based impl
    // (not mock*Once) keeps this independent of any implementation inherited
    // from a prior test under the clearMocks-only config. Two teams on retry so
    // the assertion lands on the stable list view (no auto-select detail race).
    let calls = 0;
    mockList.mockImplementation(() => {
      calls += 1;
      return calls === 1
        ? Promise.reject(new Error('core down'))
        : Promise.resolve([team('team-1', 'Alpha'), team('team-2', 'Beta')]);
    });
    render(<IntelligenceTeamsTab />);

    await screen.findByText(/core down/);
    fireEvent.click(screen.getByText('intelligence.teams.refresh'));
    expect(await screen.findByText('Alpha')).toBeInTheDocument();
    expect(screen.getByText('Beta')).toBeInTheDocument();
  });

  it('ignores a stale detail response after the selection changes', async () => {
    const a = team('team-1', 'Team A');
    const b = team('team-2', 'Team B');
    mockList.mockResolvedValue([a, b]);

    let resolveA: (v: TeamView) => void = () => {};
    const slowA = new Promise<TeamView>(resolve => {
      resolveA = resolve;
    });
    mockGet.mockImplementation((id: string) =>
      id === 'team-1' ? slowA : Promise.resolve(view(b))
    );

    render(<IntelligenceTeamsTab />);
    await screen.findByText('Team A'); // list view (no auto-select with >1 team)

    fireEvent.click(screen.getByText('Team A')); // select A — detail fetch hangs
    fireEvent.click(screen.getByText('Team B')); // switch to B before A resolves
    await screen.findByText('Audit flow'); // B's detail rendered

    resolveA(view(a)); // late A response must NOT overwrite B
    await waitFor(() => expect(screen.getByText('Team B')).toBeInTheDocument());
    expect(screen.queryByText('Team A')).not.toBeInTheDocument();
  });

  it('recovers from a detail-fetch error via the retry button', async () => {
    // Regression for the stuck-error bug: one team auto-selects, its detail
    // fetch fails, then the retry succeeds. `refresh` must clear `error` first
    // or the error branch short-circuits the render and the recovered board is
    // never shown. Counter-based impl, not mock*Once (clearMocks-only config).
    mockList.mockResolvedValue([team('team-1', 'Ship onboarding')]);
    let calls = 0;
    mockGet.mockImplementation(() => {
      calls += 1;
      return calls === 1
        ? Promise.reject(new Error('detail timeout'))
        : Promise.resolve(view(team('team-1', 'Ship onboarding')));
    });
    render(<IntelligenceTeamsTab />);

    await screen.findByText(/detail timeout/);
    fireEvent.click(screen.getByText('intelligence.teams.refresh'));
    expect(await screen.findByText('Audit flow')).toBeInTheDocument();
    expect(screen.queryByText(/detail timeout/)).not.toBeInTheDocument();
  });

  it('auto-selects and renders the board when there is exactly one team', async () => {
    const t = team('team-1', 'Ship onboarding');
    mockList.mockResolvedValue([t]);
    mockGet.mockResolvedValue(view(t));
    render(<IntelligenceTeamsTab />);

    // Await a detail-only element (the task title) so the assertion can't
    // resolve early on the brief single-team list view, then check the rest.
    expect(await screen.findByText('Audit flow')).toBeInTheDocument();
    expect(screen.getByText('Ship onboarding')).toBeInTheDocument();
    expect(screen.getByText('intelligence.teams.column.todo')).toBeInTheDocument();
    expect(screen.getByText('intelligence.teams.activity.title')).toBeInTheDocument();
    expect(mockGet).toHaveBeenCalledWith('team-1');
  });

  it('lists multiple teams and opens one on click', async () => {
    const a = team('team-1', 'Ship onboarding');
    const b = team('team-2', 'Fix billing');
    mockList.mockResolvedValue([a, b]);
    mockGet.mockResolvedValue(view(a));
    render(<IntelligenceTeamsTab />);

    // List view first (no auto-select with >1 team).
    expect(await screen.findByText('Ship onboarding')).toBeInTheDocument();
    expect(screen.getByText('Fix billing')).toBeInTheDocument();
    expect(mockGet).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText('Ship onboarding'));
    await waitFor(() => expect(mockGet).toHaveBeenCalledWith('team-1'));
    expect(await screen.findByText('Audit flow')).toBeInTheDocument();
  });

  it('refreshes the list via the refresh button', async () => {
    mockList.mockResolvedValue([team('team-1', 'A'), team('team-2', 'B')]);
    render(<IntelligenceTeamsTab />);
    await screen.findByText('A');
    expect(mockList).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByText('intelligence.teams.refresh'));
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));
  });
});
