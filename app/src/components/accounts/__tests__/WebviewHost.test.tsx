import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../../store';
import { addAccount, resetAccountsState, setAccountStatus } from '../../../store/accountsSlice';
import WebviewHost from '../WebviewHost';

// The host component reaches into the webviewAccountService for openWebview /
// hideWebview / setBounds helpers. Stub them so we don't drag the Tauri IPC
// graph (and its Meet/core-RPC siblings) into a unit test.
vi.mock('../../../services/webviewAccountService', () => ({
  hideWebviewAccount: vi.fn().mockResolvedValue(undefined),
  openWebviewAccount: vi.fn().mockResolvedValue(undefined),
  retryWebviewAccountLoad: vi.fn().mockResolvedValue(undefined),
  setWebviewAccountBounds: vi.fn().mockResolvedValue(undefined),
}));

const ACCOUNT_ID = 'acct-host-1';

function renderHost(): void {
  render(
    <Provider store={store}>
      <WebviewHost accountId={ACCOUNT_ID} provider="slack" />
    </Provider>
  );
}

function seedAccount(status: 'pending' | 'loading' | 'open' | 'timeout' | 'closed'): void {
  store.dispatch(resetAccountsState());
  store.dispatch(
    addAccount({
      id: ACCOUNT_ID,
      provider: 'slack',
      label: 'Slack',
      createdAt: new Date().toISOString(),
      status,
    })
  );
}

describe('WebviewHost — issue #1233 loading UX', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    store.dispatch(resetAccountsState());
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows the branded placeholder immediately on mount even when status is unknown', () => {
    // No account in the store at all — host must still render the
    // placeholder so the area is never visually blank.
    renderHost();
    expect(screen.getByTestId(`webview-placeholder-${ACCOUNT_ID}`)).toBeInTheDocument();
  });

  it('shows the loading spinner from frame 1 when status is unknown', () => {
    renderHost();
    expect(screen.getByTestId(`webview-loading-${ACCOUNT_ID}`)).toBeInTheDocument();
  });

  it('shows the loading spinner while account status is `pending`', () => {
    seedAccount('pending');
    renderHost();
    expect(screen.getByTestId(`webview-loading-${ACCOUNT_ID}`)).toBeInTheDocument();
  });

  it('hides the spinner once status flips to `open`', () => {
    seedAccount('open');
    renderHost();
    expect(screen.queryByTestId(`webview-loading-${ACCOUNT_ID}`)).not.toBeInTheDocument();
    // Placeholder remains so layout area is never blank during the
    // brief frame between native reveal and CEF first paint.
    expect(screen.getByTestId(`webview-placeholder-${ACCOUNT_ID}`)).toBeInTheDocument();
  });

  it('shows the timeout overlay when status is `timeout`', () => {
    seedAccount('timeout');
    renderHost();
    expect(screen.getByTestId(`webview-timeout-${ACCOUNT_ID}`)).toBeInTheDocument();
    expect(screen.queryByTestId(`webview-loading-${ACCOUNT_ID}`)).not.toBeInTheDocument();
  });

  it('renders the phase hint after 5s of loading and escalates after 10s', () => {
    seedAccount('loading');
    renderHost();

    // Frame 1: no hint yet.
    expect(screen.queryByTestId(`webview-loading-hint-${ACCOUNT_ID}`)).not.toBeInTheDocument();

    // Past the 5s threshold the hint appears.
    act(() => {
      vi.advanceTimersByTime(5_500);
    });
    expect(screen.getByTestId(`webview-loading-hint-${ACCOUNT_ID}`)).toHaveTextContent(
      /restoring session/i
    );

    // Past the 10s threshold the hint upgrades to the late copy.
    act(() => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.getByTestId(`webview-loading-hint-${ACCOUNT_ID}`)).toHaveTextContent(
      /almost ready/i
    );
  });

  it('clears the elapsed timer when the account flips to open', () => {
    seedAccount('loading');
    renderHost();

    act(() => {
      vi.advanceTimersByTime(7_000);
    });
    expect(screen.getByTestId(`webview-loading-hint-${ACCOUNT_ID}`)).toBeInTheDocument();

    // Warm-reopen path flips the account to `open`. The placeholder stays,
    // but the loading overlay (and its hint) must be gone.
    act(() => {
      store.dispatch(setAccountStatus({ accountId: ACCOUNT_ID, status: 'open' }));
    });
    expect(screen.queryByTestId(`webview-loading-${ACCOUNT_ID}`)).not.toBeInTheDocument();
    expect(screen.queryByTestId(`webview-loading-hint-${ACCOUNT_ID}`)).not.toBeInTheDocument();
  });
});

describe('WebviewHost — provider name substitution (#3759)', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    store.dispatch(resetAccountsState());
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('substitutes provider name in loading message without literal placeholder', () => {
    seedAccount('loading');
    renderHost();
    const placeholder = screen.getByTestId(`webview-placeholder-${ACCOUNT_ID}`);
    // Should show "Loading Slack..." not "Loading {providerName}... Slack..."
    expect(placeholder).toHaveTextContent('Loading Slack...');
    expect(placeholder).not.toHaveTextContent('{providerName}');
  });

  it('substitutes provider name in timeout message without literal placeholder', () => {
    seedAccount('timeout');
    renderHost();
    const timeout = screen.getByTestId(`webview-timeout-${ACCOUNT_ID}`);
    // Should show "Slack is taking longer..." not "Slack {providerName} is taking longer..."
    expect(timeout).toHaveTextContent('Slack is taking longer than expected');
    expect(timeout).not.toHaveTextContent('{providerName}');
  });

  it.each([
    { provider: 'discord' as const, expectedName: 'Discord' },
    { provider: 'gmail' as const, expectedName: 'Gmail' },
    { provider: 'telegram' as const, expectedName: 'Telegram' },
  ])('renders $expectedName correctly in loading state', ({ provider, expectedName }) => {
    const accountId = `acct-${provider}`;
    store.dispatch(
      addAccount({
        id: accountId,
        provider,
        label: expectedName,
        createdAt: new Date().toISOString(),
        status: 'loading',
      })
    );

    render(
      <Provider store={store}>
        <WebviewHost accountId={accountId} provider={provider} />
      </Provider>
    );

    const placeholder = screen.getByTestId(`webview-placeholder-${accountId}`);
    expect(placeholder).toHaveTextContent(`Loading ${expectedName}...`);
    expect(placeholder).not.toHaveTextContent('{providerName}');
  });
});
