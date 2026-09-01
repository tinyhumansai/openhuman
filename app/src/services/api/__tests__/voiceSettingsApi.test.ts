import { beforeEach, describe, expect, it, vi } from 'vitest';

import { testVoiceProvider } from '../voiceSettingsApi';

vi.mock('../../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const okResult = { ok: true, detail: 'Provider key is valid (12ms)', latency_ms: 12 };

describe('voiceSettingsApi', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  describe('testVoiceProvider', () => {
    it('omits api_key entirely when no candidate key is supplied', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(okResult);

      await testVoiceProvider('stt', 'cloud');

      // `api_key` must be ABSENT, not present-and-empty: the core reads an
      // empty candidate as "fall back to the stored credential", so sending
      // one would change which key is being tested.
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.voice_test_provider',
        params: { workload: 'stt', provider: 'cloud', validate_only: false },
        timeoutMs: 30_000,
      });
      const params = vi.mocked(callCoreRpc).mock.calls[0][0].params as Record<string, unknown>;
      expect('api_key' in params).toBe(false);
    });

    it('sends the candidate key for a dry run, trimmed', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(okResult);

      // A pasted key routinely carries a trailing newline. The guard and the
      // payload have to agree on which string is being tested (#5896 review).
      await testVoiceProvider('stt', 'elevenlabs', true, '  sk-candidate-key-123\n');

      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.voice_test_provider',
        params: {
          workload: 'stt',
          provider: 'elevenlabs',
          validate_only: true,
          api_key: 'sk-candidate-key-123',
        },
        timeoutMs: 30_000,
      });
    });

    it('treats a whitespace-only candidate key as no key at all', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce(okResult);

      await testVoiceProvider('tts', 'elevenlabs', true, '   ');

      const params = vi.mocked(callCoreRpc).mock.calls[0][0].params as Record<string, unknown>;
      expect('api_key' in params).toBe(false);
    });

    it('strips the core log prefix from the returned detail', async () => {
      const { callCoreRpc } = await import('../../coreRpcClient');
      vi.mocked(callCoreRpc).mockResolvedValueOnce({
        ok: false,
        detail: '[voice-factory] Key test failed: API returned 401',
      });

      const result = await testVoiceProvider('stt', 'elevenlabs', true, 'sk-bad');

      expect(result.detail).toBe('Key test failed: API returned 401');
      expect(result.ok).toBe(false);
    });
  });
});
