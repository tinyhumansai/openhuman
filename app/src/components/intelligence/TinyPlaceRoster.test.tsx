import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../lib/orchestration/orchestrationClient';
import TinyPlaceRoster from './TinyPlaceRoster';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function session(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: 's',
    agentId: '@peer',
    source: 'claude',
    harnessType: 'claude',
    status: 'idle',
    chatKind: 'session',
    lastMessageAt: '2026-07-06T00:00:00Z',
    unread: 0,
    active: true,
    pinned: false,
    ...over,
  };
}

describe('TinyPlaceRoster', () => {
  it('shows the empty state when there are no instance sessions', () => {
    // Pinned windows are not instances and must not count.
    const pinned = session({ sessionId: 'master', chatKind: 'master', pinned: true });
    render(<TinyPlaceRoster sessions={[pinned]} />);
    expect(screen.getByTestId('tinyplace-roster-empty')).toBeInTheDocument();
  });

  it('groups instances by harness and lists an Other group for harness-less sessions', () => {
    const sessions = [
      session({ sessionId: 'c1', harnessType: 'claude' }),
      session({ sessionId: 'x1', harnessType: 'codex', source: 'codex' }),
      session({ sessionId: 'u1', harnessType: undefined, source: 'user_created' }),
    ];
    render(<TinyPlaceRoster sessions={sessions} />);
    expect(screen.getByText('Claude')).toBeInTheDocument();
    expect(screen.getByText('Codex')).toBeInTheDocument();
    // Harness-less session lands under the (translated) Other group; no empty Gemini group.
    expect(screen.getByText('tinyplaceOrchestration.roster.other')).toBeInTheDocument();
    expect(screen.queryByText('Gemini')).toBeNull();
    expect(screen.getByTestId('instance-card-c1')).toBeInTheDocument();
    expect(screen.getByTestId('instance-card-u1')).toBeInTheDocument();
  });

  it('marks the selected instance and forwards selection', () => {
    const onSelect = vi.fn();
    const sessions = [session({ sessionId: 'c1' }), session({ sessionId: 'c2' })];
    render(<TinyPlaceRoster sessions={sessions} selectedId="c1" onSelect={onSelect} />);
    expect(screen.getByTestId('instance-card-c1')).toHaveAttribute('data-selected', 'true');
    expect(screen.getByTestId('instance-card-c2')).toHaveAttribute('data-selected', 'false');
    fireEvent.click(screen.getByTestId('instance-card-c2'));
    expect(onSelect).toHaveBeenCalledWith('c2');
  });

  it('passes resolved handles down to the cards', () => {
    render(
      <TinyPlaceRoster
        sessions={[session({ sessionId: 'c1', agentId: '@peer' })]}
        handles={{ '@peer': 'claudebot' }}
      />
    );
    const card = screen.getByTestId('instance-card-c1');
    expect(within(card).getByText('@claudebot')).toBeInTheDocument();
  });
});
