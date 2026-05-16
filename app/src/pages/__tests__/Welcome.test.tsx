import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { clearBackendUrlCache } from '../../services/backendUrl';
import { clearCoreRpcTokenCache, clearCoreRpcUrlCache } from '../../services/coreRpcClient';
import { useDeepLinkAuthState } from '../../store/deepLinkAuthState';
import { renderWithProviders } from '../../test/test-utils';
import {
  clearStoredCoreMode,
  clearStoredCoreToken,
  storeRpcUrl,
} from '../../utils/configPersistence';
import Welcome from '../Welcome';

const oauthButtonSpy = vi.fn();
const oauthOverrideSpy = vi.fn();

vi.mock('../../components/RotatingTetrahedronCanvas', () => ({
  default: () => <div data-testid="welcome-logo" />,
}));

vi.mock('../../components/oauth/OAuthProviderButton', () => ({
  default: ({
    provider,
    onClickOverride,
  }: {
    provider: { id: string };
    onClickOverride?: () => void;
  }) => (
    <button
      type="button"
      onClick={() => {
        oauthButtonSpy(provider.id);
        if (onClickOverride) {
          oauthOverrideSpy(provider.id);
          onClickOverride();
        }
      }}>
      {provider.id}
    </button>
  ),
}));

vi.mock('../../components/oauth/providerConfigs', () => ({
  oauthProviderConfigs: [
    { id: 'google', showOnWelcome: true },
    { id: 'github', showOnWelcome: true },
    { id: 'twitter', showOnWelcome: true },
    { id: 'discord', showOnWelcome: false },
  ],
}));

vi.mock('../../store/deepLinkAuthState', () => ({ useDeepLinkAuthState: vi.fn() }));

const { mockClearAllAppData } = vi.hoisted(() => ({
  mockClearAllAppData: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../utils/clearAllAppData', () => ({
  clearAllAppData: (...args: unknown[]) => mockClearAllAppData(...args),
}));

vi.mock('../../services/coreRpcClient', () => ({
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
  testCoreRpcConnection: vi.fn(),
}));

vi.mock('../../services/backendUrl', () => ({
  clearBackendUrlCache: vi.fn(),
  getBackendUrl: vi.fn().mockResolvedValue('http://localhost:5005'),
}));

vi.mock('../../utils/configPersistence', () => ({
  getStoredRpcUrl: vi.fn(() => 'http://127.0.0.1:7788/rpc'),
  peekStoredRpcUrl: vi.fn(() => null),
  storeRpcUrl: vi.fn(),
  clearStoredRpcUrl: vi.fn(),
  getStoredCoreToken: vi.fn(() => null),
  storeCoreToken: vi.fn(),
  clearStoredCoreToken: vi.fn(),
  getStoredCoreMode: vi.fn(() => null),
  storeCoreMode: vi.fn(),
  clearStoredCoreMode: vi.fn(),
  getDefaultRpcUrl: vi.fn(() => 'http://127.0.0.1:7788/rpc'),
  isValidRpcUrl: vi.fn((url: string) => {
    if (!url || url.trim().length === 0) return false;
    try {
      const parsed = new URL(url);
      return parsed.protocol === 'http:' || parsed.protocol === 'https:';
    } catch {
      return false;
    }
  }),
  normalizeRpcUrl: vi.fn((url: string) => url.trim().replace(/\/+$/, '')),
}));

describe('Welcome auth entrypoint', () => {
  beforeEach(() => {
    oauthButtonSpy.mockReset();
    oauthOverrideSpy.mockReset();
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
  });

  it('renders only the OAuth buttons when auth is idle', () => {
    renderWithProviders(<Welcome />);

    expect(screen.queryByLabelText('Email address')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Continue with email' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'google' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'github' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'twitter' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'discord' })).not.toBeInTheDocument();
  });

  it('delegates OAuth clicks to OAuthProviderButton without an override', () => {
    renderWithProviders(<Welcome />);

    fireEvent.click(screen.getByRole('button', { name: 'google' }));
    fireEvent.click(screen.getByRole('button', { name: 'github' }));
    fireEvent.click(screen.getByRole('button', { name: 'twitter' }));

    expect(oauthButtonSpy).toHaveBeenNthCalledWith(1, 'google');
    expect(oauthButtonSpy).toHaveBeenNthCalledWith(2, 'github');
    expect(oauthButtonSpy).toHaveBeenNthCalledWith(3, 'twitter');
    expect(oauthOverrideSpy).not.toHaveBeenCalled();
  });

  it('shows the deep-link processing state when auth is already in progress', () => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: true,
      errorMessage: null,
      requiresAppDataReset: false,
    });

    renderWithProviders(<Welcome />);

    expect(screen.getByRole('status')).toHaveTextContent('Signing you in...');
  });

  it('renders deep-link auth errors', () => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: 'OAuth failed',
      requiresAppDataReset: false,
    });

    renderWithProviders(<Welcome />);

    expect(screen.getByRole('alert')).toHaveTextContent('OAuth failed');
    expect(
      screen.queryByRole('button', { name: /Clear app data & restart/ })
    ).not.toBeInTheDocument();
  });
});

describe('Welcome — decryption-failure recovery action', () => {
  beforeEach(() => {
    mockClearAllAppData.mockReset().mockResolvedValue(undefined);
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: "Sign-in failed because OpenHuman couldn't decrypt locally stored data.",
      requiresAppDataReset: true,
    });
  });

  it('renders the "Clear app data & restart" button when reset is required', () => {
    renderWithProviders(<Welcome />);

    expect(screen.getByRole('button', { name: /Clear app data & restart/ })).toBeInTheDocument();
    expect(screen.getByText(/cloud account is unaffected/i)).toBeInTheDocument();
  });

  it('invokes clearAllAppData on click', async () => {
    renderWithProviders(<Welcome />);

    fireEvent.click(screen.getByRole('button', { name: /Clear app data & restart/ }));

    await waitFor(() => expect(mockClearAllAppData).toHaveBeenCalledTimes(1));
    // Pre-login path: no clearSession callback is passed.
    expect(mockClearAllAppData).toHaveBeenCalledWith();
  });

  it('surfaces an inline error if clearAllAppData rejects', async () => {
    mockClearAllAppData.mockRejectedValueOnce(new Error('reset blew up'));

    renderWithProviders(<Welcome />);
    fireEvent.click(screen.getByRole('button', { name: /Clear app data & restart/ }));

    await waitFor(() => {
      expect(screen.getByText(/Could not clear app data/)).toBeInTheDocument();
    });
    // Button re-enabled so the user can retry.
    expect(screen.getByRole('button', { name: /Clear app data & restart/ })).not.toBeDisabled();
  });
});

describe('Welcome — Select runtime button', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    vi.mocked(clearCoreRpcUrlCache).mockReset();
    vi.mocked(clearCoreRpcTokenCache).mockReset();
    vi.mocked(clearBackendUrlCache).mockReset();
    vi.mocked(storeRpcUrl).mockReset();
    vi.mocked(clearStoredCoreToken).mockReset();
    vi.mocked(clearStoredCoreMode).mockReset();
  });

  it('renders the "Select a Runtime" button below the card', () => {
    renderWithProviders(<Welcome />);

    expect(screen.getByRole('button', { name: 'Select a Runtime' })).toBeInTheDocument();
  });

  it('does not render the legacy "Configure RPC URL (Advanced)" toggle', () => {
    renderWithProviders(<Welcome />);

    expect(
      screen.queryByRole('button', { name: 'Configure RPC URL (Advanced)' })
    ).not.toBeInTheDocument();
  });

  it('clicking "Select a Runtime" clears persisted core-mode state and resets caches', () => {
    const { store } = renderWithProviders(<Welcome />, {
      preloadedState: { coreMode: { mode: { kind: 'cloud', url: 'http://x', token: 't' } } },
    });

    fireEvent.click(screen.getByRole('button', { name: 'Select a Runtime' }));

    expect(storeRpcUrl).toHaveBeenCalledWith('');
    expect(clearStoredCoreToken).toHaveBeenCalledTimes(1);
    expect(clearStoredCoreMode).toHaveBeenCalledTimes(1);
    expect(clearCoreRpcUrlCache).toHaveBeenCalledTimes(1);
    expect(clearCoreRpcTokenCache).toHaveBeenCalledTimes(1);
    expect(clearBackendUrlCache).toHaveBeenCalledTimes(1);
    // Redux coreMode slice is reset to `unset` so BootCheckGate returns to picker.
    expect((store.getState() as { coreMode: { mode: { kind: string } } }).coreMode.mode.kind).toBe(
      'unset'
    );
  });
});

describe('Welcome — OAuth buttons presence', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
  });

  it('renders all providers with showOnWelcome=true', () => {
    renderWithProviders(<Welcome />);

    expect(screen.getByRole('button', { name: 'google' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'github' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'twitter' })).toBeInTheDocument();
  });

  it('does not render providers with showOnWelcome=false', () => {
    renderWithProviders(<Welcome />);

    expect(screen.queryByRole('button', { name: 'discord' })).not.toBeInTheDocument();
  });

  it('hides OAuth buttons while auth is processing', () => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: true,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    renderWithProviders(<Welcome />);

    expect(screen.queryByRole('button', { name: 'google' })).not.toBeInTheDocument();
  });
});
