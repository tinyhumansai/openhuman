import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  loadAISettings,
  loadLocalProviderSnapshot,
  saveAISettings,
  setCloudProviderKey,
} from '../../../../services/api/aiSettingsApi';
import { renderWithProviders } from '../../../../test/test-utils';
import AIPanel from '../AIPanel';

vi.mock('../../../../services/api/aiSettingsApi', () => ({
  ALL_WORKLOADS: [
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
  clearCloudProviderKey: vi.fn(),
  serializeProviderRef: vi.fn((r: { kind: string; providerSlug?: string; model?: string }) =>
    r.kind === 'openhuman'
      ? 'openhuman'
      : r.kind === 'local'
        ? `ollama:${r.model}`
        : `${r.providerSlug}:${r.model}`
  ),
  localProvider: { download: vi.fn(), applyPreset: vi.fn() },
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

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

describe('AIPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(loadLocalProviderSnapshot).mockResolvedValue(baseLocalSnapshot);
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

  it('renders all eight workload labels', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() => expect(screen.getByText('Reasoning')).toBeInTheDocument());
    for (const label of [
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
    // Click the "Default" button on the Reasoning row to change routing.
    const defaultButtons = screen.getAllByText('Default');
    fireEvent.click(defaultButtons[0]);

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
});
