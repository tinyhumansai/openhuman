import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { ToastNotification } from '../../types/intelligence';
import { ToastContainer } from './Toast';

const notification: ToastNotification = {
  id: 'n1',
  type: 'success',
  title: 'Saved',
  message: 'Your changes were saved.',
};

describe('<ToastContainer />', () => {
  it('renders nothing when there are no notifications', () => {
    const { container } = render(<ToastContainer notifications={[]} onRemove={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders the notification title, message, and a dismiss button', () => {
    render(<ToastContainer notifications={[notification]} onRemove={vi.fn()} />);
    expect(screen.getByText('Saved')).toBeInTheDocument();
    expect(screen.getByText('Your changes were saved.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Dismiss notification' })).toBeInTheDocument();
  });

  it('renders the action button and invokes its handler on click', async () => {
    const user = userEvent.setup();
    const handler = vi.fn();
    render(
      <ToastContainer
        notifications={[{ ...notification, action: { label: 'Undo', handler } }]}
        onRemove={vi.fn()}
      />
    );
    await user.click(screen.getByRole('button', { name: 'Undo' }));
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('calls onRemove after the dismiss button is clicked', async () => {
    vi.useFakeTimers();
    const onRemove = vi.fn();
    render(<ToastContainer notifications={[notification]} onRemove={onRemove} />);
    screen.getByRole('button', { name: 'Dismiss notification' }).click();
    vi.advanceTimersByTime(250);
    expect(onRemove).toHaveBeenCalledWith('n1');
    vi.useRealTimers();
  });
});
