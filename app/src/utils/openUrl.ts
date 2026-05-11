import * as Sentry from '@sentry/react';
import { isTauri } from '@tauri-apps/api/core';
import { openUrl as tauriOpenUrl } from '@tauri-apps/plugin-opener';

const isHttpUrl = (url: string): boolean => /^https?:\/\//i.test(url);

/**
 * Opens a URL using the host OS's default handler.
 *
 * Inside Tauri the call is dispatched through `tauri-plugin-opener`
 * (which delegates to the OS shell — Finder/`open`, xdg-open, etc.)
 * so custom URL schemes like `obsidian://` actually launch their
 * registered application instead of staying inside the embedded
 * webview.
 *
 * CEF embedder note: the IPC bridge (`window.ipc.postMessage`) is
 * injected on the renderer-side after `on_after_created` fires.
 * A click landing in that gap causes the plugin's `invoke()` glue
 * to reject with `TypeError: Cannot read properties of undefined
 * (reading 'postMessage')`. For http(s) URLs we recover by falling
 * back to `window.open` so the user-facing flow still works. For
 * non-http schemes we re-throw — `window.open` would spawn a Tauri
 * webview window that cannot handle custom schemes, which is worse
 * UX than a propagated error the caller can surface.
 *
 * In a browser context (no Tauri) we keep the `window.open` path so
 * `https://` / `mailto:` links still work for dev/preview builds.
 */
export const openUrl = async (url: string): Promise<void> => {
  if (isTauri()) {
    try {
      await tauriOpenUrl(url);
      return;
    } catch (err) {
      Sentry.addBreadcrumb({
        category: 'ipc',
        level: 'warning',
        message: 'tauriOpenUrl failed; evaluating fallback',
        data: { url, error: String(err) },
      });
      if (!isHttpUrl(url)) {
        throw err;
      }
      // http(s) URL — safe to fall back to window.open.
    }
  }
  window.open(url, '_blank', 'noopener,noreferrer');
};
