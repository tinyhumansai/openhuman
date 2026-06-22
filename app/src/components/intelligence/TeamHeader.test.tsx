import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AgentTeam, AgentTeamMember } from '../../services/api/agentTeamApi';
import { TeamHeader } from './TeamHeader';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const team: AgentTeam = {
  id: 'team-1',
  leadAgentId: 'lead',
  status: 'active',
  summary: 'ship it',
  createdAt: '2026-01-01T00:00:00Z',
  updatedAt: '2026-01-01T00:00:00Z',
};

function member(
  id: string,
  name: string,
  status: AgentTeamMember['memberStatus']
): AgentTeamMember {
  return {
    id,
    teamId: 'team-1',
    name,
    memberStatus: status,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
  };
}

describe('TeamHeader', () => {
  it('renders no start affordance without onStartMember', () => {
    render(<TeamHeader team={team} members={[member('m1', 'alice', 'idle')]} taskCount={1} />);
    expect(screen.queryByLabelText(/intelligence.teams.member.start/)).not.toBeInTheDocument();
  });

  it('starts an idle member and omits the button on an active member', () => {
    const onStart = vi.fn();
    render(
      <TeamHeader
        team={team}
        members={[member('m1', 'alice', 'idle'), member('m2', 'bob', 'active')]}
        taskCount={2}
        onStartMember={onStart}
      />
    );
    // Only the idle member exposes a start button.
    const buttons = screen.getAllByLabelText(/intelligence.teams.member.start/);
    expect(buttons).toHaveLength(1);
    fireEvent.click(buttons[0]);
    expect(onStart).toHaveBeenCalledWith('m1');
  });

  it('disables the start button for the member being dispatched', () => {
    const onStart = vi.fn();
    render(
      <TeamHeader
        team={team}
        members={[member('m1', 'alice', 'idle')]}
        taskCount={1}
        onStartMember={onStart}
        startingMemberId="m1"
      />
    );
    expect(screen.getByLabelText(/intelligence.teams.member.start/)).toBeDisabled();
  });
});
