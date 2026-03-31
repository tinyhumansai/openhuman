/// <reference types="@testing-library/jest-dom/vitest" />
/**
 * Tests for Twitter/X OAuth login via OAuthProviderButton.
 *
 * Coverage areas:
 *  - Twitter button rendering (label, icon, black/dark styling)
 *  - OAuth flow in both Tauri (desktop) and web environments
 *  - Loading / disabled state management
 *  - Error handling when backend URL lookup fails
 *  - dev-mode URL construction (?responseType=json)
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import OAuthProviderButton from '../src/components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../src/components/oauth/providerConfigs';
import { renderWithProviders } from '../src/test/test-utils';

// ---------------------------------------------------------------------------
// Module mocks
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const twitterConfig = oauthProviderConfigs.find(p => p.id === 'twitter')!;

const renderTwitterButton = (props: Partial<ComponentProps<typeof OAuthProviderButton>> = {}) =>
  renderWithProviders(<OAuthProviderButton provider={twitterConfig} {...props} />);

const clickButton = (btn: HTMLElement) =>
  act(async () => {
    fireEvent.click(btn);
  });

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Twitter) — rendering', () => {
  it('shows the Twitter label', () => {
    renderTwitterButton();
    expect(screen.getByText('Twitter')).toBeInTheDocument();
  });

  it('is enabled by default', () => {
    renderTwitterButton();
    expect(screen.getByRole('button', { name: /twitter/i })).toBeEnabled();
  });

  it('is disabled when disabled prop is true', () => {
    renderTwitterButton({ disabled: true });
    expect(screen.getByRole('button', { name: /twitter/i })).toBeDisabled();
  });

  it('renders the Twitter SVG icon', () => {
    const { container } = renderTwitterButton();
    expect(container.querySelector('svg')).toBeInTheDocument();
  });

  it('has black background styling', () => {
    renderTwitterButton();
    expect(screen.getByRole('button', { name: /twitter/i })).toHaveClass('bg-black');
  });

  it('has white text', () => {
    const { container } = renderTwitterButton();
    const label = container.querySelector('span');
    expect(label).toHaveClass('text-white');
  });
});

// ---------------------------------------------------------------------------
// Web OAuth flow
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Twitter) — web OAuth flow', () => {
  const originalLocation = window.location;

  beforeEach(() => {
    mockGetBackendUrl.mockResolvedValue('http://localhost:5005');
    mockIsTauri.mockReturnValue(false);
    delete (window as unknown as Record<string, unknown>).location;
    (window as unknown as Record<string, unknown>).location = { href: '' };
  });

  afterEach(() => {
    (window as unknown as Record<string, unknown>).location = originalLocation;
  });

  it('redirects to /auth/twitter/login?responseType=json on click', async () => {
    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => {
      expect((window.location as unknown as { href: string }).href).toBe(
        'http://localhost:5005/auth/twitter/login?responseType=json'
      );
    });
  });

  it('does not call openUrl in web mode', async () => {
    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect((window.location as unknown as { href: string }).href).not.toBe(''));
    expect(mockOpenUrl).not.toHaveBeenCalled();
  });

  it('calls getBackendUrl exactly once per click', async () => {
    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect((window.location as unknown as { href: string }).href).not.toBe(''));
    expect(mockGetBackendUrl).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// Tauri OAuth flow
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Twitter) — Tauri OAuth flow', () => {
  beforeEach(() => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);
  });

  it('calls openUrl with /auth/twitter/login?responseType=json', async () => {
    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => {
      expect(mockOpenUrl).toHaveBeenCalledWith(
        'https://api.example.com/auth/twitter/login?responseType=json'
      );
    });
  });

  it('does not set window.location.href in Tauri mode', async () => {
    const originalHref = window.location.href;
    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalledTimes(1));
    expect(window.location.href).toBe(originalHref);
  });

  it('remains in loading state after openUrl resolves (awaits deep-link callback)', async () => {
    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalledTimes(1));
    expect(screen.getByText('Connecting...')).toBeInTheDocument();
    expect(document.querySelector('.animate-spin')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Loading state
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Twitter) — loading state', () => {
  it('shows spinner and "Connecting..." while getBackendUrl is pending', async () => {
    let resolve!: (_v: string) => void;
    mockGetBackendUrl.mockReturnValue(
      new Promise<string>(res => {
        resolve = res;
      })
    );
    mockIsTauri.mockReturnValue(false);

    renderTwitterButton();
    const button = screen.getByRole('button', { name: /twitter/i });
    await clickButton(button);

    await waitFor(() => expect(screen.getByText('Connecting...')).toBeInTheDocument());
    expect(document.querySelector('.animate-spin')).toBeInTheDocument();
    expect(button).toBeDisabled();

    await act(async () => {
      resolve('http://localhost:5005');
    });
  });

  it('ignores a second click while already loading', async () => {
    let resolve!: (_v: string) => void;
    mockGetBackendUrl.mockReturnValue(
      new Promise<string>(res => {
        resolve = res;
      })
    );
    mockIsTauri.mockReturnValue(false);

    renderTwitterButton();
    const button = screen.getByRole('button', { name: /twitter/i });

    await clickButton(button);
    await waitFor(() => expect(screen.getByText('Connecting...')).toBeInTheDocument());

    fireEvent.click(button);
    expect(mockGetBackendUrl).toHaveBeenCalledTimes(1);

    await act(async () => {
      resolve('http://localhost:5005');
    });
  });
});

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Twitter) — error handling', () => {
  beforeEach(() => {
    mockIsTauri.mockReturnValue(false);
  });

  it('returns to enabled state after getBackendUrl throws', async () => {
    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderTwitterButton();
    const button = screen.getByRole('button', { name: /twitter/i });
    await clickButton(button);

    await waitFor(() => expect(button).toBeEnabled());
    expect(screen.getByText('Twitter')).toBeInTheDocument();
  });

  it('does not redirect on getBackendUrl error (web mode)', async () => {
    const originalLocation = window.location;
    delete (window as unknown as Record<string, unknown>).location;
    (window as unknown as Record<string, unknown>).location = { href: '' };

    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect(screen.getByRole('button', { name: /twitter/i })).toBeEnabled());
    expect((window.location as unknown as { href: string }).href).toBe('');

    (window as unknown as Record<string, unknown>).location = originalLocation;
  });

  it('does not call openUrl on getBackendUrl error (Tauri mode)', async () => {
    mockIsTauri.mockReturnValue(true);
    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect(screen.getByRole('button', { name: /twitter/i })).toBeEnabled());
    expect(mockOpenUrl).not.toHaveBeenCalled();
  });

  it('is a no-op when disabled and clicked', async () => {
    renderTwitterButton({ disabled: true });
    await clickButton(screen.getByRole('button', { name: /twitter/i }));
    expect(mockGetBackendUrl).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// URL construction
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Twitter) — URL construction', () => {
  it('uses /auth/twitter/login path (not another provider)', async () => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);

    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    expect(mockOpenUrl.mock.calls[0][0]).toContain('/auth/twitter/login');
  });

  it('appends ?responseType=json in dev mode (Tauri)', async () => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);

    renderTwitterButton();
    await clickButton(screen.getByRole('button', { name: /twitter/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    expect(mockOpenUrl.mock.calls[0][0]).toBe(
      'https://api.example.com/auth/twitter/login?responseType=json'
    );
  });
});
