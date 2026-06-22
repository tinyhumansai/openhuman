import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { listConnections as listComposioConnections } from '../../../../lib/composio/composioApi';
import {
  clearCloudProviderKey,
  completeOpenAiCodexOAuth,
  importOpenAiCodexCliAuth,
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  setCloudProviderKey,
  startOpenAiCodexOAuth,
  testProviderModel,
} from '../../../../services/api/aiSettingsApi';
import { creditsApi } from '../../../../services/api/creditsApi';
import { renderWithProviders } from '../../../../test/test-utils';
import { connectOpenRouterViaOAuth } from '../../../../utils/openrouterOAuth';
import { openUrl } from '../../../../utils/openUrl';
// Lazy import so the typed mock is available to individual tests.
import { openhumanUpdateLocalAiSettings as openhumanUpdateLocalAiSettingsMock } from '../../../../utils/tauriCommands/config';
import {
  openhumanHeartbeatSettingsGet,
  openhumanHeartbeatSettingsSet,
  openhumanHeartbeatTickNow,
} from '../../../../utils/tauriCommands/heartbeat';
import AIPanel from '../AIPanel';

vi.mock('../../../../services/api/aiSettingsApi', () => ({
  ALL_WORKLOADS: [
    'chat',
    'reasoning',
    'agentic',
    'coding',
    'memory',
    'embeddings',
    'heartbeat',
    'learning',
    'subconscious',
  ],
  loadAISettings: vi.fn(),
  saveAISettings: vi.fn(),
  loadLocalProviderSnapshot: vi.fn(),
  testProviderModel: vi.fn(),
  modelRegistryVision: vi.fn(() => false),
  upsertModelRegistryVision: vi.fn((registry: unknown[]) => registry),
  setCloudProviderKey: vi.fn().mockResolvedValue(undefined),
  clearCloudProviderKey: vi.fn().mockResolvedValue(undefined),
  serializeProviderRef: vi.fn((r: { kind: string; providerSlug?: string; model?: string }) =>
    r.kind === 'openhuman'
      ? 'openhuman'
      : r.kind === 'local'
        ? `ollama:${r.model}`
        : `${r.providerSlug}:${r.model}`
  ),
  localProvider: { download: vi.fn(), applyPreset: vi.fn() },
  flushCloudProviders: vi.fn().mockResolvedValue(undefined),
  importOpenAiCodexCliAuth: vi.fn().mockResolvedValue(undefined),
  listProviderModels: vi.fn().mockResolvedValue([]),
  OPENAI_CODEX_OAUTH_MISSING_AUTH_URL: 'OPENAI_CODEX_OAUTH_MISSING_AUTH_URL',
  OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL: 'OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL',
  startOpenAiCodexOAuth: vi.fn(),
  completeOpenAiCodexOAuth: vi.fn(),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../../../utils/tauriCommands/heartbeat', () => ({
  openhumanHeartbeatSettingsGet: vi.fn(),
  openhumanHeartbeatSettingsSet: vi.fn(),
  openhumanHeartbeatTickNow: vi.fn(),
}));

vi.mock('../../../../services/api/creditsApi', () => ({
  creditsApi: { getTeamUsage: vi.fn(), getTransactions: vi.fn() },
}));

vi.mock('../../../../lib/composio/composioApi', () => ({ listConnections: vi.fn() }));

// OMLX persists `local_ai.{base_url,api_key,provider}` via this command. Mock
// it so the test can assert the call shape without crossing into Tauri IPC.
vi.mock('../../../../utils/tauriCommands/config', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands/config')>(
    '../../../../utils/tauriCommands/config'
  );
  return {
    ...actual,
    openhumanUpdateLocalAiSettings: vi
      .fn()
      .mockResolvedValue({ result: { config: {}, workspace_dir: '', config_path: '' }, logs: [] }),
  };
});

vi.mock('../../../../utils/openrouterOAuth', () => ({ connectOpenRouterViaOAuth: vi.fn() }));
vi.mock('../../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

const baseSettings = {
  cloudProviders: [],
  routing: {
    chat: { kind: 'openhuman' as const },
    reasoning: { kind: 'openhuman' as const },
    agentic: { kind: 'openhuman' as const },
    coding: { kind: 'openhuman' as const },
    vision: { kind: 'openhuman' as const },
    memory: { kind: 'openhuman' as const },
    embeddings: { kind: 'openhuman' as const },
    heartbeat: { kind: 'openhuman' as const },
    learning: { kind: 'openhuman' as const },
    subconscious: { kind: 'openhuman' as const },
  },
  modelRegistry: [],
};

const baseLocalSnapshot = { status: null, diagnostics: null, presets: null, installedModels: [] };

const baseHeartbeatSettings = {
  enabled: true,
  interval_minutes: 15,
  inference_enabled: true,
  notify_meetings: true,
  notify_reminders: true,
  notify_relevant_events: false,
  external_delivery_enabled: false,
  triggers_enabled: false,
  max_promotions_per_hour: 30,
  meeting_lookahead_minutes: 60,
  max_calendar_connections_per_tick: 2,
  reminder_lookahead_minutes: 30,
  subconscious_mode: 'off' as 'off' | 'simple' | 'aggressive',
};

const baseUsage = {
  remainingUsd: 1.5,
  cycleBudgetUsd: 10,
  cycleSpentUsd: 8.5,
  cycleStartDate: '2026-05-14T00:00:00.000Z',
  cycleEndsAt: '2026-05-21T00:00:00.000Z',
  plan: {
    plan: 'BASIC',
    name: 'Basic',
    marginPercent: 25,
    payAsYouGoMarginPercent: 50,
    discountVsPayAsYouGoPercent: 50,
  },
  insights: {
    period: { startDate: '2026-05-14T00:00:00.000Z', endDate: '2026-05-21T00:00:00.000Z' },
    totals: {
      inferenceUsd: 6,
      integrationsUsd: 2.5,
      totalUsd: 8.5,
      inferenceCalls: 120,
      integrationCalls: 6,
    },
    dailySeries: [],
    topModels: [],
    topIntegrations: [],
  },
};

describe('AIPanel OMLX connect', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(loadLocalProviderSnapshot).mockResolvedValue(baseLocalSnapshot);
    vi.mocked(setCloudProviderKey).mockResolvedValue(undefined);
    vi.mocked(clearCloudProviderKey).mockResolvedValue(undefined);
    vi.mocked(importOpenAiCodexCliAuth).mockResolvedValue(undefined);
    vi.mocked(testProviderModel).mockResolvedValue({ reply: 'Hello from the selected model.' });
    vi.mocked(listProviderModels).mockResolvedValue([]);
    vi.mocked(startOpenAiCodexOAuth).mockResolvedValue({
      authUrl: 'https://auth.openai.com/oauth/authorize?client_id=test',
    });
    vi.mocked(completeOpenAiCodexOAuth).mockResolvedValue(undefined);
    vi.mocked(openUrl).mockResolvedValue(undefined);
    vi.mocked(connectOpenRouterViaOAuth).mockResolvedValue('sk-or-oauth');
    vi.mocked(openhumanHeartbeatSettingsGet).mockResolvedValue({
      result: { settings: baseHeartbeatSettings },
      logs: [],
    });
    vi.mocked(openhumanHeartbeatSettingsSet).mockResolvedValue({
      result: { settings: baseHeartbeatSettings },
      logs: [],
    });
    vi.mocked(openhumanHeartbeatTickNow).mockResolvedValue({
      result: {
        summary: {
          source_events: 3,
          deliveries_attempted: 2,
          deliveries_sent: 1,
          deliveries_skipped_dedup: 1,
        },
      },
      logs: [],
    });
    vi.mocked(creditsApi.getTeamUsage).mockResolvedValue(baseUsage);
    vi.mocked(creditsApi.getTransactions).mockResolvedValue({ transactions: [], total: 0 });
    vi.mocked(listComposioConnections).mockResolvedValue({ connections: [] });
  });

  it('renders an OMLX local-runtime chip', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OMLX/i })).toBeInTheDocument()
    );
  });

  it('toggling OMLX ON shows BOTH an endpoint field (localhost:8000) and an API key field', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OMLX/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('switch', { name: /Connect OMLX/i }));

    const dialog = await screen.findByRole('dialog', { name: /Connect OMLX/i });
    const urlInput = within(dialog).getByLabelText(/Endpoint URL/i) as HTMLInputElement;
    expect(urlInput).toBeInTheDocument();
    expect(urlInput.value).toBe('http://localhost:8000/v1');
    expect(within(dialog).getByLabelText(/API key/i)).toBeInTheDocument();
  });

  it('persists provider=omlx with base_url + api_key on confirm', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OMLX/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('switch', { name: /Connect OMLX/i }));

    const dialog = await screen.findByRole('dialog', { name: /Connect OMLX/i });
    fireEvent.change(within(dialog).getByLabelText(/Endpoint URL/i), {
      target: { value: 'http://localhost:8000/v1' },
    });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), {
      target: { value: 'sk-omlx-test' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() =>
      expect(openhumanUpdateLocalAiSettingsMock).toHaveBeenCalledWith(
        expect.objectContaining({
          provider: 'omlx',
          base_url: 'http://localhost:8000/v1',
          api_key: expect.any(String),
          runtime_enabled: true,
          opt_in_confirmed: true,
        })
      )
    );
    const [arg] = vi.mocked(openhumanUpdateLocalAiSettingsMock).mock.calls[0];
    expect(arg).toMatchObject({ api_key: 'sk-omlx-test' });
  });
});
