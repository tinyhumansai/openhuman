import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { isTauri } from './common';
import { openWorkspacePath, previewWorkspaceText, revealWorkspacePath } from './workspacePaths';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('./common', () => ({ isTauri: vi.fn() }));

describe('tauriCommands/workspacePaths', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(isTauri).mockReset();
    vi.mocked(isTauri).mockReturnValue(true);
  });

  test('throws before invoking when not running in Tauri', async () => {
    vi.mocked(isTauri).mockReturnValue(false);

    await expect(openWorkspacePath('docs/readme.md')).rejects.toThrow('Not running in Tauri');

    expect(invoke).not.toHaveBeenCalled();
  });

  test('invokes open_workspace_path with a workspace-relative path', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await openWorkspacePath('memory_tree/content/summary.md');

    expect(invoke).toHaveBeenCalledWith('open_workspace_path', {
      path: 'memory_tree/content/summary.md',
    });
  });

  test('invokes reveal_workspace_path with a workspace-relative path', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await revealWorkspacePath('memory_tree/content/summary.md');

    expect(invoke).toHaveBeenCalledWith('reveal_workspace_path', {
      path: 'memory_tree/content/summary.md',
    });
  });

  test('invokes preview_workspace_text and returns preview payload', async () => {
    vi.mocked(invoke).mockResolvedValue({
      path: 'docs/readme.md',
      absolute_path: '/tmp/workspace/docs/readme.md',
      contents: '# Readme',
      truncated: false,
      size_bytes: 8,
    });

    await expect(previewWorkspaceText('docs/readme.md')).resolves.toEqual({
      path: 'docs/readme.md',
      absolutePath: '/tmp/workspace/docs/readme.md',
      contents: '# Readme',
      truncated: false,
      sizeBytes: 8,
    });

    expect(invoke).toHaveBeenCalledWith('preview_workspace_text', { path: 'docs/readme.md' });
  });
});
