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

/** Outcome surfaced to the UI for a single download attempt. */
export interface DownloadArtifactOutcome {
  ok: boolean;
  /** Absolute destination path when `ok === true`. */
  path?: string;
  /** Short, user-facing error string when `ok === false`. */
  error?: string;
}

/** Outcome surfaced to the UI for a single delete attempt (#3024). */
export interface DeleteArtifactOutcome {
  ok: boolean;
  /** Short, user-facing error string when `ok === false`. */
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
    return { ok: false, error: 'Downloads are only available in the desktop app' };
  }
  if (!artifactId.trim()) {
    return { ok: false, error: 'artifact id missing' };
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
    return { ok: false, error: `failed to resolve artifact: ${reason}` };
  }

  const sourcePath = resolved.absolute_path;
  if (!sourcePath) {
    return { ok: false, error: 'artifact path missing from core response' };
  }

  // Prefer the persisted title (came from create_artifact's
  // sanitized stem) but fall back to the caller-supplied hint.
  const title = resolved.meta?.title?.trim() || fallbackTitle.trim() || 'artifact';
  const ext = extension.trim().replace(/^\.+/, '');
  const filename = ext ? `${title}.${ext}` : title;

  try {
    const dest = await invoke<string>('download_artifact_to_downloads', { sourcePath, filename });
    return { ok: true, path: dest };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, error: reason };
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
    return { ok: false, error: 'artifact id missing' };
  }
  try {
    await callCoreRpc<unknown>({
      method: 'openhuman.ai_delete_artifact',
      params: { artifact_id: artifactId },
    });
    return { ok: true };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, error: reason };
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
