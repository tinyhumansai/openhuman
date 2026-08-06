import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { listConnections as listComposioConnections } from '../../../../lib/composio/composioApi';
import { I18nProvider } from '../../../../lib/i18n/I18nContext';
import {
  clearCloudProviderKey,
  completeOpenAiCodexOAuth,
  flushCloudProviders,
  importOpenAiCodexCliAuth,
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  loadProviderAuthErrors,
  OPENAI_CODEX_OAUTH_MISSING_AUTH_URL,
  OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL,
  saveAISettings,
  setCloudProviderKey,
  startOpenAiCodexOAuth,
  testProviderModel,
  upsertModelRegistryVision,
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
import AIPanel, {
  BackgroundLoopControls,
  buildRoutingDiffSummary,
  type RoutingMap,
} from '../AIPanel';

vi.mock('../../../../services/api/aiSettingsApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../../services/api/aiSettingsApi')>();
  return {
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
    loadProviderAuthErrors: vi.fn().mockResolvedValue([]),
    testProviderModel: vi.fn(),
    // #5341: use the REAL classifier + describer (pure, translator-driven) instead
    // of a hand-copied third implementation that could drift from the source. The
    // classification rules (403/proxy handling, etc.) are unit-tested in
    // aiSettingsApi.test.ts; here they drive the real component branch.
    classifyProviderVerificationFailure: actual.classifyProviderVerificationFailure,
    describeProviderVerificationFailure: actual.describeProviderVerificationFailure,
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
  };
});

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

// The Ollama / LM Studio toggle persists `local_ai.base_url` via this command.
// Mock it so tests can assert the call shape without crossing into Tauri IPC.
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
  cloudProviders: [
    {
      id: 'p_oh_x',
      slug: 'openhuman',
      label: 'OpenHuman',
      endpoint: 'https://api.openhuman.ai/v1',
      auth_style: 'openhuman_jwt' as const,
      has_api_key: false,
    },
  ],
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
  meeting_lookahead_minutes: 60,
  max_calendar_connections_per_tick: 2,
  reminder_lookahead_minutes: 30,
  subconscious_mode: 'off' as 'off' | 'simple' | 'aggressive' | 'event_driven',
  triggers_enabled: false,
  max_promotions_per_hour: 30,
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

const baseTransactions = [
  {
    id: 'older',
    type: 'SPEND' as const,
    action: 'SPEND:USAGE_DEDUCTION:USER',
    amountUsd: -0.25,
    balanceAfterUsd: 9.75,
    createdAt: '2026-05-17T01:00:00.000Z',
  },
  {
    id: 'earn',
    type: 'EARN' as const,
    action: 'TOPUP',
    amountUsd: 1,
    balanceAfterUsd: 10.75,
    createdAt: '2026-05-17T02:00:00.000Z',
  },
  {
    id: 'latest',
    type: 'SPEND' as const,
    action: 'HEARTBEAT',
    amountUsd: -0.5,
    balanceAfterUsd: 9.25,
    createdAt: '2026-05-17T03:00:00.000Z',
  },
];

const baseConnections = [
  { id: 'cal-1', toolkit: 'googlecalendar', status: 'ACTIVE' },
  { id: 'cal-2', toolkit: 'calendar', status: 'CONNECTED' },
  { id: 'cal-3', toolkit: 'google_calendar', status: 'ACTIVE' },
  { id: 'slack-1', toolkit: 'slack', status: 'ACTIVE' },
  { id: 'pending-cal', toolkit: 'googlecalendar', status: 'PENDING' },
];

describe('AIPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(loadLocalProviderSnapshot).mockResolvedValue(baseLocalSnapshot);
    vi.mocked(loadProviderAuthErrors).mockResolvedValue([]);
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
    vi.mocked(creditsApi.getTransactions).mockResolvedValue({
      transactions: baseTransactions,
      total: baseTransactions.length,
    });
    vi.mocked(listComposioConnections).mockResolvedValue({ connections: baseConnections });
  });

  it('renders the LLM Providers + Routing top-level section headers', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() => expect(screen.getAllByText(/^LLM Providers$/).length).toBeGreaterThan(0));
    // The Local provider sub-section was removed entirely.
    expect(screen.queryByText(/Local provider/i)).not.toBeInTheDocument();
    // The old "Auth" header was renamed to "LLM Providers"; "Cloud providers"
    // sub-label is gone in favour of the chip toggles.
    expect(screen.queryByText(/^Auth$/)).not.toBeInTheDocument();
    expect(screen.queryByText(/^Cloud providers$/)).not.toBeInTheDocument();
    expect(screen.getAllByText(/^Routing$/).length).toBeGreaterThan(0);
  });

  it('surfaces a provider-error notice when a BYO key was rejected at runtime', async () => {
    vi.mocked(loadProviderAuthErrors).mockResolvedValue([
      {
        provider: 'openrouter',
        status: 401,
        message:
          'openrouter rejected the API key (HTTP 401). Update your openrouter API key in Connections → API keys → LLM to restore it.',
        timestamp_ms: 1000,
      },
    ]);
    renderWithProviders(<AIPanel />);
    expect(await screen.findByText(/rejected the API key/i)).toBeInTheDocument();
  });

  it('renders no provider-error notice when there are no rejected keys', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() => expect(screen.getAllByText(/^LLM Providers$/).length).toBeGreaterThan(0));
    expect(screen.queryByText(/rejected the API key/i)).not.toBeInTheDocument();
  });

  it('renders the OpenHuman primary card after load', async () => {
    renderWithProviders(<AIPanel />);
    // The OpenHuman label now appears in multiple places (provider card,
    // each workload routing row's "↳ OpenHuman" resolution hint), so we
    // assert at-least-one match rather than getByText.
    await waitFor(() => expect(screen.getAllByText(/OpenHuman/i).length).toBeGreaterThan(0));
  });

  it('renders Managed as an always-on badge, not a switchable toggle (#3760)', async () => {
    renderWithProviders(<AIPanel />);
    // The Managed chip must show an "Always on" indicator...
    expect(await screen.findByText(/Always on/i)).toBeInTheDocument();
    // ...and must NOT render a toggle switch users would try (and fail) to flip.
    expect(screen.queryByRole('switch', { name: /Managed/i })).toBeNull();
    // A hint points users wanting a local model at the Routing card below.
    expect(screen.getByText(/choose a routing mode below/i)).toBeInTheDocument();
  });

  it('renders Managed, Use Your Own Models, and Advanced routing controls', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Managed/i })).toBeInTheDocument()
    );
    expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Advanced/i })).toBeInTheDocument();
  });

  it('renders all visible advanced workload labels', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Advanced/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Advanced/i }));
    await waitFor(() => expect(screen.getByText('Chat')).toBeInTheDocument());
    for (const label of [
      'Chat',
      'Reasoning',
      'Agentic',
      'Coding',
      'Vision',
      'Memory summarization',
      'Heartbeat',
      /Learning/,
      'Subconscious',
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  // ─── per-model vision flag (BYOK) ───────────────────────────────────────────

  // ─── Azure deployment names (#5213) ─────────────────────────────────────────

  // Regression: Azure's `/models` catalog lists *base model ids*, but Azure
  // routes inference by the user's *deployment name*. Before the fix a
  // non-empty catalog forced a closed <select>, so a deployment name that was
  // not in the catalog could not be entered at all and every request came back
  // "Model not found".
  const azureSettings = {
    ...baseSettings,
    cloudProviders: [
      ...baseSettings.cloudProviders,
      {
        id: 'p_azure_x',
        slug: 'azure-foundry',
        label: 'Azure Foundry',
        endpoint: 'https://my-resource.openai.azure.com/openai/v1',
        auth_style: 'bearer' as const,
        has_api_key: true,
      },
    ],
  };

  it('lets an Azure provider take a deployment name that is absent from the model catalog', async () => {
    vi.mocked(loadAISettings).mockResolvedValue(azureSettings);
    // A NON-empty catalog is the pre-fix blocker: it used to force a dropdown.
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'gpt-5.6-terra-2026-07-09' },
      { id: 'gpt-4o' },
    ]);

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Use Your Own Models/i }));

    // The field is a free-text "Deployment name" box, not a catalog dropdown.
    const deploymentInput = await screen.findByRole('textbox', { name: /Deployment name/i });
    fireEvent.change(deploymentInput, { target: { value: 'gpt-5.6-terra' } });

    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));

    await waitFor(() => expect(saveAISettings).toHaveBeenCalled());
    // The deployment name reaches the persisted routing verbatim, and the base
    // model id from the catalog is never substituted for it.
    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls.at(-1) ?? [];
    expect(nextSettings?.routing.chat).toEqual({
      kind: 'cloud',
      providerSlug: 'azure-foundry',
      model: 'gpt-5.6-terra',
    });
    expect(JSON.stringify(nextSettings)).not.toContain('gpt-5.6-terra-2026-07-09');
  });

  it('does not auto-select a catalog model id for an Azure provider', async () => {
    vi.mocked(loadAISettings).mockResolvedValue(azureSettings);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-5.6-terra-2026-07-09' }]);

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Use Your Own Models/i }));

    // Seeding the field with a base model id is what produced the bug, so the
    // deployment field must come up empty and wait for the user.
    const deploymentInput = await screen.findByRole('textbox', { name: /Deployment name/i });
    await waitFor(() => expect(deploymentInput).toHaveValue(''));
  });

  it('keeps the model dropdown for a non-Azure provider, with a manual-entry escape hatch', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({
      ...baseSettings,
      cloudProviders: [
        ...baseSettings.cloudProviders,
        {
          id: 'p_custom_openai',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
    });
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }]);

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Use Your Own Models/i }));

    // Existing behaviour is unchanged: a populated catalog still renders a
    // dropdown and there is no Azure-specific labelling.
    await waitFor(() => expect(screen.queryByText('Deployment name')).not.toBeInTheDocument());
    const toggle = await screen.findByRole('button', { name: /Enter model ID manually/i });

    // ...but the catalog is no longer a dead end for off-catalog model ids.
    fireEvent.click(toggle);
    const manualInput = await screen.findByRole('textbox', { name: /^Model$/i });
    fireEvent.change(manualInput, { target: { value: 'my-private-model' } });
    expect(manualInput).toHaveValue('my-private-model');
  });

  it('warns when a stored Azure value is verbatim a catalog base model id', async () => {
    // The fingerprint of a PRE-FIX Azure selection: the dropdown was the only
    // way to set the value, so catalog membership is exactly the signature of a
    // connection configured the broken way. It stays a hint, never a rewrite —
    // a user may legitimately name a deployment after its base model.
    vi.mocked(loadAISettings).mockResolvedValue({
      ...azureSettings,
      routing: {
        ...azureSettings.routing,
        chat: {
          kind: 'cloud' as const,
          providerSlug: 'azure-foundry',
          model: 'gpt-5.6-terra-2026-07-09',
        },
      },
    });
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'gpt-5.6-terra-2026-07-09' },
      { id: 'gpt-4o' },
    ]);

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Use Your Own Models/i }));

    expect(
      await screen.findByText(/confirm this is the name you gave your deployment/i)
    ).toBeInTheDocument();
    // The always-on explainer sits alongside it for any Azure connection.
    expect(screen.getByText(/This is not the model ID/i)).toBeInTheDocument();
  });

  it('does not warn when the Azure deployment name is absent from the catalog', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({
      ...azureSettings,
      routing: {
        ...azureSettings.routing,
        chat: { kind: 'cloud' as const, providerSlug: 'azure-foundry', model: 'my-deployment' },
      },
    });
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-5.6-terra-2026-07-09' }]);

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Use Your Own Models/i }));

    expect(await screen.findByText(/This is not the model ID/i)).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.queryByText(/confirm this is the name you gave your deployment/i)
      ).not.toBeInTheDocument()
    );
  });

  it('can toggle an Azure connection back to the catalog and out again', async () => {
    // The escape hatch has to work in BOTH directions: Azure opens on free
    // text, but a user whose deployment IS named after a catalog entry should
    // still be able to pick it, then return to typing.
    vi.mocked(loadAISettings).mockResolvedValue(azureSettings);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }]);

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Use Your Own Models/i }));

    await screen.findByRole('textbox', { name: /Deployment name/i });
    fireEvent.click(await screen.findByRole('button', { name: /Choose from list/i }));

    // Back on the catalog dropdown, with the manual escape hatch offered again.
    // The action is labelled for what the field actually holds on Azure — a
    // deployment name, not a model ID.
    await waitFor(() =>
      expect(screen.queryByRole('textbox', { name: /Deployment name/i })).not.toBeInTheDocument()
    );
    expect(
      screen.queryByRole('button', { name: /Enter model ID manually/i })
    ).not.toBeInTheDocument();
    fireEvent.click(await screen.findByRole('button', { name: /Enter deployment name manually/i }));
    expect(await screen.findByRole('textbox', { name: /Deployment name/i })).toBeInTheDocument();
  });

  it('still lets a provider be added when the live /models probe fails', async () => {
    // Regression: the `{base}/models` probe used to be a hard gate on creating
    // a provider. A gateway that serves no OpenAI-shaped listing could never be
    // connected at all, which put the model / deployment-name field permanently
    // out of reach. The probe now informs, it does not block.
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(listProviderModels).mockRejectedValue(
      new Error('provider returned 404: no /models endpoint')
    );

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));

    fireEvent.change(await screen.findByPlaceholderText('My Provider'), {
      target: { value: 'Azure Foundry' },
    });
    fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), {
      target: { value: 'https://my-resource.openai.azure.com/openai/v1' },
    });

    fireEvent.click(screen.getByRole('button', { name: /^Add Provider$/i }));

    // The failure is explained rather than swallowed, and nothing is persisted
    // behind the user's back on the first attempt.
    expect(await screen.findByText(/could not read this provider/i)).toBeInTheDocument();
    expect(saveAISettings).not.toHaveBeenCalled();

    // The escape hatch is what makes the connection reachable at all.
    fireEvent.click(screen.getByRole('button', { name: /Add without verifying/i }));

    await waitFor(() => expect(saveAISettings).toHaveBeenCalled());
    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls.at(-1) ?? [];
    expect(nextSettings?.cloudProviders.map(p => p.slug)).toContain('azure-foundry');
  });

  it('withholds the verification bypass for a legacy Azure base URL', async () => {
    // Skipping verification is a bet that the provider works anyway. On an
    // Azure host that is not the `/openai/v1` base that bet is already lost:
    // `{base}/chat/completions` is not a route Azure serves there and the
    // stored bearer auth is the wrong header. Offering the bypass would just
    // manufacture a dead provider, so the nudge has to be followed instead.
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(listProviderModels).mockRejectedValue(
      new Error('provider returned 404: no /models endpoint')
    );

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));

    fireEvent.change(await screen.findByPlaceholderText('My Provider'), {
      target: { value: 'Azure Legacy' },
    });
    fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), {
      target: { value: 'https://my-resource.openai.azure.com/openai' },
    });

    fireEvent.click(screen.getByRole('button', { name: /^Add Provider$/i }));

    expect(await screen.findByText(/use the v1 base URL/i)).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.queryByRole('button', { name: /Add without verifying/i })
      ).not.toBeInTheDocument()
    );
    expect(saveAISettings).not.toHaveBeenCalled();
  });

  it('nudges an Azure endpoint that is not the v1 base towards /openai/v1', async () => {
    // Only `/openai/v1` serves a `/models` listing and accepts the resource key
    // as a bearer token. A user who pastes the portal's bare resource URL would
    // otherwise fail the probe and then fail every inference call.
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));

    const urlField = screen.getByPlaceholderText('https://api.openai.com/v1');
    fireEvent.change(urlField, {
      target: { value: 'https://my-resource.openai.azure.com/openai' },
    });
    expect(await screen.findByText(/use the v1 base URL/i)).toBeInTheDocument();

    // Correcting the base URL clears the warning.
    fireEvent.change(urlField, {
      target: { value: 'https://my-resource.openai.azure.com/openai/v1' },
    });
    await waitFor(() => expect(screen.queryByText(/use the v1 base URL/i)).not.toBeInTheDocument());
    // The deployment-name pointer stays for any Azure endpoint.
    expect(screen.getByText(/Set your deployment name in the model field/i)).toBeInTheDocument();
  });

  it('leaves a non-Azure endpoint free of Azure guidance', async () => {
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));

    fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), {
      target: { value: 'https://litellm.mycorp.dev/v1' },
    });
    expect(screen.queryByText(/use the v1 base URL/i)).not.toBeInTheDocument();
    expect(
      screen.queryByText(/Set your deployment name in the model field/i)
    ).not.toBeInTheDocument();
  });

  it('blocks a non-probe submit failure instead of offering to skip verification', async () => {
    // The escape hatch is scoped to a rejected `/models` probe. A slug that
    // collides with an existing provider is a different class of failure and
    // must still block, or the dialog would offer to create a broken entry.
    vi.mocked(loadAISettings).mockResolvedValue(azureSettings);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }]);

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));

    fireEvent.change(await screen.findByPlaceholderText('My Provider'), {
      target: { value: 'Azure Foundry' },
    });
    fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), {
      target: { value: 'https://my-resource.openai.azure.com/openai/v1' },
    });

    // `azure-foundry` is already taken by the fixture, so the slug check trips.
    expect(screen.getByRole('button', { name: /^Add Provider$/i })).toBeDisabled();
    expect(
      screen.queryByRole('button', { name: /Add without verifying/i })
    ).not.toBeInTheDocument();
  });

  it('does not offer to skip verification when the key write is what failed', async () => {
    // The slug case above never reaches `submitProvider`'s catch. This one
    // does: the credential write rejects, so the failure travels the same path
    // as a probe rejection but must not be mistaken for one — only a typed
    // `ProviderProbeError` unlocks the bypass.
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }]);
    vi.mocked(setCloudProviderKey).mockRejectedValueOnce(new Error('keyring is locked'));

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));

    fireEvent.change(await screen.findByPlaceholderText('My Provider'), {
      target: { value: 'Azure Foundry' },
    });
    fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), {
      target: { value: 'https://my-resource.openai.azure.com/openai/v1' },
    });
    fireEvent.change(screen.getByLabelText(/API Key/i), { target: { value: 'sk-test-key' } });

    fireEvent.click(screen.getByRole('button', { name: /^Add Provider$/i }));

    expect(await screen.findByText(/keyring is locked/i)).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /Add without verifying/i })
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/could not read this provider/i)).not.toBeInTheDocument();
    expect(saveAISettings).not.toHaveBeenCalled();
  });

  it('clears the verification bypass once a later attempt fails for another reason', async () => {
    // `probeFailed` used to persist for the dialog's lifetime, so a probe
    // rejection left "Add without verifying" on screen even after the next
    // attempt failed for an unrelated reason.
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(listProviderModels).mockRejectedValueOnce(new Error('provider returned 404'));

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));

    fireEvent.change(await screen.findByPlaceholderText('My Provider'), {
      target: { value: 'Azure Foundry' },
    });
    fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), {
      target: { value: 'https://my-resource.openai.azure.com/openai/v1' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^Add Provider$/i }));
    expect(
      await screen.findByRole('button', { name: /Add without verifying/i })
    ).toBeInTheDocument();

    // Second attempt: the probe would now succeed, but the key write rejects.
    vi.mocked(setCloudProviderKey).mockRejectedValueOnce(new Error('keyring is locked'));
    fireEvent.change(screen.getByLabelText(/API Key/i), { target: { value: 'sk-test-key' } });
    fireEvent.click(screen.getByRole('button', { name: /^Add Provider$/i }));

    expect(await screen.findByText(/keyring is locked/i)).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.queryByRole('button', { name: /Add without verifying/i })
      ).not.toBeInTheDocument()
    );
  });

  it('offers a deployment-name field in the per-workload custom routing dialog', async () => {
    // The workload override dialog is a second, independent model picker. It
    // needs the same Azure treatment, or per-workload routing stays stuck on
    // catalog base model ids even after the main selector is fixed.
    vi.mocked(loadAISettings).mockResolvedValue(azureSettings);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-5.6-terra-2026-07-09' }]);

    renderWithProviders(<AIPanel />);
    // Per-workload rows live behind the advanced routing mode.
    fireEvent.click(await screen.findByRole('button', { name: /Advanced/i }));
    const chooseButtons = await screen.findAllByRole('button', {
      name: /Choose Model|Change Model/i,
    });
    fireEvent.click(chooseButtons[0]);

    // Selecting the Azure provider flips the dialog to free text and relabels.
    const providerSelect = await screen.findByDisplayValue(/Azure Foundry|OpenAI|Ollama/i);
    fireEvent.change(providerSelect, { target: { value: 'cloud:azure-foundry' } });

    const deploymentInput = await screen.findByRole('textbox', { name: /Deployment name/i });
    fireEvent.change(deploymentInput, { target: { value: 'workload-deployment' } });
    expect(deploymentInput).toHaveValue('workload-deployment');
    expect(screen.getByText(/This is not the model ID/i)).toBeInTheDocument();

    // A catalog base model id typed here is the pre-fix fingerprint, so the
    // dialog raises the same confirmation hint the main selector does.
    fireEvent.change(deploymentInput, { target: { value: 'gpt-5.6-terra-2026-07-09' } });
    expect(
      await screen.findByText(/confirm this is the name you gave your deployment/i)
    ).toBeInTheDocument();

    // Complete the flow: a working text field proves nothing if the
    // dialog-to-routing handoff drops the value. Put the deployment name back
    // and assert it reaches the persisted per-workload routing verbatim.
    fireEvent.change(deploymentInput, { target: { value: 'workload-deployment' } });
    fireEvent.click(screen.getByRole('button', { name: /^Apply$|^Save$|^Confirm$/ }));
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));

    await waitFor(() => expect(saveAISettings).toHaveBeenCalled());
    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls.at(-1) ?? [];
    const workloadRefs = Object.values(nextSettings?.routing ?? {});
    expect(workloadRefs).toContainEqual(
      expect.objectContaining({
        kind: 'cloud',
        providerSlug: 'azure-foundry',
        model: 'workload-deployment',
      })
    );
  });

  it('keeps the catalog dropdown and its manual escape hatch for a non-Azure provider in the dialog', async () => {
    // The dialog's non-Azure path must be untouched by #5213: a populated
    // catalog still renders a dropdown, and the escape hatch still reaches an
    // off-catalog model id — labelled "Model", not "Deployment name".
    vi.mocked(loadAISettings).mockResolvedValue({
      ...azureSettings,
      cloudProviders: [
        ...azureSettings.cloudProviders,
        {
          id: 'p_custom_openai',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
    });
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }]);

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('button', { name: /Advanced/i }));
    const chooseButtons = await screen.findAllByRole('button', {
      name: /Choose Model|Change Model/i,
    });
    fireEvent.click(chooseButtons[0]);

    const providerSelect = await screen.findByDisplayValue(
      /Azure Foundry|OpenAI|OpenHuman|Ollama/i
    );
    fireEvent.change(providerSelect, { target: { value: 'cloud:openai' } });

    // Catalog dropdown, no Azure labelling.
    await waitFor(() =>
      expect(screen.queryByRole('textbox', { name: /Deployment name/i })).not.toBeInTheDocument()
    );
    expect(screen.queryByText(/This is not the model ID/i)).not.toBeInTheDocument();

    // The escape hatch still works for an off-catalog id.
    fireEvent.click(await screen.findByRole('button', { name: /Enter model ID manually/i }));
    const manualInput = await screen.findByRole('textbox', { name: /^Model$/i });
    fireEvent.change(manualInput, { target: { value: 'my-private-model' } });
    expect(manualInput).toHaveValue('my-private-model');
    // ...and back to the catalog.
    fireEvent.click(await screen.findByRole('button', { name: /Choose from list/i }));
    await waitFor(() =>
      expect(screen.queryByRole('textbox', { name: /^Model$/i })).not.toBeInTheDocument()
    );
  });

  it('flags a custom BYOK model as vision-capable via the Own-model selector', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({
      ...baseSettings,
      cloudProviders: [
        ...baseSettings.cloudProviders,
        {
          id: 'p_custom_openai',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
    });
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Use Your Own Models/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Use Your Own Models/i }));

    // Enter a model id → the per-model "Supports vision" checkbox appears.
    const modelInput = await screen.findByPlaceholderText('Enter model id');
    fireEvent.change(modelInput, { target: { value: 'gpt-4o' } });

    const visionCheckbox = await screen.findByRole('checkbox', { name: /Supports vision/i });
    expect(visionCheckbox).not.toBeChecked();
    fireEvent.click(visionCheckbox);
    expect(visionCheckbox).toBeChecked();

    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));

    // The vision flag is threaded through to the registry upsert + persisted.
    await waitFor(() =>
      expect(vi.mocked(upsertModelRegistryVision)).toHaveBeenCalledWith(
        expect.anything(),
        'openai',
        'gpt-4o',
        true
      )
    );
    expect(saveAISettings).toHaveBeenCalled();
  });

  // ─── auth_style preservation ────────────────────────────────────────────────

  it('preserves auth_style: "anthropic" through save when Anthropic provider is configured', async () => {
    const settingsWithAnthropic = {
      cloudProviders: [
        {
          id: 'p_anthropic_1',
          slug: 'anthropic',
          label: 'Anthropic',
          endpoint: 'https://api.anthropic.com/v1',
          auth_style: 'anthropic' as const,
          has_api_key: true,
        },
      ],
      routing: {
        chat: { kind: 'openhuman' as const },
        reasoning: {
          kind: 'cloud' as const,
          providerSlug: 'anthropic',
          model: 'claude-3-5-sonnet-20241022',
        },
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

    vi.mocked(loadAISettings).mockResolvedValue(settingsWithAnthropic);
    vi.mocked(saveAISettings).mockResolvedValue(undefined);

    renderWithProviders(<AIPanel />);

    // Wait for load.
    await waitFor(() => expect(screen.getAllByText(/Anthropic/i).length).toBeGreaterThan(0));

    fireEvent.click(screen.getByRole('button', { name: /Managed/i }));

    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());

    // Verify auth_style was passed through correctly in the next AISettings arg.
    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls[0];
    const anthropicProvider = nextSettings.cloudProviders.find(
      (p: { slug: string }) => p.slug === 'anthropic'
    );
    expect(anthropicProvider).toBeDefined();
    expect(anthropicProvider!.auth_style).toBe('anthropic');
  });

  // ─── chip toggle: toggle ON opens API-key dialog ────────────────────────────

  it('clicking the OpenAI chip toggle (when disabled) opens the API-key dialog', async () => {
    // Load with no openai provider → chip is off.
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);
    await waitFor(() => expect(screen.getAllByText(/OpenAI/i).length).toBeGreaterThan(0));

    // Find the "Connect OpenAI" switch button and click it.
    const connectSwitch = screen.getByRole('switch', { name: /Connect OpenAI/i });
    fireEvent.click(connectSwitch);

    // ProviderKeyDialog should appear.
    await waitFor(() =>
      expect(screen.getByRole('dialog', { name: /Connect OpenAI/i })).toBeInTheDocument()
    );
    // The input for the API key should be visible.
    const dialog = screen.getByRole('dialog', { name: /Connect OpenAI/i });
    expect(within(dialog).getByLabelText(/API key/i)).toBeInTheDocument();
    expect(
      within(dialog).queryByRole('button', { name: /Sign in with ChatGPT \/ Codex/i })
    ).not.toBeInTheDocument();
    expect(within(dialog).getByTestId('settings-openai-oauth-section')).toBeInTheDocument();
    expect(within(dialog).getByTestId('settings-openai-oauth-connect')).toBeInTheDocument();
  });

  it('#5339: keeps a valid key and saves the provider when the add-time probe fails for a non-auth reason', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    // `/models` is momentarily unreachable — NOT an auth failure. The key is
    // plausibly valid and must not be discarded or the save blocked.
    vi.mocked(listProviderModels).mockRejectedValue(new Error('HTTP request failed'));

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('switch', { name: /Connect DeepSeek/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect DeepSeek/i });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), {
      target: { value: 'sk-deepseek-123' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    // Settings persisted, key kept (never cleared).
    await waitFor(() => expect(saveAISettings).toHaveBeenCalled());
    expect(setCloudProviderKey).toHaveBeenCalledWith('deepseek', 'sk-deepseek-123');
    expect(clearCloudProviderKey).not.toHaveBeenCalled();
    // A truthful advisory replaces the old "not saved" dead end. Non-auth copy
    // is the "a test call … failed" branch (not the auth "rejected it" branch).
    const advisory = await screen.findByText(/The key was saved, but a test call to 'deepseek'/);
    expect(advisory).toBeInTheDocument();
    // The advisory is dismissible.
    fireEvent.click(screen.getByRole('button', { name: /Dismiss/i }));
    await waitFor(() =>
      expect(
        screen.queryByText(/The key was saved, but a test call to 'deepseek'/)
      ).not.toBeInTheDocument()
    );
  });

  it('#5339: rejects and clears the key when the add-time probe fails with an auth error', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    // A 401 means the key itself is wrong — roll back and reject so the user fixes it.
    vi.mocked(listProviderModels).mockRejectedValue(new Error('HTTP 401 invalid api key'));

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('switch', { name: /Connect DeepSeek/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect DeepSeek/i });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), { target: { value: 'sk-bad' } });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() => expect(clearCloudProviderKey).toHaveBeenCalledWith('deepseek'));
    expect(saveAISettings).not.toHaveBeenCalled();
  });

  it('#5341: treats a bare 403/Forbidden probe failure as auth — clears the key, no save', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    // A revoked / forbidden key surfaces as a bare 403 with no "401"/"unauthorized"
    // text. It must still be rejected, not kept behind a "saved" advisory.
    vi.mocked(listProviderModels).mockRejectedValue(new Error('provider returned 403: forbidden'));

    renderWithProviders(<AIPanel />);
    fireEvent.click(await screen.findByRole('switch', { name: /Connect DeepSeek/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect DeepSeek/i });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), {
      target: { value: 'sk-revoked' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() => expect(clearCloudProviderKey).toHaveBeenCalledWith('deepseek'));
    expect(saveAISettings).not.toHaveBeenCalled();
    // No "key was saved" advisory for a rejected key.
    expect(screen.queryByText(/The key was saved, but/)).not.toBeInTheDocument();
  });

  it('#5341: logs (does not swallow) a rollback flush + key-clear failure on a rejected add', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    // Auth failure → rollback path. Both rollback legs then fail; the code must
    // log each instead of swallowing, and must not persist the provider.
    vi.mocked(listProviderModels).mockRejectedValue(new Error('HTTP 401 invalid api key'));
    // First flush (writing the new provider) succeeds; the SECOND flush (rollback
    // to the prior list) is the one that fails.
    vi.mocked(flushCloudProviders)
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error('flush boom'));
    vi.mocked(clearCloudProviderKey).mockRejectedValueOnce(new Error('clear boom'));
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      renderWithProviders(<AIPanel />);
      fireEvent.click(await screen.findByRole('switch', { name: /Connect DeepSeek/i }));
      const dialog = await screen.findByRole('dialog', { name: /Connect DeepSeek/i });
      fireEvent.change(within(dialog).getByLabelText(/API key/i), { target: { value: 'sk-bad' } });
      fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

      await waitFor(() =>
        expect(warnSpy).toHaveBeenCalledWith(
          expect.stringContaining('rollback clearCloudProviderKey failed'),
          expect.any(Error)
        )
      );
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining('rollback flush failed'),
        expect.any(Error)
      );
      expect(saveAISettings).not.toHaveBeenCalled();
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('#5341: logs (does not swallow) a rollback failure when a custom-provider add is rejected', async () => {
    // Same un-swallow guarantee on the custom-provider editor path.
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(listProviderModels).mockRejectedValue(new Error('provider returned 500'));
    vi.mocked(flushCloudProviders)
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error('flush boom'));
    vi.mocked(clearCloudProviderKey).mockRejectedValueOnce(new Error('clear boom'));
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      renderWithProviders(<AIPanel />);
      fireEvent.click(await screen.findByRole('button', { name: /Add Custom Provider/i }));
      fireEvent.change(await screen.findByPlaceholderText('My Provider'), {
        target: { value: 'My Host' },
      });
      fireEvent.change(screen.getByPlaceholderText('https://api.openai.com/v1'), {
        target: { value: 'https://my-host.example.com/v1' },
      });
      fireEvent.change(screen.getByLabelText(/API Key/i), { target: { value: 'sk-test-key' } });
      fireEvent.click(screen.getByRole('button', { name: /^Add Provider$/i }));

      await waitFor(() =>
        expect(warnSpy).toHaveBeenCalledWith(
          expect.stringContaining('rollback clearCloudProviderKey failed'),
          expect.any(Error)
        )
      );
      expect(warnSpy).toHaveBeenCalledWith(
        expect.stringContaining('rollback flush failed'),
        expect.any(Error)
      );
      expect(saveAISettings).not.toHaveBeenCalled();
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('shows a localized Kimi platform link and opens the supported .ai platform', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(
      <I18nProvider>
        <AIPanel />
      </I18nProvider>
    );

    fireEvent.click(await screen.findByRole('switch', { name: /Kimi \(Moonshot\)/i }));
    const dialog = await screen.findByRole('dialog', { name: /Kimi \(Moonshot\)/i });
    const link = within(dialog).getByRole('link', { name: /^Get API key$/i });

    expect(link).toHaveAttribute('href', 'https://platform.kimi.ai?aff=openhuman');

    fireEvent.click(link);

    expect(openUrl).toHaveBeenCalledWith('https://platform.kimi.ai?aff=openhuman');
  });

  it('logs Kimi platform link open failures without changing the dialog', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    vi.mocked(openUrl).mockRejectedValueOnce('blocked');
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    try {
      renderWithProviders(
        <I18nProvider>
          <AIPanel />
        </I18nProvider>
      );

      fireEvent.click(await screen.findByRole('switch', { name: /Kimi \(Moonshot\)/i }));
      const dialog = await screen.findByRole('dialog', { name: /Kimi \(Moonshot\)/i });
      fireEvent.click(within(dialog).getByRole('link', { name: /^Get API key$/i }));

      await waitFor(() => {
        expect(warnSpy).toHaveBeenCalledWith('[ai-settings] provider platform link open failed', {
          slug: 'moonshot',
          error: 'blocked',
        });
      });
      expect(dialog).toBeInTheDocument();
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('localizes the Kimi platform link text for Chinese', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(
      <I18nProvider>
        <AIPanel />
      </I18nProvider>,
      { preloadedState: { locale: { current: 'zh-CN' } } }
    );

    fireEvent.click(await screen.findByRole('switch', { name: /Kimi \(Moonshot\)/i }));
    const dialog = await screen.findByRole('dialog', { name: /Kimi \(Moonshot\)/i });

    expect(within(dialog).getByRole('link', { name: '获取 API Key' })).toBeInTheDocument();
  });

  it('reserves logical inline space for long translated Kimi link labels', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(
      <I18nProvider>
        <AIPanel />
      </I18nProvider>,
      { preloadedState: { locale: { current: 'fr' } } }
    );

    fireEvent.click(await screen.findByRole('switch', { name: /Kimi \(Moonshot\)/i }));
    const dialog = await screen.findByRole('dialog', { name: /Kimi \(Moonshot\)/i });
    const heading = within(dialog).getByRole('heading', {
      name: /Connecter un fournisseur Kimi \(Moonshot\)/i,
    });

    expect(heading.parentElement).toHaveStyle({ paddingInlineEnd: '9rem' });
  });

  it('positions the Kimi link at the logical inline end for RTL locales', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(
      <I18nProvider>
        <AIPanel />
      </I18nProvider>,
      { preloadedState: { locale: { current: 'ar' } } }
    );

    fireEvent.click(await screen.findByRole('switch', { name: /Kimi \(Moonshot\)/i }));
    const dialog = await screen.findByRole('dialog', { name: /Kimi \(Moonshot\)/i });
    const link = within(dialog).getByRole('link', { name: 'احصل على مفتاح API' });

    expect(document.documentElement).toHaveAttribute('dir', 'rtl');
    expect(link).toHaveStyle({ insetInlineEnd: '1.5rem' });
    expect(link).not.toHaveClass('right-6');
  });

  it('does not show the Kimi platform link for other providers', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(
      <I18nProvider>
        <AIPanel />
      </I18nProvider>
    );

    fireEvent.click(await screen.findByRole('switch', { name: /Connect OpenAI/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect OpenAI/i });

    expect(within(dialog).queryByRole('link', { name: /^Get API key$/i })).not.toBeInTheDocument();
  });

  it('renders Phase 1 built-in provider chips including SumoPod', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);

    for (const label of ['Groq', 'DeepSeek', 'MiniMax', 'SumoPod']) {
      await waitFor(() =>
        expect(
          screen.getByRole('switch', { name: new RegExp(`Connect ${label}`, 'i') })
        ).toBeInTheDocument()
      );
    }
  });

  it('connects SumoPod with the native endpoint and provider:sumopod key', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('switch', { name: /Connect SumoPod/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect SumoPod/i });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), {
      target: { value: 'sk-sumopod-test' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() =>
      expect(vi.mocked(setCloudProviderKey)).toHaveBeenCalledWith('sumopod', 'sk-sumopod-test')
    );
    await waitFor(() => expect(vi.mocked(listProviderModels)).toHaveBeenCalledWith('sumopod'));
    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());

    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls[0];
    expect(nextSettings.cloudProviders).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          slug: 'sumopod',
          label: 'SumoPod',
          endpoint: 'https://ai.sumopod.com/v1',
          auth_style: 'bearer',
          has_api_key: true,
        }),
      ])
    );
  });

  it('connects MiniMax via its OpenAI-compatible /v1 endpoint with bearer auth', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('switch', { name: /Connect MiniMax/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect MiniMax/i });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), {
      target: { value: 'sk-minimax-test' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() =>
      expect(vi.mocked(setCloudProviderKey)).toHaveBeenCalledWith('minimax', 'sk-minimax-test')
    );
    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());

    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls[0];
    // MiniMax speaks OpenAI on `/v1` (chat/completions + models). The old
    // `/anthropic` base + anthropic auth pointed at its Messages API, which
    // OpenHuman doesn't speak — both paths 404'd (Sentry TAURI-RUST-8X3).
    expect(nextSettings.cloudProviders).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          slug: 'minimax',
          label: 'MiniMax',
          endpoint: 'https://api.minimax.io/v1',
          auth_style: 'bearer',
        }),
      ])
    );
  });

  it('surfaces provider setup errors in an alert with technical details collapsed', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    vi.mocked(listProviderModels).mockRejectedValueOnce(
      new Error('Could not reach OpenAI: provider returned 401 Unauthorized')
    );

    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('switch', { name: /Connect OpenAI/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect OpenAI/i });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), {
      target: { value: 'sk-bad-key' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    const alert = await within(dialog).findByRole('alert');
    expect(alert).toHaveTextContent(/rejected the credentials/i);
  });

  it('clicking the OpenRouter chip shows both API key entry and the OAuth button', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OpenRouter/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('switch', { name: /Connect OpenRouter/i }));

    const dialog = await screen.findByRole('dialog', { name: /Connect OpenRouter/i });
    expect(within(dialog).getByLabelText(/API key/i)).toBeInTheDocument();
    expect(
      within(dialog).getByRole('button', { name: /Sign in with OpenRouter/i })
    ).toBeInTheDocument();
  });

  it('stores the OpenRouter OAuth key and enables the provider chip', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    vi.mocked(connectOpenRouterViaOAuth).mockResolvedValue('sk-or-from-oauth');

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OpenRouter/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('switch', { name: /Connect OpenRouter/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect OpenRouter/i });
    fireEvent.click(within(dialog).getByRole('button', { name: /Sign in with OpenRouter/i }));

    await waitFor(() => expect(connectOpenRouterViaOAuth).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(setCloudProviderKey).toHaveBeenCalledWith('openrouter', 'sk-or-from-oauth')
    );
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Disconnect OpenRouter/i })).toBeInTheDocument()
    );
  });

  it('clicking Add Custom Provider opens the CloudProviderEditor', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Add Custom Provider/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('button', { name: /Add Custom Provider/i }));

    await waitFor(() => expect(screen.getByText(/Add cloud provider/i)).toBeInTheDocument());
    expect(screen.getByLabelText(/^Name$/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/OpenAI URL/i)).toBeInTheDocument();
  });

  // ─── chip toggle: toggle OFF scrubs routing entries ──────────────────────────

  it('toggling OFF an enabled provider scrubs routing entries that reference it', async () => {
    const settingsWithOpenAI = {
      cloudProviders: [
        {
          id: 'p_openai_1',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
      routing: {
        chat: { kind: 'openhuman' as const },
        reasoning: { kind: 'cloud' as const, providerSlug: 'openai', model: 'gpt-4o' },
        agentic: { kind: 'cloud' as const, providerSlug: 'openai', model: 'gpt-4o-mini' },
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
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOpenAI);
    vi.mocked(saveAISettings).mockResolvedValue(undefined);

    renderWithProviders(<AIPanel />);

    // Wait for load — OpenAI chip should be ON.
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Disconnect OpenAI/i })).toBeInTheDocument()
    );

    // Toggle OFF.
    fireEvent.click(screen.getByRole('switch', { name: /Disconnect OpenAI/i }));

    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());

    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls[0];

    // Provider should be gone.
    expect(
      nextSettings.cloudProviders.find((p: { slug: string }) => p.slug === 'openai')
    ).toBeUndefined();

    // Routing entries that were pinned to openai must be reset to the user default route.
    expect(nextSettings.routing.reasoning).toEqual({ kind: 'default' });
    expect(nextSettings.routing.agentic).toEqual({ kind: 'default' });
    // Entries that were already OpenHuman-managed remain unchanged.
    expect(nextSettings.routing.coding).toEqual({ kind: 'openhuman' });
  });

  // ─── chip toggle: local runtime toggle OFF scrubs orphaned local routing ─────

  it('toggling OFF a local runtime resets workloads routed to it back to default', async () => {
    const settingsWithOllama = {
      cloudProviders: [
        {
          id: 'p_ollama_1',
          slug: 'ollama',
          label: 'Ollama',
          endpoint: 'http://localhost:11434/v1',
          auth_style: 'bearer' as const,
          has_api_key: false,
        },
      ],
      routing: {
        chat: { kind: 'local' as const, model: 'llama3' },
        reasoning: { kind: 'local' as const, model: 'llama3' },
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
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOllama);
    vi.mocked(saveAISettings).mockResolvedValue(undefined);

    renderWithProviders(<AIPanel />);

    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Disconnect Ollama/i })).toBeInTheDocument()
    );

    // Toggle Ollama OFF — no other local runtime remains, so its routed
    // workloads are orphaned and must reset to the user default route.
    fireEvent.click(screen.getByRole('switch', { name: /Disconnect Ollama/i }));

    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());
    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls[0];

    expect(
      nextSettings.cloudProviders.find((p: { slug: string }) => p.slug === 'ollama')
    ).toBeUndefined();
    // Local-routed workloads reset to default (the fix — previously left orphaned).
    expect(nextSettings.routing.chat).toEqual({ kind: 'default' });
    expect(nextSettings.routing.reasoning).toEqual({ kind: 'default' });
    // Already-managed entries unchanged.
    expect(nextSettings.routing.agentic).toEqual({ kind: 'openhuman' });
  });

  // ─── API-key dialog: failed setCloudProviderKey does not add provider ────────

  it('when setCloudProviderKey throws, the provider is NOT added to the draft', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    // Make setCloudProviderKey reject.
    vi.mocked(setCloudProviderKey).mockRejectedValue(new Error('key store failed'));

    renderWithProviders(<AIPanel />);

    // Wait for OpenAI chip to render (disabled).
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OpenAI/i })).toBeInTheDocument()
    );

    // Count provider chips before dialog interaction.
    const chipsBefore = screen.getAllByRole('switch').length;

    // Open the dialog.
    fireEvent.click(screen.getByRole('switch', { name: /Connect OpenAI/i }));
    await waitFor(() =>
      expect(screen.getByRole('dialog', { name: /Connect OpenAI/i })).toBeInTheDocument()
    );

    // Fill in a key and submit.
    fireEvent.change(screen.getByLabelText(/API key/i), { target: { value: 'sk-bad-key' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/i }));

    // The panel silently catches the setCloudProviderKey error and does NOT
    // mutate the draft. Because the panel's onSubmit returns (doesn't throw),
    // the dialog's handleSave resolves without entering its catch block, leaving
    // the dialog in the 'saving' phase with the button showing "Saving…".
    //
    // Wait for setCloudProviderKey to have been called (confirms the flow ran).
    await waitFor(() => expect(vi.mocked(setCloudProviderKey)).toHaveBeenCalled());

    // The dialog must still be open (setKeyDialogFor was never set to null).
    expect(screen.getByRole('dialog', { name: /Connect OpenAI/i })).toBeInTheDocument();

    // The number of provider toggle switches must not have grown — the failed
    // provider was never added to the draft.
    expect(screen.getAllByRole('switch').length).toBe(chipsBefore);

    // Specifically: no "Disconnect OpenAI" switch (chip is still in off state).
    expect(screen.queryByRole('switch', { name: /Disconnect OpenAI/i })).not.toBeInTheDocument();
  });

  // Regression for #4852: the Codex auth button had a hardcoded Korean fallback
  // (`Codex 인증`) because the `settings.ai.codexAuthButton` key was missing from
  // every locale, so the English UI rendered Korean text. Assert the English
  // label renders and no Korean survives.
  it('renders the Codex auth button with the active-locale (English) label', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);

    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OpenAI/i })).toBeInTheDocument()
    );

    const codexButton = screen.getByRole('button', { name: /Connect Codex/i });
    expect(codexButton).toBeInTheDocument();
    // The Korean fallback must be gone from the English onboarding screen.
    expect(screen.queryByText(/인증/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Codex 인증/i })).not.toBeInTheDocument();
  });

  it('connects OpenAI through Codex CLI auth without storing an API key', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);

    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OpenAI/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('button', { name: /Connect Codex/i }));

    await waitFor(() => expect(vi.mocked(importOpenAiCodexCliAuth)).toHaveBeenCalledTimes(1));
    expect(vi.mocked(startOpenAiCodexOAuth)).not.toHaveBeenCalled();
    expect(vi.mocked(openUrl)).not.toHaveBeenCalled();
    expect(vi.mocked(completeOpenAiCodexOAuth)).not.toHaveBeenCalled();
    expect(vi.mocked(setCloudProviderKey)).not.toHaveBeenCalled();
    expect(vi.mocked(clearCloudProviderKey)).toHaveBeenCalledWith('openai');
    expect(vi.mocked(listProviderModels)).not.toHaveBeenCalledWith('openai');

    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());
    expect(vi.mocked(saveAISettings).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(clearCloudProviderKey).mock.invocationCallOrder[0]
    );
    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls[0];
    expect(nextSettings.cloudProviders).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer',
          has_api_key: true,
        }),
      ])
    );
  });

  it.each([
    [OPENAI_CODEX_OAUTH_MISSING_AUTH_URL, /Codex OAuth did not return an authorization URL/i],
    [
      OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL,
      /Paste the redirect URL from your browser after signing in/i,
    ],
  ])('localizes Codex CLI auth error code %s', async (errorCode, expectedMessage) => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    vi.mocked(importOpenAiCodexCliAuth).mockRejectedValueOnce(new Error(errorCode));

    renderWithProviders(<AIPanel />);

    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OpenAI/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('button', { name: /Connect Codex/i }));

    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(expectedMessage));
    expect(vi.mocked(setCloudProviderKey)).not.toHaveBeenCalled();
    expect(vi.mocked(clearCloudProviderKey)).not.toHaveBeenCalled();
    warnSpy.mockRestore();
  });

  it('wraps long provider setup errors and hides raw JSON behind technical details', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    vi.mocked(listProviderModels).mockRejectedValue(
      new Error(
        'provider returned 401: {"error":{"message":"Incorrect API key provided: sk-this-is-a-very-long-invalid-key-value-that-should-not-dominate-the-modal-or-force-horizontal-overflow. You can find your API key at https://platform.openai.com/account/api-keys.","type":"invalid_request_error","param":null,"code":"invalid_api_key"},"request_id":"req_1234567890abcdefghijklmnopqrstuvwxyz"}'
      )
    );

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect OpenAI/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('switch', { name: /Connect OpenAI/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect OpenAI/i });
    fireEvent.change(within(dialog).getByLabelText(/API key/i), {
      target: { value: 'sk-bad-key' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    const alert = await within(dialog).findByRole('alert');
    expect(alert).toHaveClass('max-w-full', 'min-w-0', 'overflow-hidden');
    expect(
      within(alert).getByText('OpenAI rejected the credentials. Check the API key and try again.')
    ).toBeInTheDocument();
    expect(within(alert).getByText('Technical details')).toBeInTheDocument();
    expect(within(alert).getByText(/provider returned 401/)).toBeInTheDocument();
    expect(screen.queryByRole('switch', { name: /Disconnect OpenAI/i })).not.toBeInTheDocument();
  });

  it('summarizes advanced provider editor JSON errors and preserves details', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    vi.mocked(listProviderModels).mockRejectedValue(
      new Error(
        'provider returned 418: {"error":{"message":"Provider teapot says no. Try another endpoint."},"request_id":"req_teapot"}'
      )
    );

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Add Custom Provider/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('button', { name: /Add Custom Provider/i }));
    await waitFor(() => expect(screen.getByText(/Add cloud provider/i)).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText(/^Name$/i), { target: { value: 'Team Gateway' } });
    fireEvent.change(screen.getByLabelText(/OpenAI URL/i), {
      target: { value: 'https://api.openai.com/v1' },
    });
    fireEvent.change(screen.getByPlaceholderText('sk-...'), { target: { value: 'sk-test-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Add provider/i }));

    const alert = await screen.findByRole('alert');
    expect(
      within(alert).getByText(
        'Could not reach Team Gateway: Provider teapot says no. Try another endpoint.'
      )
    ).toBeInTheDocument();
    expect(within(alert).getByText('Technical details')).toBeInTheDocument();
    expect(within(alert).getByText(/provider returned 418/)).toBeInTheDocument();
    expect(
      screen.queryByRole('switch', { name: /Disconnect Team Gateway/i })
    ).not.toBeInTheDocument();
  });

  it('derives the custom provider slug from the entered name', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Add Custom Provider/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('button', { name: /Add Custom Provider/i }));
    await waitFor(() => expect(screen.getByText(/Add cloud provider/i)).toBeInTheDocument());

    fireEvent.change(screen.getByLabelText(/^Name$/i), { target: { value: 'My Team Gateway' } });
    expect(screen.getByText(/Slug:/i)).toHaveTextContent('Slug: my-team-gateway');

    fireEvent.change(screen.getByLabelText(/OpenAI URL/i), {
      target: { value: 'https://gateway.example.com/v1' },
    });
    fireEvent.change(screen.getByPlaceholderText('sk-...'), { target: { value: 'sk-team-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Add provider/i }));

    await waitFor(() =>
      expect(vi.mocked(setCloudProviderKey)).toHaveBeenCalledWith('my-team-gateway', 'sk-team-key')
    );
    await waitFor(() =>
      expect(vi.mocked(listProviderModels)).toHaveBeenCalledWith('my-team-gateway')
    );
  });

  // ─── local runtime: Ollama endpoint URL dialog ──────────────────────────────

  it('toggling Ollama ON shows an Endpoint URL field with localhost default', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect Ollama/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('switch', { name: /Connect Ollama/i }));

    // ProviderKeyDialog renders in endpoint mode for local runtimes: the
    // input is labelled "Endpoint URL", not "API key".
    const dialog = await screen.findByRole('dialog', { name: /Connect Ollama/i });
    const urlInput = within(dialog).getByLabelText(/Endpoint URL/i) as HTMLInputElement;
    expect(urlInput).toBeInTheDocument();
    expect(urlInput.value).toBe('http://localhost:11434/v1');
    expect(within(dialog).queryByLabelText(/API key/i)).not.toBeInTheDocument();
  });

  it('rejects a non-http endpoint URL and keeps the dialog open', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect Ollama/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('switch', { name: /Connect Ollama/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect Ollama/i });
    const urlInput = within(dialog).getByLabelText(/Endpoint URL/i);
    fireEvent.change(urlInput, { target: { value: 'ftp://nope' } });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    // Inline error appears; dialog stays mounted; base_url persist never fires.
    await waitFor(() =>
      expect(within(dialog).getByText(/must start with http/i)).toBeInTheDocument()
    );
    expect(vi.mocked(openhumanUpdateLocalAiSettingsMock)).not.toHaveBeenCalled();
  });

  it('Ollama save normalizes the endpoint and persists local_ai.base_url', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect Ollama/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('switch', { name: /Connect Ollama/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect Ollama/i });

    // Type a host with no path — the URL normalizer must append `/v1` for
    // the /models probe and the base_url derivation strips it back off.
    fireEvent.change(within(dialog).getByLabelText(/Endpoint URL/i), {
      target: { value: 'http://10.0.0.4:11434' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() => expect(openhumanUpdateLocalAiSettingsMock).toHaveBeenCalled());
    const [arg] = vi.mocked(openhumanUpdateLocalAiSettingsMock).mock.calls[0];
    expect(arg).toMatchObject({
      base_url: 'http://10.0.0.4:11434',
      provider: 'ollama',
      runtime_enabled: true,
      opt_in_confirmed: true,
    });
  });

  it('passes Ollama 0.0.0.0 endpoint through to the Rust normalizer', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect Ollama/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('switch', { name: /Connect Ollama/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect Ollama/i });

    fireEvent.change(within(dialog).getByLabelText(/Endpoint URL/i), {
      target: { value: 'http://0.0.0.0:11434' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() => expect(openhumanUpdateLocalAiSettingsMock).toHaveBeenCalled());
    const [arg] = vi.mocked(openhumanUpdateLocalAiSettingsMock).mock.calls[0];
    expect(arg).toMatchObject({ base_url: 'http://0.0.0.0:11434' });
  });

  it('lets users edit an existing Ollama endpoint from the provider chip', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({
      ...baseSettings,
      cloudProviders: [
        {
          id: 'p_ollama_1',
          slug: 'ollama',
          label: 'Ollama',
          endpoint: 'http://127.0.0.1:11434/v1',
          auth_style: 'none' as const,
          has_api_key: true,
        },
      ],
    });
    renderWithProviders(<AIPanel />);
    const editButton = await screen.findByRole('button', { name: /Edit endpoint/i });
    fireEvent.click(editButton);

    const dialog = await screen.findByRole('dialog', { name: /Connect Ollama/i });
    const urlInput = within(dialog).getByLabelText(/Endpoint URL/i) as HTMLInputElement;
    expect(urlInput.value).toBe('http://127.0.0.1:11434/v1');
  });

  it('LM Studio save persists the local_ai provider and endpoint', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    renderWithProviders(<AIPanel />);
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect LM Studio/i })).toBeInTheDocument()
    );
    fireEvent.click(screen.getByRole('switch', { name: /Connect LM Studio/i }));
    const dialog = await screen.findByRole('dialog', { name: /Connect LM Studio/i });

    fireEvent.change(within(dialog).getByLabelText(/Endpoint URL/i), {
      target: { value: 'http://127.0.0.1:1234/v1' },
    });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));

    await waitFor(() => expect(openhumanUpdateLocalAiSettingsMock).toHaveBeenCalled());
    const [arg] = vi.mocked(openhumanUpdateLocalAiSettingsMock).mock.calls[0];
    expect(arg).toMatchObject({
      base_url: 'http://127.0.0.1:1234/v1',
      provider: 'lm_studio',
      runtime_enabled: true,
      opt_in_confirmed: true,
    });
  });

  // ─── local runtime: edit endpoint button on enabled chip ────────────────────

  it('shows an edit-endpoint button on enabled Ollama chip', async () => {
    const settingsWithOllama = {
      ...baseSettings,
      cloudProviders: [
        ...baseSettings.cloudProviders,
        {
          id: 'p_ollama_1',
          slug: 'ollama',
          label: 'Ollama',
          endpoint: 'http://192.168.1.5:11434/v1',
          auth_style: 'bearer' as const,
          has_api_key: false,
        },
      ],
      modelRegistry: [],
    };
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOllama);
    renderWithProviders(<AIPanel />);

    const editBtn = await screen.findByRole('button', { name: /Edit endpoint/i });
    expect(editBtn).toBeInTheDocument();
  });

  it('edit-endpoint button opens the dialog pre-populated with the saved URL', async () => {
    const settingsWithOllama = {
      ...baseSettings,
      cloudProviders: [
        ...baseSettings.cloudProviders,
        {
          id: 'p_ollama_1',
          slug: 'ollama',
          label: 'Ollama',
          endpoint: 'http://192.168.1.5:11434/v1',
          auth_style: 'bearer' as const,
          has_api_key: false,
        },
      ],
      modelRegistry: [],
    };
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOllama);
    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('button', { name: /Edit endpoint/i }));

    const dialog = await screen.findByRole('dialog', { name: /Connect Ollama/i });
    const urlInput = within(dialog).getByLabelText(/Endpoint URL/i) as HTMLInputElement;
    expect(urlInput.value).toBe('http://192.168.1.5:11434/v1');
  });

  it('does not show an edit-endpoint button when Ollama is disabled', async () => {
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });
    renderWithProviders(<AIPanel />);

    await waitFor(() =>
      expect(screen.getByRole('switch', { name: /Connect Ollama/i })).toBeInTheDocument()
    );
    expect(screen.queryByRole('button', { name: /Edit endpoint/i })).not.toBeInTheDocument();
  });

  // ─── Custom routing dialog: per-workload temperature override ───────────────

  it('Custom routing dialog saves the routing change immediately from the modal', async () => {
    const settingsWithOpenAI = {
      cloudProviders: [
        {
          id: 'p_openai_1',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
      routing: {
        ...baseSettings.routing,
        reasoning: { kind: 'cloud' as const, providerSlug: 'openai', model: 'gpt-4o' },
      },
      modelRegistry: [],
    };
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOpenAI);
    vi.mocked(saveAISettings).mockResolvedValue(undefined);
    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('button', { name: /Advanced/i }));
    const reasoningRow = await screen.findByText('Reasoning');
    const rowEl = reasoningRow.closest('div.flex.items-center.justify-between');
    expect(rowEl).not.toBeNull();
    fireEvent.click(within(rowEl as HTMLElement).getByRole('button', { name: /Change Model/i }));

    const dialog = await screen.findByRole('dialog', { name: /Custom routing/i });

    // Enable temperature override; the slider + numeric input become visible.
    const tempToggle = within(dialog).getByLabelText(/Temperature override/i);
    fireEvent.click(tempToggle);

    const tempValueInput = within(dialog).getByLabelText(
      /Temperature override \(value\)/i
    ) as HTMLInputElement;
    expect(tempValueInput).toBeInTheDocument();
    fireEvent.change(tempValueInput, { target: { value: '0.2' } });

    // Save dialog → persists immediately without requiring the sticky Save bar.
    fireEvent.click(within(dialog).getByRole('button', { name: /^Save$/i }));
    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: /Custom routing/i })).not.toBeInTheDocument()
    );
    expect(screen.queryByText(/unsaved change/i)).not.toBeInTheDocument();

    const [, next] = vi.mocked(saveAISettings).mock.calls[0];
    expect(next.routing.reasoning).toEqual({
      kind: 'cloud',
      providerSlug: 'openai',
      model: 'gpt-4o',
      temperature: 0.2,
    });
  });

  it('Custom routing dialog can test the selected cloud model and show its reply', async () => {
    const settingsWithOpenAI = {
      cloudProviders: [
        {
          id: 'p_openai_1',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
      routing: {
        ...baseSettings.routing,
        reasoning: { kind: 'cloud' as const, providerSlug: 'openai', model: 'gpt-4o' },
      },
      modelRegistry: [],
    };
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOpenAI);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }, { id: 'gpt-4o-mini' }]);
    vi.mocked(testProviderModel).mockResolvedValue({ reply: 'Hello from gpt-4o.' });

    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('button', { name: /Advanced/i }));
    const reasoningRow = await screen.findByText('Reasoning');
    const rowEl = reasoningRow.closest('div.flex.items-center.justify-between');
    expect(rowEl).not.toBeNull();
    fireEvent.click(within(rowEl as HTMLElement).getByRole('button', { name: /Change Model/i }));

    const dialog = await screen.findByRole('dialog', { name: /Custom routing/i });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Test$/i }));

    await waitFor(() =>
      expect(vi.mocked(testProviderModel)).toHaveBeenCalledWith(
        'reasoning',
        'openai:gpt-4o',
        'Hello world'
      )
    );
    expect(await within(dialog).findByText('Model response')).toBeInTheDocument();
    expect(within(dialog).getByText('Hello from gpt-4o.')).toBeInTheDocument();
  });

  it('Custom routing dialog shows in-flight test status immediately', async () => {
    const settingsWithOpenAI = {
      cloudProviders: [
        {
          id: 'p_openai_1',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
      routing: {
        ...baseSettings.routing,
        reasoning: { kind: 'cloud' as const, providerSlug: 'openai', model: 'gpt-4o' },
      },
      modelRegistry: [],
    };
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOpenAI);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }]);
    let resolveTest: (value: { reply: string }) => void = () => {};
    const pendingTest = new Promise<{ reply: string }>(resolve => {
      resolveTest = resolve;
    });
    vi.mocked(testProviderModel).mockReturnValue(pendingTest);

    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('button', { name: /Advanced/i }));
    const reasoningRow = await screen.findByText('Reasoning');
    const rowEl = reasoningRow.closest('div.flex.items-center.justify-between');
    expect(rowEl).not.toBeNull();
    fireEvent.click(within(rowEl as HTMLElement).getByRole('button', { name: /Change Model/i }));

    const dialog = await screen.findByRole('dialog', { name: /Custom routing/i });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Test$/i }));

    expect(await within(dialog).findByText('Testing model...')).toBeInTheDocument();
    expect(within(dialog).getByText(/Provider: openai:gpt-4o/i)).toBeInTheDocument();
    expect(within(dialog).getByText(/Prompt: Hello world/i)).toBeInTheDocument();

    resolveTest({ reply: 'Hello from gpt-4o.' });
    expect(await within(dialog).findByText('Model response')).toBeInTheDocument();
  });

  it('Custom routing dialog shows test errors inline', async () => {
    const settingsWithOpenAI = {
      cloudProviders: [
        {
          id: 'p_openai_1',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer' as const,
          has_api_key: true,
        },
      ],
      routing: {
        ...baseSettings.routing,
        reasoning: { kind: 'cloud' as const, providerSlug: 'openai', model: 'gpt-4o' },
      },
      modelRegistry: [],
    };
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOpenAI);
    vi.mocked(listProviderModels).mockResolvedValue([{ id: 'gpt-4o' }]);
    vi.mocked(testProviderModel).mockRejectedValue(new Error('401 invalid api key'));

    renderWithProviders(<AIPanel />);

    fireEvent.click(await screen.findByRole('button', { name: /Advanced/i }));
    const reasoningRow = await screen.findByText('Reasoning');
    const rowEl = reasoningRow.closest('div.flex.items-center.justify-between');
    expect(rowEl).not.toBeNull();
    fireEvent.click(within(rowEl as HTMLElement).getByRole('button', { name: /Change Model/i }));

    const dialog = await screen.findByRole('dialog', { name: /Custom routing/i });
    fireEvent.click(within(dialog).getByRole('button', { name: /^Test$/i }));

    // The banner carries the actionable message keyed to the provider slug,
    // not the raw upstream string (which can echo request material).
    const alert = await within(dialog).findByRole('alert');
    expect(alert).toHaveTextContent('rejected it');
    expect(alert).toHaveTextContent('openai');
    expect(alert).not.toHaveTextContent('401 invalid api key');
  });

  it('renders background loop diagnostics with newest spend row and budget math', async () => {
    // BackgroundLoopControls was moved out of AIPanel into standalone panels.
    renderWithProviders(
      <BackgroundLoopControls
        view="all"
        routing={baseSettings.routing}
        cloudProviders={baseSettings.cloudProviders}
      />
    );

    await waitFor(() => expect(screen.getByText('Background loops')).toBeInTheDocument());

    expect(screen.getByText('Heartbeat controls')).toBeInTheDocument();
    expect(screen.getByText('Recent usage ledger')).toBeInTheDocument();
    expect(screen.getByText('Loop map')).toBeInTheDocument();
    expect(screen.getByText('Heartbeat planner')).toBeInTheDocument();
    expect(screen.getByText('Subconscious tick')).toBeInTheDocument();
    expect(screen.getByText('Memory tree workers')).toBeInTheDocument();
    expect(screen.getByText('Reflection rebuild')).toBeInTheDocument();
    expect(screen.getByText('Composio sync')).toBeInTheDocument();

    expect(screen.getByText('Week budget')).toBeInTheDocument();
    expect(screen.getByText('$10.0000')).toBeInTheDocument();
    expect(screen.getByText('Cycle remaining')).toBeInTheDocument();
    expect(screen.getByText('$1.5000')).toBeInTheDocument();
    expect(screen.getByText('Avg spend row')).toBeInTheDocument();
    expect(screen.getByText('Bg API reads')).toBeInTheDocument();
    expect(screen.getByText('Bg wakeups')).toBeInTheDocument();

    expect(screen.getByText('Rows left')).toBeInTheDocument();
    expect(screen.getByText('Rows per full week budget')).toBeInTheDocument();
    expect(screen.getByText('Sample burn rate')).toBeInTheDocument();
    expect(screen.getByText('Projected empty')).toBeInTheDocument();
    expect(screen.getByText('API reads per $ remaining')).toBeInTheDocument();
    expect(screen.getByText('Loop call budget')).toBeInTheDocument();
    expect(screen.getByText('Calendar fanout cap')).toBeInTheDocument();
    expect(screen.getByText('Subconscious model calls')).toBeInTheDocument();
    expect(screen.getByText('Composio sync scans')).toBeInTheDocument();
    expect(screen.getByText('Memory worker polls')).toBeInTheDocument();

    expect(screen.getByText(/3 Composio read call\(s\)\/tick/)).toBeInTheDocument();
    expect(screen.getByText(/1 calendar link\(s\) over cap skipped/)).toBeInTheDocument();
    expect(screen.getByText(/2\/3 conn\/tick/)).toBeInTheDocument();
    expect(screen.getByText('HEARTBEAT')).toBeInTheDocument();
    expect(screen.getByText('SPEND:USAGE_DEDUCTION:USER')).toBeInTheDocument();
    expect(screen.getByText(/Latest spend: \$0\.5000/)).toBeInTheDocument();
  });

  it('patches heartbeat controls and runs a manual planner tick', async () => {
    let currentSettings = { ...baseHeartbeatSettings };
    vi.mocked(openhumanHeartbeatSettingsGet).mockImplementation(async () => ({
      result: { settings: currentSettings },
      logs: [],
    }));
    vi.mocked(openhumanHeartbeatSettingsSet).mockImplementation(async patch => {
      currentSettings = { ...currentSettings, ...patch };
      return { result: { settings: currentSettings }, logs: [] };
    });

    // BackgroundLoopControls was moved out of AIPanel into standalone panels.
    renderWithProviders(
      <BackgroundLoopControls
        view="all"
        routing={baseSettings.routing}
        cloudProviders={baseSettings.cloudProviders}
      />
    );
    await waitFor(() => expect(screen.getByText('Heartbeat controls')).toBeInTheDocument());

    const clickToggle = async (label: string, expectedPatch: Record<string, unknown>) => {
      const row = screen.getByText(label).parentElement!.parentElement!;
      fireEvent.click(within(row).getByRole('switch'));
      await waitFor(() =>
        expect(vi.mocked(openhumanHeartbeatSettingsSet)).toHaveBeenLastCalledWith(expectedPatch)
      );
    };

    await clickToggle('Heartbeat loop', { enabled: false });
    await clickToggle('Subconscious inference', { inference_enabled: false });
    await clickToggle('Calendar meeting checks', { notify_meetings: false });
    await clickToggle('Cron reminder checks', { notify_reminders: false });
    await clickToggle('Relevant notification checks', { notify_relevant_events: true });
    await clickToggle('External delivery', { external_delivery_enabled: true });

    fireEvent.change(screen.getByLabelText('Calendar cap'), { target: { value: '3' } });
    await waitFor(() =>
      expect(vi.mocked(openhumanHeartbeatSettingsSet)).toHaveBeenLastCalledWith({
        max_calendar_connections_per_tick: 3,
      })
    );

    fireEvent.change(screen.getByLabelText('Meeting lookahead'), { target: { value: '120' } });
    await waitFor(() =>
      expect(vi.mocked(openhumanHeartbeatSettingsSet)).toHaveBeenLastCalledWith({
        meeting_lookahead_minutes: 120,
      })
    );

    fireEvent.change(screen.getByLabelText('Reminder lookahead'), { target: { value: '60' } });
    await waitFor(() =>
      expect(vi.mocked(openhumanHeartbeatSettingsSet)).toHaveBeenLastCalledWith({
        reminder_lookahead_minutes: 60,
      })
    );

    fireEvent.change(screen.getByLabelText('Interval'), { target: { value: '30' } });
    await waitFor(() =>
      expect(vi.mocked(openhumanHeartbeatSettingsSet)).toHaveBeenLastCalledWith({
        interval_minutes: 30,
      })
    );

    fireEvent.click(screen.getByRole('button', { name: 'Planner tick now' }));
    await waitFor(() => expect(vi.mocked(openhumanHeartbeatTickNow)).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText(/Planner: 3 source events/)).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    fireEvent.click(screen.getByRole('button', { name: 'Reload' }));
    await waitFor(() => expect(vi.mocked(openhumanHeartbeatSettingsGet)).toHaveBeenCalled());
  });

  it('shows heartbeat load and planner errors without crashing diagnostics', async () => {
    vi.mocked(openhumanHeartbeatSettingsGet).mockRejectedValueOnce(new Error('heartbeat offline'));
    vi.mocked(openhumanHeartbeatTickNow).mockRejectedValueOnce(new Error('tick failed'));

    // BackgroundLoopControls was moved out of AIPanel into standalone panels.
    renderWithProviders(
      <BackgroundLoopControls
        view="all"
        routing={baseSettings.routing}
        cloudProviders={baseSettings.cloudProviders}
      />
    );

    await waitFor(() => expect(screen.getByText('heartbeat offline')).toBeInTheDocument());
    expect(screen.getByText('Heartbeat controls unavailable.')).toBeInTheDocument();

    vi.mocked(openhumanHeartbeatSettingsGet).mockResolvedValueOnce({
      result: { settings: baseHeartbeatSettings },
      logs: [],
    });
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(screen.getByText('Heartbeat controls')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Planner tick now' }));
    await waitFor(() => expect(screen.getByText('tick failed')).toBeInTheDocument());
  });
});

describe('buildRoutingDiffSummary', () => {
  const allDefault = (): RoutingMap => ({
    chat: { kind: 'default' },
    reasoning: { kind: 'default' },
    agentic: { kind: 'default' },
    coding: { kind: 'default' },
    vision: { kind: 'default' },
    memory: { kind: 'default' },
    heartbeat: { kind: 'default' },
    learning: { kind: 'default' },
    subconscious: { kind: 'default' },
  });

  it('emits one "<label> → <target>" entry per changed workload and skips unchanged ones', () => {
    // Identity `t` so we can assert the workload's i18n label key is used.
    const t = (key: string) => key;
    const saved = allDefault();
    saved.coding = { kind: 'cloud', providerSlug: 'x', model: 'y' };
    const draft = allDefault();
    draft.chat = { kind: 'cloud', providerSlug: 'openai', model: 'gpt-4o', temperature: 0.3 };
    draft.reasoning = { kind: 'openhuman' };
    draft.agentic = { kind: 'local', model: 'llama3' };
    // coding: saved=cloud, draft=default → changed; describe(default) === 'cloud'.

    expect(buildRoutingDiffSummary(saved, draft, t)).toEqual([
      'settings.ai.routing.workload.chat.label → openai:gpt-4o@0.30',
      'settings.ai.routing.workload.reasoning.label → openhuman',
      'settings.ai.routing.workload.agentic.label → local:llama3',
      'settings.ai.routing.workload.coding.label → cloud',
    ]);
  });

  it('returns an empty list when draft matches saved', () => {
    const routing = allDefault();
    expect(buildRoutingDiffSummary(routing, { ...routing }, k => k)).toEqual([]);
  });
});
