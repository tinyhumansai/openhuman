import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import TunnelList from './TunnelList';

// Identity translator so we can assert on stable i18n keys.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));
vi.mock('../../hooks/useBackendUrl', () => ({ useBackendUrl: () => 'http://localhost:8787' }));
vi.mock('../../services/api/tunnelsApi', () => ({
  tunnelsApi: { ingressUrl: () => 'http://localhost:8787/ingress/x' },
}));

const baseProps = {
  tunnels: [],
  registrations: [],
  loading: false,
  onCreateTunnel: vi.fn().mockResolvedValue({}),
  onDeleteTunnel: vi.fn().mockResolvedValue(undefined),
  onRefresh: vi.fn().mockResolvedValue(undefined),
  onRegisterEcho: vi.fn().mockResolvedValue(undefined),
  onUnregisterEcho: vi.fn().mockResolvedValue(undefined),
};

describe('TunnelList', () => {
  it('calls onRefresh when the refresh button is clicked', () => {
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    render(<TunnelList {...baseProps} onRefresh={onRefresh} />);
    fireEvent.click(screen.getByRole('button', { name: 'common.refresh' }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it('opens the create form, cancels it, then creates a tunnel', async () => {
    const onCreateTunnel = vi.fn().mockResolvedValue({});
    render(<TunnelList {...baseProps} onCreateTunnel={onCreateTunnel} />);

    // Open the create form.
    fireEvent.click(screen.getByRole('button', { name: 'webhooks.tunnels.newTunnel' }));
    // Cancel hides it again.
    fireEvent.click(screen.getByRole('button', { name: 'common.cancel' }));
    // Reopen and create.
    fireEvent.click(screen.getByRole('button', { name: 'webhooks.tunnels.newTunnel' }));
    const nameInput = screen.getAllByRole('textbox')[0];
    fireEvent.change(nameInput, { target: { value: 'my-tunnel' } });
    fireEvent.click(screen.getByRole('button', { name: 'common.create' }));

    await waitFor(() => expect(onCreateTunnel).toHaveBeenCalledWith('my-tunnel', undefined));
  });
});
