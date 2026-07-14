import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../lib/orchestration/orchestrationClient';
import InstanceCard from './InstanceCard';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function session(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: 'w1',
    agentId: '6wNaBJkatir4B86cw5ykHZWQ3xoNaKygX5vAU9MQbHSh',
    source: 'claude',
    harnessType: 'claude',
    status: 'idle',
    currentTask: 'drafting hub cards',
    chatKind: 'session',
    lastMessageAt: '2026-07-06T00:00:00Z',
    unread: 0,
    active: true,
    pinned: false,
    ...over,
  };
}

describe('InstanceCard', () => {
  it('renders the harness glyph, status dot, task and shortened address', () => {
    render(<InstanceCard session={session()} />);
    expect(screen.getByTestId('harness-glyph')).toHaveAttribute('data-harness', 'claude');
    expect(screen.getByTestId('instance-status-dot')).toHaveAttribute('data-status', 'idle');
    expect(screen.getByText('drafting hub cards')).toBeInTheDocument();
    expect(screen.getByText('6wNaBJ…QbHSh')).toBeInTheDocument();
  });

  it('prefers a resolved @handle over the address', () => {
    render(<InstanceCard session={session()} handle="claudebot" />);
    expect(screen.getByText('@claudebot')).toBeInTheDocument();
  });

  it('shows the unread pill only when unread > 0 and fires onSelect', () => {
    const onSelect = vi.fn();
    const { rerender } = render(<InstanceCard session={session()} onSelect={onSelect} />);
    expect(screen.queryByTestId('instance-card-unread')).toBeNull();

    rerender(<InstanceCard session={session({ unread: 3 })} onSelect={onSelect} />);
    expect(screen.getByTestId('instance-card-unread')).toHaveTextContent('3');

    fireEvent.click(screen.getByTestId('instance-card-w1'));
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it('falls back to the OpenHuman glyph when no harness is set', () => {
    render(<InstanceCard session={session({ harnessType: undefined, source: 'user_created' })} />);
    expect(screen.getByTestId('harness-glyph')).toHaveAttribute('data-harness', 'openhuman');
  });
});
