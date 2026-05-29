import { invoke } from '@tauri-apps/api/core';

import { isTauri } from './common';

interface RawWorkspaceTextPreview {
  path: string;
  absolute_path: string;
  contents: string;
  truncated: boolean;
  size_bytes: number;
}

export interface WorkspaceTextPreview {
  path: string;
  absolutePath: string;
  contents: string;
  truncated: boolean;
  sizeBytes: number;
}

function assertTauri() {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
}

export async function openWorkspacePath(path: string): Promise<void> {
  assertTauri();
  await invoke<void>('open_workspace_path', { path });
}

export async function revealWorkspacePath(path: string): Promise<void> {
  assertTauri();
  await invoke<void>('reveal_workspace_path', { path });
}

export async function previewWorkspaceText(path: string): Promise<WorkspaceTextPreview> {
  assertTauri();
  const preview = await invoke<RawWorkspaceTextPreview>('preview_workspace_text', { path });
  return {
    path: preview.path,
    absolutePath: preview.absolute_path,
    contents: preview.contents,
    truncated: preview.truncated,
    sizeBytes: preview.size_bytes,
  };
}

/**
 * Resolve a workspace-relative path to its canonical absolute path on disk,
 * after the Rust side validates it stays inside the active OpenHuman
 * workspace. Useful for UI flows that need to compose an absolute path into a
 * platform-specific URL scheme (e.g. `obsidian://open?path=<abs>`) without
 * re-implementing path normalization in the renderer.
 */
export async function resolveWorkspaceAbsolutePath(path: string): Promise<string> {
  assertTauri();
  return invoke<string>('resolve_workspace_absolute_path', { path });
}
