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
