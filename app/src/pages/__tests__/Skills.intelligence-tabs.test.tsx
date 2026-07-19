import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

// The "API keys" group tabs (llm / voice / embeddings / search / usage /
// composio-key) render relocated settings panels inside the Connections
// two-pane shell. Stub each so the per-tab branches in Skills are exercised
// without their deep trees. The LLM tab renders the 3-chip LlmConnectionsPanel.
vi.mock('../../components/settings/panels/LlmConnectionsPanel', () => ({
  default: () => <div data-testid="skills-llm-panel" />,
}));
vi.mock('../../components/settings/panels/VoicePanel', () => ({
  default: () => <div data-testid="skills-voice-panel" />,
}));
vi.mock('../../components/settings/panels/EmbeddingsPanel', () => ({
  default: () => <div data-testid="skills-embeddings-panel" />,
}));
vi.mock('../../components/settings/panels/SearchPanel', () => ({
  default: () => <div data-testid="skills-search-panel" />,
}));
vi.mock('../../components/settings/panels/ComposioPanel', () => ({
  default: () => <div data-testid="skills-composio-panel" />,
}));
vi.mock('../../components/settings/panels/UsagePanel', () => ({
  default: () => <div data-testid="skills-usage-panel" />,
}));

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));
vi.mock('../../lib/skills/skillsApi', () => ({
  installSkill: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../lib/skills/hooks', () => ({
  useAvailableSkills: () => ({ skills: [], loading: false, refresh: vi.fn() }),
}));
vi.mock('../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: [],
    connectionByToolkit: new Map(),
    connectionsByToolkit: new Map(),
    refresh: vi.fn(),
    loading: false,
    error: null,
  }),
  useAgentReadyComposioToolkits: () => ({
    agentReady: new Set<string>(),
    loading: true,
    error: null,
  }),
}));
vi.mock('../../lib/coreState/store', async () => {
  const actual = await vi.importActual<typeof import('../../lib/coreState/store')>(
    '../../lib/coreState/store'
  );
  return { ...actual, getCoreStateSnapshot: () => ({ snapshot: { sessionToken: 'jwt-abc' } }) };
});
vi.mock('../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../utils/tauriCommands')>(
    '../../utils/tauriCommands'
  );
  return {
    ...actual,
    openhumanComposioGetMode: vi.fn(async () => ({
      result: { mode: 'backend', api_key_set: true },
      logs: [],
    })),
  };
});

describe('Skills page — API keys (intelligence) tabs', () => {
  it.each([
    ['llm', 'skills-llm-panel'],
    ['voice', 'skills-voice-panel'],
    ['embeddings', 'skills-embeddings-panel'],
    ['search', 'skills-search-panel'],
    ['usage', 'skills-usage-panel'],
    ['composio-key', 'skills-composio-panel'],
  ])('renders the %s panel for ?tab=%s', async (tab, testId) => {
    renderWithProviders(<Skills />, { initialEntries: [`/connections?tab=${tab}`] });

    await waitFor(() => {
      expect(screen.getByTestId(testId)).toBeInTheDocument();
    });
  });
});
