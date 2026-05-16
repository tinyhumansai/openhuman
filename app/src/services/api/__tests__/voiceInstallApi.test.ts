import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  installPiper,
  installWhisper,
  piperInstallStatus,
  type VoiceInstallStatus,
  whisperInstallStatus,
} from '../voiceInstallApi';

vi.mock('../../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const buildStatus = (overrides: Partial<VoiceInstallStatus> = {}): VoiceInstallStatus => ({
  engine: 'whisper',
  state: 'installed',
  progress: 100,
  downloaded_bytes: null,
  total_bytes: null,
  stage: null,
  error_detail: null,
  ...overrides,
});

describe('voiceInstallApi', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  describe('installWhisper', () => {
    it('passes model_size and force flags through to the RPC', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(buildStatus({ engine: 'whisper' }));
      const result = await installWhisper({ modelSize: 'tiny', force: true });
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.local_ai_install_whisper',
        params: { model_size: 'tiny', force: true },
      });
      expect(result.engine).toBe('whisper');
      expect(result.state).toBe('installed');
    });

    it('omits undefined params and lets the core apply defaults', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(buildStatus());
      await installWhisper();
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.local_ai_install_whisper',
        params: { model_size: undefined, force: undefined },
      });
    });

    it('propagates a thrown RPC error so the UI can surface it', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('boom'));
      await expect(installWhisper({ modelSize: 'tiny' })).rejects.toThrow('boom');
    });
  });

  describe('installPiper', () => {
    it('passes voice_id and force flags through to the RPC', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(
        buildStatus({ engine: 'piper', state: 'installing', progress: 25 })
      );
      const result = await installPiper({ voiceId: 'en_US-lessac-medium', force: false });
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.local_ai_install_piper',
        params: { voice_id: 'en_US-lessac-medium', force: false },
      });
      expect(result.state).toBe('installing');
      expect(result.progress).toBe(25);
    });

    it('omits undefined params and lets the core apply defaults', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(buildStatus({ engine: 'piper' }));
      await installPiper();
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.local_ai_install_piper',
        params: { voice_id: undefined, force: undefined },
      });
    });
  });

  describe('whisperInstallStatus', () => {
    it('calls the status RPC with empty params', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(
        buildStatus({ engine: 'whisper', state: 'missing', progress: null })
      );
      const result = await whisperInstallStatus();
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.local_ai_whisper_install_status',
        params: {},
      });
      expect(result.state).toBe('missing');
    });
  });

  describe('piperInstallStatus', () => {
    it('calls the status RPC with empty params', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(
        buildStatus({ engine: 'piper', state: 'error', error_detail: 'network down' })
      );
      const result = await piperInstallStatus();
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.local_ai_piper_install_status',
        params: {},
      });
      expect(result.state).toBe('error');
      expect(result.error_detail).toBe('network down');
    });
  });
});
