/**
 * Persona file façade for the Settings → Persona panel (issue #2345).
 *
 * Wraps the core's workspace-file JSON-RPC surface so the panel never imports
 * `coreRpcClient` directly. Only the bundled persona prompt files are editable;
 * the core enforces the same allowlist, so an unknown name is rejected on both
 * sides.
 */
import { callCoreRpc } from '../coreRpcClient';

/** Files the Persona panel may read / edit / reset. Mirrors the core allowlist. */
export const PERSONA_FILE_SOUL = 'SOUL.md';

/** Shape returned by every `openhuman.workspace_file_*` method. */
export interface WorkspaceFile {
  filename: string;
  contents: string;
  /** True when the contents are the bundled default (missing on read, or reset). */
  is_default: boolean;
  path: string;
}

export async function readPersonaFile(filename: string): Promise<WorkspaceFile> {
  return callCoreRpc<WorkspaceFile>({
    method: 'openhuman.workspace_file_read',
    params: { filename },
  });
}

export async function writePersonaFile(filename: string, contents: string): Promise<WorkspaceFile> {
  return callCoreRpc<WorkspaceFile>({
    method: 'openhuman.workspace_file_write',
    params: { filename, contents },
  });
}

export async function resetPersonaFile(filename: string): Promise<WorkspaceFile> {
  return callCoreRpc<WorkspaceFile>({
    method: 'openhuman.workspace_file_reset',
    params: { filename },
  });
}
