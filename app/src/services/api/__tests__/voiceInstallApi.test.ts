import { beforeEach, describe, expect, it, vi } from 'vitest';

import { installPiper, piperInstallStatus, type VoiceInstallStatus } from '../voiceInstallApi';

vi.mock('../../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const buildStatus = (overrides: Partial<VoiceInstallStatus> = {}): VoiceInstallStatus => ({
  engine: 'piper',
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

  describe('installPiper', () => {
    it('passes voice_id and force flags through to the RPC', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(
        buildStatus({ engine: 'piper', state: 'installing', progress: 25 })
      );
      const result = await installPiper({ voiceId: 'en_US-lessac-medium', force: false });
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.inference_install_piper',
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
        method: 'openhuman.inference_install_piper',
        params: { voice_id: undefined, force: undefined },
      });
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
        method: 'openhuman.inference_piper_install_status',
        params: {},
      });
      expect(result.state).toBe('error');
      expect(result.error_detail).toBe('network down');
    });
  });
});
