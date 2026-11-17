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

import en from '../../../lib/i18n/en';
import coreModeReducer, { type CoreModeState } from '../../../store/coreModeSlice';
import localeReducer from '../../../store/localeSlice';
import { clearStoredCoreToken, storeCoreMode, storeRpcUrl } from '../../../utils/configPersistence';
import BootCheckGate from '../BootCheckGate';

// The global test setup mocks isTauri()=>false (web). The existing picker
// behavior under test was written for desktop (local option visible,
// pre-selected). Force desktop runtime for those describes; the new web
// describe at the bottom flips it back to false.
const mockedIsTauri = vi.mocked(isTauri);

// Assert against the real English dictionary so copy edits do not silently
// turn these behavioural tests red (or green).
const copy = (key: string): string => {
  const value = (en as Record<string, string>)[key];
  if (value === undefined) throw new Error(`missing en key ${key}`);
  return value;
};

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

const mockRunBootCheck = vi.fn();
vi.mock('../../../lib/bootCheck', () => ({
  runBootCheck: (...args: unknown[]) => mockRunBootCheck(...args),
}));

const mockRecoverPortConflict = vi.fn();
const mockForceQuitPortOwner = vi.fn();
vi.mock('../../../services/bootCheckService', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../services/bootCheckService')>();
  return {
    ...actual,
    recoverPortConflict: (...args: unknown[]) => mockRecoverPortConflict(...args),
    forceQuitPortOwner: (...args: unknown[]) => mockForceQuitPortOwner(...args),
  };
});

const mockTestCoreRpcConnection = vi.fn();
const mockProbeCoreRealtime = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: vi.fn(),
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
  testCoreRpcConnection: (...args: unknown[]) => mockTestCoreRpcConnection(...args),
  probeCoreRealtime: (...args: unknown[]) => mockProbeCoreRealtime(...args),
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

/**
 * Reach the picker the way a real desktop user now does: first launch
 * auto-boots local (no picker), the boot check comes back unreachable, and the
 * user clicks copy('bootCheck.switchMode'). Consumes exactly one
 * `mockRunBootCheck` result (queued with `Once`), so callers may set their own
 * default result beforehand for what happens after the picker's Continue.
 */
async function renderPicker(store = makeStore()) {
  mockRunBootCheck.mockResolvedValueOnce({ kind: 'unreachable', reason: 'Connection refused' });
  const view = renderGate(store);
  fireEvent.click(await screen.findByRole('button', { name: copy('bootCheck.switchMode') }));
  await screen.findByText(copy('bootCheck.chooseCoreMode'));
  return view;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// All describes below assume desktop unless they explicitly opt out.
beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
});

describe('BootCheckGate — first launch on desktop (unset mode)', () => {
  beforeEach(() => {
    mockRunBootCheck.mockReset();
    vi.mocked(storeCoreMode).mockClear();
  });

  // The picker used to be the first screen. It asked an infrastructure
  // question before the app showed anything, and its cloud branch cannot be
  // answered on a genuine first run (needs a URL + bearer for an already
  // deployed core). It is now a fallback, not an entry point.
  it('does not show the picker and proceeds to the checking phase', async () => {
    mockRunBootCheck.mockImplementation(() => new Promise(() => {}));
    const store = makeStore();
    renderGate(store);

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.checkingCore'))).toBeInTheDocument();
    });
    expect(screen.queryByText(copy('bootCheck.chooseCoreMode'))).not.toBeInTheDocument();
    expect(screen.queryByText(copy('bootCheck.localRecommended'))).not.toBeInTheDocument();
    expect(store.getState().coreMode.mode).toEqual({ kind: 'local' });
  });

  it('adopts local mode and persists the marker', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });
    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(storeCoreMode).toHaveBeenCalledWith('local');
    expect(clearStoredCoreToken).toHaveBeenCalled();
    expect(storeRpcUrl).toHaveBeenCalledWith('');
    expect(mockRunBootCheck).toHaveBeenCalledWith({ kind: 'local' }, expect.any(Object));
  });

  it('reaches the picker as a fallback when local boot fails', async () => {
    await renderPicker();
    expect(screen.getByText(copy('bootCheck.localRecommended'))).toBeInTheDocument();
    expect(screen.getByText(copy('bootCheck.cloudMode'))).toBeInTheDocument();
  });
});

describe('BootCheckGate — picker (reached via Pick a Different Runtime)', () => {
  it('shows the mode picker when coreMode is unset', async () => {
    await renderPicker();
    expect(screen.getByText(copy('bootCheck.chooseCoreMode'))).toBeInTheDocument();
    expect(screen.getByText(copy('bootCheck.localRecommended'))).toBeInTheDocument();
    expect(screen.getByText(copy('bootCheck.cloudMode'))).toBeInTheDocument();
  });

  it('does NOT render children while in picker', async () => {
    await renderPicker();
    expect(screen.queryByTestId('app-content')).not.toBeInTheDocument();
  });

  it('continues with local mode when user clicks Continue', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    await renderPicker();

    // Local is pre-selected — just click Continue
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
  });

  it('shows URL input when user selects Cloud', async () => {
    await renderPicker();

    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));

    expect(screen.getByPlaceholderText(/https:\/\/core\.example\.com/)).toBeInTheDocument();
  });

  it('shows URL validation error when cloud URL is empty', async () => {
    await renderPicker();

    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(copy('bootCheck.invalidUrl'))).toBeInTheDocument();
  });

  it('shows URL validation error for non-http URL', async () => {
    await renderPicker();

    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    const input = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(input, { target: { value: 'ftp://invalid' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/start with http/)).toBeInTheDocument();
  });

  it('shows URL validation error for malformed URL string', async () => {
    await renderPicker();

    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    const input = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(input, { target: { value: 'not a url at all' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/That doesn't look like a valid URL/)).toBeInTheDocument();
  });

  it('shows token validation error when cloud URL is valid but token is missing', async () => {
    await renderPicker();

    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    const urlInput = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(urlInput, { target: { value: 'https://core.example.com/rpc' } });
    // Token left blank.
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/We'll need an auth token to connect/i)).toBeInTheDocument();
  });

  it('accepts a Tailscale HTTP core URL in cloud mode', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    await renderPicker();
    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'http://100.116.244.64:7788/rpc' },
    });
    fireEvent.change(screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder')), {
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

  it('normalizes a cloud core base URL to the /rpc endpoint before continuing', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    await renderPicker();
    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'https://example.trycloudflare.com/' },
    });
    fireEvent.change(screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder')), {
      target: { value: 'tok-1234' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'cloud',
        url: 'https://example.trycloudflare.com/rpc',
        token: 'tok-1234',
      }),
      expect.any(Object)
    );
  });

  it('warns about public HTTP cloud URLs but does not block them', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    await renderPicker();

    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    const urlInput = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(urlInput, { target: { value: 'http://core.example.com/rpc' } });

    // Non-blocking warning shows inline as soon as the public HTTP URL is typed.
    expect(screen.getByText(/traffic will not be encrypted/i)).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder')), {
      target: { value: 'tok-1234' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    // The boot check still proceeds with the HTTP URL.
    await waitFor(() => {
      expect(mockRunBootCheck).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: 'cloud',
          url: 'http://core.example.com/rpc',
          token: 'tok-1234',
        }),
        expect.any(Object)
      );
    });
  });

  it('clears the token error as soon as the user types into the token field', async () => {
    await renderPicker();

    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(screen.getByText(/We'll need an auth token to connect/i)).toBeInTheDocument();

    const tokenInput = screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder'));
    fireEvent.change(tokenInput, { target: { value: 'tok' } });

    expect(screen.queryByText(/We'll need an auth token to connect/i)).not.toBeInTheDocument();
  });

  it('advances past picker and triggers boot check when cloud URL + token are both set', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    await renderPicker();
    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: 'https://core.example.com/rpc' },
    });
    fireEvent.change(screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder')), {
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
    mockProbeCoreRealtime.mockReset();
    mockProbeCoreRealtime.mockResolvedValue('ok');
  });

  function fillCloudInputs(url = 'https://core.example.com/rpc', token = 'tok-abc') {
    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    fireEvent.change(screen.getByPlaceholderText(/https:\/\/core\.example\.com/), {
      target: { value: url },
    });
    fireEvent.change(screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder')), {
      target: { value: token },
    });
  }

  it('flags a core with realtime (Socket.IO) disabled even though RPC works', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ result: { ok: true } }),
    } as unknown as Response);
    mockProbeCoreRealtime.mockResolvedValue('disabled');

    await renderPicker();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-socket-disabled')).toHaveTextContent('--jsonrpc-only');
    });
    expect(screen.queryByTestId('test-status-ok')).not.toBeInTheDocument();
  });

  it('shows Connected on a 200 response', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ result: { ok: true } }),
    } as unknown as Response);

    await renderPicker();
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

  it('tests /rpc when the user enters a cloud core base URL', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ result: { ok: true } }),
    } as unknown as Response);

    await renderPicker();
    fillCloudInputs('https://example.trycloudflare.com/');
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-ok')).toBeInTheDocument();
    });
    expect(mockTestCoreRpcConnection).toHaveBeenCalledWith(
      'https://example.trycloudflare.com/rpc',
      'tok-abc'
    );
  });

  it('shows Auth failed on a 401 response', async () => {
    mockTestCoreRpcConnection.mockResolvedValue({
      ok: false,
      status: 401,
      json: async () => ({ error: 'unauthorized' }),
    } as unknown as Response);

    await renderPicker();
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

    await renderPicker();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-auth')).toBeInTheDocument();
    });
  });

  it('shows Unreachable when fetch rejects', async () => {
    mockTestCoreRpcConnection.mockRejectedValue(new Error('network down'));

    await renderPicker();
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

    await renderPicker();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    await waitFor(() => {
      expect(screen.getByTestId('test-status-unreachable')).toBeInTheDocument();
    });
    expect(screen.getByTestId('test-status-unreachable').textContent).toMatch(/HTTP 500/);
  });

  it('does not call the test endpoint when URL is missing', async () => {
    await renderPicker();
    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));

    expect(mockTestCoreRpcConnection).not.toHaveBeenCalled();
    expect(screen.getByText(copy('bootCheck.invalidUrl'))).toBeInTheDocument();
  });

  it('does not call the test endpoint when token is missing', async () => {
    await renderPicker();
    fireEvent.click(screen.getByText(copy('bootCheck.cloudMode')));
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

    await renderPicker();
    fillCloudInputs();
    fireEvent.click(screen.getByRole('button', { name: 'Test Connection' }));
    await waitFor(() => {
      expect(screen.getByTestId('test-status-ok')).toBeInTheDocument();
    });

    fireEvent.change(screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder')), {
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

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.checkingCore'))).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — match result', () => {
  it('renders children once boot check returns match', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — daemonDetected', () => {
  it('shows daemon detection screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'daemonDetected' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.legacyDetected'))).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Remove and Continue' })).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — outdatedLocal', () => {
  it('shows outdated local screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'outdatedLocal' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.localNeedsRestart'))).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: copy('bootCheck.restartCore') })
      ).toBeInTheDocument();
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
      expect(screen.getByText(copy('bootCheck.cloudNeedsUpdate'))).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: copy('bootCheck.updateCloudCore') })
      ).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — noVersionMethod', () => {
  it('shows no version method screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'noVersionMethod' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.versionCheckFailed'))).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — unreachable', () => {
  it('shows unreachable screen with quit and switch mode buttons', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.cannotReach'))).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Quit' })).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: copy('bootCheck.switchMode') })
      ).toBeInTheDocument();
    });
  });

  it('returns to picker when the switch-mode button is clicked', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate();

    await waitFor(() => {
      expect(
        screen.getByRole('button', { name: copy('bootCheck.switchMode') })
      ).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: copy('bootCheck.switchMode') }));

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.chooseCoreMode'))).toBeInTheDocument();
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
      expect(screen.getByText(copy('bootCheck.checkingCore'))).toBeInTheDocument();
    });

    expect(screen.queryByText(copy('bootCheck.chooseCoreMode'))).not.toBeInTheDocument();
  });
});

describe('BootCheckGate — port conflict recovery', () => {
  beforeEach(() => {
    mockRecoverPortConflict.mockReset();
    mockRunBootCheck.mockReset();
  });

  it('shows "Fix Automatically" button when portConflict=true', async () => {
    mockRunBootCheck.mockResolvedValue({
      kind: 'unreachable',
      reason: 'port conflict',
      portConflict: true,
    });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('fix-automatically-btn')).toBeInTheDocument();
    });
    expect(screen.getByTestId('fix-automatically-btn').textContent).toBe('Fix Automatically');
  });

  it('does not show "Fix Automatically" button when portConflict is not set', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'some other error' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByText(copy('bootCheck.cannotReach'))).toBeInTheDocument();
    });
    expect(screen.queryByTestId('fix-automatically-btn')).not.toBeInTheDocument();
  });

  it('calls recoverPortConflict when "Fix Automatically" is clicked', async () => {
    mockRunBootCheck
      .mockResolvedValueOnce({ kind: 'unreachable', reason: 'port conflict', portConflict: true })
      .mockResolvedValue({ kind: 'match' });
    mockRecoverPortConflict.mockResolvedValue({ success: true, message: 'ok', new_port: 7789 });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('fix-automatically-btn')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('fix-automatically-btn'));

    await waitFor(() => {
      expect(mockRecoverPortConflict).toHaveBeenCalled();
    });
  });

  it('re-runs boot check after successful recovery', async () => {
    mockRunBootCheck
      .mockResolvedValueOnce({ kind: 'unreachable', reason: 'port conflict', portConflict: true })
      .mockResolvedValue({ kind: 'match' });
    mockRecoverPortConflict.mockResolvedValue({ success: true, message: 'ok', new_port: 7789 });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('fix-automatically-btn')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('fix-automatically-btn'));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledTimes(2);
  });

  it('shows portConflictFixFailed message when recovery fails', async () => {
    mockRunBootCheck.mockResolvedValue({
      kind: 'unreachable',
      reason: 'port conflict',
      portConflict: true,
    });
    mockRecoverPortConflict.mockResolvedValue({
      success: false,
      message: 'still busy',
      new_port: undefined,
    });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('fix-automatically-btn')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('fix-automatically-btn'));

    await waitFor(() => {
      expect(
        screen.getByText("Automatic fix didn't work. Please restart your computer and try again.")
      ).toBeInTheDocument();
    });
  });

  it('the switch-mode button still renders as secondary for port conflict', async () => {
    mockRunBootCheck.mockResolvedValue({
      kind: 'unreachable',
      reason: 'port conflict',
      portConflict: true,
    });

    renderGate();

    await waitFor(() => {
      expect(
        screen.getByRole('button', { name: copy('bootCheck.switchMode') })
      ).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — foreign port owner', () => {
  beforeEach(() => {
    mockRecoverPortConflict.mockReset();
    mockForceQuitPortOwner.mockReset();
    mockRunBootCheck.mockReset();
  });

  const foreignResult = {
    kind: 'unreachable' as const,
    reason: 'port conflict',
    portConflict: true,
    foreignOwner: { pid: 4242, name: 'Skype.exe' },
  };

  it('names the foreign owner and offers force-quit instead of Fix Automatically', async () => {
    mockRunBootCheck.mockResolvedValue(foreignResult);

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('force-quit-owner-btn')).toBeInTheDocument();
    });
    expect(screen.getByText(/Skype\.exe \(PID 4242\)/)).toBeInTheDocument();
    expect(screen.getByTestId('force-quit-owner-btn').textContent).toContain('Skype.exe');
    expect(screen.queryByTestId('fix-automatically-btn')).not.toBeInTheDocument();
  });

  it('force-quits the owner and re-runs the boot check on success', async () => {
    mockRunBootCheck.mockResolvedValueOnce(foreignResult).mockResolvedValue({ kind: 'match' });
    mockForceQuitPortOwner.mockResolvedValue({ success: true, message: 'ok', new_port: 7789 });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('force-quit-owner-btn')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId('force-quit-owner-btn'));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockForceQuitPortOwner).toHaveBeenCalledWith(4242);
    expect(mockRunBootCheck).toHaveBeenCalledTimes(2);
  });

  it('shows the force-quit failed message when termination fails', async () => {
    mockRunBootCheck.mockResolvedValue(foreignResult);
    mockForceQuitPortOwner.mockResolvedValue({ success: false, message: 'access denied' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('force-quit-owner-btn')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId('force-quit-owner-btn'));

    await waitFor(() => {
      expect(screen.getByText(/You may need to close it manually/)).toBeInTheDocument();
    });
  });

  it('upgrades to a force-quit button when Fix Automatically surfaces an owner', async () => {
    mockRunBootCheck.mockResolvedValue({
      kind: 'unreachable',
      reason: 'port conflict',
      portConflict: true,
    });
    mockRecoverPortConflict.mockResolvedValue({
      success: false,
      message: 'still busy',
      foreign_owner: { pid: 99, name: 'node.exe' },
    });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('fix-automatically-btn')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId('fix-automatically-btn'));

    await waitFor(() => {
      expect(screen.getByTestId('force-quit-owner-btn')).toBeInTheDocument();
    });
    expect(screen.getByText(/node\.exe \(PID 99\)/)).toBeInTheDocument();
  });

  it('renders cleanly when the owner name is unknown (empty)', async () => {
    mockRunBootCheck.mockResolvedValue({
      kind: 'unreachable',
      reason: 'port conflict',
      portConflict: true,
      foreignOwner: { pid: 4242, name: '' },
    });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('force-quit-owner-btn')).toBeInTheDocument();
    });
    // No leftover "{name}" / no duplicated PID; the pid still shows.
    expect(screen.getByText(/\(PID 4242\) is using/)).toBeInTheDocument();
    expect(screen.getByTestId('force-quit-owner-btn').textContent?.trim()).toBe('Force-quit');
  });
});

describe('BootCheckGate — picker (web build, !isTauri)', () => {
  beforeEach(() => {
    mockedIsTauri.mockReturnValue(false);
    vi.mocked(storeCoreMode).mockClear();
  });

  it('shows the picker as the FIRST screen and does not auto-adopt local', () => {
    renderGate();

    expect(screen.getByText(copy('bootCheck.connectToCore'))).toBeInTheDocument();
    expect(screen.queryByText(copy('bootCheck.checkingCore'))).not.toBeInTheDocument();
    expect(storeCoreMode).not.toHaveBeenCalled();
    expect(mockRunBootCheck).not.toHaveBeenCalled();
  });

  it('uses the web-friendly title and hides the Local option', () => {
    renderGate();

    expect(screen.getByText(copy('bootCheck.connectToCore'))).toBeInTheDocument();
    expect(screen.queryByText(copy('bootCheck.chooseCoreMode'))).not.toBeInTheDocument();
    expect(screen.queryByText(copy('bootCheck.localRecommended'))).not.toBeInTheDocument();
    // The selectable Cloud tile is also gone — cloud is implicit and the
    // URL/token form is rendered directly.
    expect(
      screen.queryByRole('button', { name: copy('bootCheck.cloudMode') })
    ).not.toBeInTheDocument();
  });

  it('renders the cloud form fields immediately (cloud is the only option)', () => {
    renderGate();

    expect(screen.getByPlaceholderText(/https:\/\/core\.example\.com/)).toBeInTheDocument();
    expect(
      screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder'))
    ).toBeInTheDocument();
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
    fireEvent.change(screen.getByPlaceholderText(copy('bootCheck.bearerTokenPlaceholder')), {
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
