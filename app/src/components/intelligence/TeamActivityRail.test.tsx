import { fireEvent, render, screen, waitFor } from '@testing-library/react';
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

  it('renders no composer without onSend', () => {
    render(<TeamActivityRail messages={[]} members={members} />);
    expect(
      screen.queryByPlaceholderText('intelligence.teams.composer.placeholder')
    ).not.toBeInTheDocument();
  });

  it('sends a broadcast (null recipient) with the typed content and clears the draft', async () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<TeamActivityRail messages={[]} members={members} onSend={onSend} />);
    const input = screen.getByPlaceholderText(
      'intelligence.teams.composer.placeholder'
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: 'ship it' } });
    fireEvent.click(screen.getByLabelText('intelligence.teams.composer.send'));
    expect(onSend).toHaveBeenCalledWith(null, 'ship it');
    await waitFor(() => expect(input.value).toBe(''));
  });

  it('sends to the selected teammate and submits on Enter', () => {
    const onSend = vi.fn().mockResolvedValue(undefined);
    render(<TeamActivityRail messages={[]} members={members} onSend={onSend} />);
    fireEvent.change(screen.getByLabelText('intelligence.teams.composer.recipient'), {
      target: { value: 'm2' },
    });
    const input = screen.getByPlaceholderText('intelligence.teams.composer.placeholder');
    fireEvent.change(input, { target: { value: 'your turn' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSend).toHaveBeenCalledWith('m2', 'your turn');
  });

  it('does not send blank content', () => {
    const onSend = vi.fn();
    render(<TeamActivityRail messages={[]} members={members} onSend={onSend} />);
    fireEvent.change(screen.getByPlaceholderText('intelligence.teams.composer.placeholder'), {
      target: { value: '   ' },
    });
    fireEvent.click(screen.getByLabelText('intelligence.teams.composer.send'));
    expect(onSend).not.toHaveBeenCalled();
  });
});
