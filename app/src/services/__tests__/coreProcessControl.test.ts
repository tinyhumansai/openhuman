/**
 * Tests for coreProcessControl — covers changed lines 13-15, 17.
 */
import { describe, expect, it, vi } from 'vitest';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock, isTauri: vi.fn(() => false) }));

// isTauri() in production code is from tauriCommands/common, which calls
// coreIsTauri() from @tauri-apps/api/core. Mock it to return false (non-Tauri env).
vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: vi.fn(() => false) }));

describe('coreProcessControl — restartCoreProcess', () => {
  it('throws "only available in the desktop app" when not in Tauri (lines 13-15)', async () => {
    // isTauri() resolves to false in the Vitest environment (no Tauri IPC bridge).
    const { restartCoreProcess } = await import('../coreProcessControl');

    await expect(restartCoreProcess()).rejects.toThrow(
      'Restart Core is only available in the desktop app.'
    );
    // invoke must not be called when the guard fires.
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
