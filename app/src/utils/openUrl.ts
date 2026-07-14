import * as Sentry from '@sentry/react';
import { revealItemInDir, openUrl as tauriOpenUrl } from '@tauri-apps/plugin-opener';
import { platform } from '@tauri-apps/plugin-os';

import { isTauri } from './tauriCommands/common';

const isHttpUrl = (url: string): boolean => /^https?:\/\//i.test(url.trim());

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
  const normalizedUrl = url.trim();

  if (isTauri()) {
    try {
      await tauriOpenUrl(normalizedUrl);
      return;
    } catch (err) {
      Sentry.addBreadcrumb({
        category: 'ipc',
        level: 'warning',
        message: 'tauriOpenUrl failed; evaluating fallback',
        data: { url: getTelemetryUrl(normalizedUrl), error: String(err) },
      });
      if (!isHttpUrl(normalizedUrl)) {
        throw err;
      }
      // http(s) URL — safe to fall back to window.open.
    }
  }
  window.open(normalizedUrl, '_blank', 'noopener,noreferrer');
};

/**
 * Detects a filesystem path that belongs to a different OS family than the one
 * running this frontend — a Windows path (`C:\…` or a `\\UNC` share) on a POSIX
 * host, or a POSIX absolute path (`/…`) on Windows.
 *
 * This is the cross-host guard for issue #4278: `openhuman-core` can serve a
 * path that lives on its own (possibly different-OS) host, and revealing such a
 * path locally would fail with a cryptic opener error. Returns `false` when the
 * OS is unknown so we never block a legitimate same-host reveal.
 */
export const isForeignFsPath = (path: string, clientOs: string | undefined): boolean => {
  const p = path.trim();
  if (!p) return false;
  const looksWindows = /^[a-zA-Z]:[\\/]/.test(p) || /^\\\\/.test(p);
  const looksPosixAbs = p.startsWith('/');
  const os = (clientOs ?? '').toLowerCase();
  if (os === 'windows') return looksPosixAbs && !looksWindows;
  if (os === 'macos' || os === 'linux') return looksWindows;
  return false;
};

/**
 * Reveals a filesystem path in the host OS file manager
 * (Finder on macOS, Explorer on Windows, the default file manager on
 * Linux). Used as a guaranteed-works fallback when a third-party
 * deep link (e.g. `obsidian://`) may silently no-op because the
 * target app isn't installed.
 *
 * Outside Tauri this is a no-op — there's no OS shell to drive.
 *
 * Rejects with a clear error when `path` belongs to a different OS than this
 * device (issue #4278) — e.g. a shared `openhuman-core` running on another OS
 * served its own absolute path — instead of letting the opener fail cryptically.
 */
export const revealPath = async (path: string): Promise<void> => {
  if (!isTauri()) return;
  let clientOs: string | undefined;
  try {
    clientOs = await platform();
  } catch {
    clientOs = undefined;
  }
  if (isForeignFsPath(path, clientOs)) {
    throw new Error(
      `Cannot reveal "${path}" on this device — it is a path on the openhuman-core host's filesystem (a different OS). Open it on the machine running the core.`
    );
  }
  await revealItemInDir(path);
};
