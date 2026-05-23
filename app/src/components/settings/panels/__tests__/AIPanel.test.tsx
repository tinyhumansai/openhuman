import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { listConnections as listComposioConnections } from '../../../../lib/composio/composioApi';
import {
  listProviderModels,
  loadAISettings,
  loadLocalProviderSnapshot,
  saveAISettings,
  setCloudProviderKey,
} from '../../../../services/api/aiSettingsApi';
import { creditsApi } from '../../../../services/api/creditsApi';
import { renderWithProviders } from '../../../../test/test-utils';
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
  setCloudProviderKey: vi.fn(),
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
  listProviderModels: vi.fn().mockResolvedValue([]),
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
    memory: { kind: 'openhuman' as const },
    embeddings: { kind: 'openhuman' as const },
    heartbeat: { kind: 'openhuman' as const },
    learning: { kind: 'openhuman' as const },
    subconscious: { kind: 'openhuman' as const },
  },
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
    vi.mocked(setCloudProviderKey).mockResolvedValue(undefined);
    vi.mocked(listProviderModels).mockResolvedValue([]);
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

  it('renders the OpenHuman primary card after load', async () => {
    renderWithProviders(<AIPanel />);
    // The OpenHuman label now appears in multiple places (provider card,
    // each workload routing row's "↳ OpenHuman" resolution hint), so we
    // assert at-least-one match rather than getByText.
    await waitFor(() => expect(screen.getAllByText(/OpenHuman/i).length).toBeGreaterThan(0));
  });

  it('renders all nine workload labels', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() => expect(screen.getByText('Chat')).toBeInTheDocument());
    for (const label of [
      'Chat',
      'Reasoning',
      'Agentic',
      'Coding',
      'Memory summarization',
      'Embeddings',
      'Heartbeat',
      /Learning/,
      'Subconscious',
    ]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
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
        memory: { kind: 'openhuman' as const },
        embeddings: { kind: 'openhuman' as const },
        heartbeat: { kind: 'openhuman' as const },
        learning: { kind: 'openhuman' as const },
        subconscious: { kind: 'openhuman' as const },
      },
    };

    vi.mocked(loadAISettings).mockResolvedValue(settingsWithAnthropic);
    vi.mocked(saveAISettings).mockResolvedValue(undefined);

    renderWithProviders(<AIPanel />);

    // Wait for load.
    await waitFor(() => expect(screen.getAllByText(/Anthropic/i).length).toBeGreaterThan(0));

    // Trigger a routing change so the SaveBar appears, then save.
    // Click the "Default" button specifically on the Reasoning row (which is
    // currently set to custom cloud routing) to switch it back to openhuman.
    const reasoningRow = screen
      .getByText('Reasoning')
      .closest('[class*="flex items-center justify-between"]');
    fireEvent.click(within(reasoningRow as HTMLElement).getByText('Default'));

    // SaveBar should appear.
    await waitFor(() => expect(screen.getByText(/unsaved change/i)).toBeInTheDocument());

    // Click Save in the SaveBar.
    const saveButton = screen.getByRole('button', { name: /^Save$/i });
    fireEvent.click(saveButton);

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
    expect(screen.getByLabelText(/API key/i)).toBeInTheDocument();
  });

  it('clicking the Custom chip (when disabled) opens the CloudProviderEditor, not the key dialog', async () => {
    // Load with no custom provider → chip is off.
    vi.mocked(loadAISettings).mockResolvedValue({ ...baseSettings, cloudProviders: [] });

    renderWithProviders(<AIPanel />);
    await waitFor(() => expect(screen.getAllByText(/Custom/i).length).toBeGreaterThan(0));

    // Find the "Connect Custom" switch and click it.
    const connectSwitch = screen.getByRole('switch', { name: /Connect Custom/i });
    fireEvent.click(connectSwitch);

    // The full CloudProviderEditor should appear (has "Add cloud provider" heading).
    await waitFor(() => expect(screen.getByText(/Add cloud provider/i)).toBeInTheDocument());
    // The simple ProviderKeyDialog should NOT appear.
    expect(screen.queryByRole('dialog', { name: /Connect Custom/i })).not.toBeInTheDocument();
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
        memory: { kind: 'openhuman' as const },
        embeddings: { kind: 'openhuman' as const },
        heartbeat: { kind: 'openhuman' as const },
        learning: { kind: 'openhuman' as const },
        subconscious: { kind: 'openhuman' as const },
      },
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

    // A SaveBar must appear because the draft changed.
    await waitFor(() => expect(screen.getByText(/unsaved change/i)).toBeInTheDocument());

    // Save to capture the nextSettings arg.
    fireEvent.click(screen.getByRole('button', { name: /^Save$/i }));
    await waitFor(() => expect(vi.mocked(saveAISettings)).toHaveBeenCalled());

    const [, nextSettings] = vi.mocked(saveAISettings).mock.calls[0];

    // Provider should be gone.
    expect(
      nextSettings.cloudProviders.find((p: { slug: string }) => p.slug === 'openai')
    ).toBeUndefined();

    // Routing entries that were pinned to openai must be reset to openhuman.
    expect(nextSettings.routing.reasoning).toEqual({ kind: 'openhuman' });
    expect(nextSettings.routing.agentic).toEqual({ kind: 'openhuman' });
    // Entries that were already openhuman remain unchanged.
    expect(nextSettings.routing.coding).toEqual({ kind: 'openhuman' });
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
      expect(screen.getByRole('switch', { name: /Connect Custom/i })).toBeInTheDocument()
    );

    fireEvent.click(screen.getByRole('switch', { name: /Connect Custom/i }));
    await waitFor(() => expect(screen.getByText(/Add cloud provider/i)).toBeInTheDocument());
    fireEvent.change(screen.getByPlaceholderText('sk-...'), { target: { value: 'sk-test-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Add provider/i }));

    const alert = await screen.findByRole('alert');
    expect(
      within(alert).getByText(
        'Could not reach OpenAI: Provider teapot says no. Try another endpoint.'
      )
    ).toBeInTheDocument();
    expect(within(alert).getByText('Technical details')).toBeInTheDocument();
    expect(within(alert).getByText(/provider returned 418/)).toBeInTheDocument();
    expect(screen.queryByRole('switch', { name: /Disconnect OpenAI/i })).not.toBeInTheDocument();
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
    };
    vi.mocked(loadAISettings).mockResolvedValue(settingsWithOpenAI);
    vi.mocked(saveAISettings).mockResolvedValue(undefined);
    renderWithProviders(<AIPanel />);

    // Wait for the Reasoning workload row (identified by its unique
    // description text), then click its "Custom" segment to open the
    // Custom routing dialog.
    const reasoningRow = await screen.findByText(/Main chat agent/i);
    const rowEl = reasoningRow.closest('div.flex.items-center.justify-between');
    expect(rowEl).not.toBeNull();
    fireEvent.click(within(rowEl as HTMLElement).getByRole('button', { name: /Custom/i }));

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

  it('renders background loop diagnostics with newest spend row and budget math', async () => {
    renderWithProviders(<AIPanel />);

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

    renderWithProviders(<AIPanel />);
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

    renderWithProviders(<AIPanel />);

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
