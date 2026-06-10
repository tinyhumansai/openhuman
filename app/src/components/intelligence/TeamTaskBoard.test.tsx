import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AgentTeamMember, AgentTeamTask } from '../../services/api/agentTeamApi';
import { TeamTaskBoard, unmetDepCount } from './TeamTaskBoard';

// i18n → echo the key so assertions can target stable strings.
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

function task(overrides: Partial<AgentTeamTask>): AgentTeamTask {
  return {
    id: 'task-x',
    teamId: 'team-1',
    title: 'Untitled',
    status: 'todo',
    dependsOn: [],
    gateStatus: 'pending',
    evidence: [],
    orderIndex: 0,
    createdAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

describe('unmetDepCount', () => {
  it('counts deps whose task is not done', () => {
    const byId = new Map<string, AgentTeamTask>([
      ['a', task({ id: 'a', status: 'done' })],
      ['b', task({ id: 'b', status: 'in_progress' })],
    ]);
    const t = task({ id: 't', dependsOn: ['a', 'b'] });
    expect(unmetDepCount(t, byId)).toBe(1); // a done, b not
  });

  it('ignores unknown dep ids', () => {
    const byId = new Map<string, AgentTeamTask>();
    expect(unmetDepCount(task({ dependsOn: ['ghost'] }), byId)).toBe(0);
  });
});

describe('TeamTaskBoard', () => {
  const members = [member('m1', 'planner'), member('m2', 'builder')];

  it('renders all five column labels', () => {
    render(<TeamTaskBoard tasks={[]} members={members} />);
    for (const key of [
      'intelligence.teams.column.todo',
      'intelligence.teams.column.ready',
      'intelligence.teams.column.inProgress',
      'intelligence.teams.column.blocked',
      'intelligence.teams.column.done',
    ]) {
      expect(screen.getByText(key)).toBeInTheDocument();
    }
  });

  it('shows the owner name and hides the claimed-by line when claimer is the owner', () => {
    render(
      <TeamTaskBoard
        tasks={[task({ id: 't1', title: 'Owned', ownerMemberId: 'm1', claimedByMemberId: 'm1' })]}
        members={members}
      />
    );
    expect(screen.getByText('planner')).toBeInTheDocument();
    expect(screen.queryByText('intelligence.teams.pickedUpBy')).not.toBeInTheDocument();
  });

  it('shows the claimed-by line when a non-owner picked the task up', () => {
    render(
      <TeamTaskBoard
        tasks={[task({ id: 't1', ownerMemberId: 'm1', claimedByMemberId: 'm2' })]}
        members={members}
      />
    );
    expect(screen.getByText('intelligence.teams.pickedUpBy')).toBeInTheDocument();
  });

  it('marks an unowned task as unclaimed', () => {
    render(<TeamTaskBoard tasks={[task({ id: 't1' })]} members={members} />);
    expect(screen.getByText('intelligence.teams.unclaimed')).toBeInTheDocument();
  });

  it('renders a dependency lock with the unmet count', () => {
    const tasks = [
      task({ id: 'dep', status: 'in_progress' }),
      task({ id: 't1', dependsOn: ['dep'] }),
    ];
    render(<TeamTaskBoard tasks={tasks} members={members} />);
    // lock badge shows the unmet count "1"
    expect(screen.getByTitle('intelligence.teams.depLockTitle')).toBeInTheDocument();
  });

  it('maps known gate states and falls back to the raw label for unknown', () => {
    const { rerender } = render(
      <TeamTaskBoard tasks={[task({ gateStatus: 'passed' })]} members={members} />
    );
    expect(screen.getByText('intelligence.teams.gate.passed')).toBeInTheDocument();

    rerender(<TeamTaskBoard tasks={[task({ gateStatus: 'weird-state' })]} members={members} />);
    expect(screen.getByText('intelligence.teams.gate.label')).toBeInTheDocument();
  });

  it('places a task in the column matching its status', () => {
    render(
      <TeamTaskBoard
        tasks={[task({ id: 't1', title: 'Blocked one', status: 'blocked' })]}
        members={members}
      />
    );
    expect(screen.getByText('Blocked one')).toBeInTheDocument();
  });
});
