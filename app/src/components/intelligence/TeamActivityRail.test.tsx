import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AgentTeamMember, TeamMessage } from '../../services/api/agentTeamApi';
import { TeamActivityRail } from './TeamActivityRail';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function member(id: string, name: string): AgentTeamMember {
  return {
    id,
    teamId: 'team-1',
    name,
    memberStatus: 'active',
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  };
}

function message(seq: number, from: string, to: string | null, content: string): TeamMessage {
  return {
    runId: 'team-1',
    sequence: seq,
    eventType: 'team_message',
    payload: { from, to, content, visibility: 'team' },
    timestamp: '2026-01-01T00:00:00Z',
  };
}

const members = [member('m1', 'planner'), member('m2', 'builder')];

describe('TeamActivityRail', () => {
  it('shows the empty state when there are no messages', () => {
    render(<TeamActivityRail messages={[]} members={members} />);
    expect(screen.getByText('intelligence.teams.activity.empty')).toBeInTheDocument();
  });

  it('renders sender name and message content', () => {
    render(
      <TeamActivityRail messages={[message(1, 'm1', 'm2', 'split the build')]} members={members} />
    );
    expect(screen.getByText('planner')).toBeInTheDocument();
    expect(screen.getByText('split the build')).toBeInTheDocument();
    expect(screen.getByText('builder', { exact: false })).toBeInTheDocument();
  });

  it('labels a broadcast (null recipient) as the team', () => {
    render(<TeamActivityRail messages={[message(1, 'm1', null, 'hi all')]} members={members} />);
    expect(
      screen.getByText('intelligence.teams.activity.toTeam', { exact: false })
    ).toBeInTheDocument();
  });
});
