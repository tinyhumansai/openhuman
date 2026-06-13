/**
 * SecurityPanel — coverage for keyring status display and consent actions.
 *
 * Target lines: 11, 27, 103, 118, 123, 128, 133
 *
 * 11  — MODE_BADGE_VARIANT constant drives modeBadgeVariant
 * 27  — modeBadgeVariant calculated from keyringStatus.activeMode
 * 103 — retry probe button click → handleRetryProbe
 * 118 — consent section visible when keyring unavailable
 * 123 — grantConsent button shown when activeMode !== 'local_encrypted'
 * 128 — revokeConsent button shown when activeMode !== 'declined'
 * 133 — consent button calls handleConsentChange
 *
 * NOTE: useT is NOT mocked here — the real I18nContext default (en.ts) is used,
 * so all assertions use the actual English strings from en.ts.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import SecurityPanel from '../SecurityPanel';

// ── Mocks ─────────────────────────────────────────────────────────────────────

const mockRetryKeyringProbe = vi.fn();
const mockDecideKeyringConsent = vi.fn();

vi.mock('../../../../services/keyringApi', () => ({
  retryKeyringProbe: (...args: unknown[]) => mockRetryKeyringProbe(...args),
  decideKeyringConsent: (...args: unknown[]) => mockDecideKeyringConsent(...args),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

const mockUseCoreState = vi.fn();

vi.mock('../../../../providers/CoreStateProvider', () => ({
  useCoreState: () => mockUseCoreState(),
}));

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeKeyringStatus(overrides: Record<string, unknown> = {}) {
  return {
    activeMode: 'os_keyring',
    available: true,
    backendName: 'macOS Keychain',
    failureReason: null,
    ...overrides,
  };
}

function setupState(keyringStatus: ReturnType<typeof makeKeyringStatus>) {
  mockUseCoreState.mockReturnValue({ snapshot: { keyringStatus } });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('SecurityPanel — storage mode badge (lines 11, 27)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRetryKeyringProbe.mockResolvedValue(undefined);
  });

  it('renders os_keyring mode badge — key not in en.ts so t() returns key', () => {
    setupState(makeKeyringStatus({ activeMode: 'os_keyring' }));
    renderWithProviders(<SecurityPanel />);
    // 'keyring.settings.mode.os_keyring' not in en.ts → rendered as key
    expect(screen.getByText('keyring.settings.mode.os_keyring')).toBeInTheDocument();
  });

  it('renders local_encrypted mode badge', () => {
    setupState(makeKeyringStatus({ activeMode: 'local_encrypted' }));
    renderWithProviders(<SecurityPanel />);
    expect(screen.getByText('keyring.settings.mode.local_encrypted')).toBeInTheDocument();
  });

  it('renders declined mode badge (line 27 — MODE_BADGE_VARIANT.declined = danger)', () => {
    setupState(makeKeyringStatus({ activeMode: 'declined' }));
    renderWithProviders(<SecurityPanel />);
    // en.ts has keyring.settings.mode.declined = 'Declined'
    expect(screen.getByText('Declined')).toBeInTheDocument();
  });

  it('renders consent_pending mode badge', () => {
    setupState(makeKeyringStatus({ activeMode: 'consent_pending' }));
    renderWithProviders(<SecurityPanel />);
    expect(screen.getByText('keyring.settings.mode.consent_pending')).toBeInTheDocument();
  });
});

describe('SecurityPanel — availability section', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRetryKeyringProbe.mockResolvedValue(undefined);
  });

  it('shows available indicator when keyring is available (line 90)', () => {
    setupState(makeKeyringStatus({ available: true }));
    renderWithProviders(<SecurityPanel />);
    // en.ts: 'keyring.settings.available': 'OS keychain is available'
    expect(screen.getByText('OS keychain is available')).toBeInTheDocument();
  });

  it('shows unavailable indicator when keyring is not available (line 91)', () => {
    setupState(makeKeyringStatus({ available: false }));
    renderWithProviders(<SecurityPanel />);
    // en.ts: 'keyring.settings.unavailable': 'OS keychain is unavailable'
    expect(screen.getByText('OS keychain is unavailable')).toBeInTheDocument();
  });

  it('shows failure reason when present', () => {
    setupState(makeKeyringStatus({ available: false, failureReason: 'Keychain locked' }));
    renderWithProviders(<SecurityPanel />);
    expect(screen.getByText('Keychain locked')).toBeInTheDocument();
  });
});

describe('SecurityPanel — retry probe (line 103)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRetryKeyringProbe.mockResolvedValue(undefined);
  });

  it('calls retryKeyringProbe when retry button is clicked (line 103)', async () => {
    setupState(makeKeyringStatus());
    renderWithProviders(<SecurityPanel />);

    // en.ts: 'keyring.settings.retryButton': 'Retry keychain detection'
    fireEvent.click(screen.getByText('Retry keychain detection'));

    await waitFor(() => {
      expect(mockRetryKeyringProbe).toHaveBeenCalledTimes(1);
    });
  });

  it('shows error when retryKeyringProbe fails', async () => {
    mockRetryKeyringProbe.mockRejectedValue(new Error('probe failed'));
    setupState(makeKeyringStatus());
    renderWithProviders(<SecurityPanel />);

    fireEvent.click(screen.getByText('Retry keychain detection'));

    await waitFor(() => {
      // en.ts: 'keyring.settings.retryFailed': 'Retry failed. Keychain is still unavailable.'
      expect(screen.getByText('Retry failed. Keychain is still unavailable.')).toBeInTheDocument();
    });
  });
});

describe('SecurityPanel — consent management (lines 118, 123, 128, 133)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockDecideKeyringConsent.mockResolvedValue(undefined);
    mockRetryKeyringProbe.mockResolvedValue(undefined);
  });

  it('renders consent section when keyring is unavailable (line 118)', () => {
    setupState(makeKeyringStatus({ available: false, activeMode: 'consent_pending' }));
    renderWithProviders(<SecurityPanel />);
    // en.ts: 'keyring.settings.consentTitle': 'Storage consent'
    expect(screen.getByText('Storage consent')).toBeInTheDocument();
  });

  it('does NOT render consent section when keyring is available', () => {
    setupState(makeKeyringStatus({ available: true }));
    renderWithProviders(<SecurityPanel />);
    expect(screen.queryByText('Storage consent')).not.toBeInTheDocument();
  });

  it('shows grantConsent button when mode is not local_encrypted (line 123)', () => {
    setupState(makeKeyringStatus({ available: false, activeMode: 'consent_pending' }));
    renderWithProviders(<SecurityPanel />);
    // en.ts: 'keyring.settings.grantConsent': 'Allow local encrypted storage'
    expect(screen.getByText('Allow local encrypted storage')).toBeInTheDocument();
  });

  it('hides grantConsent button when mode is already local_encrypted', () => {
    setupState(makeKeyringStatus({ available: false, activeMode: 'local_encrypted' }));
    renderWithProviders(<SecurityPanel />);
    expect(screen.queryByText('Allow local encrypted storage')).not.toBeInTheDocument();
  });

  it('shows revokeConsent button when mode is not declined (line 128)', () => {
    setupState(makeKeyringStatus({ available: false, activeMode: 'consent_pending' }));
    renderWithProviders(<SecurityPanel />);
    // en.ts: 'keyring.settings.revokeConsent': 'Decline local storage'
    expect(screen.getByText('Decline local storage')).toBeInTheDocument();
  });

  it('hides revokeConsent button when mode is already declined', () => {
    setupState(makeKeyringStatus({ available: false, activeMode: 'declined' }));
    renderWithProviders(<SecurityPanel />);
    expect(screen.queryByText('Decline local storage')).not.toBeInTheDocument();
  });

  it('calls decideKeyringConsent with local_encrypted when grantConsent clicked (line 133)', async () => {
    setupState(makeKeyringStatus({ available: false, activeMode: 'consent_pending' }));
    renderWithProviders(<SecurityPanel />);

    fireEvent.click(screen.getByText('Allow local encrypted storage'));

    await waitFor(() => {
      expect(mockDecideKeyringConsent).toHaveBeenCalledWith('local_encrypted');
    });
  });

  it('calls decideKeyringConsent with declined when revokeConsent clicked (line 133)', async () => {
    setupState(makeKeyringStatus({ available: false, activeMode: 'consent_pending' }));
    renderWithProviders(<SecurityPanel />);

    fireEvent.click(screen.getByText('Decline local storage'));

    await waitFor(() => {
      expect(mockDecideKeyringConsent).toHaveBeenCalledWith('declined');
    });
  });

  it('shows error when decideKeyringConsent fails', async () => {
    mockDecideKeyringConsent.mockRejectedValue(new Error('consent rejected'));
    setupState(makeKeyringStatus({ available: false, activeMode: 'consent_pending' }));
    renderWithProviders(<SecurityPanel />);

    fireEvent.click(screen.getByText('Allow local encrypted storage'));

    await waitFor(() => {
      // en.ts: 'keyring.consent.error': 'Failed to save preference. Please try again.'
      expect(screen.getByText('Failed to save preference. Please try again.')).toBeInTheDocument();
    });
  });
});
