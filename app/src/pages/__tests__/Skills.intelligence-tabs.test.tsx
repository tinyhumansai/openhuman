import { screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

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
vi.mock('../../components/settings/panels/LiveVoicePanel', () => ({
  default: () => <div data-testid="skills-live-voice-panel" />,
}));
vi.mock('../../components/settings/panels/EmbeddingsPanel', () => ({
  default: () => <div data-testid="skills-embeddings-panel" />,
}));
vi.mock('../../components/settings/panels/SearchPanel', () => ({
  default: () => <div data-testid="skills-search-panel" />,
}));
vi.mock('../../components/settings/panels/ComputerPanel', () => ({
  default: ({ section }: { section: string }) => (
    <div data-testid="skills-computer-panel" data-section={section} />
  ),
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
  it('groups Computer (desktop and browser) with integrations', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=computer'] });
    const group = screen.getByText('Integrations').parentElement?.parentElement;
    expect(group).toBeTruthy();
    expect(within(group!).getByTestId('two-pane-nav-computer')).toBeInTheDocument();
    expect(within(group!).queryByTestId('two-pane-nav-browser')).not.toBeInTheDocument();
    expect(within(group!).queryByTestId('two-pane-nav-desktop')).not.toBeInTheDocument();
  });

  it.each([
    ['/connections?tab=browser', 'browser'],
    ['/connections?tab=desktop', 'desktop'],
    ['/connections?tab=computer&section=models', 'models'],
    ['/connections?tab=computer', 'desktop'],
  ])('routes %s to the Computer %s section', async (entry, section) => {
    renderWithProviders(<Skills />, { initialEntries: [entry] });
    await waitFor(() => {
      expect(screen.getByTestId('skills-computer-panel')).toHaveAttribute('data-section', section);
    });
  });

  it.each([
    ['llm', 'skills-llm-panel'],
    ['voice', 'skills-voice-panel'],
    ['voice-agents', 'skills-live-voice-panel'],
    // Aliases for the live voice agents tab.
    ['voice-agent', 'skills-live-voice-panel'],
    ['live-voice', 'skills-live-voice-panel'],
    ['embeddings', 'skills-embeddings-panel'],
    ['search', 'skills-search-panel'],
    ['computer', 'skills-computer-panel'],
    ['usage', 'skills-usage-panel'],
    ['composio-key', 'skills-composio-panel'],
  ])('renders the %s panel for ?tab=%s', async (tab, testId) => {
    renderWithProviders(<Skills />, { initialEntries: [`/connections?tab=${tab}`] });

    await waitFor(() => {
      expect(screen.getByTestId(testId)).toBeInTheDocument();
    });
  });

  it('lists Voice agents in the API keys group and marks it selected via its alias', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/connections?tab=live-voice'] });
    const row = await screen.findByTestId('two-pane-nav-voice-agents');
    expect(row).toHaveAttribute('aria-current', 'page');
    const group = screen.getByText('API keys').parentElement?.parentElement;
    expect(within(group!).getByTestId('two-pane-nav-voice-agents')).toBeInTheDocument();
  });
});
