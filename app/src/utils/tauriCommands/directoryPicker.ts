/**
 * Native directory chooser (#5831).
 *
 * The renderer cannot learn where a directory lives on its own. An
 * `<input type="file" webkitdirectory>` yields `File` objects, and a `File`
 * carries no filesystem location — `File.path` is an Electron extension that
 * Wry's WKWebView/WebView2/WebKitGTK do not implement. So the path has to
 * come from the host process, which is what
 * `directory_picker::pick_directory_via_dialog` provides.
 */
import { isTauri, safeInvoke } from './common';

export type DirectoryPickResult =
  /** The user chose a directory; `path` is absolute. */
  | { ok: true; path: string }
  /**
   * No path was obtained. Callers must NOT substitute a guess: storing a
   * value that cannot resolve is the defect #5831 describes, where a source
   * looked configured and failed once per sync cycle forever.
   *
   * - `cancelled` — the user dismissed the dialog. Leave the field alone.
   * - `unavailable` — no host to ask (a plain browser context, or the IPC
   *   bridge is not wired yet).
   * - `failed` — the host tried and could not produce an absolute path.
   */
  | { ok: false; reason: 'cancelled' | 'unavailable' | 'failed'; message?: string };

/**
 * Open the OS-native directory chooser.
 *
 * Never throws: every failure mode is returned as a discriminated result so
 * the caller can decide what the user sees. `unavailable` and `failed` are
 * kept distinct because only the latter means the host actually tried.
 */
export async function pickDirectoryNatively(): Promise<DirectoryPickResult> {
  if (!isTauri()) {
    return { ok: false, reason: 'unavailable' };
  }
  try {
    const picked = await safeInvoke<string | null>('pick_directory_via_dialog');
    if (picked === null || picked === undefined || picked.length === 0) {
      return { ok: false, reason: 'cancelled' };
    }
    return { ok: true, path: picked };
  } catch (err) {
    return {
      ok: false,
      reason: 'failed',
      message: err instanceof Error ? err.message : String(err),
    };
  }
}
