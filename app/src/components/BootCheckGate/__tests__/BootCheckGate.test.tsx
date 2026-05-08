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
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import coreModeReducer, { type CoreModeState } from '../../../store/coreModeSlice';
import BootCheckGate from '../BootCheckGate';

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

const mockRunBootCheck = vi.fn();
vi.mock('../../../lib/bootCheck', () => ({
  runBootCheck: (...args: unknown[]) => mockRunBootCheck(...args),
}));

vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: vi.fn(),
  clearCoreRpcUrlCache: vi.fn(),
  clearCoreRpcTokenCache: vi.fn(),
}));

vi.mock('../../../utils/configPersistence', () => ({
  storeRpcUrl: vi.fn(),
  storeCoreToken: vi.fn(),
  clearStoredCoreToken: vi.fn(),
  storeCoreMode: vi.fn(),
  clearStoredCoreMode: vi.fn(),
  isValidRpcUrl: vi.fn().mockReturnValue(true),
}));

// ---------------------------------------------------------------------------
// Store factory
// ---------------------------------------------------------------------------

function makeStore(initialMode?: CoreModeState['mode']) {
  return configureStore({
    reducer: { coreMode: coreModeReducer },
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

describe('BootCheckGate — picker (unset mode)', () => {
  it('shows the mode picker when coreMode is unset', () => {
    renderGate();
    expect(screen.getByText('Choose core mode')).toBeInTheDocument();
    expect(screen.getByText('Local (recommended)')).toBeInTheDocument();
    expect(screen.getByText('Cloud')).toBeInTheDocument();
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

    fireEvent.click(screen.getByText('Cloud'));

    expect(screen.getByPlaceholderText(/https:\/\/core\.example\.com/)).toBeInTheDocument();
  });

  it('shows URL validation error when cloud URL is empty', () => {
    renderGate();

    fireEvent.click(screen.getByText('Cloud'));
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText('Please enter a core URL.')).toBeInTheDocument();
  });

  it('shows URL validation error for non-http URL', () => {
    renderGate();

    fireEvent.click(screen.getByText('Cloud'));
    const input = screen.getByPlaceholderText(/https:\/\/core\.example\.com/);
    fireEvent.change(input, { target: { value: 'ftp://invalid' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(screen.getByText(/must start with http/)).toBeInTheDocument();
  });
});

describe('BootCheckGate — checking state', () => {
  it('shows checking spinner while boot check is in flight', async () => {
    // Never resolves during this test
    mockRunBootCheck.mockImplementation(() => new Promise(() => {}));

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Checking core…')).toBeInTheDocument();
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
      expect(screen.getByText('Legacy background core detected')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Remove and continue' })).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — outdatedLocal', () => {
  it('shows outdated local screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'outdatedLocal' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Local core needs a restart')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Restart core' })).toBeInTheDocument();
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
      expect(screen.getByText('Cloud core needs an update')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Update cloud core' })).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — noVersionMethod', () => {
  it('shows no version method screen', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'noVersionMethod' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Core version check failed')).toBeInTheDocument();
    });
  });
});

describe('BootCheckGate — unreachable', () => {
  it('shows unreachable screen with quit and switch mode buttons', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByText('Could not reach core')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Quit' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Switch mode' })).toBeInTheDocument();
    });
  });

  it('returns to picker when Switch mode is clicked', async () => {
    mockRunBootCheck.mockResolvedValue({ kind: 'unreachable', reason: 'Connection refused' });

    renderGate();
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Switch mode' })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Switch mode' }));

    await waitFor(() => {
      expect(screen.getByText('Choose core mode')).toBeInTheDocument();
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
      expect(screen.getByText('Checking core…')).toBeInTheDocument();
    });

    expect(screen.queryByText('Choose core mode')).not.toBeInTheDocument();
  });
});
