/**
 * Artifact export service (#2779).
 *
 * Every path first resolves the artifact's meta via the
 * `openhuman.ai_get_artifact` core RPC (which also reports a file moved or
 * deleted outside OpenHuman), then hands the artifact id + filename hint to a
 * Tauri command. The shell resolves the id through the core's artifact store
 * itself — it never trusts a renderer-supplied path (#5505).
 *
 * - {@link saveArtifactViaDialog}: the OS save dialog (`save_artifact_as`);
 *   the user picks the destination. A closed dialog is `'CANCELLED'`.
 * - {@link downloadArtifact}: a copy into the user's Downloads directory with
 *   a non-colliding name (`download_artifact_to_downloads`), so the UI can
 *   offer "Reveal in Finder".
 *
 * No-ops outside Tauri (browser dev preview) — export only makes sense in
 * the desktop shell.
 */
import { revealPath } from '../utils/openUrl';
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
  | 'DELETE_FAILED'
  | 'REVEAL_FAILED';

/** Outcome surfaced to the UI for a single download attempt. */
interface DownloadArtifactOutcome {
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
interface DeleteArtifactOutcome {
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

type ListedArtifactKind = import('../types/threadWire.generated').ArtifactKind;

interface ListedThreadArtifact {
  artifactId: string;
  kind: ListedArtifactKind;
  title: string;
  path: string;
  sizeBytes: number;
}

interface ListThreadArtifactsOutcome {
  ok: boolean;
  artifacts: ListedThreadArtifact[];
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

interface AiListArtifactsData {
  artifacts?: AiListArtifactMeta[];
}

interface AiListArtifactMeta {
  id?: string;
  kind?: string;
  title?: string;
  path?: string;
  size_bytes?: number;
  status?: string;
}

function listedArtifactKind(raw: string | undefined): ListedArtifactKind | null {
  switch (raw) {
    case 'presentation':
    case 'document':
    case 'image':
    case 'video':
    case 'other':
      return raw;
    default:
      return null;
  }
}

function readyListedArtifact(meta: AiListArtifactMeta): ListedThreadArtifact | null {
  if (meta.status !== 'ready') return null;
  const kind = listedArtifactKind(meta.kind);
  if (!kind) return null;
  if (!meta.id || !meta.title || !meta.path) return null;
  if (typeof meta.size_bytes !== 'number' || !Number.isFinite(meta.size_bytes)) return null;
  return {
    artifactId: meta.id,
    kind,
    title: meta.title,
    path: meta.path,
    sizeBytes: meta.size_bytes,
  };
}

export async function listArtifactsForThread(
  threadId: string
): Promise<ListThreadArtifactsOutcome> {
  const trimmedThreadId = threadId.trim();
  if (!trimmedThreadId) {
    return { ok: false, artifacts: [], error: 'thread id missing' };
  }
  try {
    const limit = 200;
    const artifacts: ListedThreadArtifact[] = [];

    for (let offset = 0; ; offset += limit) {
      const raw = await callCoreRpc<AiListArtifactsData>({
        method: 'openhuman.ai_list_artifacts',
        params: { thread_id: trimmedThreadId, offset, limit },
      });
      const page = raw?.artifacts ?? [];
      artifacts.push(
        ...page
          .map(readyListedArtifact)
          .filter((artifact): artifact is ListedThreadArtifact => artifact !== null)
      );
      if (page.length < limit) break;
    }

    return { ok: true, artifacts };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, artifacts: [], error: reason };
  }
}

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
  // requested extension (case-insensitive), or in another real one, don't
  // append. Prevents `deck.pptx.pptx`, and keeps a title like `deck.pdf` as
  // the user sees it. A real extension starts with a letter, so a title that
  // only looks like it has one ("Release v1.2") still gets the extension.
  const titleHasExtension = /\.[a-z][a-z0-9]{0,4}$/i.test(title);
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
      artifactId,
      filename: resolved.filename,
    });
    return { ok: true, path: dest };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, code: 'DOWNLOAD_FAILED', error: reason };
  }
}

/**
 * Let the user choose where to save an artifact: the OS save dialog
 * (`save_artifact_as`), prefilled with the artifact's own filename. The shell
 * copies the file the core's artifact store vouches for.
 *
 * Returns `{ ok: true, path }` once saved and `{ ok: false, code: 'CANCELLED' }`
 * when the user closes the dialog; other failures carry the usual codes.
 * Desktop only (`NOT_DESKTOP` elsewhere).
 */
export async function saveArtifactViaDialog(
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
    const dest = await invoke<string | null>('save_artifact_as', {
      artifactId,
      filename: resolved.filename,
    });
    if (!dest) return { ok: false, code: 'CANCELLED' };
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

/** Outcome of a "Show in folder" request. */
export type RevealArtifactOutcome =
  | { ok: true }
  | { ok: false; code: ArtifactErrorCode; error: string };

/**
 * Open the OS file manager at an artifact's file (#5505). The file lives in
 * the visible files folder (`~/OpenHuman/projects/Files` by default), so this
 * shows the real file, not a Downloads copy. The path comes from the core
 * (`ai_get_artifact`), which also reports a file moved or deleted outside
 * OpenHuman; `revealPath` refuses a path from a core running on another OS.
 */
export async function revealArtifact(artifactId: string): Promise<RevealArtifactOutcome> {
  if (!isTauri()) {
    return { ok: false, code: 'NOT_DESKTOP', error: 'Show in folder needs the desktop app' };
  }
  const resolved = await resolveArtifactForExport(artifactId, '', '');
  if (!resolved.ok) {
    return { ok: false, code: resolved.code, error: resolved.error };
  }
  try {
    await revealPath(resolved.sourcePath);
    return { ok: true };
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    return { ok: false, code: 'REVEAL_FAILED', error: reason };
  }
}
