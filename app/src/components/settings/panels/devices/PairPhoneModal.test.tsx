/**
 * Tests for PairPhoneModal.
 *
 * Timer strategy: most tests use real timers + mocked callCoreRpc.
 * Tests that validate timer-driven state (expiry, poll, auto-close) use
 * vi.useFakeTimers scoped per-test and flush promises with act()+Promise.resolve().
 */
import { act, fireEvent, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../../services/coreRpcClient';
import { renderWithProviders } from '../../../../test/test-utils';
import PairPhoneModal from './PairPhoneModal';

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('qrcode.react', () => ({
  QRCodeSVG: ({ value }: { value: string }) => <div data-testid="qr-code" data-value={value} />,
}));

const mockCall = vi.mocked(callCoreRpc);

const CHANNEL_ID = 'ABCDEFGHIJ1234567890AB';
const PAIRING_TOKEN = 'tok_abc123';
const CORE_PUBKEY = 'pubkey_base64url_value';

function makePairingSession(overrides = {}) {
  return {
    channel_id: CHANNEL_ID,
    pairing_token: PAIRING_TOKEN,
    core_pubkey: CORE_PUBKEY,
    rpc_url: null,
    expires_at: new Date(Date.now() + 600_000).toISOString(),
    ...overrides,
  };
}

function makeDevice(overrides = {}) {
  return {
    channel_id: CHANNEL_ID,
    label: "Alice's iPhone",
    peer_online: true,
    revoked: false,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function setupRealTimers() {
  vi.useRealTimers();
}

function setupFakeTimers() {
  vi.useFakeTimers({ shouldAdvanceTime: false });
}

/** Advance fake timers + flush promise microtasks. */
async function advanceAndFlush(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const onClose = vi.fn();
const onPaired = vi.fn();

describe('PairPhoneModal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setupRealTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // ---------------------------------------------------------------------------
  // QR render + URL validation (no timer tricks needed)
  // ---------------------------------------------------------------------------

  it('shows loading then renders a QR code after create_pairing resolves', async () => {
    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') return makePairingSession();
      return { devices: [] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    expect(screen.getByText(/Generating pairing code/i)).toBeInTheDocument();
    expect(await screen.findByTestId('qr-code')).toBeInTheDocument();
  });

  it('QR code value contains all required URL params', async () => {
    const session = makePairingSession({ rpc_url: 'http://192.168.1.5:7788/rpc' });
    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') return session;
      return { devices: [] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    const qr = await screen.findByTestId('qr-code');
    const value = qr.getAttribute('data-value') ?? '';
    const url = new URL(value);
    expect(url.protocol).toBe('openhuman:');
    expect(url.searchParams.get('cid')).toBe(CHANNEL_ID);
    expect(url.searchParams.get('pt')).toBe(PAIRING_TOKEN);
    expect(url.searchParams.get('cpk')).toBe(CORE_PUBKEY);
    expect(url.searchParams.get('rpc')).toBe('http://192.168.1.5:7788/rpc');
    expect(url.searchParams.get('exp')).toBeTruthy();
  });

  // ---------------------------------------------------------------------------
  // Poll-based pairing detection
  // ---------------------------------------------------------------------------

  it('transitions to success state when device appears on poll', async () => {
    setupFakeTimers();

    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') return makePairingSession();
      return { devices: [makeDevice()] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    // Flush the create_pairing promise so the QR renders.
    await advanceAndFlush(0);
    // Advance past the 2s poll interval and flush the list call.
    await advanceAndFlush(2_100);

    expect(screen.getByText(/Paired with iPhone/i)).toBeInTheDocument();
    expect(screen.getByText("Alice's iPhone")).toBeInTheDocument();
  });

  it('calls onPaired after 3 s auto-close on success', async () => {
    setupFakeTimers();

    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') return makePairingSession();
      return { devices: [makeDevice()] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    // create_pairing + 2s poll.
    await advanceAndFlush(0);
    await advanceAndFlush(2_100);
    expect(screen.getByText(/Paired with iPhone/i)).toBeInTheDocument();

    // 3 s auto-close timer.
    await advanceAndFlush(3_100);

    expect(onPaired).toHaveBeenCalledWith(CHANNEL_ID);
  });

  // ---------------------------------------------------------------------------
  // Expiry
  // ---------------------------------------------------------------------------

  it('shows QR expired when the session deadline passes', async () => {
    setupFakeTimers();

    const session = makePairingSession({ expires_at: new Date(Date.now() + 50).toISOString() });
    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') return session;
      return { devices: [] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    await advanceAndFlush(0);
    expect(screen.getByTestId('qr-code')).toBeInTheDocument();

    // Advance past the 50 ms expiry.
    await advanceAndFlush(200);

    expect(screen.getByText(/QR code expired/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Generate new code/i })).toBeInTheDocument();
  });

  it('re-issues create_pairing when "Generate new code" is clicked', async () => {
    setupFakeTimers();

    const expiredSession = makePairingSession({
      expires_at: new Date(Date.now() + 50).toISOString(),
    });
    const freshSession = makePairingSession({ channel_id: 'NEW_CHANNEL_XYZ' });

    let createCount = 0;
    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') {
        return createCount++ === 0 ? expiredSession : freshSession;
      }
      return { devices: [] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    await advanceAndFlush(0);
    expect(screen.getByTestId('qr-code')).toBeInTheDocument();

    await advanceAndFlush(200);
    expect(screen.getByText(/QR code expired/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Generate new code/i }));
    // Loading + fresh QR
    await advanceAndFlush(0);
    expect(screen.getByTestId('qr-code')).toBeInTheDocument();
    expect(createCount).toBe(2);
  });

  // ---------------------------------------------------------------------------
  // Error state
  // ---------------------------------------------------------------------------

  it('shows error state when devices_create_pairing fails', async () => {
    mockCall.mockRejectedValue(new Error('tunnel unavailable'));

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    expect(await screen.findByText(/Something went wrong/i)).toBeInTheDocument();
    expect(screen.getByText(/tunnel unavailable/i)).toBeInTheDocument();
  });

  // ---------------------------------------------------------------------------
  // Close + details toggle (no timer tricks needed)
  // ---------------------------------------------------------------------------

  it('calls onClose when the X button is pressed', async () => {
    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') return makePairingSession();
      return { devices: [] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    await screen.findByTestId('qr-code');
    fireEvent.click(screen.getByRole('button', { name: /Close/i }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onPaired).not.toHaveBeenCalled();
  });

  it('toggles details section when "Show details" / "Hide details" is clicked', async () => {
    mockCall.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.devices_create_pairing') return makePairingSession();
      return { devices: [] };
    });

    renderWithProviders(<PairPhoneModal onClose={onClose} onPaired={onPaired} />, {
      initialEntries: ['/settings/devices'],
    });

    await screen.findByTestId('qr-code');
    expect(screen.queryByText('Channel ID')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Show details/i }));
    expect(screen.getByText('Channel ID')).toBeInTheDocument();
    expect(screen.getByText(CHANNEL_ID)).toBeInTheDocument();

    // Toggle state is synchronous.
    fireEvent.click(screen.getByRole('button', { name: /Hide details/i }));
    expect(screen.queryByText('Channel ID')).not.toBeInTheDocument();
  });
});
