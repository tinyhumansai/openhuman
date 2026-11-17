import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import type { ToastNotification } from '../../types/intelligence';
import { toast, Toaster } from '../ui/Toast';
import { ToastContainer } from './Toast';

const notification: ToastNotification = {
  id: 'n1',
  type: 'success',
  title: 'Saved',
  message: 'Your changes were saved.',
};

describe('<ToastContainer />', () => {
  afterEach(() => act(() => toast.close()));
  it('renders nothing when there are no notifications', () => {
    const { container } = render(<ToastContainer notifications={[]} onRemove={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders the notification through the shared toaster', async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <>
        <Toaster />
        <ToastContainer notifications={[notification]} onRemove={vi.fn()} />
      </>
    );
    expect(await screen.findByText('Saved')).toBeInTheDocument();
    expect(screen.getByText('Your changes were saved.')).toBeInTheDocument();
    await user.hover(await screen.findByTestId('toast'));
    expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument();
  });

  it('renders the action button and invokes its handler on click', async () => {
    const user = userEvent.setup();
    const handler = vi.fn();
    renderWithProviders(
      <>
        <Toaster />
        <ToastContainer
          notifications={[{ ...notification, action: { label: 'Undo', handler } }]}
          onRemove={vi.fn()}
        />
      </>
    );
    await user.click(await screen.findByRole('button', { name: 'Undo' }));
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('removes legacy state after handoff and lets the shared toaster dismiss it', async () => {
    const user = userEvent.setup();
    const onRemove = vi.fn();
    renderWithProviders(
      <>
        <Toaster />
        <ToastContainer notifications={[notification]} onRemove={onRemove} />
      </>
    );
    expect(onRemove).toHaveBeenCalledWith('n1');
    await user.hover(await screen.findByTestId('toast'));
    await user.click(await screen.findByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByTestId('toast')).not.toBeInTheDocument());
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});
