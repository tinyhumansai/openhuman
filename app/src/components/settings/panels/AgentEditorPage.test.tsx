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
    expect(mockNavigate).toHaveBeenCalledWith('/settings/agents', expect.anything());
  });

  it('offers the vision tier + hint as model options', async () => {
    mockCreate.mockResolvedValue(agent({ id: 'looker', name: 'Looker' }));
    renderAt('/settings/agents/new');

    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Looker' } });
    fireEvent.change(screen.getByLabelText('Description'), {
      target: { value: 'Looks at images.' },
    });
    // Both the vision hint and the resolved tier alias are selectable.
    expect(screen.getByRole('option', { name: 'hint:vision' })).toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'vision-v1' })).toBeInTheDocument();
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'hint:vision' } });

    fireEvent.click(screen.getByRole('button', { name: /Create agent/ }));
    await waitFor(() => expect(mockCreate).toHaveBeenCalledTimes(1));
    expect(mockCreate.mock.calls[0][0].model).toBe('hint:vision');
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

  it('shows source badge (custom/default) next to name in edit mode (line 259)', async () => {
    mockGet.mockResolvedValue(agent({ source: 'custom' }));
    renderAt('/settings/agents/edit/finance');

    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    // Custom agents show their source badge
    expect(await screen.findByText('Custom')).toBeInTheDocument();
  });

  it('auto-slugifies the ID field from the Name field on create (lines 280-282)', () => {
    renderAt('/settings/agents/new');

    // The Name input is present on create
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'My Cool Agent' } });

    // ID is auto-derived from name via slugify — until user touches it
    const idInput = screen.getByLabelText('ID') as HTMLInputElement;
    expect(idInput.value).toBe('my-cool-agent');
  });

  it('allows manual ID override once the ID field is touched (lines 280-282)', () => {
    renderAt('/settings/agents/new');

    const idInput = screen.getByLabelText('ID');
    fireEvent.change(idInput, { target: { value: 'custom-id' } });

    // After touching, changing name should NOT overwrite the custom ID
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'New Name' } });
    expect((screen.getByLabelText('ID') as HTMLInputElement).value).toBe('custom-id');
  });

  it('shows custom model text input when __custom__ model is selected (line 350)', () => {
    renderAt('/settings/agents/new');

    // Select the custom model option
    fireEvent.change(screen.getByRole('combobox'), { target: { value: '__custom__' } });

    // A custom model text input appears with the custom placeholder as aria-label
    // The en.ts label is: 'settings.agents.editor.modelCustomPlaceholder': 'e.g. anthropic/claude-sonnet-4'
    const customInput = screen.getByLabelText(/e\.g\. anthropic/i);
    expect(customInput).toBeInTheDocument();
  });

  it('updates system prompt textarea on change (line 373)', () => {
    renderAt('/settings/agents/new');

    // The label uses t('settings.agents.editor.systemPrompt') → 'System prompt (optional)'
    const promptArea = screen.getByLabelText('System prompt (optional)') as HTMLTextAreaElement;
    fireEvent.change(promptArea, { target: { value: 'Be helpful and precise.' } });

    expect(promptArea.value).toBe('Be helpful and precise.');
  });

  it('shows All Tools chip when toolAllowlist contains "*" (line 390)', async () => {
    renderAt('/settings/agents/new');

    // Open tool picker via the Add tools button
    fireEvent.click(screen.getByText('Add tools'));
    await waitFor(() => expect(mockAvailableTools).toHaveBeenCalled());
    await screen.findByText('Search the web for information.');

    // Toggle all tools via the "Allow all tools (*)" button
    fireEvent.click(screen.getByRole('button', { name: /Allow all tools/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));

    // The "All tools" chip should appear (toolsAllSelected branch)
    await waitFor(() => {
      expect(screen.getByText('All tools')).toBeInTheDocument();
    });
  });

  it('removes a selected tool chip via its X button (line 410)', async () => {
    renderAt('/settings/agents/new');

    // Open picker and select a tool
    fireEvent.click(screen.getByText('Add tools'));
    await waitFor(() => expect(mockAvailableTools).toHaveBeenCalled());
    await screen.findByText('Search the web for information.');

    fireEvent.click(screen.getByText('web_search'));
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));

    // Chip for web_search appears
    await waitFor(() => expect(screen.getAllByText('web_search').length).toBeGreaterThan(0));

    // Click the X remove button on the chip — the aria-label is 'Remove web_search'
    // since t('settings.agents.editor.removeToolAria') = 'Remove {tool}'
    const removeBtn = screen.getByRole('button', { name: 'Remove web_search' });
    fireEvent.click(removeBtn);

    // The chip remove button should be gone
    await waitFor(() => {
      expect(screen.queryByRole('button', { name: 'Remove web_search' })).not.toBeInTheDocument();
    });
  });
});
