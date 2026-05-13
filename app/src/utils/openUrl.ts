import * as Sentry from '@sentry/react';
import { isTauri } from '@tauri-apps/api/core';
import { openUrl as tauriOpenUrl } from '@tauri-apps/plugin-opener';

const isHttpUrl = (url: string): boolean => /^https?:\/\//i.test(url);

/**
 * Returns a low-PII representation of `url` for telemetry breadcrumbs.
 * For http(s) we keep only the origin so the host is identifiable but the
 * pathname/query/fragment (which may carry tokens, emails, or local paths)
 * never leave the device. For other schemes (`mailto:`, `obsidian://`, …)
 * we keep only the protocol — the rest of the URL is the payload itself
 * (the email address, the vault path) and must not be logged.
 */
const getTelemetryUrl = (url: string): string => {
  try {
    const parsed = new URL(url);
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:') {
      return parsed.origin;
    }
    return parsed.protocol;
  } catch {
    return 'invalid-url';
  }
};

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
        data: { url: getTelemetryUrl(url), error: String(err) },
      });
      if (!isHttpUrl(url)) {
        throw err;
      }
      // http(s) URL — safe to fall back to window.open.
    }
  }
  window.open(url, '_blank', 'noopener,noreferrer');
};
