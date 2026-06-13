import { beforeEach, describe, expect, it, vi } from 'vitest';

import { applyOpenRouterFreeModels, OPENROUTER_FREE_MODEL } from '../openrouterFreeModels';

const {
  mockConnectOpenRouterViaOAuth,
  mockLoadAISettings,
  mockSaveAISettings,
  mockSetCloudProviderKey,
  mockRequestUsageRefresh,
} = vi.hoisted(() => ({
  mockConnectOpenRouterViaOAuth: vi.fn(),
  mockLoadAISettings: vi.fn(),
  mockSaveAISettings: vi.fn(),
  mockSetCloudProviderKey: vi.fn(),
  mockRequestUsageRefresh: vi.fn(),
}));

vi.mock('../../../utils/openrouterOAuth', () => ({
  connectOpenRouterViaOAuth: () => mockConnectOpenRouterViaOAuth(),
}));

vi.mock('../../../hooks/usageRefresh', () => ({
  requestUsageRefresh: () => mockRequestUsageRefresh(),
}));

vi.mock('../aiSettingsApi', async () => {
  const actual = await vi.importActual<typeof import('../aiSettingsApi')>('../aiSettingsApi');
  return {
    ...actual,
    loadAISettings: () => mockLoadAISettings(),
    saveAISettings: (...args: unknown[]) => mockSaveAISettings(...args),
    setCloudProviderKey: (...args: unknown[]) => mockSetCloudProviderKey(...args),
  };
});

function settings(hasOpenRouterKey = false) {
  return {
    cloudProviders: hasOpenRouterKey
      ? [
          {
            id: 'p_openrouter',
            slug: 'openrouter',
            label: 'OpenRouter',
            endpoint: 'https://openrouter.ai/api/v1',
            auth_style: 'bearer',
            has_api_key: true,
          },
        ]
      : [],
    routing: {
      chat: { kind: 'openhuman' },
      reasoning: { kind: 'openhuman' },
      agentic: { kind: 'openhuman' },
      coding: { kind: 'openhuman' },
      memory: { kind: 'openhuman' },
      heartbeat: { kind: 'openhuman' },
      learning: { kind: 'openhuman' },
      subconscious: { kind: 'openhuman' },
    },
  };
}

describe('applyOpenRouterFreeModels', () => {
  beforeEach(() => {
    mockConnectOpenRouterViaOAuth.mockReset();
    mockLoadAISettings.mockReset();
    mockSaveAISettings.mockReset();
    mockSetCloudProviderKey.mockReset();
    mockRequestUsageRefresh.mockReset();
  });

  it('routes chat workloads to the OpenRouter free router when a key already exists', async () => {
    const current = settings(true);
    mockLoadAISettings.mockResolvedValue(current);

    await applyOpenRouterFreeModels();

    expect(mockConnectOpenRouterViaOAuth).not.toHaveBeenCalled();
    expect(mockSetCloudProviderKey).not.toHaveBeenCalled();
    expect(mockSaveAISettings).toHaveBeenCalledWith(
      current,
      expect.objectContaining({
        routing: expect.objectContaining({
          chat: { kind: 'cloud', providerSlug: 'openrouter', model: OPENROUTER_FREE_MODEL },
          reasoning: { kind: 'cloud', providerSlug: 'openrouter', model: OPENROUTER_FREE_MODEL },
          agentic: { kind: 'cloud', providerSlug: 'openrouter', model: OPENROUTER_FREE_MODEL },
          coding: { kind: 'cloud', providerSlug: 'openrouter', model: OPENROUTER_FREE_MODEL },
          memory: { kind: 'openhuman' },
        }),
      })
    );
    expect(mockRequestUsageRefresh).toHaveBeenCalledTimes(1);
  });

  it('connects OpenRouter before routing when no key is stored', async () => {
    const current = settings(false);
    mockLoadAISettings.mockResolvedValue(current);
    mockConnectOpenRouterViaOAuth.mockResolvedValue('sk-or-test');

    await applyOpenRouterFreeModels();

    expect(mockConnectOpenRouterViaOAuth).toHaveBeenCalledTimes(1);
    expect(mockSetCloudProviderKey).toHaveBeenCalledWith('openrouter', 'sk-or-test');
    expect(mockSaveAISettings).toHaveBeenCalledWith(
      current,
      expect.objectContaining({
        cloudProviders: [
          expect.objectContaining({
            slug: 'openrouter',
            endpoint: 'https://openrouter.ai/api/v1',
            has_api_key: true,
          }),
        ],
      })
    );
  });
});
