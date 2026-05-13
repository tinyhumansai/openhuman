/**
 * Unit tests for `openUrl`. The Tauri path is exercised in callers'
 * integration tests; here we focus on the browser fallback and the
 * CEF-IPC-not-ready recovery so the non-Tauri branch (used by dev
 * preview builds) and the CEF gap window (#1472 / REACT-T/S/R) do
 * not regress.
 */
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

const isTauriMock = vi.fn();
const tauriOpenUrlMock = vi.fn();
const addBreadcrumbMock = vi.fn();

vi.mock('./tauriCommands/common', () => ({ isTauri: () => isTauriMock() }));

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: (url: string) => tauriOpenUrlMock(url) }));

vi.mock('@sentry/react', () => ({
  addBreadcrumb: (...args: unknown[]) => addBreadcrumbMock(...args),
}));

describe('openUrl', () => {
  let originalWindowOpen: typeof window.open;
  let windowOpenMock: Mock;

  beforeEach(() => {
    vi.clearAllMocks();
    originalWindowOpen = window.open;
    windowOpenMock = vi.fn();
    window.open = windowOpenMock as unknown as typeof window.open;
  });

  afterEach(() => {
    window.open = originalWindowOpen;
  });

  it('routes through tauri-plugin-opener when running inside Tauri', async () => {
    isTauriMock.mockReturnValue(true);
    tauriOpenUrlMock.mockResolvedValue(undefined);

    const { openUrl } = await import('./openUrl');
    await openUrl('https://example.com/page');

    expect(tauriOpenUrlMock).toHaveBeenCalledWith('https://example.com/page');
    // Browser fallback must NOT fire when the Tauri call succeeded.
    expect(windowOpenMock).not.toHaveBeenCalled();
    expect(addBreadcrumbMock).not.toHaveBeenCalled();
  });

  it('falls back to window.open in a browser context (non-Tauri)', async () => {
    isTauriMock.mockReturnValue(false);

    const { openUrl } = await import('./openUrl');
    await openUrl('https://docs.example.com/');

    expect(windowOpenMock).toHaveBeenCalledWith(
      'https://docs.example.com/',
      '_blank',
      'noopener,noreferrer'
    );
    expect(tauriOpenUrlMock).not.toHaveBeenCalled();
    expect(addBreadcrumbMock).not.toHaveBeenCalled();
  });

  it('propagates Tauri opener errors for non-http schemes (no silent fallback)', async () => {
    // Regression guard: `window.open` cannot launch custom-scheme
    // URLs (`obsidian://`, `mailto:`, …) — it spawns a useless Tauri
    // webview window. For those we MUST propagate the error to the
    // caller, even when the failure is the CEF IPC race.
    isTauriMock.mockReturnValue(true);
    tauriOpenUrlMock.mockRejectedValue(new Error('scheme not allowed'));

    const { openUrl } = await import('./openUrl');
    await expect(openUrl('obsidian://open?path=/Users/me/Vault')).rejects.toThrow(
      'scheme not allowed'
    );
    expect(windowOpenMock).not.toHaveBeenCalled();
    // Non-http schemes log only the protocol — the rest of the URL (here the
    // vault path) is the payload itself and must not leak to Sentry.
    expect(addBreadcrumbMock).toHaveBeenCalledWith(
      expect.objectContaining({
        category: 'ipc',
        level: 'warning',
        message: 'tauriOpenUrl failed; evaluating fallback',
        data: expect.objectContaining({ url: 'obsidian:' }),
      })
    );
    const call = addBreadcrumbMock.mock.calls[0]?.[0] as { data?: { url?: string } } | undefined;
    expect(call?.data?.url).not.toContain('Vault');
    expect(call?.data?.url).not.toContain('/Users/me');
  });

  it('falls back to window.open when tauriOpenUrl rejects on an http URL (CEF IPC race recovery, #1472)', async () => {
    // Concrete repro for OPENHUMAN-REACT-T/S/R: CEF embedder
    // injects `window.ipc.postMessage` after `on_after_created`. A
    // click landing in that gap causes `tauriOpenUrl` to reject with
    // a TypeError. For http(s) URLs the safe recovery is to hand off
    // to `window.open` so the Billing dashboard still opens.
    isTauriMock.mockReturnValue(true);
    const ipcError = new TypeError("Cannot read properties of undefined (reading 'postMessage')");
    tauriOpenUrlMock.mockRejectedValue(ipcError);

    const { openUrl } = await import('./openUrl');
    await openUrl('https://tinyhumans.ai/dashboard?token=secret-redact-me');

    expect(windowOpenMock).toHaveBeenCalledWith(
      'https://tinyhumans.ai/dashboard?token=secret-redact-me',
      '_blank',
      'noopener,noreferrer'
    );
    // Breadcrumb keeps only origin for http(s) — pathname + query (which may
    // carry tokens / emails / vault paths) must not be sent to Sentry.
    expect(addBreadcrumbMock).toHaveBeenCalledWith(
      expect.objectContaining({
        category: 'ipc',
        level: 'warning',
        message: 'tauriOpenUrl failed; evaluating fallback',
        data: expect.objectContaining({ url: 'https://tinyhumans.ai' }),
      })
    );
    const call = addBreadcrumbMock.mock.calls[0]?.[0] as { data?: { url?: string } } | undefined;
    expect(call?.data?.url).not.toContain('secret-redact-me');
    expect(call?.data?.url).not.toContain('/dashboard');
  });
});
