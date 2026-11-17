import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { toast, Toaster } from './Toast';

const raise = (options: Parameters<typeof toast.add>[0]) => {
  let id = '';
  act(() => {
    id = toast.add(options);
  });
  return id;
};

describe('Toast', () => {
  afterEach(() => {
    act(() => toast.close());
  });

  it('renders a typed toast with its title, description and type icon', async () => {
    renderWithProviders(<Toaster />);
    raise({ type: 'success', title: 'Voice agent switched', description: 'Gemini answers now.' });

    const item = await screen.findByTestId('toast');
    expect(item).toHaveAttribute('data-type', 'success');
    expect(within(item).getByText('Voice agent switched')).toBeInTheDocument();
    expect(within(item).getByText('Gemini answers now.')).toBeInTheDocument();
    expect(item.querySelector('[data-slot="toast-icon"] svg')).not.toBeNull();
  });

  it('uses a custom icon from data in place of the type icon', async () => {
    renderWithProviders(<Toaster />);
    raise({ type: 'success', title: 'Key added', data: { icon: <span data-testid="logo" /> } });

    const item = await screen.findByTestId('toast');
    expect(within(item).getByTestId('logo')).toBeInTheDocument();
  });

  it('renders no icon tile for an untyped toast without one', async () => {
    renderWithProviders(<Toaster />);
    raise({ title: 'Plain' });

    const item = await screen.findByTestId('toast');
    expect(item.querySelector('[data-slot="toast-icon"]')).toBeNull();
  });

  it('runs an action and closes from the close button', async () => {
    const onClick = vi.fn();
    renderWithProviders(<Toaster />);
    raise({ type: 'error', title: 'Save failed', actionProps: { children: 'Retry', onClick } });

    const item = await screen.findByTestId('toast');
    fireEvent.click(within(item).getByRole('button', { name: 'Retry' }));
    expect(onClick).toHaveBeenCalledTimes(1);

    fireEvent.click(within(item).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByTestId('toast')).not.toBeInTheDocument());
  });
});
