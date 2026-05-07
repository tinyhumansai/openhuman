import { isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/config', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;
  let openhumanUpdateLocalAiSettings: typeof import('./config').openhumanUpdateLocalAiSettings;
  let openhumanUpdateMeetSettings: typeof import('./config').openhumanUpdateMeetSettings;
  let openhumanGetMeetSettings: typeof import('./config').openhumanGetMeetSettings;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('./config')>('./config');
    openhumanUpdateLocalAiSettings = actual.openhumanUpdateLocalAiSettings;
    openhumanUpdateMeetSettings = actual.openhumanUpdateMeetSettings;
    openhumanGetMeetSettings = actual.openhumanGetMeetSettings;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('openhumanUpdateLocalAiSettings', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanUpdateLocalAiSettings({ runtime_enabled: true })).rejects.toThrow(
        'Not running in Tauri'
      );
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards the patch to openhuman.update_local_ai_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
        logs: [],
      });
      const patch = { runtime_enabled: true, usage_embeddings: true, usage_subconscious: false };
      await openhumanUpdateLocalAiSettings(patch);
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.update_local_ai_settings',
        params: patch,
      });
    });
  });

  describe('openhumanUpdateMeetSettings (#1299)', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(
        openhumanUpdateMeetSettings({ auto_orchestrator_handoff: true })
      ).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards the patch to openhuman.config_update_meet_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
        logs: [],
      });
      await openhumanUpdateMeetSettings({ auto_orchestrator_handoff: true });
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_update_meet_settings',
        params: { auto_orchestrator_handoff: true },
      });
    });
  });

  describe('openhumanGetMeetSettings (#1299)', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanGetMeetSettings()).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('reads via openhuman.config_get_meet_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: { auto_orchestrator_handoff: true }, logs: [] });
      const out = await openhumanGetMeetSettings();
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_get_meet_settings',
      });
      expect(out.result.auto_orchestrator_handoff).toBe(true);
    });
  });
});
