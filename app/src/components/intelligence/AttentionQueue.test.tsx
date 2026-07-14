import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AttentionQueue } from '../../lib/orchestration/orchestrationClient';
import AttentionQueueView from './AttentionQueue';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const populated: AttentionQueue = {
  items: [
    {
      id: 'approval:req-1',
      kind: 'approval',
      instanceId: 'req-1',
      title: 'shell',
      summary: 'run a command',
      action: { type: 'approval', requestId: 'req-1' },
    },
    {
      id: 'unread:h-1',
      kind: 'unread',
      instanceId: 'h-1',
      title: 'Codex',
      count: 2,
      action: { type: 'open-session', sessionId: 'h-1' },
    },
  ],
  counts: { total: 2, approvals: 1, needsInput: 0, unread: 1 },
};

const empty: AttentionQueue = {
  items: [],
  counts: { total: 0, approvals: 0, needsInput: 0, unread: 0 },
};

describe('AttentionQueue', () => {
  it('renders each item and a total badge when populated', () => {
    render(<AttentionQueueView queue={populated} />);
    expect(screen.getByTestId('attention-queue-count')).toHaveTextContent('2');
    expect(screen.getByTestId('attention-item-approval:req-1')).toBeInTheDocument();
    expect(screen.getByTestId('attention-item-unread:h-1')).toBeInTheDocument();
    expect(screen.queryByTestId('attention-queue-empty')).toBeNull();
  });

  it('renders the empty state and no badge when the queue is clear', () => {
    render(<AttentionQueueView queue={empty} />);
    expect(screen.getByTestId('attention-queue-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('attention-queue-count')).toBeNull();
  });

  it('renders nothing while the first load is pending', () => {
    const { container } = render(<AttentionQueueView queue={null} loading />);
    expect(container).toBeEmptyDOMElement();
  });
});
