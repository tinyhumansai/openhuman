import { requestUsageRefresh } from '../../hooks/usageRefresh';
import { connectOpenRouterViaOAuth } from '../../utils/openrouterOAuth';
import {
  type AISettings,
  CHAT_WORKLOADS,
  type CloudProviderView,
  loadAISettings,
  type ProviderRef,
  saveAISettings,
  setCloudProviderKey,
} from './aiSettingsApi';

export const OPENROUTER_FREE_MODEL = 'openrouter/free';

function openRouterProvider(): CloudProviderView {
  return {
    id: 'builtin_openrouter',
    slug: 'openrouter',
    label: 'OpenRouter',
    endpoint: 'https://openrouter.ai/api/v1',
    auth_style: 'bearer',
    has_api_key: true,
  };
}

function withOpenRouterProvider(settings: AISettings): AISettings {
  const existing = settings.cloudProviders.find(p => p.slug === 'openrouter');
  if (existing) {
    return {
      ...settings,
      cloudProviders: settings.cloudProviders.map(p =>
        p.slug === 'openrouter' ? { ...p, has_api_key: true } : p
      ),
    };
  }
  return { ...settings, cloudProviders: [...settings.cloudProviders, openRouterProvider()] };
}

export async function applyOpenRouterFreeModels(): Promise<void> {
  const current = await loadAISettings();
  const existingOpenRouter = current.cloudProviders.find(p => p.slug === 'openrouter');
  if (!existingOpenRouter?.has_api_key) {
    const apiKey = await connectOpenRouterViaOAuth();
    await setCloudProviderKey('openrouter', apiKey);
  }

  const withProvider = withOpenRouterProvider(current);
  const next: AISettings = {
    ...withProvider,
    routing: {
      ...withProvider.routing,
      ...CHAT_WORKLOADS.reduce<Record<string, ProviderRef>>((acc, workload) => {
        acc[workload] = { kind: 'cloud', providerSlug: 'openrouter', model: OPENROUTER_FREE_MODEL };
        return acc;
      }, {}),
    },
  };
  await saveAISettings(current, next);
  requestUsageRefresh();
}
