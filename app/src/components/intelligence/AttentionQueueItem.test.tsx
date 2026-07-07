import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AttentionItem } from '../../lib/orchestration/orchestrationClient';
import AttentionQueueItem from './AttentionQueueItem';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const approval: AttentionItem = {
  id: 'approval:req-1',
  kind: 'approval',
  instanceId: 'req-1',
  title: 'shell',
  summary: 'run `ls -la`',
  action: { type: 'approval', requestId: 'req-1' },
  createdAt: '2026-07-06T10:00:00Z',
};

// A REMOTE agent session parked on an approval: same `approval` kind as a local
// gate approval, but its action opens the orchestration chat window instead of
// the local approval surface.
const remoteApproval: AttentionItem = {
  id: 'remote-approval:h-remote',
  kind: 'approval',
  instanceId: 'h-remote',
  title: 'Claude · deploy',
  summary: 'approve `rm -rf build/`',
  action: { type: 'open-session', sessionId: 'h-remote' },
  createdAt: '2026-07-06T12:00:00Z',
};

const needsInput: AttentionItem = {
  id: 'needs-input:run-1',
  kind: 'needs-input',
  instanceId: 'run-1',
  title: 'researcher',
  summary: 'blocked on a clarifying question',
  action: { type: 'open-thread', threadId: 'thread-9' },
};

const unread: AttentionItem = {
  id: 'unread:h-1',
  kind: 'unread',
  instanceId: 'h-1',
  title: 'Claude · repo audit',
  count: 4,
  action: { type: 'open-session', sessionId: 'h-1' },
};

describe('AttentionQueueItem', () => {
  it('renders an approval with its summary and a Review action', () => {
    const onAction = vi.fn();
    render(<AttentionQueueItem item={approval} onAction={onAction} />);
    const row = screen.getByTestId('attention-item-approval:req-1');
    expect(row).toHaveAttribute('data-kind', 'approval');
    expect(screen.getByText('shell')).toBeInTheDocument();
    expect(screen.getByText('run `ls -la`')).toBeInTheDocument();
    expect(screen.getByText('tinyplaceOrchestration.attention.kind.approval')).toBeInTheDocument();

    const action = screen.getByTestId('attention-item-action');
    expect(action).toHaveTextContent('tinyplaceOrchestration.attention.review');
    fireEvent.click(action);
    expect(onAction).toHaveBeenCalledWith({ type: 'approval', requestId: 'req-1' });
  });

  it('renders a remote approval (approval kind) whose action opens its session', () => {
    const onAction = vi.fn();
    render(<AttentionQueueItem item={remoteApproval} onAction={onAction} />);
    const row = screen.getByTestId('attention-item-remote-approval:h-remote');
    expect(row).toHaveAttribute('data-kind', 'approval');
    // Approval kind → the amber "Review" verb, and the summary is shown.
    expect(screen.getByText('Claude · deploy')).toBeInTheDocument();
    expect(screen.getByText('approve `rm -rf build/`')).toBeInTheDocument();
    const action = screen.getByTestId('attention-item-action');
    expect(action).toHaveTextContent('tinyplaceOrchestration.attention.review');
    // Clicking routes the open-session action so the tab opens the chat window.
    fireEvent.click(action);
    expect(onAction).toHaveBeenCalledWith({ type: 'open-session', sessionId: 'h-remote' });
  });

  it('renders a needs-input row with the Open action carrying its thread', () => {
    const onAction = vi.fn();
    render(<AttentionQueueItem item={needsInput} onAction={onAction} />);
    expect(screen.getByText('blocked on a clarifying question')).toBeInTheDocument();
    const action = screen.getByTestId('attention-item-action');
    expect(action).toHaveTextContent('tinyplaceOrchestration.attention.open');
    fireEvent.click(action);
    expect(onAction).toHaveBeenCalledWith({ type: 'open-thread', threadId: 'thread-9' });
  });

  it('renders unread with a localized label + count pill, not a summary', () => {
    render(<AttentionQueueItem item={unread} />);
    // Unread ships no summary string — the label is localized instead.
    expect(screen.getByText('tinyplaceOrchestration.attention.unread')).toBeInTheDocument();
    expect(screen.getByTestId('attention-item-count')).toHaveTextContent('4');
    expect(screen.getByText('Claude · repo audit')).toBeInTheDocument();
  });

  it('does not throw when onAction is omitted', () => {
    render(<AttentionQueueItem item={unread} />);
    // Clicking with no handler is a no-op, not a crash.
    expect(() => fireEvent.click(screen.getByTestId('attention-item-action'))).not.toThrow();
  });
});
