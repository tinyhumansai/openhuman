/**
 * Unit tests for aiSettingsApi.ts
 *
 * All external deps (tauriCommands/auth, tauriCommands/config, coreRpcClient,
 * tauriCommands/common) are mocked so no Tauri runtime is needed.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

// ─── Import SUT after mocks ───────────────────────────────────────────────────

import {
  type AISettings,
  clearCloudProviderKey,
  listProviderModels,
  loadAISettings,
  parseProviderString,
  type ProviderRef,
  saveAISettings,
  serializeProviderRef,
  setCloudProviderKey,
} from '../aiSettingsApi';

// ─── Mock declarations (must be hoisted before imports) ───────────────────────

const mockOpenhumanGetClientConfig = vi.fn();
const mockAuthListProviderCredentials = vi.fn();
const mockOpenhumanUpdateModelSettings = vi.fn();
const mockAuthStoreProviderCredentials = vi.fn();
const mockAuthRemoveProviderCredentials = vi.fn();
const mockCallCoreRpc = vi.fn();
const mockIsTauri = vi.fn(() => true);

vi.mock('../../coreRpcClient', () => ({ callCoreRpc: (a: unknown) => mockCallCoreRpc(a) }));

vi.mock('../../../utils/tauriCommands/common', () => ({
  isTauri: () => mockIsTauri(),
  CommandResponse: {},
}));

vi.mock('../../../utils/tauriCommands/auth', () => ({
  authListProviderCredentials: (a?: unknown) => mockAuthListProviderCredentials(a),
  authStoreProviderCredentials: (a: unknown) => mockAuthStoreProviderCredentials(a),
  authRemoveProviderCredentials: (a: unknown) => mockAuthRemoveProviderCredentials(a),
}));

vi.mock('../../../utils/tauriCommands/config', () => ({
  openhumanGetClientConfig: () => mockOpenhumanGetClientConfig(),
  openhumanUpdateModelSettings: (a: unknown) => mockOpenhumanUpdateModelSettings(a),
  openhumanUpdateLocalAiSettings: vi.fn().mockResolvedValue({ result: {} }),
}));

vi.mock('../../../utils/tauriCommands/localAi', () => ({
  openhumanLocalAiStatus: vi.fn().mockResolvedValue({ result: null }),
  openhumanLocalAiDiagnostics: vi.fn().mockResolvedValue(null),
  openhumanLocalAiPresets: vi.fn().mockResolvedValue(null),
  openhumanLocalAiApplyPreset: vi.fn().mockResolvedValue({}),
  openhumanLocalAiDownload: vi.fn().mockResolvedValue({}),
  openhumanLocalAiSetOllamaPath: vi.fn().mockResolvedValue({}),
  openhumanLocalAiShutdownOwned: vi.fn().mockResolvedValue({}),
}));

// ─── Helpers ─────────────────────────────────────────────────────────────────

function makeClientConfigResult(overrides: Record<string, unknown> = {}) {
  return {
    result: {
      api_url: null,
      inference_url: null,
      default_model: null,
      app_version: '0.0.0-test',
      api_key_set: false,
      model_routes: [],
      cloud_providers: [],
      primary_cloud: null,
      reasoning_provider: null,
      agentic_provider: null,
      coding_provider: null,
      memory_provider: null,
      embeddings_provider: null,
      heartbeat_provider: null,
      learning_provider: null,
      subconscious_provider: null,
      ...overrides,
    },
  };
}

function makeAuthProfileResult(profiles: Array<{ id: string; provider: string }> = []) {
  return { result: profiles.map(p => ({ ...p, profile_name: 'default', kind: 'token' })) };
}

// ─── parseProviderString ─────────────────────────────────────────────────────

describe('parseProviderString', () => {
  it('returns openhuman for empty string', () => {
    expect(parseProviderString('')).toEqual({ kind: 'openhuman' });
  });

  it('returns openhuman for null/undefined', () => {
    expect(parseProviderString(null)).toEqual({ kind: 'openhuman' });
    expect(parseProviderString(undefined)).toEqual({ kind: 'openhuman' });
  });

  it('returns openhuman for the "cloud" sentinel', () => {
    expect(parseProviderString('cloud')).toEqual({ kind: 'openhuman' });
  });

  it('returns openhuman for the "openhuman" literal', () => {
    expect(parseProviderString('openhuman')).toEqual({ kind: 'openhuman' });
  });

  it('returns openhuman for "openhuman:<anything>"', () => {
    expect(parseProviderString('openhuman:gpt-4o')).toEqual({ kind: 'openhuman' });
  });

  it('parses ollama provider strings', () => {
    expect(parseProviderString('ollama:llama3.1:8b')).toEqual({
      kind: 'local',
      model: 'llama3.1:8b',
    });
  });

  it('parses cloud slug:model strings', () => {
    expect(parseProviderString('openai:gpt-4o')).toEqual({
      kind: 'cloud',
      providerSlug: 'openai',
      model: 'gpt-4o',
    });
    expect(parseProviderString('anthropic:claude-3-5-sonnet-20241022')).toEqual({
      kind: 'cloud',
      providerSlug: 'anthropic',
      model: 'claude-3-5-sonnet-20241022',
    });
  });

  it('falls back to openhuman for unrecognised bare strings', () => {
    expect(parseProviderString('unknown-provider')).toEqual({ kind: 'openhuman' });
  });
});

// ─── serializeProviderRef ─────────────────────────────────────────────────────

describe('serializeProviderRef', () => {
  it('serializes openhuman refs', () => {
    const ref: ProviderRef = { kind: 'openhuman' };
    expect(serializeProviderRef(ref)).toBe('openhuman');
  });

  it('serializes cloud refs to slug:model', () => {
    const ref: ProviderRef = { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o' };
    expect(serializeProviderRef(ref)).toBe('openai:gpt-4o');
  });

  it('serializes local refs to ollama:model', () => {
    const ref: ProviderRef = { kind: 'local', model: 'llama3.1:8b' };
    expect(serializeProviderRef(ref)).toBe('ollama:llama3.1:8b');
  });

  it('round-trips through parseProviderString', () => {
    const cases: ProviderRef[] = [
      { kind: 'openhuman' },
      { kind: 'cloud', providerSlug: 'anthropic', model: 'claude-3-haiku-20240307' },
      { kind: 'local', model: 'llama3:latest' },
    ];
    for (const ref of cases) {
      expect(parseProviderString(serializeProviderRef(ref))).toEqual(ref);
    }
  });
});

// ─── loadAISettings ──────────────────────────────────────────────────────────

describe('loadAISettings', () => {
  beforeEach(() => {
    mockOpenhumanGetClientConfig.mockReset();
    mockAuthListProviderCredentials.mockReset();
  });

  it('returns cloudProviders with has_api_key=false when no profiles stored', async () => {
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({
        cloud_providers: [
          {
            id: 'p_openai_1',
            slug: 'openai',
            label: 'OpenAI',
            endpoint: 'https://api.openai.com/v1',
            auth_style: 'bearer',
          },
        ],
      })
    );
    mockAuthListProviderCredentials.mockResolvedValue(makeAuthProfileResult([]));

    const settings = await loadAISettings();

    expect(settings.cloudProviders).toHaveLength(1);
    expect(settings.cloudProviders[0].slug).toBe('openai');
    expect(settings.cloudProviders[0].auth_style).toBe('bearer');
    expect(settings.cloudProviders[0].has_api_key).toBe(false);
  });

  it('sets has_api_key=true when a matching provider:<slug> profile is stored', async () => {
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({
        cloud_providers: [
          {
            id: 'p_anthropic_1',
            slug: 'anthropic',
            label: 'Anthropic',
            endpoint: 'https://api.anthropic.com/v1',
            auth_style: 'anthropic',
          },
        ],
      })
    );
    // New-style key format: "provider:<slug>"
    mockAuthListProviderCredentials.mockResolvedValue(
      makeAuthProfileResult([{ id: 'prof-1', provider: 'provider:anthropic' }])
    );

    const settings = await loadAISettings();

    expect(settings.cloudProviders[0].has_api_key).toBe(true);
    // auth_style must survive the round-trip unmodified.
    expect(settings.cloudProviders[0].auth_style).toBe('anthropic');
  });

  it('also accepts legacy bare-slug auth profiles', async () => {
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({
        cloud_providers: [
          {
            id: 'p_openai_2',
            slug: 'openai',
            label: 'OpenAI',
            endpoint: 'https://api.openai.com/v1',
            auth_style: 'bearer',
          },
        ],
      })
    );
    // Legacy format: bare slug, no "provider:" prefix
    mockAuthListProviderCredentials.mockResolvedValue(
      makeAuthProfileResult([{ id: 'prof-2', provider: 'openai' }])
    );

    const settings = await loadAISettings();
    expect(settings.cloudProviders[0].has_api_key).toBe(true);
  });

  it('parses non-default per-workload routing strings correctly', async () => {
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({
        cloud_providers: [],
        reasoning_provider: 'openai:gpt-4o',
        agentic_provider: 'anthropic:claude-3-5-sonnet-20241022',
        coding_provider: 'ollama:codellama:13b',
        memory_provider: null,
        embeddings_provider: null,
        heartbeat_provider: null,
        learning_provider: null,
        subconscious_provider: null,
      })
    );
    mockAuthListProviderCredentials.mockResolvedValue(makeAuthProfileResult([]));

    const settings = await loadAISettings();

    expect(settings.routing.reasoning).toEqual({
      kind: 'cloud',
      providerSlug: 'openai',
      model: 'gpt-4o',
    });
    expect(settings.routing.agentic).toEqual({
      kind: 'cloud',
      providerSlug: 'anthropic',
      model: 'claude-3-5-sonnet-20241022',
    });
    expect(settings.routing.coding).toEqual({ kind: 'local', model: 'codellama:13b' });
    expect(settings.routing.memory).toEqual({ kind: 'openhuman' });
  });

  it('degrades gracefully when authListProviderCredentials throws', async () => {
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({
        cloud_providers: [
          {
            id: 'p_openai_3',
            slug: 'openai',
            label: 'OpenAI',
            endpoint: 'https://api.openai.com/v1',
            auth_style: 'bearer',
          },
        ],
      })
    );
    mockAuthListProviderCredentials.mockRejectedValue(new Error('no profiles file'));

    const settings = await loadAISettings();

    // Should not throw; has_api_key should default to false.
    expect(settings.cloudProviders[0].has_api_key).toBe(false);
  });

  it('includes two cloud providers with correct labels and endpoints', async () => {
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({
        cloud_providers: [
          {
            id: 'p_openai_4',
            slug: 'openai',
            label: 'OpenAI',
            endpoint: 'https://api.openai.com/v1',
            auth_style: 'bearer',
          },
          {
            id: 'p_anthropic_4',
            slug: 'anthropic',
            label: 'Anthropic',
            endpoint: 'https://api.anthropic.com/v1',
            auth_style: 'anthropic',
          },
        ],
        reasoning_provider: 'openai:gpt-4o',
        agentic_provider: 'anthropic:claude-3-5-sonnet-20241022',
      })
    );
    mockAuthListProviderCredentials.mockResolvedValue(
      makeAuthProfileResult([
        { id: 'prof-openai', provider: 'provider:openai' },
        { id: 'prof-anthropic', provider: 'provider:anthropic' },
      ])
    );

    const settings = await loadAISettings();

    expect(settings.cloudProviders).toHaveLength(2);
    const openai = settings.cloudProviders.find(p => p.slug === 'openai')!;
    const anthropic = settings.cloudProviders.find(p => p.slug === 'anthropic')!;

    expect(openai.label).toBe('OpenAI');
    expect(openai.endpoint).toBe('https://api.openai.com/v1');
    expect(openai.auth_style).toBe('bearer');
    expect(openai.has_api_key).toBe(true);

    expect(anthropic.label).toBe('Anthropic');
    expect(anthropic.endpoint).toBe('https://api.anthropic.com/v1');
    expect(anthropic.auth_style).toBe('anthropic');
    expect(anthropic.has_api_key).toBe(true);

    expect(settings.routing.reasoning).toEqual({
      kind: 'cloud',
      providerSlug: 'openai',
      model: 'gpt-4o',
    });
    expect(settings.routing.agentic).toEqual({
      kind: 'cloud',
      providerSlug: 'anthropic',
      model: 'claude-3-5-sonnet-20241022',
    });
  });
});

// ─── saveAISettings ──────────────────────────────────────────────────────────

describe('saveAISettings', () => {
  beforeEach(() => {
    mockOpenhumanUpdateModelSettings.mockReset();
    mockOpenhumanUpdateModelSettings.mockResolvedValue({ result: {} });
  });

  function makeSettings(overrides: Partial<AISettings> = {}): AISettings {
    return {
      cloudProviders: [
        {
          id: 'p_openai_1',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer',
          has_api_key: true,
        },
      ],
      routing: {
        reasoning: { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o' },
        agentic: { kind: 'openhuman' },
        coding: { kind: 'openhuman' },
        memory: { kind: 'openhuman' },
        embeddings: { kind: 'openhuman' },
        heartbeat: { kind: 'openhuman' },
        learning: { kind: 'openhuman' },
        subconscious: { kind: 'openhuman' },
      },
      ...overrides,
    };
  }

  it('issues no RPC call when nothing changed', async () => {
    const settings = makeSettings();
    await saveAISettings(settings, settings);
    expect(mockOpenhumanUpdateModelSettings).not.toHaveBeenCalled();
  });

  it('sends only changed routing fields when providers are unchanged', async () => {
    const prev = makeSettings();
    const next = makeSettings({ routing: { ...prev.routing, reasoning: { kind: 'openhuman' } } });

    await saveAISettings(prev, next);

    expect(mockOpenhumanUpdateModelSettings).toHaveBeenCalledOnce();
    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.reasoning_provider).toBe('openhuman');
    // Other workloads unchanged — should not appear in patch.
    expect(patch.agentic_provider).toBeUndefined();
    expect(patch.cloud_providers).toBeUndefined();
  });

  it('sends cloud_providers list when a provider is added', async () => {
    const prev = makeSettings({ cloudProviders: [] });
    const next = makeSettings();

    await saveAISettings(prev, next);

    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.cloud_providers).toHaveLength(1);
    expect(patch.cloud_providers![0].slug).toBe('openai');
    // has_api_key must NOT be present in the wire payload — it's not part of
    // CloudProviderCreds.
    expect(patch.cloud_providers![0]).not.toHaveProperty('has_api_key');
  });

  it('preserves auth_style through save round-trip for anthropic', async () => {
    const anthropicProvider = {
      id: 'p_anthropic_1',
      slug: 'anthropic',
      label: 'Anthropic',
      endpoint: 'https://api.anthropic.com/v1',
      auth_style: 'anthropic' as const,
      has_api_key: true,
    };
    const prev: AISettings = {
      cloudProviders: [],
      routing: {
        reasoning: { kind: 'openhuman' },
        agentic: { kind: 'openhuman' },
        coding: { kind: 'openhuman' },
        memory: { kind: 'openhuman' },
        embeddings: { kind: 'openhuman' },
        heartbeat: { kind: 'openhuman' },
        learning: { kind: 'openhuman' },
        subconscious: { kind: 'openhuman' },
      },
    };
    const next: AISettings = { cloudProviders: [anthropicProvider], routing: { ...prev.routing } };

    await saveAISettings(prev, next);

    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.cloud_providers![0].auth_style).toBe('anthropic');
  });

  it('sends both providers and routing when both change', async () => {
    const prev = makeSettings({ cloudProviders: [] });
    const next = makeSettings({
      routing: {
        ...makeSettings().routing,
        coding: { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o-mini' },
      },
    });

    await saveAISettings(prev, next);

    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.cloud_providers).toBeDefined();
    expect(patch.coding_provider).toBe('openai:gpt-4o-mini');
  });
});

// ─── setCloudProviderKey ──────────────────────────────────────────────────────

describe('setCloudProviderKey', () => {
  beforeEach(() => {
    mockAuthStoreProviderCredentials.mockReset();
    mockAuthStoreProviderCredentials.mockResolvedValue({ result: {} });
  });

  it('calls authStoreProviderCredentials with provider:<slug> key format', async () => {
    await setCloudProviderKey('openai', 'sk-test-key');

    expect(mockAuthStoreProviderCredentials).toHaveBeenCalledOnce();
    const args = mockAuthStoreProviderCredentials.mock.calls[0][0];
    expect(args.provider).toBe('provider:openai');
    expect(args.token).toBe('sk-test-key');
    expect(args.profile).toBe('default');
    expect(args.setActive).toBe(true);
  });

  it('throws when slug is "openhuman" (session JWT — not configurable)', async () => {
    await expect(setCloudProviderKey('openhuman', 'some-key')).rejects.toThrow();
    expect(mockAuthStoreProviderCredentials).not.toHaveBeenCalled();
  });

  it('uses provider:<slug> namespace for anthropic slug', async () => {
    await setCloudProviderKey('anthropic', 'sk-ant-key');
    const args = mockAuthStoreProviderCredentials.mock.calls[0][0];
    expect(args.provider).toBe('provider:anthropic');
  });
});

// ─── clearCloudProviderKey ────────────────────────────────────────────────────

describe('clearCloudProviderKey', () => {
  beforeEach(() => {
    mockAuthRemoveProviderCredentials.mockReset();
    mockAuthRemoveProviderCredentials.mockResolvedValue({ result: { removed: true } });
  });

  it('calls authRemoveProviderCredentials with provider:<slug> format', async () => {
    await clearCloudProviderKey('openai');

    expect(mockAuthRemoveProviderCredentials).toHaveBeenCalledOnce();
    const args = mockAuthRemoveProviderCredentials.mock.calls[0][0];
    expect(args.provider).toBe('provider:openai');
    expect(args.profile).toBe('default');
  });

  it('is a no-op for "openhuman" (session-managed, no key to clear)', async () => {
    await clearCloudProviderKey('openhuman');
    expect(mockAuthRemoveProviderCredentials).not.toHaveBeenCalled();
  });
});

// ─── listProviderModels ───────────────────────────────────────────────────────

describe('listProviderModels', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
    mockIsTauri.mockReturnValue(true);
  });

  it('dispatches openhuman.providers_list_models with provider_id and returns models', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: {
        models: [
          { id: 'gpt-4o', owned_by: 'openai', context_window: 128000 },
          { id: 'gpt-4o-mini', owned_by: 'openai', context_window: 128000 },
        ],
      },
    });

    const models = await listProviderModels('p_openai_1');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.providers_list_models',
      params: { provider_id: 'p_openai_1' },
    });
    expect(models).toHaveLength(2);
    expect(models[0].id).toBe('gpt-4o');
    expect(models[1].id).toBe('gpt-4o-mini');
  });

  it('returns empty array when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    const models = await listProviderModels('p_openai_1');

    expect(models).toEqual([]);
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  it('returns empty array on RPC error (graceful degradation)', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('network error'));

    const models = await listProviderModels('p_openai_1');

    expect(models).toEqual([]);
  });

  it('returns empty array when result has no models field', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: {} });

    const models = await listProviderModels('p_openai_1');

    expect(models).toEqual([]);
  });
});
