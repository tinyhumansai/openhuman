import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import { ShareMessageButton } from './ShareMessageButton';

vi.mock('./ShareCardModal', () => ({
  ShareCardModal: ({ onClose }: { onClose: () => void }) => (
    <div data-testid="share-modal">
      <button onClick={onClose}>close</button>
    </div>
  ),
}));

describe('ShareMessageButton', () => {
  test('renders nothing when there is no shareable content', () => {
    const { container } = render(<ShareMessageButton content="   " agentName="Tiny" />);
    expect(container.firstChild).toBeNull();
  });

  test('renders a share trigger for agent content', () => {
    render(<ShareMessageButton content="Did the thing" agentName="Tiny" />);
    expect(screen.getByTestId('chat-message-share')).toBeInTheDocument();
    expect(screen.queryByTestId('share-modal')).not.toBeInTheDocument();
  });

  test('opens and closes the share modal', async () => {
    const user = userEvent.setup();
    render(<ShareMessageButton content="Did the thing" agentName="Tiny" threadId="t1" />);

    await user.click(screen.getByTestId('chat-message-share'));
    expect(screen.getByTestId('share-modal')).toBeInTheDocument();

    await user.click(screen.getByText('close'));
    expect(screen.queryByTestId('share-modal')).not.toBeInTheDocument();
  });
});
