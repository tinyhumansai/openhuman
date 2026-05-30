import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import AgentEditorPage from './AgentEditorPage';

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

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

const mockGet = vi.mocked(agentRegistryApi.get);
const mockAvailableTools = vi.mocked(agentRegistryApi.availableTools);
const mockCreate = vi.mocked(agentRegistryApi.createCustom);
const mockUpdate = vi.mocked(agentRegistryApi.update);

function agent(overrides: Partial<AgentRegistryEntry> = {}): AgentRegistryEntry {
  return {
    id: 'finance',
    name: 'Finance',
    description: 'Crunches numbers.',
    source: 'custom',
    enabled: true,
    model: 'reasoning-v1',
    system_prompt: 'Be precise.',
    tool_allowlist: ['memory.search'],
    ...overrides,
  };
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/settings/agents/new" element={<AgentEditorPage />} />
        <Route path="/settings/agents/edit/:id" element={<AgentEditorPage />} />
      </Routes>
    </MemoryRouter>
  );
}

describe('AgentEditorPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockAvailableTools.mockResolvedValue([
      { name: 'web_search', description: 'Search the web for information.' },
      { name: 'memory.search', description: 'Search the user memory store.' },
    ]);
  });

  it('creates a custom agent from the form', async () => {
    mockCreate.mockResolvedValue(agent({ id: 'helper', name: 'Helper' }));
    renderAt('/settings/agents/new');

    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Helper' } });
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Helps out.' } });
    // Model dropdown offers known tiers/hints.
    expect(screen.getByRole('option', { name: 'reasoning-v1' })).toBeInTheDocument();
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'hint:coding' } });

    fireEvent.click(screen.getByRole('button', { name: /Create agent/ }));

    await waitFor(() => expect(mockCreate).toHaveBeenCalledTimes(1));
    const arg = mockCreate.mock.calls[0][0];
    expect(arg.id).toBe('helper'); // auto-slugified from name
    expect(arg.name).toBe('Helper');
    expect(arg.model).toBe('hint:coding');
    expect(mockNavigate).toHaveBeenCalledWith('/settings/agents');
  });

  it('picks tools from the searchable modal and shows chips', async () => {
    renderAt('/settings/agents/new');

    fireEvent.click(screen.getByText('Add tools'));
    await waitFor(() => expect(mockAvailableTools).toHaveBeenCalled());

    // Tool descriptions are shown in the modal.
    expect(await screen.findByText('Search the web for information.')).toBeInTheDocument();

    // Search filters the list.
    fireEvent.change(screen.getByLabelText('Search tools…'), { target: { value: 'web' } });
    expect(screen.queryByText('Search the user memory store.')).not.toBeInTheDocument();

    fireEvent.click(screen.getByText('web_search'));
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));

    // Chip for the selected tool appears on the page.
    await waitFor(() => expect(screen.getAllByText('web_search').length).toBeGreaterThan(0));
  });

  it('loads an existing agent for editing with a read-only name', async () => {
    mockGet.mockResolvedValue(agent());
    renderAt('/settings/agents/edit/finance');

    await waitFor(() => expect(mockGet).toHaveBeenCalledWith('finance'));
    // Name is read-only in edit mode — no editable Name input is rendered.
    expect(screen.queryByLabelText('Name')).toBeNull();
    expect(screen.getByDisplayValue('Crunches numbers.')).toBeInTheDocument();
    expect((screen.getByRole('combobox') as HTMLSelectElement).value).toBe('reasoning-v1');

    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Updated.' } });
    fireEvent.click(screen.getByRole('button', { name: /^Save$/ }));
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith('finance', expect.any(Object)));
  });

  it('shows a read-only notice for built-in agents instead of the form', async () => {
    mockGet.mockResolvedValue(agent({ id: 'researcher', name: 'Researcher', source: 'default' }));
    renderAt('/settings/agents/edit/researcher');

    await waitFor(() => expect(mockGet).toHaveBeenCalledWith('researcher'));
    expect(screen.getByText(/Built-in agents can.t be edited/)).toBeInTheDocument();
    // No editable form fields are rendered.
    expect(screen.queryByLabelText('Description')).toBeNull();
    expect(screen.queryByRole('button', { name: /^Save$/ })).toBeNull();
  });
});
