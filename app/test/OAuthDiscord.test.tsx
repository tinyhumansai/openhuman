/// <reference types="@testing-library/jest-dom/vitest" />
/**
 * Tests for Discord OAuth login via OAuthProviderButton.
 *
 * Coverage areas:
 *  - Discord button rendering (label, icon, indigo styling)
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

const discordConfig = oauthProviderConfigs.find(p => p.id === 'discord')!;

const renderDiscordButton = (props: Partial<ComponentProps<typeof OAuthProviderButton>> = {}) =>
  renderWithProviders(<OAuthProviderButton provider={discordConfig} {...props} />);

const clickButton = (btn: HTMLElement) =>
  act(async () => {
    fireEvent.click(btn);
  });

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Discord) — rendering', () => {
  it('shows the Discord label', () => {
    renderDiscordButton();
    expect(screen.getByText('Discord')).toBeInTheDocument();
  });

  it('is enabled by default', () => {
    renderDiscordButton();
    expect(screen.getByRole('button', { name: /discord/i })).toBeEnabled();
  });

  it('is disabled when disabled prop is true', () => {
    renderDiscordButton({ disabled: true });
    expect(screen.getByRole('button', { name: /discord/i })).toBeDisabled();
  });

  it('renders the Discord SVG icon', () => {
    const { container } = renderDiscordButton();
    expect(container.querySelector('svg')).toBeInTheDocument();
  });

  it('has indigo background styling', () => {
    renderDiscordButton();
    expect(screen.getByRole('button', { name: /discord/i })).toHaveClass('bg-indigo-600');
  });

  it('has white text', () => {
    const { container } = renderDiscordButton();
    const label = container.querySelector('span');
    expect(label).toHaveClass('text-white');
  });
});

// ---------------------------------------------------------------------------
// Web OAuth flow
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Discord) — web OAuth flow', () => {
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

  it('redirects to /auth/discord/login?responseType=json on click', async () => {
    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => {
      expect((window.location as unknown as { href: string }).href).toBe(
        'http://localhost:5005/auth/discord/login?responseType=json'
      );
    });
  });

  it('does not call openUrl in web mode', async () => {
    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect((window.location as unknown as { href: string }).href).not.toBe(''));
    expect(mockOpenUrl).not.toHaveBeenCalled();
  });

  it('calls getBackendUrl exactly once per click', async () => {
    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect((window.location as unknown as { href: string }).href).not.toBe(''));
    expect(mockGetBackendUrl).toHaveBeenCalledTimes(1);
  });
});

// ---------------------------------------------------------------------------
// Tauri OAuth flow
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Discord) — Tauri OAuth flow', () => {
  beforeEach(() => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);
  });

  it('calls openUrl with /auth/discord/login?responseType=json', async () => {
    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => {
      expect(mockOpenUrl).toHaveBeenCalledWith(
        'https://api.example.com/auth/discord/login?responseType=json'
      );
    });
  });

  it('does not set window.location.href in Tauri mode', async () => {
    const originalHref = window.location.href;
    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalledTimes(1));
    expect(window.location.href).toBe(originalHref);
  });

  it('remains in loading state after openUrl resolves (awaits deep-link callback)', async () => {
    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalledTimes(1));
    expect(screen.getByText('Connecting...')).toBeInTheDocument();
    expect(document.querySelector('.animate-spin')).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------------------
// Loading state
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Discord) — loading state', () => {
  it('shows spinner and "Connecting..." while getBackendUrl is pending', async () => {
    let resolve!: (_v: string) => void;
    mockGetBackendUrl.mockReturnValue(
      new Promise<string>(res => {
        resolve = res;
      })
    );
    mockIsTauri.mockReturnValue(false);

    renderDiscordButton();
    const button = screen.getByRole('button', { name: /discord/i });
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

    renderDiscordButton();
    const button = screen.getByRole('button', { name: /discord/i });

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

describe('OAuthProviderButton (Discord) — error handling', () => {
  beforeEach(() => {
    mockIsTauri.mockReturnValue(false);
  });

  it('returns to enabled state after getBackendUrl throws', async () => {
    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderDiscordButton();
    const button = screen.getByRole('button', { name: /discord/i });
    await clickButton(button);

    await waitFor(() => expect(button).toBeEnabled());
    expect(screen.getByText('Discord')).toBeInTheDocument();
  });

  it('does not redirect on getBackendUrl error (web mode)', async () => {
    const originalLocation = window.location;
    delete (window as unknown as Record<string, unknown>).location;
    (window as unknown as Record<string, unknown>).location = { href: '' };

    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect(screen.getByRole('button', { name: /discord/i })).toBeEnabled());
    expect((window.location as unknown as { href: string }).href).toBe('');

    (window as unknown as Record<string, unknown>).location = originalLocation;
  });

  it('does not call openUrl on getBackendUrl error (Tauri mode)', async () => {
    mockIsTauri.mockReturnValue(true);
    mockGetBackendUrl.mockRejectedValue(new Error('network error'));

    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect(screen.getByRole('button', { name: /discord/i })).toBeEnabled());
    expect(mockOpenUrl).not.toHaveBeenCalled();
  });

  it('is a no-op when disabled and clicked', async () => {
    renderDiscordButton({ disabled: true });
    await clickButton(screen.getByRole('button', { name: /discord/i }));
    expect(mockGetBackendUrl).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// URL construction
// ---------------------------------------------------------------------------

describe('OAuthProviderButton (Discord) — URL construction', () => {
  it('uses /auth/discord/login path (not another provider)', async () => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);

    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    expect(mockOpenUrl.mock.calls[0][0]).toContain('/auth/discord/login');
  });

  it('appends ?responseType=json in dev mode (Tauri)', async () => {
    mockGetBackendUrl.mockResolvedValue('https://api.example.com');
    mockIsTauri.mockReturnValue(true);
    mockOpenUrl.mockResolvedValue(undefined);

    renderDiscordButton();
    await clickButton(screen.getByRole('button', { name: /discord/i }));

    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalled());
    expect(mockOpenUrl.mock.calls[0][0]).toBe(
      'https://api.example.com/auth/discord/login?responseType=json'
    );
  });
});
