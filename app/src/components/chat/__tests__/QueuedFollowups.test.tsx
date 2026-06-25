import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { QueuedFollowup } from '../../../store/chatRuntimeSlice';
import QueuedFollowups from '../QueuedFollowups';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const fup = (id: string, label: string, content = label): QueuedFollowup => ({
  message: {
    id,
    content,
    type: 'text',
    extraMetadata: {},
    sender: 'user',
    createdAt: '2026-01-01T00:00:00.000Z',
  },
  label,
});

describe('QueuedFollowups', () => {
  it('renders nothing when there are no queued items', () => {
    const { container } = render(<QueuedFollowups items={[]} onClear={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('lists queued follow-up labels with a count', () => {
    render(
      <QueuedFollowups
        items={[fup('a', 'ask about pricing'), fup('b', 'and the timeline')]}
        onClear={vi.fn()}
      />
    );

    expect(screen.getByText('ask about pricing')).toBeInTheDocument();
    expect(screen.getByText('and the timeline')).toBeInTheDocument();
    // Label key + count are rendered together ("chat.queuedFollowups.label · 2").
    expect(screen.getByText(/chat\.queuedFollowups\.label · 2/)).toBeInTheDocument();
  });

  it('falls back to the attachment-name label for an attachments-only follow-up', () => {
    render(<QueuedFollowups items={[fup('a', 'photo.png', '')]} onClear={vi.fn()} />);
    // content is empty (attachments only) but the label keeps the row non-blank.
    expect(screen.getByText('photo.png')).toBeInTheDocument();
  });

  it('invokes onClear when the clear control is pressed', () => {
    const onClear = vi.fn();
    render(<QueuedFollowups items={[fup('a', 'one')]} onClear={onClear} />);

    fireEvent.click(screen.getByText('chat.queuedFollowups.clear'));
    expect(onClear).toHaveBeenCalledTimes(1);
  });
});
