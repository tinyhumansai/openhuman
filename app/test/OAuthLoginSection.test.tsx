/// <reference types="@testing-library/jest-dom/vitest" />
/**
 * Tests for Google OAuth login via OAuthLoginSection and OAuthProviderButton.
 *
 * Coverage areas:
 *  - Section renders all providers including Google
 *  - Google button initiates OAuth in both Tauri (desktop) and web environments
 *  - Loading / disabled state management during login
 *  - Error handling when the backend URL lookup fails
 *  - dev-mode URL construction (responseType=json query param)
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import OAuthLoginSection from '../src/components/oauth/OAuthLoginSection';
import OAuthProviderButton from '../src/components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../src/components/oauth/providerConfigs';
import { renderWithProviders } from '../src/test/test-utils';

// ---------------------------------------------------------------------------
// Module mocks
// vi.hoisted() ensures mock functions are available inside vi.mock() factories
// (which are hoisted to the top of the file by Vitest).
// ---------------------------------------------------------------------------

const { mockGetBackendUrl, mockOpenUrl, mockIsTauri } = vi.hoisted(() => ({
  mockGetBackendUrl: vi.fn(),
  mockOpenUrl: vi.fn(),
  mockIsTauri: vi.fn(),
}));

vi.mock('../src/services/backendUrl', () => ({ getBackendUrl: mockGetBackendUrl }));

vi.mock('../src/utils/openUrl', () => ({ openUrl: mockOpenUrl }));

vi.mock('../src/utils/tauriCommands', async importOriginal => {
  const actual = await importOriginal<Record<string, unknown>>();
  return { ...actual, isTauri: mockIsTauri };
});

// IS_DEV is set to `true` by the global setup mock of '../utils/config'

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const googleConfig = oauthProviderConfigs.find(p => p.id === 'google')!;

const renderSection = (props: Partial<ComponentProps<typeof OAuthLoginSection>> = {}) =>
  renderWithProviders(<OAuthLoginSection {...props} />);

const renderGoogleButton = (props: Partial<ComponentProps<typeof OAuthProviderButton>> = {}) =>
  renderWithProviders(<OAuthProviderButton provider={googleConfig} {...props} />);

// act() with an async callback returns Promise<void>, making await valid.
const clickButton = (btn: HTMLElement) =>
  act(async () => {
    fireEvent.click(btn);
  });

// ---------------------------------------------------------------------------
// OAuthLoginSection — rendering
// ---------------------------------------------------------------------------

describe('OAuthLoginSection', () => {
  it('renders the "Continue with" heading', () => {
    renderSection();
    expect(screen.getByText('Continue with')).toBeInTheDocument();
  });

  it('renders a button for every configured OAuth provider', () => {
    renderSection();
    for (const provider of oauthProviderConfigs) {
      expect(
        screen.getByRole('button', { name: new RegExp(provider.name, 'i') })
      ).toBeInTheDocument();
    }
  });

  it('renders a Google login button', () => {
    renderSection();
    expect(screen.getByRole('button', { name: /google/i })).toBeInTheDocument();
  });

  it('renders buttons in a 2-column grid', () => {
    const { container } = renderSection();
    const grid = container.querySelector('.grid.grid-cols-2');
    expect(grid).toBeInTheDocument();
    expect(grid!.children).toHaveLength(oauthProviderConfigs.length);
  });

  it('applies extra className to the wrapper div', () => {
    const { container } = renderSection({ className: 'mt-8' });
    expect(container.firstChild).toHaveClass('mt-8');
  });

  it('forwards disabled prop to every provider button', () => {
    renderSection({ disabled: true });
    for (const btn of screen.getAllByRole('button')) {
      expect(btn).toBeDisabled();
    }
  });
});

// ---------------------------------------------------------------------------
// Google button — initial render
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Google) — rendering', () => {
  beforeEach(() => {
    mockGetBackendUrl.mockResolvedValue('http://localhost:5005');
    mockIsTauri.mockReturnValue(false);
  });

  it('shows the Google label', () => {
    renderGoogleButton();
    expect(screen.getByText('Google')).toBeInTheDocument();
  });

  it('is enabled by default', () => {
    renderGoogleButton();
    expect(screen.getByRole('button', { name: /google/i })).toBeEnabled();
  });

  it('is disabled when disabled prop is true', () => {
    renderGoogleButton({ disabled: true });
    expect(screen.getByRole('button', { name: /google/i })).toBeDisabled();
  });

  it('renders the Google SVG icon', () => {
    const { container } = renderGoogleButton();
    expect(container.querySelector('svg')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Google button — web environment OAuth flow
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Google) — web OAuth flow', () => {
  const originalLocation = window.location;

  beforeEach(() => {
    mockGetBackendUrl.mockResolvedValue('http://localhost:5005');
    mockIsTauri.mockReturnValue(false);

    // Replace window.location so we can assert href changes
    delete (window as unknown as Record<string, unknown>).location;
    (window as unknown as Record<string, unknown>).location = { href: '' };
  });

  afterEach(() => {
    (window as unknown as Record<string, unknown>).location = originalLocation;
  });

  it('redirects to backend Google OAuth URL on click (web, IS_DEV=true)', async () => {
    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => {
      expect((window.location as unknown as { href: string }).href).toBe(
        'http://localhost:5005/auth/google/login?responseType=json'
      );
    });
  });

  it('does not call openUrl (Tauri) in web mode', async () => {
    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect((window.location as unknown as { href: string }).href).not.toBe(''));
    expect(mockOpenUrl).not.toHaveBeenCalled();
  });

  it('calls getBackendUrl exactly once per click', async () => {
    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect((window.location as unknown as { href: string }).href).not.toBe(''));
    expect(mockGetBackendUrl).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// Google button — Tauri (desktop) OAuth flow
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Google) — Tauri OAuth flow', () => {
  beforeEach(() => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);
  });

  it('calls openUrl with the Google OAuth URL (Tauri, IS_DEV=true)', async () => {
    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => {
      expect(mockOpenUrl).toHaveBeenCalledWith(
        'https://api.example.com/auth/google/login?responseType=json'
      );
    });
  });

  it('does not set window.location.href in Tauri mode', async () => {
    const originalHref = window.location.href;
    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalledTimes(1));
    expect(window.location.href).toBe(originalHref);
  });
});

// ---------------------------------------------------------------------------
// Google button — loading state
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Google) — loading state', () => {
  it('shows spinner and "Connecting..." text while login is in progress', async () => {
    let resolveBackendUrl!: (_v: string) => void;
    mockGetBackendUrl.mockReturnValue(
      new Promise<string>(res => {
        resolveBackendUrl = res;
      })
    );
    mockIsTauri.mockReturnValue(false);

    renderGoogleButton();
    const button = screen.getByRole('button', { name: /google/i });

    await clickButton(button);

    await waitFor(() => expect(screen.getByText('Connecting...')).toBeInTheDocument());
    expect(document.querySelector('.animate-spin')).toBeInTheDocument();
    expect(button).toBeDisabled();

    // Settle the promise so React doesn't warn about state updates after unmount
    await act(async () => {
      resolveBackendUrl('http://localhost:5005');
    });
  });

  it('does not respond to a second click while already loading', async () => {
    let resolveBackendUrl!: (_v: string) => void;
    mockGetBackendUrl.mockReturnValue(
      new Promise<string>(res => {
        resolveBackendUrl = res;
      })
    );
    mockIsTauri.mockReturnValue(false);

    renderGoogleButton();
    const button = screen.getByRole('button', { name: /google/i });

    await clickButton(button);
    await waitFor(() => expect(screen.getByText('Connecting...')).toBeInTheDocument());

    // Second click while loading — getBackendUrl must still be called only once
    fireEvent.click(button);
    expect(mockGetBackendUrl).toHaveBeenCalledTimes(1);

    await act(async () => {
      resolveBackendUrl('http://localhost:5005');
    });
  });

  it('remains in loading state after successful Tauri openUrl (awaits deep-link callback)', async () => {
    // By design: the app calls openUrl() to open the system browser and then waits
    // for the deep-link callback. setIsLoading(false) is only called on error, so
    // the button intentionally stays in "Connecting..." state.
    mockGetBackendUrl.mockResolvedValue('http://localhost:5005');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);

    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalledTimes(1));
    expect(screen.getByText('Connecting...')).toBeInTheDocument();
    expect(document.querySelector('.animate-spin')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Google button — error handling
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Google) — error handling', () => {
  beforeEach(() => {
    mockIsTauri.mockReturnValue(false);
  });

  it('returns to enabled state after getBackendUrl throws', async () => {
    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderGoogleButton();
    const button = screen.getByRole('button', { name: /google/i });
    await clickButton(button);

    await waitFor(() => expect(button).toBeEnabled());
    expect(screen.getByText('Google')).toBeInTheDocument();
  });

  it('does not redirect when getBackendUrl throws (web mode)', async () => {
    const originalLocation = window.location;
    delete (window as unknown as Record<string, unknown>).location;
    (window as unknown as Record<string, unknown>).location = { href: '' };

    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect(screen.getByRole('button', { name: /google/i })).toBeEnabled());
    expect((window.location as unknown as { href: string }).href).toBe('');

    (window as unknown as Record<string, unknown>).location = originalLocation;
  });

  it('does not call openUrl when getBackendUrl throws in Tauri mode', async () => {
    mockIsTauri.mockReturnValue(true);
    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect(screen.getByRole('button', { name: /google/i })).toBeEnabled());
    expect(mockOpenUrl).not.toHaveBeenCalled();
  });

  it('is a no-op when the button is disabled and clicked', async () => {
    renderGoogleButton({ disabled: true });
    await clickButton(screen.getByRole('button', { name: /google/i }));
    expect(mockGetBackendUrl).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// URL construction — dev mode query params (IS_DEV=true via global setup mock)
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Google) — dev mode URL params', () => {
  // The global setup.ts mocks IS_DEV=true, so these assertions run in that context.

  it('appends ?responseType=json to the Google OAuth URL in dev mode (Tauri)', async () => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);

    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    const calledUrl: string = mockOpenUrl.mock.calls[0][0];
    expect(calledUrl).toContain('?responseType=json');
    expect(calledUrl).toBe('https://api.example.com/auth/google/login?responseType=json');
  });

  it('appends ?responseType=json to the Google OAuth URL in dev mode (web)', async () => {
    const originalLocation = window.location;
    delete (window as unknown as Record<string, unknown>).location;
    (window as unknown as Record<string, unknown>).location = { href: '' };

    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(false);

    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect((window.location as unknown as { href: string }).href).not.toBe(''));
    expect((window.location as unknown as { href: string }).href).toBe(
      'https://api.example.com/auth/google/login?responseType=json'
    );

    (window as unknown as Record<string, unknown>).location = originalLocation;
  });

  it('uses the /auth/google/login path (not another provider)', async () => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);

    renderGoogleButton();
    await clickButton(screen.getByRole('button', { name: /google/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    const calledUrl: string = mockOpenUrl.mock.calls[0][0];
    expect(calledUrl).toContain('/auth/google/login');
  });
});
