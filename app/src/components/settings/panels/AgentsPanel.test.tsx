import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import AgentsPanel from './AgentsPanel';

vi.mock('../../../services/api/agentRegistryApi', () => ({
  agentRegistryApi: {
    list: vi.fn(),
    get: vi.fn(),
    availableTools: vi.fn(),
    createCustom: vi.fn(),
    update: vi.fn(),
    setEnabled: vi.fn(),
    remove: vi.fn(),
  },
}));

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

const mockNavigateToSettings = vi.fn();
vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: mockNavigateToSettings,
    breadcrumbs: [],
  }),
}));

vi.mock('../SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

const mockList = vi.mocked(agentRegistryApi.list);
const mockSetEnabled = vi.mocked(agentRegistryApi.setEnabled);

const renderPanel = () =>
  render(
    <MemoryRouter>
      <AgentsPanel />
    </MemoryRouter>
  );

function agent(overrides: Partial<AgentRegistryEntry> = {}): AgentRegistryEntry {
  return {
    id: 'researcher',
    name: 'Researcher',
    description: 'Looks things up.',
    source: 'default',
    enabled: true,
    tool_allowlist: ['*'],
    ...overrides,
  };
}

describe('AgentsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockList.mockResolvedValue([
      agent({ id: 'orchestrator', name: 'Orchestrator' }),
      agent({ id: 'researcher', name: 'Researcher' }),
      agent({
        id: 'finance',
        name: 'Finance',
        source: 'custom',
        tool_allowlist: ['memory.search'],
      }),
    ]);
  });

  it('lists agents with their source badges', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    expect(screen.getByText('Orchestrator')).toBeInTheDocument();
    expect(screen.getByText('Finance')).toBeInTheDocument();
    expect(screen.getByText('Custom')).toBeInTheDocument();
    expect(screen.getAllByText('Built-in').length).toBe(2);
  });

  it('toggles a non-orchestrator agent via setEnabled', async () => {
    mockSetEnabled.mockResolvedValue(agent({ id: 'researcher', enabled: false }));
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    const switches = screen.getAllByRole('switch');
    // Order matches list order: [orchestrator, researcher, finance].
    expect(switches[0]).toBeDisabled(); // orchestrator is always enabled
    fireEvent.click(switches[1]);

    await waitFor(() => expect(mockSetEnabled).toHaveBeenCalledWith('researcher', false));
  });

  it('navigates to the create editor page', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: /New agent/ }));
    // Routes through useSettingsNavigation so the desktop modal backdrop is
    // preserved; the hook prefixes `/settings/`.
    expect(mockNavigateToSettings).toHaveBeenCalledWith('agents/new');
  });

  it('only offers Edit for custom agents and navigates to the edit page', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText('Finance')).toBeInTheDocument());
    // Two built-ins (orchestrator, researcher) + one custom (finance) — only the
    // custom agent exposes an Edit button.
    const editButtons = screen.getAllByRole('button', { name: /Edit/ });
    expect(editButtons).toHaveLength(1);
    fireEvent.click(editButtons[0]);
    expect(mockNavigateToSettings).toHaveBeenCalledWith('agents/edit/finance');
  });

  it('shows an error when loading fails', async () => {
    mockList.mockRejectedValueOnce(new Error('boom'));
    renderPanel();
    await waitFor(() => expect(screen.getByText(/Couldn't load agents/)).toBeInTheDocument());
  });
});
