import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  clearLiveVoiceProviderKey,
  fetchLiveVoiceProviders,
  fetchLiveVoiceSettings,
  saveLiveVoiceProviderKey,
  testLiveVoiceProvider,
  updateLiveVoiceSettings,
} from './liveVoiceApi';

const mocks = vi.hoisted(() => ({
  callCoreRpc: vi.fn(),
  store: vi.fn(async () => ({ result: {}, logs: [] })),
  remove: vi.fn(async () => ({ result: {}, logs: [] })),
}));

vi.mock('../coreRpcClient', () => ({ callCoreRpc: mocks.callCoreRpc }));
vi.mock('../../utils/tauriCommands/auth', () => ({
  authStoreProviderCredentials: mocks.store,
  authRemoveProviderCredentials: mocks.remove,
}));

describe('liveVoiceApi', () => {
  beforeEach(() => vi.clearAllMocks());

  it('fetches providers and fills missing voice/language lists', async () => {
    mocks.callCoreRpc.mockResolvedValueOnce({
      default_provider: 'gemini-hosted',
      providers: [
        { id: 'gemini-hosted', label: 'Gemini', kind: 'hosted', configured: true, key_slug: null },
        {
          id: 'sarvam',
          label: 'Sarvam AI',
          kind: 'byok',
          configured: false,
          key_slug: 'sarvam',
          voices: ['anushka'],
          languages: ['hi-IN'],
        },
      ],
    });
    const result = await fetchLiveVoiceProviders();
    expect(mocks.callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.voice_live_providers' });
    expect(result.default_provider).toBe('gemini-hosted');
    expect(result.providers[0]).toMatchObject({ voices: [], languages: [] });
    expect(result.providers[1]).toMatchObject({ voices: ['anushka'], languages: ['hi-IN'] });
  });

  it('tolerates an empty provider response', async () => {
    mocks.callCoreRpc.mockResolvedValueOnce(null);
    await expect(fetchLiveVoiceProviders()).resolves.toEqual({
      default_provider: '',
      providers: [],
    });
  });

  it('normalises settings with null blocks', async () => {
    mocks.callCoreRpc.mockResolvedValueOnce({
      default_provider: 'sarvam',
      sarvam: { speaker: 'x' },
    });
    await expect(fetchLiveVoiceSettings()).resolves.toEqual({
      default_provider: 'sarvam',
      gemini: { model: null, voice: null, language: null },
      sarvam: { language: null, speaker: 'x', model: null },
      elevenlabs: { voice_id: null },
    });
    expect(mocks.callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.voice_live_settings_get' });
  });

  it('sends a settings patch and returns the full settings', async () => {
    mocks.callCoreRpc.mockResolvedValueOnce({
      default_provider: 'gemini',
      gemini: { model: 'm', voice: 'Puck', language: 'en-US' },
      sarvam: { language: null, speaker: null, model: null },
      elevenlabs: { voice_id: 'v' },
    });
    const result = await updateLiveVoiceSettings({ gemini: { voice: 'Puck' } });
    expect(mocks.callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.voice_live_settings_set',
      params: { gemini: { voice: 'Puck' } },
    });
    expect(result.gemini.voice).toBe('Puck');
    expect(result.elevenlabs.voice_id).toBe('v');
  });

  it('tests a provider with a long timeout and normalises the result', async () => {
    mocks.callCoreRpc.mockResolvedValueOnce({ ok: true, latency_ms: 312 });
    await expect(testLiveVoiceProvider('gemini')).resolves.toEqual({
      ok: true,
      latency_ms: 312,
      error: null,
    });
    expect(mocks.callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        method: 'openhuman.voice_live_test_provider',
        params: { provider: 'gemini' },
        timeoutMs: 30_000,
      })
    );

    mocks.callCoreRpc.mockResolvedValueOnce({ ok: false, latency_ms: null, error: 'bad key' });
    await expect(testLiveVoiceProvider('sarvam')).resolves.toEqual({
      ok: false,
      latency_ms: null,
      error: 'bad key',
    });
  });

  it('stores and removes BYOK keys under provider:<slug>', async () => {
    await saveLiveVoiceProviderKey('google', '  sk-123 \n');
    expect(mocks.store).toHaveBeenCalledWith({ provider: 'provider:google', token: 'sk-123' });
    await clearLiveVoiceProviderKey('sarvam');
    expect(mocks.remove).toHaveBeenCalledWith({ provider: 'provider:sarvam' });
  });
});
