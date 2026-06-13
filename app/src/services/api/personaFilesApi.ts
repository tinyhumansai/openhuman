/**
 * Persona file façade for the Settings → Persona panel (issue #2345).
 *
 * Wraps the core's workspace-file JSON-RPC surface so the panel never imports
 * `coreRpcClient` directly. Only the bundled persona prompt files are editable;
 * the core enforces the same allowlist, so an unknown name is rejected on both
 * sides.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('persona:files');

/** Files the Persona panel may read / edit / reset. Mirrors the core allowlist. */
export const PERSONA_FILE_SOUL = 'SOUL.md';

/** Shape returned by every `openhuman.workspace_file_*` method. */
export interface WorkspaceFile {
  filename: string;
  contents: string;
  /** True when the contents are the bundled default (missing on read, or reset). */
  is_default: boolean;
}

export async function readPersonaFile(filename: string): Promise<WorkspaceFile> {
  log('[rpc] read:start file=%s', filename);
  try {
    const file = await callCoreRpc<WorkspaceFile>({
      method: 'openhuman.workspace_file_read',
      params: { filename },
    });
    log(
      '[rpc] read:ok file=%s is_default=%s bytes=%d',
      filename,
      file.is_default,
      file.contents.length
    );
    return file;
  } catch (err) {
    log('[rpc] read:error file=%s err=%s', filename, err instanceof Error ? err.message : err);
    throw err;
  }
}

export async function writePersonaFile(filename: string, contents: string): Promise<WorkspaceFile> {
  log('[rpc] write:start file=%s bytes=%d', filename, contents.length);
  try {
    const file = await callCoreRpc<WorkspaceFile>({
      method: 'openhuman.workspace_file_write',
      params: { filename, contents },
    });
    log('[rpc] write:ok file=%s', filename);
    return file;
  } catch (err) {
    log('[rpc] write:error file=%s err=%s', filename, err instanceof Error ? err.message : err);
    throw err;
  }
}

export async function resetPersonaFile(filename: string): Promise<WorkspaceFile> {
  log('[rpc] reset:start file=%s', filename);
  try {
    const file = await callCoreRpc<WorkspaceFile>({
      method: 'openhuman.workspace_file_reset',
      params: { filename },
    });
    log('[rpc] reset:ok file=%s', filename);
    return file;
  } catch (err) {
    log('[rpc] reset:error file=%s err=%s', filename, err instanceof Error ? err.message : err);
    throw err;
  }
}
