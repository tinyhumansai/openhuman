/**
 * Unit tests for `openUrl`. The Tauri path is exercised in callers'
 * integration tests; here we focus on the browser fallback so the
 * non-Tauri branch (used by dev preview builds) doesn't regress.
 */
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

const isTauriMock = vi.fn();
const tauriOpenUrlMock = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => isTauriMock() }));

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: (url: string) => tauriOpenUrlMock(url) }));

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
    // Browser fallback must NOT fire under Tauri — it would spawn a
    // new webview window with no useful behaviour for custom schemes.
    expect(windowOpenMock).not.toHaveBeenCalled();
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
  });

  it('propagates Tauri opener errors to the caller (no silent fallback)', async () => {
    // Regression guard: the previous implementation swallowed the
    // error and called window.open, which spawned a useless webview
    // window for unhandled custom schemes (`obsidian://...`).
    isTauriMock.mockReturnValue(true);
    tauriOpenUrlMock.mockRejectedValue(new Error('scheme not allowed'));

    const { openUrl } = await import('./openUrl');
    await expect(openUrl('obsidian://open?path=/x')).rejects.toThrow('scheme not allowed');
    expect(windowOpenMock).not.toHaveBeenCalled();
  });
});
