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
const revealItemInDirMock = vi.fn();
const addBreadcrumbMock = vi.fn();
const platformMock = vi.fn();

vi.mock('./tauriCommands/common', () => ({ isTauri: () => isTauriMock() }));

vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: (url: string) => tauriOpenUrlMock(url),
  revealItemInDir: (path: string) => revealItemInDirMock(path),
}));

vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => platformMock() }));

vi.mock('@sentry/react', () => ({
  addBreadcrumb: (...args: unknown[]) => addBreadcrumbMock(...args),
}));

describe('openUrl', () => {
  let originalWindowOpen: typeof window.open;
  let windowOpenMock: Mock;

  beforeEach(() => {
    vi.clearAllMocks();
    // Default this device to macOS so existing POSIX-path reveal tests pass; the
    // #4278 cross-host tests override per-case.
    platformMock.mockResolvedValue('macos');
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

  it('revealPath dispatches to tauri-plugin-opener under Tauri (#2281 Reveal Folder fallback)', async () => {
    isTauriMock.mockReturnValue(true);
    revealItemInDirMock.mockResolvedValue(undefined);

    const { revealPath } = await import('./openUrl');
    await revealPath('/Users/me/Vault');

    expect(revealItemInDirMock).toHaveBeenCalledWith('/Users/me/Vault');
  });

  it('revealPath is a no-op outside Tauri (no shell to drive)', async () => {
    isTauriMock.mockReturnValue(false);

    const { revealPath } = await import('./openUrl');
    await revealPath('/Users/me/Vault');

    expect(revealItemInDirMock).not.toHaveBeenCalled();
  });

  it('revealPath propagates underlying tauri-plugin-opener errors to the caller', async () => {
    isTauriMock.mockReturnValue(true);
    revealItemInDirMock.mockRejectedValue(new Error('reveal failed'));

    const { revealPath } = await import('./openUrl');
    await expect(revealPath('/Users/me/Vault')).rejects.toThrow('reveal failed');
  });

  // #4278: a shared openhuman-core on a different OS serves its own absolute
  // path; revealing it locally must fail with a clear error, not cryptically.
  it('revealPath rejects a foreign-OS path instead of revealing it (#4278)', async () => {
    isTauriMock.mockReturnValue(true);
    platformMock.mockResolvedValue('windows'); // core served a POSIX path to a Windows frontend
    revealItemInDirMock.mockResolvedValue(undefined);

    const { revealPath } = await import('./openUrl');
    await expect(revealPath('/home/leigh/OHvault')).rejects.toThrow(/openhuman-core host/);
    expect(revealItemInDirMock).not.toHaveBeenCalled();
  });

  it('revealPath still reveals a path native to this device (#4278 regression)', async () => {
    isTauriMock.mockReturnValue(true);
    platformMock.mockResolvedValue('windows');
    revealItemInDirMock.mockResolvedValue(undefined);

    const { revealPath } = await import('./openUrl');
    await revealPath('C:\\Users\\me\\Vault');

    expect(revealItemInDirMock).toHaveBeenCalledWith('C:\\Users\\me\\Vault');
  });

  describe('isForeignFsPath', () => {
    it('flags a POSIX path on Windows and a Windows path on POSIX', async () => {
      const { isForeignFsPath } = await import('./openUrl');
      expect(isForeignFsPath('/home/leigh/OHvault', 'windows')).toBe(true);
      expect(isForeignFsPath('C:\\Users\\me\\Vault', 'macos')).toBe(true);
      expect(isForeignFsPath('\\\\server\\share', 'linux')).toBe(true);
    });

    it('treats same-family paths as local and is permissive on unknown OS', async () => {
      const { isForeignFsPath } = await import('./openUrl');
      expect(isForeignFsPath('/Users/me/Vault', 'macos')).toBe(false);
      expect(isForeignFsPath('C:\\x', 'windows')).toBe(false);
      expect(isForeignFsPath('/home/x', undefined)).toBe(false);
      expect(isForeignFsPath('', 'windows')).toBe(false);
    });
  });

  it('trims surrounding whitespace before classifying an http URL for fallback', async () => {
    isTauriMock.mockReturnValue(true);
    tauriOpenUrlMock.mockRejectedValue(
      new TypeError("Cannot read properties of undefined (reading 'postMessage')")
    );

    const { openUrl } = await import('./openUrl');
    await openUrl('  https://tinyhumans.ai/dashboard?token=secret-redact-me  ');

    expect(tauriOpenUrlMock).toHaveBeenCalledWith(
      'https://tinyhumans.ai/dashboard?token=secret-redact-me'
    );
    expect(windowOpenMock).toHaveBeenCalledWith(
      'https://tinyhumans.ai/dashboard?token=secret-redact-me',
      '_blank',
      'noopener,noreferrer'
    );
    expect(addBreadcrumbMock).toHaveBeenCalledWith(
      expect.objectContaining({ data: expect.objectContaining({ url: 'https://tinyhumans.ai' }) })
    );
  });
});
