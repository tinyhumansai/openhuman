import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import QueuedFollowups from '../QueuedFollowups';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('QueuedFollowups', () => {
  it('renders nothing when there are no queued items', () => {
    const { container } = render(<QueuedFollowups items={[]} onClear={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('lists queued follow-up texts with a count', () => {
    render(
      <QueuedFollowups
        items={[
          { id: 'a', text: 'ask about pricing' },
          { id: 'b', text: 'and the timeline' },
        ]}
        onClear={vi.fn()}
      />
    );

    expect(screen.getByText('ask about pricing')).toBeInTheDocument();
    expect(screen.getByText('and the timeline')).toBeInTheDocument();
    // Label key + count are rendered together ("chat.queuedFollowups.label · 2").
    expect(screen.getByText(/chat\.queuedFollowups\.label · 2/)).toBeInTheDocument();
  });

  it('invokes onClear when the clear control is pressed', () => {
    const onClear = vi.fn();
    render(<QueuedFollowups items={[{ id: 'a', text: 'one' }]} onClear={onClear} />);

    fireEvent.click(screen.getByText('chat.queuedFollowups.clear'));
    expect(onClear).toHaveBeenCalledTimes(1);
  });
});
