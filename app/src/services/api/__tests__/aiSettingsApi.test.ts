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
  completeOpenAiCodexOAuth,
  flushCloudProviders,
  importOpenAiCodexCliAuth,
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  loadProviderAuthErrors,
  localProvider,
  modelRegistryVision,
  OPENAI_CODEX_OAUTH_MISSING_AUTH_URL,
  OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL,
  parseProviderString,
  type ProviderRef,
  saveAISettings,
  serializeProviderRef,
  setCloudProviderKey,
  setLocalRuntimeEnabled,
  startOpenAiCodexOAuth,
  testProviderModel,
  upsertModelRegistryVision,
} from '../aiSettingsApi';

// ─── Mock declarations (must be hoisted before imports) ───────────────────────

const mockOpenhumanGetClientConfig = vi.fn();
const mockAuthListProviderCredentials = vi.fn();
const mockOpenhumanUpdateModelSettings = vi.fn();
const mockOpenhumanUpdateLocalAiSettings = vi.fn();
const mockAuthStoreProviderCredentials = vi.fn();
const mockAuthRemoveProviderCredentials = vi.fn();
const mockCallCoreRpc = vi.fn();
const mockIsTauri = vi.fn(() => true);
const mockOpenhumanLocalAiStatus = vi.fn();
const mockOpenhumanLocalAiDiagnostics = vi.fn();
const mockOpenhumanLocalAiPresets = vi.fn();
const mockOpenhumanLocalAiApplyPreset = vi.fn();

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
  openhumanUpdateLocalAiSettings: (a: unknown) => mockOpenhumanUpdateLocalAiSettings(a),
}));

vi.mock('../../../utils/tauriCommands/localAi', () => ({
  openhumanLocalAiStatus: (...args: unknown[]) => mockOpenhumanLocalAiStatus(...args),
  openhumanLocalAiDiagnostics: (...args: unknown[]) => mockOpenhumanLocalAiDiagnostics(...args),
  openhumanLocalAiPresets: (...args: unknown[]) => mockOpenhumanLocalAiPresets(...args),
  openhumanLocalAiApplyPreset: (...args: unknown[]) => mockOpenhumanLocalAiApplyPreset(...args),
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
      model_registry: [],
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
  it('returns default for empty string', () => {
    expect(parseProviderString('')).toEqual({ kind: 'default' });
  });

  it('returns default for null/undefined', () => {
    expect(parseProviderString(null)).toEqual({ kind: 'default' });
    expect(parseProviderString(undefined)).toEqual({ kind: 'default' });
  });

  it('returns default for the "cloud" sentinel', () => {
    expect(parseProviderString('cloud')).toEqual({ kind: 'default' });
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

  // The `@<temp>` suffix is the per-workload temperature override added with
  // the LLM-routing UI redesign. It must round-trip through parse/serialize
  // and degrade gracefully when the tail isn't a finite number.
  describe('temperature suffix grammar', () => {
    it('parses @temp suffix on cloud strings', () => {
      expect(parseProviderString('openai:gpt-4o@0.7')).toEqual({
        kind: 'cloud',
        providerSlug: 'openai',
        model: 'gpt-4o',
        temperature: 0.7,
      });
    });

    it('parses @temp suffix on ollama strings (including model ids with colons)', () => {
      expect(parseProviderString('ollama:llama3.1:8b@0.2')).toEqual({
        kind: 'local',
        model: 'llama3.1:8b',
        temperature: 0.2,
      });
    });

    it('treats a non-numeric @tail as part of the model id', () => {
      // Guards against silently dropping a chunk of the model id when the
      // user happens to pick a tag like `:beta` after an `@`.
      expect(parseProviderString('openai:gpt@beta')).toEqual({
        kind: 'cloud',
        providerSlug: 'openai',
        model: 'gpt@beta',
      });
    });

    it('drops the temperature key when not configured (toEqual contract)', () => {
      // Existing call sites compare with toEqual — emitting an extra
      // `temperature: null` would break unrelated snapshots.
      const ref = parseProviderString('openai:gpt-4o');
      expect(ref).toEqual({ kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o' });
    });
  });
});

// ─── serializeProviderRef ─────────────────────────────────────────────────────

describe('serializeProviderRef', () => {
  it('serializes openhuman refs', () => {
    const ref: ProviderRef = { kind: 'openhuman' };
    expect(serializeProviderRef(ref)).toBe('openhuman');
  });

  it('serializes default refs', () => {
    const ref: ProviderRef = { kind: 'default' };
    expect(serializeProviderRef(ref)).toBe('cloud');
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
      { kind: 'default' },
      { kind: 'cloud', providerSlug: 'anthropic', model: 'claude-3-haiku-20240307' },
      { kind: 'local', model: 'llama3:latest' },
    ];
    for (const ref of cases) {
      expect(parseProviderString(serializeProviderRef(ref))).toEqual(ref);
    }
  });

  it('appends @temp suffix when temperature is set, omits when not', () => {
    expect(serializeProviderRef({ kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o' })).toBe(
      'openai:gpt-4o'
    );
    expect(
      serializeProviderRef({
        kind: 'cloud',
        providerSlug: 'openai',
        model: 'gpt-4o',
        temperature: 0.7,
      })
    ).toBe('openai:gpt-4o@0.7');
    expect(serializeProviderRef({ kind: 'local', model: 'llama3', temperature: 1.25 })).toBe(
      'ollama:llama3@1.25'
    );
  });

  it('rounds temperature to 2 decimal places on the wire', () => {
    // Stops floating-point drift (0.7 + 0.0000001) from leaking into the
    // persisted provider string and confusing the Rust factory.
    expect(
      serializeProviderRef({
        kind: 'cloud',
        providerSlug: 'openai',
        model: 'gpt-4o',
        temperature: 0.7000001,
      })
    ).toBe('openai:gpt-4o@0.7');
  });

  it('treats non-finite temperatures as unset', () => {
    expect(serializeProviderRef({ kind: 'local', model: 'llama3', temperature: Number.NaN })).toBe(
      'ollama:llama3'
    );
  });

  it('round-trips temperature through parse + serialize', () => {
    const ref: ProviderRef = {
      kind: 'cloud',
      providerSlug: 'openai',
      model: 'gpt-4o',
      temperature: 0.2,
    };
    expect(parseProviderString(serializeProviderRef(ref))).toEqual(ref);
  });
});

// ─── loadAISettings ──────────────────────────────────────────────────────────

describe('loadAISettings', () => {
  beforeEach(() => {
    mockOpenhumanGetClientConfig.mockReset();
    mockAuthListProviderCredentials.mockReset();
    mockOpenhumanUpdateLocalAiSettings.mockReset();
    mockOpenhumanLocalAiStatus.mockReset();
    mockOpenhumanLocalAiDiagnostics.mockReset();
    mockOpenhumanLocalAiPresets.mockReset();
    mockOpenhumanLocalAiApplyPreset.mockReset();
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

  it('parses per-tier credits_bypass into creditsBypass, defaulting to false (#3767)', async () => {
    mockAuthListProviderCredentials.mockResolvedValue(makeAuthProfileResult([]));

    // Absent in an older snapshot → both tiers conservative false.
    mockOpenhumanGetClientConfig.mockResolvedValue(makeClientConfigResult({}));
    expect((await loadAISettings()).creditsBypass).toEqual({ chat: false, reasoning: false });

    // Per-tier: chat true, reasoning absent → chat true, reasoning false.
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({ credits_bypass: { chat: true } })
    );
    expect((await loadAISettings()).creditsBypass).toEqual({ chat: true, reasoning: false });

    // Both present.
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({ credits_bypass: { chat: true, reasoning: true } })
    );
    expect((await loadAISettings()).creditsBypass).toEqual({ chat: true, reasoning: true });
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
    expect(settings.routing.memory).toEqual({ kind: 'default' });
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

  it('keeps local runtime endpoint providers so the AI panel can edit them', async () => {
    mockOpenhumanGetClientConfig.mockResolvedValue(
      makeClientConfigResult({
        cloud_providers: [
          {
            id: 'p_ollama_1',
            slug: 'ollama',
            label: 'Ollama',
            endpoint: 'http://127.0.0.1:11434/v1',
            auth_style: 'none',
          },
        ],
      })
    );
    mockAuthListProviderCredentials.mockResolvedValue(makeAuthProfileResult([]));

    const settings = await loadAISettings();

    expect(settings.cloudProviders).toHaveLength(1);
    expect(settings.cloudProviders[0].slug).toBe('ollama');
    expect(settings.cloudProviders[0].endpoint).toBe('http://127.0.0.1:11434/v1');
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

describe('local provider facade', () => {
  beforeEach(() => {
    mockOpenhumanUpdateLocalAiSettings.mockReset();
    mockOpenhumanLocalAiStatus.mockReset();
    mockOpenhumanLocalAiDiagnostics.mockReset();
    mockOpenhumanLocalAiPresets.mockReset();
    mockOpenhumanLocalAiApplyPreset.mockReset();
  });

  it('loadLocalProviderSnapshot joins status diagnostics and presets', async () => {
    mockOpenhumanLocalAiStatus.mockResolvedValue({ result: { state: 'ready' } });
    mockOpenhumanLocalAiDiagnostics.mockResolvedValue({
      installed_models: [{ name: 'gemma3:1b-it-qat', size: 123 }],
    });
    mockOpenhumanLocalAiPresets.mockResolvedValue({
      recommended_tier: 'ram_2_4gb',
      current_tier: 'ram_2_4gb',
      selected_tier: 'ram_2_4gb',
      presets: [],
      device: {
        total_ram_bytes: 1,
        cpu_count: 1,
        cpu_brand: 'cpu',
        os_name: 'os',
        os_version: '1',
        has_gpu: false,
        gpu_description: null,
      },
    });

    const snapshot = await loadLocalProviderSnapshot();

    expect(snapshot.status).toEqual({ state: 'ready' });
    expect(snapshot.installedModels).toEqual([{ name: 'gemma3:1b-it-qat', size: 123 }]);
    expect(snapshot.presets?.recommended_tier).toBe('ram_2_4gb');
  });

  it('setLocalRuntimeEnabled updates runtime_enabled and opt_in_confirmed together', async () => {
    mockOpenhumanUpdateLocalAiSettings.mockResolvedValue({ result: {} });

    await setLocalRuntimeEnabled(true);

    expect(mockOpenhumanUpdateLocalAiSettings).toHaveBeenCalledWith({
      runtime_enabled: true,
      opt_in_confirmed: true,
    });
  });

  it('localProvider facade delegates applyPreset and setEnabled', async () => {
    mockOpenhumanLocalAiApplyPreset.mockResolvedValue({ applied_tier: 'ram_2_4gb' });
    mockOpenhumanUpdateLocalAiSettings.mockResolvedValue({ result: {} });

    await localProvider.applyPreset('ram_2_4gb');
    await localProvider.setEnabled(false);

    expect(mockOpenhumanLocalAiApplyPreset).toHaveBeenCalledWith('ram_2_4gb');
    expect(mockOpenhumanUpdateLocalAiSettings).toHaveBeenCalledWith({
      runtime_enabled: false,
      opt_in_confirmed: false,
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
        chat: { kind: 'openhuman' },
        reasoning: { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o' },
        agentic: { kind: 'openhuman' },
        coding: { kind: 'openhuman' },
        vision: { kind: 'openhuman' },
        memory: { kind: 'openhuman' },

        heartbeat: { kind: 'openhuman' },
        learning: { kind: 'openhuman' },
        subconscious: { kind: 'openhuman' },
      },
      modelRegistry: [],
      creditsBypass: { chat: false, reasoning: false },
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

  it('preserves local runtime providers in the cloud_providers payload', async () => {
    const prev = makeSettings({ cloudProviders: [] });
    const next = makeSettings({
      cloudProviders: [
        {
          id: 'p_ollama_1',
          slug: 'ollama',
          label: 'Ollama',
          endpoint: 'http://127.0.0.1:11434/v1',
          auth_style: 'none',
          has_api_key: true,
        },
      ],
    });

    await saveAISettings(prev, next);

    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.cloud_providers).toHaveLength(1);
    expect(patch.cloud_providers![0]).toMatchObject({
      slug: 'ollama',
      endpoint: 'http://127.0.0.1:11434/v1',
      auth_style: 'none',
    });
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
        chat: { kind: 'openhuman' },
        reasoning: { kind: 'openhuman' },
        agentic: { kind: 'openhuman' },
        coding: { kind: 'openhuman' },
        vision: { kind: 'openhuman' },
        memory: { kind: 'openhuman' },

        heartbeat: { kind: 'openhuman' },
        learning: { kind: 'openhuman' },
        subconscious: { kind: 'openhuman' },
      },
      modelRegistry: [],
    };
    const next: AISettings = {
      cloudProviders: [anthropicProvider],
      routing: { ...prev.routing },
      modelRegistry: [],
    };

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
        vision: { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o-mini' },
      },
    });

    await saveAISettings(prev, next);

    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.cloud_providers).toBeDefined();
    expect(patch.coding_provider).toBe('openai:gpt-4o-mini');
    expect(patch.vision_provider).toBe('openai:gpt-4o-mini');
  });

  it('sends model_registry when the vision flag changes', async () => {
    const prev = makeSettings({ modelRegistry: [] });
    const next = makeSettings({
      modelRegistry: [{ id: 'my-llava', provider: 'openai', cost_per_1m_output: 0, vision: true }],
    });
    await saveAISettings(prev, next);
    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.model_registry).toEqual([
      {
        id: 'my-llava',
        provider: 'openai',
        cost_per_1m_input: 0,
        cost_per_1m_cached_input: 0,
        cost_per_1m_output: 0,
        context_window: 0,
        vision: true,
      },
    ]);
  });

  it('omits model_registry when unchanged', async () => {
    const registry = [{ id: 'my-llava', provider: 'openai', cost_per_1m_output: 0, vision: true }];
    const prev = makeSettings({ modelRegistry: registry });
    const next = makeSettings({
      modelRegistry: [...registry],
      routing: {
        ...makeSettings().routing,
        coding: { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o-mini' },
        vision: { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o-mini' },
      },
    });
    await saveAISettings(prev, next);
    const patch = mockOpenhumanUpdateModelSettings.mock.calls[0][0];
    expect(patch.model_registry).toBeUndefined();
    expect(patch.coding_provider).toBe('openai:gpt-4o-mini');
    expect(patch.vision_provider).toBe('openai:gpt-4o-mini');
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

// ─── OpenAI Codex OAuth helpers ──────────────────────────────────────────────

describe('OpenAI Codex OAuth helpers', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('throws a stable code when OAuth start returns no authorization URL', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: {} });

    await expect(startOpenAiCodexOAuth()).rejects.toThrow(OPENAI_CODEX_OAUTH_MISSING_AUTH_URL);
  });

  it('returns the OAuth start payload when an authorization URL is present', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { authUrl: '  https://auth.openai.com/oauth/authorize?client_id=test  ' },
    });

    await expect(startOpenAiCodexOAuth()).resolves.toEqual({
      authUrl: '  https://auth.openai.com/oauth/authorize?client_id=test  ',
    });
  });

  it('throws a stable code when OAuth completion is missing the callback URL', async () => {
    await expect(completeOpenAiCodexOAuth('  ')).rejects.toThrow(
      OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL
    );

    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  it('completes OAuth with a trimmed callback URL', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: {} });

    await completeOpenAiCodexOAuth('  openhuman://oauth/callback?code=abc  ');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_openai_oauth_complete',
      params: { callback_url: 'openhuman://oauth/callback?code=abc' },
    });
  });

  it('imports Codex CLI auth through core RPC', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: {} });

    await importOpenAiCodexCliAuth();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_openai_oauth_import_codex_cli',
      params: {},
    });
  });
});

// ─── listProviderModels ───────────────────────────────────────────────────────

describe('listProviderModels', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
    mockIsTauri.mockReturnValue(true);
  });

  it('dispatches openhuman.inference_list_models with provider slug and returns models', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: {
        models: [
          { id: 'gpt-4o', owned_by: 'openai', context_window: 128000 },
          { id: 'gpt-4o-mini', owned_by: 'openai', context_window: 128000 },
        ],
      },
    });

    const models = await listProviderModels('openai');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_list_models',
      params: { provider_id: 'openai' },
    });
    expect(models).toHaveLength(2);
    expect(models[0].id).toBe('gpt-4o');
    expect(models[1].id).toBe('gpt-4o-mini');
  });

  it('returns empty array when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    const models = await listProviderModels('openai');

    expect(models).toEqual([]);
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  it('throws on RPC error so callers can surface retry UI', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('network error'));

    await expect(listProviderModels('openai')).rejects.toThrow('network error');
  });

  it('returns empty array when result has no models field', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: {} });

    const models = await listProviderModels('openai');

    expect(models).toEqual([]);
  });
});

describe('loadProviderAuthErrors', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
    mockIsTauri.mockReturnValue(true);
  });

  it('dispatches openhuman.inference_provider_auth_errors and returns the errors', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: {
        errors: [
          {
            provider: 'openrouter',
            status: 401,
            message: 'openrouter rejected the API key (HTTP 401). Update it in Settings → AI.',
            timestamp_ms: 1000,
          },
        ],
      },
    });

    const errors = await loadProviderAuthErrors();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_provider_auth_errors',
      params: {},
    });
    expect(errors).toHaveLength(1);
    expect(errors[0].provider).toBe('openrouter');
    expect(errors[0].status).toBe(401);
  });

  it('returns empty array when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    const errors = await loadProviderAuthErrors();

    expect(errors).toEqual([]);
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });

  it('returns empty array when result has no errors field', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: {} });

    const errors = await loadProviderAuthErrors();

    expect(errors).toEqual([]);
  });
});

describe('testProviderModel', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
    mockIsTauri.mockReturnValue(true);
  });

  it('dispatches openhuman.inference_test_provider_model and returns the reply', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { reply: 'Hello from model' } });

    const result = await testProviderModel('reasoning', 'openai:gpt-4o');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.inference_test_provider_model',
      params: { workload: 'reasoning', provider: 'openai:gpt-4o', prompt: 'Hello world' },
      timeoutMs: 120000,
    });
    expect(result).toEqual({ reply: 'Hello from model' });
  });

  it('throws when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    await expect(testProviderModel('reasoning', 'openai:gpt-4o')).rejects.toThrow(
      'Model testing is only available in the desktop app.'
    );
    expect(mockCallCoreRpc).not.toHaveBeenCalled();
  });
});

// ─── flushCloudProviders ──────────────────────────────────────────────────────

describe('flushCloudProviders', () => {
  beforeEach(() => {
    mockOpenhumanUpdateModelSettings.mockReset();
    mockIsTauri.mockReturnValue(true);
  });

  it('calls update_model_settings with the cloud_providers array', async () => {
    mockOpenhumanUpdateModelSettings.mockResolvedValue({});
    const providers = [
      {
        id: 'p_openai_1',
        slug: 'openai',
        label: 'OpenAI',
        endpoint: 'https://api.openai.com/v1',
        auth_style: 'bearer' as const,
      },
    ];
    await flushCloudProviders(providers);
    expect(mockOpenhumanUpdateModelSettings).toHaveBeenCalledWith({ cloud_providers: providers });
  });

  it('no-ops when not running in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await flushCloudProviders([]);
    expect(mockOpenhumanUpdateModelSettings).not.toHaveBeenCalled();
  });
});

describe('model registry vision helpers', () => {
  const reg = [
    { id: 'gpt-4o', provider: 'openai', cost_per_1m_output: 0, vision: true },
    { id: 'text-only', provider: 'openai', cost_per_1m_output: 0, vision: false },
  ];

  it('modelRegistryVision matches by (provider, id)', () => {
    expect(modelRegistryVision(reg, 'openai', 'gpt-4o')).toBe(true);
    expect(modelRegistryVision(reg, 'openai', 'text-only')).toBe(false);
    expect(modelRegistryVision(reg, 'openai', 'unlisted')).toBe(false);
    expect(modelRegistryVision(reg, 'azure', 'gpt-4o')).toBe(false);
  });

  it('upsertModelRegistryVision adds, flips, and removes entries', () => {
    const added = upsertModelRegistryVision([], 'openai', 'my-llava', true);
    expect(added).toEqual([
      {
        id: 'my-llava',
        provider: 'openai',
        cost_per_1m_input: 0,
        cost_per_1m_cached_input: 0,
        cost_per_1m_output: 0,
        context_window: 0,
        vision: true,
      },
    ]);
    // vision:false removes the entry (absence ⇒ no vision).
    const removed = upsertModelRegistryVision(reg, 'openai', 'gpt-4o', false);
    expect(removed.find(e => e.id === 'gpt-4o')).toBeUndefined();
    expect(removed.find(e => e.id === 'text-only')).toBeDefined();
    // Flipping an existing entry on stays idempotent on the key.
    const flipped = upsertModelRegistryVision(reg, 'openai', 'text-only', true);
    expect(flipped.filter(e => e.id === 'text-only')).toHaveLength(1);
    expect(modelRegistryVision(flipped, 'openai', 'text-only')).toBe(true);
  });
});
