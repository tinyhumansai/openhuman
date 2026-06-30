/**
 * Artifact export service (#2779, #3162).
 *
 * All paths first resolve the artifact's absolute on-disk path + meta
 * via the `openhuman.ai_get_artifact` core RPC, then hand a source path
 * + filename hint to a Tauri command:
 *
 *  - {@link saveArtifactViaDialog} (#3162) — native Save-As dialog
 *    pre-filled with the filename; user picks the destination. Cancel is
 *    reported as `{ ok: false, code: 'CANCELLED' }`. Falls back to the
 *    Downloads copy if the dialog itself is unavailable.
 *  - {@link downloadArtifact} (#2779) — copies into the user's Downloads
 *    directory with a non-colliding name and returns the dest path so the
 *    UI can offer "Reveal in Finder".
 *
 * No-ops outside Tauri (browser dev preview) — export only makes sense in
 * the desktop shell.
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
  | 'CANCELLED'
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

/** Resolved source path + filename hint for an artifact export. */
interface ResolvedExport {
  ok: true;
  sourcePath: string;
  filename: string;
}
type ResolveExportResult = ResolvedExport | { ok: false; code: ArtifactErrorCode; error: string };

/**
 * Resolve an artifact's absolute on-disk path (via `ai_get_artifact`)
 * and build the suggested filename. Shared by the Save-As and Downloads
 * export paths so both apply identical title/extension handling.
 */
async function resolveArtifactForExport(
  artifactId: string,
  fallbackTitle: string,
  extension: string
): Promise<ResolveExportResult> {
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

  return { ok: true, sourcePath, filename };
}

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

  const resolved = await resolveArtifactForExport(artifactId, fallbackTitle, extension);
  if (!resolved.ok) {
    return { ok: false, code: resolved.code, error: resolved.error };
  }

  try {
    const dest = await invoke<string>('download_artifact_to_downloads', {
      sourcePath: resolved.sourcePath,
      filename: resolved.filename,
    });
    return { ok: true, path: dest };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, code: 'DOWNLOAD_FAILED', error: reason };
  }
}

/**
 * Export an artifact via the native Save-As dialog (#3162), pre-filled
 * with the artifact's filename. On the user dismissing the dialog,
 * returns `{ ok: false, code: 'CANCELLED' }` (the caller should treat
 * this as a no-op, not an error). If the dialog itself is unavailable —
 * e.g. headless Linux with no xdg-desktop portal — falls back to the
 * Downloads copy so the artifact is still recoverable.
 */
export async function saveArtifactViaDialog(
  artifactId: string,
  fallbackTitle: string,
  extension: string
): Promise<DownloadArtifactOutcome> {
  if (!isTauri()) {
    return { ok: false, code: 'NOT_DESKTOP', error: 'Saving is only available in the desktop app' };
  }

  const resolved = await resolveArtifactForExport(artifactId, fallbackTitle, extension);
  if (!resolved.ok) {
    return { ok: false, code: resolved.code, error: resolved.error };
  }

  try {
    // Command returns the saved path, or `null` when the user cancelled.
    const dest = await invoke<string | null>('save_artifact_via_dialog', {
      sourcePath: resolved.sourcePath,
      suggestedFilename: resolved.filename,
    });
    if (dest == null) {
      return { ok: false, code: 'CANCELLED', error: 'save cancelled by user' };
    }
    return { ok: true, path: dest };
  } catch (err) {
    // Dialog unavailable (no portal / unsupported) — recover the artifact
    // via the Downloads copy rather than stranding the user.
    const reason = err instanceof Error ? err.message : String(err);
    console.warn('[artifact] save dialog failed, falling back to Downloads:', reason);
    return downloadArtifact(artifactId, fallbackTitle, extension);
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
