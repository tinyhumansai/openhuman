import { isTauri } from '@tauri-apps/api/core';
import { openUrl as tauriOpenUrl } from '@tauri-apps/plugin-opener';

/**
 * Opens a URL using the host OS's default handler.
 *
 * Inside Tauri the call is dispatched through `tauri-plugin-opener`
 * (which delegates to the OS shell — Finder/`open`, xdg-open, etc.)
 * so custom URL schemes like `obsidian://` actually launch their
 * registered application instead of staying inside the embedded
 * webview.
 *
 * On the Tauri side errors propagate to the caller — we deliberately
 * do NOT fall back to `window.open` for desktop. The fallback would
 * spawn a Tauri webview window that has no useful behaviour for
 * custom schemes (Obsidian, mailto, etc.) and the call would appear
 * to "open in a new window" instead of handing off to the OS.
 *
 * In a browser context (no Tauri) we keep the `window.open` path so
 * `https://` / `mailto:` links still work for dev/preview builds.
 */
export const openUrl = async (url: string): Promise<void> => {
  if (isTauri()) {
    await tauriOpenUrl(url);
    return;
  }
  window.open(url, '_blank', 'noopener,noreferrer');
};
