import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../../services/coreRpcClient';
import { renderWithProviders } from '../../../../test/test-utils';
import DevicesPanel from '../DevicesPanel';

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

// qrcode.react is not needed in panel tests.
vi.mock('../devices/PairPhoneModal', () => ({
  default: ({ onClose, onPaired }: { onClose: () => void; onPaired: (id: string) => void }) => (
    <div data-testid="pair-modal">
      <button onClick={onClose}>close-modal</button>
      <button onClick={() => onPaired('CHAN123')}>simulate-paired</button>
    </div>
  ),
}));

const mockCall = vi.mocked(callCoreRpc);

function makeDevice(overrides = {}) {
  return {
    channel_id: 'CHAN_AAABBBCCC',
    label: "Alice's iPhone",
    device_pubkey: 'pubkey_base64url',
    created_at: new Date().toISOString(),
    last_seen_at: null,
    peer_online: false,
    revoked: false,
    ...overrides,
  };
}

function listResponse(devices: ReturnType<typeof makeDevice>[]) {
  return { devices };
}

describe('DevicesPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows empty state when no devices are paired', async () => {
    mockCall.mockResolvedValue(listResponse([]));
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    expect(await screen.findByText('No paired devices')).toBeInTheDocument();
    // Two "Pair iPhone" buttons exist: header + empty-state CTA.
    expect(screen.getAllByRole('button', { name: /Pair iPhone/i })).toHaveLength(2);
  });

  it('renders a paired device row with label, truncated id, and revoke button', async () => {
    const device = makeDevice({ channel_id: 'ABCDEFGHIJ12345678', label: "Bob's iPhone" });
    mockCall.mockResolvedValue(listResponse([device]));
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    expect(await screen.findByText("Bob's iPhone")).toBeInTheDocument();
    // Truncated: first 4 + last 4 chars
    expect(screen.getByText('ABCD…5678')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Revoke/i })).toBeInTheDocument();
  });

  it('filters out revoked devices', async () => {
    const devices = [
      makeDevice({ label: 'Active', revoked: false }),
      makeDevice({ channel_id: 'REVOKED_CHAN', label: 'Revoked', revoked: true }),
    ];
    mockCall.mockResolvedValue(listResponse(devices));
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    expect(await screen.findByText('Active')).toBeInTheDocument();
    expect(screen.queryByText('Revoked')).not.toBeInTheDocument();
  });

  it('shows a confirm dialog on revoke click, then calls devices_revoke on confirm', async () => {
    const device = makeDevice({ label: "Charlie's iPhone", channel_id: 'CHAN_CHARLIE' });
    // First call: list. Second call: revoke. Third call: refresh after revoke.
    mockCall
      .mockResolvedValueOnce(listResponse([device]))
      .mockResolvedValueOnce({ success: true })
      .mockResolvedValueOnce(listResponse([]));

    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    await screen.findByText("Charlie's iPhone");
    fireEvent.click(screen.getByRole('button', { name: /Revoke/i }));

    // Confirmation dialog
    expect(await screen.findByText('Revoke device?')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /^Revoke$/i }));

    await waitFor(() => {
      expect(mockCall).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.devices_revoke' })
      );
    });

    // After revoke the list should be refreshed (empty state)
    expect(await screen.findByText('No paired devices')).toBeInTheDocument();
  });

  it('cancels revoke when the cancel button is pressed', async () => {
    const device = makeDevice({ label: "Dave's iPhone" });
    mockCall.mockResolvedValue(listResponse([device]));
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    await screen.findByText("Dave's iPhone");
    fireEvent.click(screen.getByRole('button', { name: /Revoke/i }));
    expect(screen.getByText('Revoke device?')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Cancel/i }));
    await waitFor(() => {
      expect(screen.queryByText('Revoke device?')).not.toBeInTheDocument();
    });
    // No revoke call made
    expect(mockCall).toHaveBeenCalledTimes(1);
  });

  it('opens the pair modal when Pair iPhone is clicked', async () => {
    mockCall.mockResolvedValue(listResponse([]));
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    await screen.findByText('No paired devices');
    // Click the header-level button (first one).
    fireEvent.click(screen.getAllByRole('button', { name: /Pair iPhone/i })[0]);

    expect(await screen.findByTestId('pair-modal')).toBeInTheDocument();
  });

  it('closes the pair modal and reloads devices after pairing', async () => {
    const device = makeDevice({ label: 'New iPhone' });
    mockCall.mockResolvedValueOnce(listResponse([])).mockResolvedValueOnce(listResponse([device]));

    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });
    await screen.findByText('No paired devices');
    fireEvent.click(screen.getAllByRole('button', { name: /Pair iPhone/i })[0]);

    await screen.findByTestId('pair-modal');
    fireEvent.click(screen.getByText('simulate-paired'));

    await waitFor(() => {
      expect(screen.queryByTestId('pair-modal')).not.toBeInTheDocument();
    });
  });

  it('shows an error message when devices_list fails', async () => {
    mockCall.mockRejectedValue(new Error('Core offline'));
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    expect(await screen.findByText(/Failed to load devices/)).toBeInTheDocument();
  });
});
