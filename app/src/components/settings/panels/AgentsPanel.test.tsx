import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import AgentsPanel from './AgentsPanel';

vi.mock('../../../services/api/agentRegistryApi', () => ({
  agentRegistryApi: {
    list: vi.fn(),
    get: vi.fn(),
    createCustom: vi.fn(),
    update: vi.fn(),
    setEnabled: vi.fn(),
    remove: vi.fn(),
  },
}));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn() }),
}));

vi.mock('../SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

const mockList = vi.mocked(agentRegistryApi.list);
const mockSetEnabled = vi.mocked(agentRegistryApi.setEnabled);

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
    render(<AgentsPanel />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    expect(screen.getByText('Orchestrator')).toBeInTheDocument();
    expect(screen.getByText('Finance')).toBeInTheDocument();
    expect(screen.getByText('Custom')).toBeInTheDocument();
    expect(screen.getAllByText('Built-in').length).toBe(2);
  });

  it('toggles a non-orchestrator agent via setEnabled', async () => {
    mockSetEnabled.mockResolvedValue(agent({ id: 'researcher', enabled: false }));
    render(<AgentsPanel />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    const switches = screen.getAllByRole('switch');
    // Order matches list order: [orchestrator, researcher, finance].
    expect(switches[0]).toBeDisabled(); // orchestrator is always enabled
    fireEvent.click(switches[1]);

    await waitFor(() => expect(mockSetEnabled).toHaveBeenCalledWith('researcher', false));
  });

  it('opens the create editor', async () => {
    render(<AgentsPanel />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: /New agent/ }));
    expect(screen.getByRole('button', { name: 'Create agent' })).toBeInTheDocument();
    expect(screen.getByText('ID')).toBeInTheDocument();
  });

  it('shows an error when loading fails', async () => {
    mockList.mockRejectedValueOnce(new Error('boom'));
    render(<AgentsPanel />);
    await waitFor(() => expect(screen.getByText(/Couldn't load agents/)).toBeInTheDocument());
  });
});
