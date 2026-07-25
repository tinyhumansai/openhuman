/**
 * Component tests for BootCheckGate.
 *
 * Strategy:
 *   - Mock runBootCheck so we control the result without real RPC/invoke.
 *   - Use a minimal Redux store that starts with coreMode.mode = 'unset'
 *     (picker) or set (check flow).
 *   - Assert rendered text and dispatched actions for each meaningful state.
 *
 * Desktop (isTauri=true) never shows a picker: the gate auto-selects local
 * mode and, on failure, auto-runs the same remediation a button used to
 * require before collapsing to one generic error + Retry screen. Web
 * (isTauri=false) keeps the original picker + per-kind result screens,
 * since a real URL/token choice still exists there.
 */
import { configureStore } from '@reduxjs/toolkit';
import { isTauri } from '@tauri-apps/api/core';
import { render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import coreModeReducer, { type CoreModeState } from '../../../store/coreModeSlice';
import localeReducer from '../../../store/localeSlice';
import BootCheckGate from '../BootCheckGate';

const mockedIsTauri = vi.mocked(isTauri);

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
// Desktop tests (isTauri=true) — no picker, ever.
// ---------------------------------------------------------------------------

describe('BootCheckGate — desktop (isTauri=true)', () => {
  beforeEach(() => {
    mockedIsTauri.mockReturnValue(true);
    mockRunBootCheck.mockReset();
    mockRecoverPortConflict.mockReset();
    mockForceQuitPortOwner.mockReset();
  });

  it('never renders a picker, even when coreMode starts unset', async () => {
    mockRunBootCheck.mockImplementation(() => new Promise(() => {}));

    renderGate();

    expect(screen.queryByText('Select a Runtime')).not.toBeInTheDocument();
    expect(screen.queryByText('Connect to Your Runtime')).not.toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByText('Waking up your runtime…')).toBeInTheDocument();
    });
  });

  it('auto-commits to local mode and runs the boot check without any click', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'local' }),
      expect.any(Object)
    );
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

  it('renders children once the boot check matches', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate(makeStore({ kind: 'local' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
  });

  it('auto-remediates a detected legacy daemon without a button click, then re-runs the check', async () => {
    mockRunBootCheck
      .mockResolvedValueOnce({ kind: 'daemonDetected' })
      .mockResolvedValue({ kind: 'match' });

    renderGate(makeStore({ kind: 'local' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledTimes(2);
    // No daemon-detection screen or button was ever shown to the user.
    expect(screen.queryByText('Legacy Background Runtime Detected')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Remove and Continue' })).not.toBeInTheDocument();
  });

  it('auto-restarts on outdatedLocal without a button click', async () => {
    mockRunBootCheck
      .mockResolvedValueOnce({ kind: 'outdatedLocal' })
      .mockResolvedValue({ kind: 'match' });

    renderGate(makeStore({ kind: 'local' }));

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck).toHaveBeenCalledTimes(2);
    expect(screen.queryByText('Local Runtime Needs a Restart')).not.toBeInTheDocument();
  });

  it('auto-recovers a port conflict silently and re-runs the check', async () => {
    mockRunBootCheck
      .mockResolvedValueOnce({ kind: 'unreachable', reason: 'port conflict', portConflict: true })
      .mockResolvedValue({ kind: 'match' });
    mockRecoverPortConflict.mockResolvedValue({ success: true, message: 'ok', new_port: 7789 });

    renderGate(makeStore({ kind: 'local' }));

    await waitFor(() => {
      expect(mockRecoverPortConflict).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(screen.queryByTestId('fix-automatically-btn')).not.toBeInTheDocument();
  });

  it('gives up after the auto-recovery attempt budget and shows one generic error with only a Retry button', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'still down' });

    renderGate(makeStore({ kind: 'local' }));

    await waitFor(() => {
      expect(screen.getByTestId('generic-retry-btn')).toBeInTheDocument();
    });

    // Exactly one screen, one button — no Quit, no Switch Mode, no
    // kind-specific action buttons.
    expect(screen.getByText("Can't Reach the Runtime")).toBeInTheDocument();
    expect(screen.getByText('still down')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Quit' })).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Pick a Different Runtime' })
    ).not.toBeInTheDocument();
    expect(screen.getAllByRole('button')).toHaveLength(1);
  });

  it('never auto-terminates a foreign process — that always waits for the user to press Retry', async () => {
    mockRunBootCheck.mockResolvedValue({
      kind: 'unreachable',
      reason: 'port conflict',
      portConflict: true,
      foreignOwner: { pid: 4242, name: 'Skype.exe' },
    });

    renderGate(makeStore({ kind: 'local' }));

    await waitFor(() => {
      expect(screen.getByTestId('generic-retry-btn')).toBeInTheDocument();
    });
    expect(mockForceQuitPortOwner).not.toHaveBeenCalled();
    expect(screen.getByText(/Skype\.exe \(PID 4242\)/)).toBeInTheDocument();
    expect(screen.getAllByRole('button')).toHaveLength(1);
  });

  it('Retry resets the auto-recovery budget for a fresh cycle', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'still down' });

    renderGate(makeStore({ kind: 'local' }));

    await waitFor(() => {
      expect(screen.getByTestId('generic-retry-btn')).toBeInTheDocument();
    });
    const callsBeforeRetry = mockRunBootCheck.mock.calls.length;

    mockRunBootCheck.mockResolvedValue({ kind: 'match' });
    screen.getByTestId('generic-retry-btn').click();

    await waitFor(() => {
      expect(screen.getByTestId('app-content')).toBeInTheDocument();
    });
    expect(mockRunBootCheck.mock.calls.length).toBeGreaterThan(callsBeforeRetry);
  });
});

// ---------------------------------------------------------------------------
// Web build tests (isTauri=false) — picker + detailed result screens remain,
// since a real remote-URL/token choice genuinely exists there.
// ---------------------------------------------------------------------------

describe('BootCheckGate — web (isTauri=false)', () => {
  beforeEach(() => {
    mockedIsTauri.mockReturnValue(false);
    mockRunBootCheck.mockReset();
  });

  it('shows the cloud-only picker (no local option, no desktop copy)', () => {
    renderGate();

    expect(screen.getByText('Connect to Your Runtime')).toBeInTheDocument();
    expect(screen.queryByText('Select a Runtime')).not.toBeInTheDocument();
    expect(screen.queryByText('Run Locally (Recommended)')).not.toBeInTheDocument();
  });

  it('renders the cloud form fields immediately (cloud is the only option)', () => {
    renderGate();

    expect(screen.getByPlaceholderText(/https:\/\/core\.example\.com/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Bearer token/i)).toBeInTheDocument();
  });

  it('continues into a cloud boot check when URL + token are provided', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'match' });

    renderGate();

    (await screen.findByPlaceholderText(/https:\/\/core\.example\.com/)).dispatchEvent(
      new Event('input', { bubbles: true })
    );
    const urlInput = screen.getByPlaceholderText(
      /https:\/\/core\.example\.com/
    ) as HTMLInputElement;
    const tokenInput = screen.getByPlaceholderText(/Bearer token/i) as HTMLInputElement;
    const setValue = (el: HTMLInputElement, value: string) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
      setter?.call(el, value);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    };
    setValue(urlInput, 'https://core.example.com/rpc');
    setValue(tokenInput, 'tok-web');
    screen.getByRole('button', { name: 'Continue' }).click();

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

  it('shows the detailed unreachable screen with Quit and Switch-mode buttons', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate(makeStore({ kind: 'cloud', url: 'https://core.example.com/rpc', token: 'tok' }));

    await waitFor(() => {
      expect(screen.getByText("Can't Reach the Runtime")).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Quit' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Pick a Different Runtime' })).toBeInTheDocument();
    });
  });

  it("returns to picker when 'Pick a Different Runtime' is clicked", async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate(makeStore({ kind: 'cloud', url: 'https://core.example.com/rpc', token: 'tok' }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Pick a Different Runtime' })).toBeInTheDocument();
    });
    screen.getByRole('button', { name: 'Pick a Different Runtime' }).click();

    await waitFor(() => {
      expect(screen.getByText('Connect to Your Runtime')).toBeInTheDocument();
    });
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
  });
});
