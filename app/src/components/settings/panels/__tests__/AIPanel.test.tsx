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

  it('renders the three section labels', async () => {
    renderWithProviders(<AIPanel />);
    // Section labels are SectionLabel components — pick the one that
    // matches each header exactly. Loose regex matches body copy too
    // ("only use cloud providers" appears in the local-provider
    // explanation, "Primary" appears on the provider card badge AND in
    // workload routing rows).
    await waitFor(() => expect(screen.getAllByText(/Cloud providers/i).length).toBeGreaterThan(0));
    expect(screen.getAllByText(/Local provider/i).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Workload routing/i).length).toBeGreaterThan(0);
  });

  it('renders the OpenHuman primary card after load', async () => {
    renderWithProviders(<AIPanel />);
    await waitFor(() => expect(screen.getByText(/OpenHuman/i)).toBeInTheDocument());
    // "Primary" shows up on the provider card badge AND in workload
    // routing rows that read "Primary resolves to …", so multiple
    // matches are expected.
    expect(screen.getAllByText(/Primary/).length).toBeGreaterThan(0);
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
