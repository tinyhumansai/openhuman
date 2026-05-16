import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { loadAISettings, loadLocalProviderSnapshot } from '../../../../services/api/aiSettingsApi';
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
  serializeProviderRef: vi.fn((r: { kind: string; model?: string }) =>
    r.kind === 'primary' ? 'cloud' : r.kind === 'local' ? `ollama:${r.model}` : `cloud:${r.model}`
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
      type: 'openhuman' as const,
      endpoint: 'https://api.openhuman.ai/v1',
      default_model: 'reasoning-v1',
      has_api_key: false,
    },
  ],
  primaryCloudId: 'p_oh_x',
  routing: {
    reasoning: { kind: 'primary' as const },
    agentic: { kind: 'primary' as const },
    coding: { kind: 'primary' as const },
    memory: { kind: 'primary' as const },
    embeddings: { kind: 'primary' as const },
    heartbeat: { kind: 'primary' as const },
    learning: { kind: 'primary' as const },
    subconscious: { kind: 'primary' as const },
  },
};

describe('AIPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(loadAISettings).mockResolvedValue(baseSettings);
    vi.mocked(loadLocalProviderSnapshot).mockResolvedValue({
      status: null,
      diagnostics: null,
      presets: null,
      installedModels: [],
    });
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
});
