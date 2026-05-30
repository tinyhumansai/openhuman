/**
 * Artifact download service (#2779).
 *
 * Flow:
 *
 *  1. Call `openhuman.ai_get_artifact` via the existing core RPC client
 *     to resolve the artifact's absolute on-disk path + meta.
 *  2. Invoke the Tauri `download_artifact_to_downloads` command with
 *     the source path + a filename hint built from the artifact's
 *     title. The command picks a non-colliding name under the user's
 *     Downloads directory and copies the file.
 *  3. Return the resolved dest path so the UI can show a "Saved to …"
 *     toast with a "Reveal in Finder" button (the `opener` plugin's
 *     `reveal-item-in-dir` capability is already wired).
 *
 * No-ops outside Tauri (browser dev preview) — the download flow only
 * makes sense in the desktop shell.
 */
import { revealItemInDir } from '@tauri-apps/plugin-opener';

import { safeInvoke as invoke, isTauri } from '../utils/tauriCommands/common';
import { callCoreRpc } from './coreRpcClient';

/**
 * Stable, machine-readable failure reasons surfaced by the artifact
 * download/delete flows. UI layers should branch on `code` and route to
 * their own `t(...)` strings — `error` is kept as a diagnostic detail
 * (RPC text, transport error) and MUST NOT be the sole label shown to a
 * non-English locale. Codes are intentionally narrow so adding a new
 * arm requires a deliberate change here, not a free-form string.
 */
export type ArtifactErrorCode =
  | 'NOT_DESKTOP'
  | 'MISSING_ARTIFACT_ID'
  | 'MISSING_ARTIFACT_PATH'
  | 'RESOLVE_FAILED'
  | 'DOWNLOAD_FAILED'
  | 'DELETE_FAILED';

/** Outcome surfaced to the UI for a single download attempt. */
export interface DownloadArtifactOutcome {
  ok: boolean;
  /** Absolute destination path when `ok === true`. */
  path?: string;
  /**
   * Stable failure code when `ok === false`. Pair with `error` (raw
   * detail) — UI maps `code` to a localized string via `t(...)`.
   */
  code?: ArtifactErrorCode;
  /**
   * Diagnostic detail (RPC text, transport error). Not localized; the
   * UI should treat this as a developer-facing hint, not the headline.
   */
  error?: string;
}

/** Outcome surfaced to the UI for a single delete attempt (#3024). */
export interface DeleteArtifactOutcome {
  ok: boolean;
  /**
   * Stable failure code when `ok === false`. Pair with `error` (raw
   * detail) — UI maps `code` to a localized string via `t(...)`.
   */
  code?: ArtifactErrorCode;
  /**
   * Diagnostic detail (RPC text, transport error). Not localized; the
   * UI should treat this as a developer-facing hint, not the headline.
   */
  error?: string;
}

/**
 * Shape of the `data` field returned by the
 * `openhuman.ai_get_artifact` JSON-RPC method. We pull only the
 * fields we need; extra fields are tolerated.
 */
interface AiGetArtifactData {
  absolute_path?: string;
  /** Full ArtifactMeta nested under this key on the core RPC response. */
  meta?: { id?: string; title?: string; path?: string; kind?: string; status?: string };
}

/**
 * Resolve the source path + filename hint, then copy to Downloads.
 *
 * `extension` is the file extension WITHOUT the leading dot
 * (`"pptx"`, `"pdf"`, …). Used to build the Downloads filename when
 * the title doesn't already carry one.
 */
export async function downloadArtifact(
  artifactId: string,
  fallbackTitle: string,
  extension: string
): Promise<DownloadArtifactOutcome> {
  if (!isTauri()) {
    return {
      ok: false,
      code: 'NOT_DESKTOP',
      error: 'Downloads are only available in the desktop app',
    };
  }
  if (!artifactId.trim()) {
    return { ok: false, code: 'MISSING_ARTIFACT_ID', error: 'artifact id missing' };
  }

  let resolved: AiGetArtifactData;
  try {
    const raw = await callCoreRpc<AiGetArtifactData>({
      method: 'openhuman.ai_get_artifact',
      params: { artifact_id: artifactId },
    });
    resolved = raw ?? {};
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, code: 'RESOLVE_FAILED', error: reason };
  }

  const sourcePath = resolved.absolute_path;
  if (!sourcePath) {
    return {
      ok: false,
      code: 'MISSING_ARTIFACT_PATH',
      error: 'artifact path missing from core response',
    };
  }

  // Prefer the persisted title (came from create_artifact's
  // sanitized stem) but fall back to the caller-supplied hint.
  const title = resolved.meta?.title?.trim() || fallbackTitle.trim() || 'artifact';
  const ext = extension.trim().replace(/^\.+/, '');
  // Guard against double extensions: if `title` already ends in the
  // requested extension (case-insensitive, with any other extension also
  // tolerated), don't append again. Prevents `deck.pptx.pptx` when the
  // persisted title is `deck.pptx` and the caller passes `'pptx'`.
  const titleHasExtension = /\.[^./\\]+$/.test(title);
  const titleHasSameExt = ext.length > 0 && title.toLowerCase().endsWith(`.${ext.toLowerCase()}`);
  const filename = ext && !titleHasExtension && !titleHasSameExt ? `${title}.${ext}` : title;

  try {
    const dest = await invoke<string>('download_artifact_to_downloads', { sourcePath, filename });
    return { ok: true, path: dest };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, code: 'DOWNLOAD_FAILED', error: reason };
  }
}

/**
 * Open the user's file manager pointed at the just-downloaded file.
 * Uses the existing `opener:allow-reveal-item-in-dir` capability —
 * no new permission needed. Returns `false` when not in Tauri or the
 * invoke fails (caller usually ignores the result).
 */
/**
 * Delete the artifact and its on-disk blob via the core RPC (#3024).
 * Caller is expected to optimistically remove the slice row first and
 * re-insert on `{ ok: false }`. Distinct from the runtime in-memory
 * slice ledger — this drops the file on disk and the persistent
 * `ArtifactMeta` row in the workspace registry.
 *
 * Returns `{ ok: false, error }` on any transport or RPC error
 * (network drop, core gone, unknown id, file vanished). The core
 * treats "missing meta" / "file already gone" as success.
 */
export async function deleteArtifact(artifactId: string): Promise<DeleteArtifactOutcome> {
  if (!artifactId.trim()) {
    return { ok: false, code: 'MISSING_ARTIFACT_ID', error: 'artifact id missing' };
  }
  try {
    await callCoreRpc<unknown>({
      method: 'openhuman.ai_delete_artifact',
      params: { artifact_id: artifactId },
    });
    return { ok: true };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, code: 'DELETE_FAILED', error: reason };
  }
}

export async function revealArtifactInFileManager(absolutePath: string): Promise<boolean> {
  if (!isTauri()) return false;
  if (!absolutePath.trim()) return false;
  try {
    // Use the plugin's typed binding — the raw `invoke('plugin:opener|
    // reveal_item_in_dir', { path })` shape silently no-ops because the
    // plugin expects `{ paths: [absolutePath] }` (array). The binding
    // handles the wrap.
    await revealItemInDir(absolutePath);
    return true;
  } catch (err) {
    // Swallow — reveal is best-effort, the file is already saved.
    console.warn('[artifact] revealItemInDir failed:', err);
    return false;
  }
}
