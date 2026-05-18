/**
 * Component tests for BootCheckGate.
 *
 * Strategy:
 *   - Mock runBootCheck so we control the result without real RPC/invoke.
 *   - Use a minimal Redux store that starts with coreMode.mode = 'unset'
 *     (picker) or set (check flow).
 *   - Assert rendered text and dispatched actions for each meaningful state.
 */
import { configureStore } from '@reduxjs/toolkit';
import { isTauri } from '@tauri-apps/api/core';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import coreModeReducer, { type CoreModeState } from '../../../store/coreModeSlice';
import localeReducer from '../../../store/localeSlice';
import BootCheckGate from '../BootCheckGate';

// The global test setup mocks isTauri()=>false (web). The existing picker
// behavior under test was written for desktop (local option visible,
// pre-selected). Force desktop runtime for those describes; the new web
// describe at the bottom flips it back to false.
const mockedIsTauri = vi.mocked(isTauri);

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

const mockRunBootCheck = vi.fn();
vi.mock('../../../lib/bootCheck', () => ({
  runBootCheck: (...args: unknown[]) => mockRunBootCheck(...args),
}));

const mockTestCoreRpcConnection = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: vi.fn(),
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
  testCoreRpcConnection: (...args: unknown[]) => mockTestCoreRpcConnection(...args),
}));

vi.mock('../../../utils/configPersistence', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../utils/configPersistence')>();
  return {
    ...actual,
    storeRpcUrl: vi.fn(),
    storeCoreToken: vi.fn(),
    clearStoredCoreToken: vi.fn(),
    storeCoreMode: vi.fn(),
    clearStoredCoreMode: vi.fn(),
  };
});

// ---------------------------------------------------------------------------
// Store factory
// ---------------------------------------------------------------------------

function makeStore(initialMode?: CoreModeState['mode']) {
  return configureStore({
    reducer: { coreMode: coreModeReducer, locale: localeReducer },
    preloadedState: {
      coreMode: { mode: initialMode ?? { kind: 'unset' } } satisfies CoreModeState,
    },
  });
}

function renderGate(store = makeStore()) {
  return render(
    <Provider store={store}>
      <BootCheckGate>
        <div data-testid="app-content">App Content</div>
      </BootCheckGate>
    </Provider>
  );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// All describes below assume desktop unless they explicitly opt out.
beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
});

describe('BootCheckGate — picker (unset mode)', () => {
  it('shows the mode picker when coreMode is unset', () => {
    renderGate();
    expect(screen.getByText('Select a Runtime')).toBeInTheDocument();
    expect(screen.getByText('Run Locally (Recommended)')).toBeInTheDocument();
    expect(screen.getByText('Run on the Cloud (Complex)')).toBeInTheDocument();
  });

  it('does NOT render children while in picker', () => {
    renderGate();
    expect(screen.queryByTestId('app-content')).not.toBeInTheDocument();
  });

  it('continues with local mode when user clicks Continue', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();

    // Local is pre-selected — just click Continue
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
  });

  it('shows URL input when user selects Cloud', () => {
    renderGate();

    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));

    expect(screen.getByPlaceholderText(/https:\/\/core\.example\.com/)).toBeInTheDocument();
  });

  it('shows URL validation error when cloud URL is empty', () => {
    renderGate();

    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText('Please enter a runtime URL.')).toBeInTheDocument();
  });

  it('shows URL validation error for non-http URL', () => {
    renderGate();

    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    const input = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(input, { target: { value: 'ftp://invalid' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/start with http/)).toBeInTheDocument();
  });

  it('shows URL validation error for malformed URL string', () => {
    renderGate();

    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    const input = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(input, { target: { value: 'not a url at all' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/That doesn't look like a valid URL/)).toBeInTheDocument();
  });

  it('shows token validation error when cloud URL is valid but token is missing', () => {
    renderGate();

    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    const urlInput = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(urlInput, { target: { value: 'https://core.example.com/rpc' } });
    // Token left blank.
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/We'll need an auth token to connect/i)).toBeInTheDocument();
  });

  it('accepts a Tailscale HTTP core URL in cloud mode', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();
    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'http://100.116.244.64:7788/rpc' },
    });
    fireEvent.change(screen.getByPlaceholderText(/Bearer token/i), {
      target: { value: 'tok-1234' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'cloud',
        url: 'http://100.116.244.64:7788/rpc',
        token: 'tok-1234',
      }),
      expect.any(Object)
    );
  });

  it('rejects public HTTP cloud URLs', () => {
    renderGate();

    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    const urlInput = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(urlInput, { target: { value: 'http://core.example.com/rpc' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/HTTP core URLs are only allowed/)).toBeInTheDocument();
  });

  it('clears the token error as soon as the user types into the token field', () => {
    renderGate();

    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(screen.getByText(/We'll need an auth token to connect/i)).toBeInTheDocument();

    const tokenInput = screen.getByPlaceholderText(/Bearer token/i);
    fireEvent.change(tokenInput, { target: { value: 'tok' } });

    expect(screen.queryByText(/We'll need an auth token to connect/i)).not.toBeInTheDocument();
  });

  it('advances past picker and triggers boot check when cloud URL + token are both set', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();
    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.change(screen.getByPlaceholderText(/Bearer token/i), {
      target: { value: 'tok-1234' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'cloud',
        url: 'https://core.example.com/rpc',
        token: 'tok-1234',
      }),
      expect.any(Object)
    );
  });
});

describe('BootCheckGate — picker test connection', () => {
  beforeEach(() => {
    mockTestCoreRpcConnection.mockReset();
  });

  function fillCloudInputs(url = 'https://core.example.com/rpc', token = 'tok-abc') {
    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: url },
    });
    fireEvent.change(screen.getByPlaceholderText(/Bearer token/i), { target: { value: token } });
  }

  it('shows Connected on a 200 response', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ result: { ok: true } }),
    } as unknown as Response);

    renderGate();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-ok')).toBeInTheDocument();
    });
    expect(mockTestCoreRpcConnection).toHaveBeenCalledWith(
      'https://core.example.com/rpc',
      'tok-abc'
    );
  });

  it('shows Auth failed on a 401 response', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: false,
      status: 401,
      json: async () => ({ error: 'unauthorized' }),
    } as unknown as Response);

    renderGate();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-auth')).toBeInTheDocument();
    });
  });

  it('shows Auth failed on a 403 response', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: false,
      status: 403,
      json: async () => ({}),
    } as unknown as Response);

    renderGate();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-auth')).toBeInTheDocument();
    });
  });

  it('shows Unreachable when fetch rejects', async () => {
    mockTestCoreRpcConnection.mockRejectedValue(new Error('network down'));

    renderGate();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-unreachable')).toBeInTheDocument();
    });
    expect(screen.getByTestId('test-status-unreachable').textContent).toMatch(/network down/);
  });

  it('shows Unreachable on non-2xx non-auth response', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: false,
      status: 500,
      json: async () => ({}),
    } as unknown as Response);

    renderGate();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-unreachable')).toBeInTheDocument();
    });
    expect(screen.getByTestId('test-status-unreachable').textContent).toMatch(/HTTP 500/);
  });

  it('does not call the test endpoint when URL is missing', () => {
    renderGate();
    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    expect(mockTestCoreRpcConnection).not.toHaveBeenCalled();
    expect(screen.getByText('Please enter a runtime URL.')).toBeInTheDocument();
  });

  it('does not call the test endpoint when token is missing', () => {
    renderGate();
    fireEvent.click(screen.getByText('Run on the Cloud (Complex)'));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    expect(mockTestCoreRpcConnection).not.toHaveBeenCalled();
    expect(screen.getByText(/We'll need an auth token to connect/i)).toBeInTheDocument();
  });

  it('clears a stale ok status when the user edits inputs again', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({}),
    } as unknown as Response);

    renderGate();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));
    await waitFor(() => {
      expect(screen.getByTestId('test-status-ok')).toBeInTheDocument();
    });

    fireEvent.change(screen.getByPlaceholderText(/Bearer token/i), {
      target: { value: 'tok-def' },
    });

    expect(screen.queryByTestId('test-status-ok')).not.toBeInTheDocument();
  });
});

describe('BootCheckGate — checking state', () => {
  it('shows checking spinner while boot check is in flight', async () => {
    // Never resolves during this test
    mockRunBootCheck.mockImplementation(() => new Promise(() => {}));

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Waking up your runtime…')).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — match result', () => {
  it('renders children once boot check returns match', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — daemonDetected', () => {
  it('shows daemon detection screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'daemonDetected' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Legacy Background Runtime Detected')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Remove and Continue' })).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — outdatedLocal', () => {
  it('shows outdated local screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'outdatedLocal' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Local Runtime Needs a Restart')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Restart Runtime' })).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — outdatedCloud', () => {
  it('shows outdated cloud screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'outdatedCloud' });

    const store = makeStore({ kind: 'cloud', url: 'https://core.example.com/rpc' });
    // Trigger the check by rendering with an already-set mode
    mockRunBootCheck.mockResolvedValue({ kind: 'outdatedCloud' });
    render(
      <Provider store={store}>
        <BootCheckGate>
          <div data-testid="app-content">App Content</div>
        </BootCheckGate>
      </Provider>
    );

    await waitFor(() => {
      expect(screen.getByText('Cloud Runtime Needs an Update')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Update Cloud Runtime' })).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — noVersionMethod', () => {
  it('shows no version method screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'noVersionMethod' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Runtime Version Check Failed')).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — unreachable', () => {
  it('shows unreachable screen with quit and switch mode buttons', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText("Can't Reach the Runtime")).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Quit' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Pick a Different Runtime' })).toBeInTheDocument();
    });
  });

  it("returns to picker when 'Pick a Different Runtime' is clicked", async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Pick a Different Runtime' })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Pick a Different Runtime' }));

    await waitFor(() => {
      expect(screen.getByText('Select a Runtime')).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — pre-set mode (subsequent launches)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('skips picker and goes directly to checking when mode is already set', async () => {
    mockRunBootCheck.mockImplementation(() => new Promise(() => {}));

    const store = makeStore({ kind: 'local' });
    render(
      <Provider store={store}>
        <BootCheckGate>
          <div data-testid="app-content">App Content</div>
        </BootCheckGate>
      </Provider>
    );

    await waitFor(() => {
      expect(screen.getByText('Waking up your runtime…')).toBeInTheDocument();
    });

    expect(screen.queryByText('Select a Runtime')).not.toBeInTheDocument();
  });
});

describe('BootCheckGate — picker (web build, !isTauri)', () => {
  beforeEach(() => {
    mockedIsTauri.mockReturnValue(false);
  });

  it('uses the web-friendly title and hides the Local option', () => {
    renderGate();

    expect(screen.getByText('Connect to Your Runtime')).toBeInTheDocument();
    expect(screen.queryByText('Select a Runtime')).not.toBeInTheDocument();
    expect(screen.queryByText('Run Locally (Recommended)')).not.toBeInTheDocument();
    // The selectable Cloud tile is also gone — cloud is implicit and the
    // URL/token form is rendered directly.
    expect(
      screen.queryByRole('button', { name: 'Run on the Cloud (Complex)' })
    ).not.toBeInTheDocument();
  });

  it('renders the cloud form fields immediately (cloud is the only option)', () => {
    renderGate();

    expect(screen.getByPlaceholderText(/https:\/\/core\.example\.com/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Bearer token/i)).toBeInTheDocument();
  });

  it('shows a Download desktop app CTA linking to the release page', () => {
    renderGate();

    const cta = screen.getByTestId('web-download-cta');
    expect(cta).toBeInTheDocument();
    const link = cta.querySelector('a');
    expect(link).not.toBeNull();
    expect(link?.getAttribute('href')).toMatch(
      /github\.com\/tinyhumansai\/openhuman\/releases\/latest/
    );
    expect(link?.getAttribute('target')).toBe('_blank');
    expect(link?.getAttribute('rel')).toMatch(/noopener/);
  });

  it('continues into a cloud boot check when URL + token are provided', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();

    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.change(screen.getByPlaceholderText(/Bearer token/i), {
      target: { value: 'tok-web' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'cloud',
        url: 'https://core.example.com/rpc',
        token: 'tok-web',
      }),
      expect.any(Object)
    );
  });
});
