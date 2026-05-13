import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { clearBackendUrlCache } from '../../services/backendUrl';
import { clearCoreRpcUrlCache, testCoreRpcConnection } from '../../services/coreRpcClient';
import { useDeepLinkAuthState } from '../../store/deepLinkAuthState';
import {
  clearStoredRpcUrl,
  getDefaultRpcUrl,
  getStoredRpcUrl,
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
    vi.mocked(clearCoreRpcUrlCache).mockReset();
    vi.mocked(clearBackendUrlCache).mockReset();
    vi.mocked(storeRpcUrl).mockReset();
    vi.mocked(clearStoredRpcUrl).mockReset();
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(getDefaultRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(testCoreRpcConnection).mockReset();
  });

  it('renders only the OAuth buttons when auth is idle', () => {
    render(<Welcome />);

    expect(screen.queryByLabelText('Email address')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Continue with email' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'google' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'github' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'twitter' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'discord' })).not.toBeInTheDocument();
  });

  it('delegates OAuth clicks to OAuthProviderButton without an override', () => {
    render(<Welcome />);

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

    render(<Welcome />);

    expect(screen.getByRole('status')).toHaveTextContent('Signing you in...');
  });

  it('renders deep-link auth errors', () => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: 'OAuth failed',
      requiresAppDataReset: false,
    });

    render(<Welcome />);

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
    render(<Welcome />);

    expect(screen.getByRole('button', { name: /Clear app data & restart/ })).toBeInTheDocument();
    expect(screen.getByText(/cloud account is unaffected/i)).toBeInTheDocument();
  });

  it('invokes clearAllAppData on click', async () => {
    render(<Welcome />);

    fireEvent.click(screen.getByRole('button', { name: /Clear app data & restart/ }));

    await waitFor(() => expect(mockClearAllAppData).toHaveBeenCalledTimes(1));
    // Pre-login path: no clearSession callback is passed.
    expect(mockClearAllAppData).toHaveBeenCalledWith();
  });

  it('surfaces an inline error if clearAllAppData rejects', async () => {
    mockClearAllAppData.mockRejectedValueOnce(new Error('reset blew up'));

    render(<Welcome />);
    fireEvent.click(screen.getByRole('button', { name: /Clear app data & restart/ }));

    await waitFor(() => {
      expect(screen.getByText(/Could not clear app data/)).toBeInTheDocument();
    });
    // Button re-enabled so the user can retry.
    expect(screen.getByRole('button', { name: /Clear app data & restart/ })).not.toBeDisabled();
  });
});

describe('Welcome — RPC URL advanced panel', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    vi.mocked(clearCoreRpcUrlCache).mockReset();
    vi.mocked(clearBackendUrlCache).mockReset();
    vi.mocked(storeRpcUrl).mockReset();
    vi.mocked(clearStoredRpcUrl).mockReset();
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(getDefaultRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(testCoreRpcConnection).mockReset();
  });

  it('renders with advanced panel collapsed by default', () => {
    render(<Welcome />);

    expect(screen.queryByLabelText('Core RPC URL')).not.toBeInTheDocument();
    expect(screen.queryByPlaceholderText('http://127.0.0.1:7788/rpc')).not.toBeInTheDocument();
  });

  it('shows the "Configure RPC URL (Advanced)" toggle when panel is collapsed', () => {
    render(<Welcome />);

    expect(
      screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' })
    ).toBeInTheDocument();
  });

  it('clicking the toggle opens the advanced panel', () => {
    render(<Welcome />);

    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));

    expect(screen.getByPlaceholderText('http://127.0.0.1:7788/rpc')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reset to Default' })).toBeInTheDocument();
  });

  it('panel shows the stored RPC URL as the initial input value', () => {
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://custom-host:9999/rpc');

    render(<Welcome />);
    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));

    expect(screen.getByPlaceholderText('http://127.0.0.1:7788/rpc')).toHaveValue(
      'http://custom-host:9999/rpc'
    );
  });

  it('panel shows the default URL when nothing custom is stored', () => {
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');

    render(<Welcome />);
    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));

    expect(screen.getByPlaceholderText('http://127.0.0.1:7788/rpc')).toHaveValue(
      'http://127.0.0.1:7788/rpc'
    );
  });

  it('clicking Close hides the advanced panel', () => {
    render(<Welcome />);

    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));
    expect(screen.getByPlaceholderText('http://127.0.0.1:7788/rpc')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByPlaceholderText('http://127.0.0.1:7788/rpc')).not.toBeInTheDocument();
  });
});

describe('Welcome — Save button', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    vi.mocked(clearCoreRpcUrlCache).mockReset();
    vi.mocked(clearBackendUrlCache).mockReset();
    vi.mocked(storeRpcUrl).mockReset();
    vi.mocked(clearStoredRpcUrl).mockReset();
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(getDefaultRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
  });

  function openPanel() {
    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));
  }

  it('clicking Save with a valid URL calls storeRpcUrl with the normalised URL', () => {
    render(<Welcome />);
    openPanel();

    const input = screen.getByPlaceholderText('http://127.0.0.1:7788/rpc');
    fireEvent.change(input, { target: { value: 'http://192.168.1.1:8000/rpc' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(storeRpcUrl).toHaveBeenCalledWith('http://192.168.1.1:8000/rpc');
  });

  it('clicking Save calls clearCoreRpcUrlCache()', () => {
    render(<Welcome />);
    openPanel();

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(clearCoreRpcUrlCache).toHaveBeenCalledTimes(1);
  });

  it('clicking Save calls clearBackendUrlCache()', () => {
    render(<Welcome />);
    openPanel();

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(clearBackendUrlCache).toHaveBeenCalledTimes(1);
  });

  it('clicking Save with an invalid URL shows a validation error and does NOT call storeRpcUrl', () => {
    render(<Welcome />);
    openPanel();

    const input = screen.getByPlaceholderText('http://127.0.0.1:7788/rpc');
    fireEvent.change(input, { target: { value: 'not-a-valid-url' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(screen.getByText('Please enter a valid HTTP or HTTPS URL')).toBeInTheDocument();
    expect(storeRpcUrl).not.toHaveBeenCalled();
  });

  it('clicking Save with empty string shows a validation error', () => {
    render(<Welcome />);
    openPanel();

    const input = screen.getByPlaceholderText('http://127.0.0.1:7788/rpc');
    fireEvent.change(input, { target: { value: '' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(screen.getByText('Please enter a valid HTTP or HTTPS URL')).toBeInTheDocument();
    expect(storeRpcUrl).not.toHaveBeenCalled();
  });

  it('shows a success message after a successful save', async () => {
    render(<Welcome />);
    openPanel();

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(await screen.findByText('URL saved successfully.')).toBeInTheDocument();
  });
});

describe('Welcome — Test Connection button', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    vi.mocked(clearCoreRpcUrlCache).mockReset();
    vi.mocked(clearBackendUrlCache).mockReset();
    vi.mocked(storeRpcUrl).mockReset();
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(getDefaultRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(testCoreRpcConnection).mockReset();
  });

  function openPanel() {
    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));
  }

  it('clicking Test Connection fires testCoreRpcConnection with the entered URL', async () => {
    vi.mocked(testCoreRpcConnection).mockResolvedValueOnce({ ok: true, status: 200 } as Response);

    render(<Welcome />);
    openPanel();

    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    await waitFor(() => {
      expect(testCoreRpcConnection).toHaveBeenCalledWith('http://127.0.0.1:7788/rpc');
    });
  });

  it('successful probe (200 ok) shows success message', async () => {
    vi.mocked(testCoreRpcConnection).mockResolvedValueOnce({ ok: true, status: 200 } as Response);

    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    await screen.findByText('URL saved successfully.');
  });

  it('successful probe with 405 status (expected for JSON-RPC ping) shows success message', async () => {
    vi.mocked(testCoreRpcConnection).mockResolvedValueOnce({
      ok: false,
      status: 405,
      statusText: 'Method Not Allowed',
    } as Response);

    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    await screen.findByText('URL saved successfully.');
  });

  it('failed probe (4xx/5xx status) shows an error message', async () => {
    vi.mocked(testCoreRpcConnection).mockResolvedValueOnce({
      ok: false,
      status: 503,
      statusText: 'Service Unavailable',
    } as Response);

    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    await screen.findByText('Connection failed: 503 Service Unavailable');
  });

  it('failed probe (network error) shows an error message with the error text', async () => {
    vi.mocked(testCoreRpcConnection).mockRejectedValueOnce(new Error('ECONNREFUSED'));

    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    await screen.findByText('Connection failed: ECONNREFUSED');
  });

  it('Test Connection with invalid URL shows validation error without calling testCoreRpcConnection', async () => {
    render(<Welcome />);
    openPanel();

    const input = screen.getByPlaceholderText('http://127.0.0.1:7788/rpc');
    fireEvent.change(input, { target: { value: 'bad-url' } });
    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    expect(screen.getByText('Please enter a valid HTTP or HTTPS URL')).toBeInTheDocument();
    expect(testCoreRpcConnection).not.toHaveBeenCalled();
  });

  it('shows loading state while the probe is in flight', async () => {
    let resolveProbe!: (r: Response) => void;
    vi.mocked(testCoreRpcConnection).mockReturnValueOnce(
      new Promise<Response>(resolve => {
        resolveProbe = resolve;
      })
    );

    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    // During flight the button label changes to "Testing" and the button is disabled
    const testBtn = screen.getByRole('button', { name: /testing/i });
    expect(testBtn).toBeDisabled();

    resolveProbe({ ok: true, status: 200 } as Response);
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: /testing/i })).not.toBeInTheDocument()
    );
  });

  it('successful probe stores the URL and clears both the RPC and backend URL caches', async () => {
    vi.mocked(testCoreRpcConnection).mockResolvedValueOnce({ ok: true, status: 200 } as Response);

    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Test' }));

    await waitFor(() => {
      expect(storeRpcUrl).toHaveBeenCalledWith('http://127.0.0.1:7788/rpc');
      expect(clearCoreRpcUrlCache).toHaveBeenCalledTimes(1);
      expect(clearBackendUrlCache).toHaveBeenCalledTimes(1);
    });
  });
});

describe('Welcome — Reset to Default button', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    vi.mocked(clearCoreRpcUrlCache).mockReset();
    vi.mocked(clearBackendUrlCache).mockReset();
    vi.mocked(storeRpcUrl).mockReset();
    vi.mocked(clearStoredRpcUrl).mockReset();
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://custom:9999/rpc');
    vi.mocked(getDefaultRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
  });

  function openPanel() {
    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));
  }

  it('clicking Reset calls clearStoredRpcUrl()', () => {
    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Reset to Default' }));

    expect(clearStoredRpcUrl).toHaveBeenCalledTimes(1);
  });

  it('clicking Reset calls clearCoreRpcUrlCache()', () => {
    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Reset to Default' }));

    expect(clearCoreRpcUrlCache).toHaveBeenCalledTimes(1);
  });

  it('clicking Reset calls clearBackendUrlCache()', () => {
    render(<Welcome />);
    openPanel();
    fireEvent.click(screen.getByRole('button', { name: 'Reset to Default' }));

    expect(clearBackendUrlCache).toHaveBeenCalledTimes(1);
  });

  it('after Reset, input value reverts to the default URL', () => {
    render(<Welcome />);
    openPanel();

    // Input starts with the custom stored value
    expect(screen.getByPlaceholderText('http://127.0.0.1:7788/rpc')).toHaveValue(
      'http://custom:9999/rpc'
    );

    fireEvent.click(screen.getByRole('button', { name: 'Reset to Default' }));

    expect(screen.getByPlaceholderText('http://127.0.0.1:7788/rpc')).toHaveValue(
      'http://127.0.0.1:7788/rpc'
    );
  });
});

describe('Welcome — URL input behaviour', () => {
  beforeEach(() => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: false,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    vi.mocked(clearCoreRpcUrlCache).mockReset();
    vi.mocked(clearBackendUrlCache).mockReset();
    vi.mocked(storeRpcUrl).mockReset();
    vi.mocked(getStoredRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
    vi.mocked(getDefaultRpcUrl).mockReturnValue('http://127.0.0.1:7788/rpc');
  });

  function openPanel() {
    fireEvent.click(screen.getByRole('button', { name: 'Configure RPC URL (Advanced)' }));
  }

  it('typing in the input updates the displayed value', () => {
    render(<Welcome />);
    openPanel();

    const input = screen.getByPlaceholderText('http://127.0.0.1:7788/rpc');
    fireEvent.change(input, { target: { value: 'http://new-host:5555/rpc' } });

    expect(input).toHaveValue('http://new-host:5555/rpc');
  });

  it('typing a valid URL clears any existing error', () => {
    render(<Welcome />);
    openPanel();

    // First trigger an error
    const input = screen.getByPlaceholderText('http://127.0.0.1:7788/rpc');
    fireEvent.change(input, { target: { value: 'bad' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.getByText('Please enter a valid HTTP or HTTPS URL')).toBeInTheDocument();

    // Then type a valid URL — error should clear
    fireEvent.change(input, { target: { value: 'http://valid-host:9000/rpc' } });
    expect(screen.queryByText('Please enter a valid HTTP or HTTPS URL')).not.toBeInTheDocument();
  });

  it('invalid URL (missing protocol) shows inline error on Save', () => {
    render(<Welcome />);
    openPanel();

    const input = screen.getByPlaceholderText('http://127.0.0.1:7788/rpc');
    fireEvent.change(input, { target: { value: 'localhost:9999' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(screen.getByText('Please enter a valid HTTP or HTTPS URL')).toBeInTheDocument();
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
    render(<Welcome />);

    expect(screen.getByRole('button', { name: 'google' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'github' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'twitter' })).toBeInTheDocument();
  });

  it('does not render providers with showOnWelcome=false', () => {
    render(<Welcome />);

    expect(screen.queryByRole('button', { name: 'discord' })).not.toBeInTheDocument();
  });

  it('hides OAuth buttons while auth is processing', () => {
    vi.mocked(useDeepLinkAuthState).mockReturnValue({
      isProcessing: true,
      errorMessage: null,
      requiresAppDataReset: false,
    });
    render(<Welcome />);

    expect(screen.queryByRole('button', { name: 'google' })).not.toBeInTheDocument();
  });
});
