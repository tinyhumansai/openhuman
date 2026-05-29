/**
 * PairScreen tests — happy path + error states.
 *
 * Mocks:
 *  - @tauri-apps/plugin-barcode-scanner: controlled scan() return
 *  - services/transport/TransportManager: controlled isHealthy()
 *  - services/transport/profileStore: spy on saveProfile
 *  - lib/platform: forced iOS
 *  - react-router-dom: mock useNavigate
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { clearTestPlatform, setTestPlatform } from '../../lib/platform';
import { PairScreen } from './PairScreen';

// -- module mocks ------------------------------------------------------------

const mockScan = vi.fn();
vi.mock('@tauri-apps/plugin-barcode-scanner', () => ({
  // Include Format enum so PairScreen can import and use Format.QRCode.
  Format: {
    QRCode: 'QR_CODE',
    UPC_A: 'UPC_A',
    EAN8: 'EAN_8',
    EAN13: 'EAN_13',
    Code39: 'CODE_39',
    Code93: 'CODE_93',
    Code128: 'CODE_128',
  },
  scan: (args: unknown) => mockScan(args),
}));

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mockNavigate };
});

const mockSaveProfile = vi.fn();
vi.mock('../../services/transport/profileStore', () => ({
  saveProfile: (profile: unknown) => mockSaveProfile(profile),
  getProfile: vi.fn(),
  listProfileIds: vi.fn(() => []),
  listProfiles: vi.fn(() => []),
  deleteProfile: vi.fn(),
}));

const mockGetTransport = vi.fn();
const mockIsHealthy = vi.fn();
vi.mock('../../services/transport/TransportManager', () => ({
  createTransportManager: vi.fn(() => ({
    getTransport: mockGetTransport,
    close: vi.fn().mockResolvedValue(undefined),
    reset: vi.fn().mockResolvedValue(undefined),
  })),
}));

// -- helpers -----------------------------------------------------------------

function buildPairUrl(
  overrides: Partial<{ cid: string; pt: string; cpk: string; rpc: string; exp: number }> = {}
): string {
  const futureSecs = Math.floor(Date.now() / 1000) + 300; // 5 min from now
  const params = new URLSearchParams({
    cid: overrides.cid ?? 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567',
    pt: overrides.pt ?? 'dGhpcyBpcyBhIHRva2Vu',
    cpk: overrides.cpk ?? 'MCowBQYDK2VuAyEAtestpubkey',
    exp: String(overrides.exp ?? futureSecs),
  });
  if (overrides.rpc) params.set('rpc', overrides.rpc);
  return `openhuman://pair?${params.toString()}`;
}

function renderPairScreen() {
  return render(
    <MemoryRouter initialEntries={['/pair']}>
      <PairScreen />
    </MemoryRouter>
  );
}

// -- setup / teardown --------------------------------------------------------

beforeEach(() => {
  setTestPlatform('ios');
  mockScan.mockReset();
  mockNavigate.mockReset();
  mockSaveProfile.mockReset();
  mockGetTransport.mockReset();
  mockIsHealthy.mockReset();
});

afterEach(() => {
  clearTestPlatform();
  vi.clearAllMocks();
});

// -- tests -------------------------------------------------------------------

describe('PairScreen', () => {
  it('renders welcome copy and scan button', () => {
    renderPairScreen();
    expect(screen.getByText(/pair with your desktop/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /scan qr code/i })).toBeInTheDocument();
  });

  it('happy path: valid QR -> saves profile -> navigates to /human', async () => {
    const pairUrl = buildPairUrl();
    mockScan.mockResolvedValueOnce({ content: pairUrl });
    mockIsHealthy.mockResolvedValue(true);
    mockGetTransport.mockResolvedValue({
      kind: 'tunnel',
      isHealthy: mockIsHealthy,
      close: vi.fn().mockResolvedValue(undefined),
    });

    renderPairScreen();
    await userEvent.click(screen.getByRole('button', { name: /scan qr code/i }));

    await waitFor(() => {
      expect(mockSaveProfile).toHaveBeenCalledOnce();
    });

    const savedProfile = mockSaveProfile.mock.calls[0][0];
    expect(savedProfile.kind).toBe('tunnel');
    expect(savedProfile.channelId).toBe('ABCDEFGHIJKLMNOPQRSTUVWXYZ234567');
    expect(savedProfile.pairingToken).toBeTruthy();
    // Sensitive fields: just check they exist, not the value.
    expect(typeof savedProfile.devicePrivkey).toBe('string');
    expect(savedProfile.devicePrivkey.length).toBeGreaterThan(0);

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith('/human', { replace: true });
    });
  });

  it('expired QR -> shows expired message, no navigation', async () => {
    const expiredUrl = buildPairUrl({ exp: Math.floor(Date.now() / 1000) - 10 });
    mockScan.mockResolvedValueOnce({ content: expiredUrl });

    renderPairScreen();
    await userEvent.click(screen.getByRole('button', { name: /scan qr code/i }));

    await waitFor(() => {
      expect(screen.getByText(/qr code expired/i)).toBeInTheDocument();
    });
    expect(mockNavigate).not.toHaveBeenCalled();
    expect(mockSaveProfile).not.toHaveBeenCalled();
  });

  it('invalid QR URL -> shows error message', async () => {
    mockScan.mockResolvedValueOnce({ content: 'https://example.com/not-a-pair-url' });

    renderPairScreen();
    await userEvent.click(screen.getByRole('button', { name: /scan qr code/i }));

    await waitFor(() => {
      expect(screen.getByText(/invalid qr code/i)).toBeInTheDocument();
    });
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('QR missing required fields -> shows error', async () => {
    // Missing pt (pairingToken)
    const badUrl = 'openhuman://pair?cid=ABCDEF&cpk=testkey&exp=9999999999';
    mockScan.mockResolvedValueOnce({ content: badUrl });

    renderPairScreen();
    await userEvent.click(screen.getByRole('button', { name: /scan qr code/i }));

    await waitFor(() => {
      expect(screen.getByText(/invalid qr code/i)).toBeInTheDocument();
    });
  });

  it('transport unhealthy -> shows connection error', async () => {
    const pairUrl = buildPairUrl();
    mockScan.mockResolvedValueOnce({ content: pairUrl });
    mockIsHealthy.mockResolvedValue(false);
    mockGetTransport.mockResolvedValue({
      kind: 'tunnel',
      isHealthy: mockIsHealthy,
      close: vi.fn().mockResolvedValue(undefined),
    });

    renderPairScreen();
    await userEvent.click(screen.getByRole('button', { name: /scan qr code/i }));

    await waitFor(() => {
      expect(screen.getByText(/could not reach the desktop/i)).toBeInTheDocument();
    });
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('scan rejection -> shows camera error', async () => {
    mockScan.mockRejectedValueOnce(new Error('Camera denied'));

    renderPairScreen();
    await userEvent.click(screen.getByRole('button', { name: /scan qr code/i }));

    await waitFor(() => {
      expect(screen.getByText(/camera scan failed/i)).toBeInTheDocument();
    });
  });

  it('retry button resets to idle and allows another scan', async () => {
    mockScan.mockRejectedValueOnce(new Error('Camera denied'));

    renderPairScreen();
    await userEvent.click(screen.getByRole('button', { name: /scan qr code/i }));

    await waitFor(() => {
      expect(screen.getByText(/camera scan failed/i)).toBeInTheDocument();
    });

    // Click retry scan
    const retryBtn = screen.getByRole('button', { name: /retry scan/i });
    mockScan.mockRejectedValueOnce(new Error('Camera denied again'));
    await userEvent.click(retryBtn);

    // Error should reappear after second failure
    await waitFor(() => {
      expect(screen.getByText(/camera scan failed/i)).toBeInTheDocument();
    });
  });
});
