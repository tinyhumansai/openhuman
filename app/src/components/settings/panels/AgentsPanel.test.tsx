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

const mockList = vi.mocked(agentRegistryApi.list);
const mockSetEnabled = vi.mocked(agentRegistryApi.setEnabled);
const mockRemove = vi.mocked(agentRegistryApi.remove);

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
  // --- Paths the original five cases left uncovered -----------------------
  //
  // Coverage before these: 79.03% stmts / 62.50% branch, with lines 65-66
  // (the toggle failure branch), 77-87 (`handleRemove` in full) and 131 (its
  // wiring) unexecuted. `handleRemove` is the destructive action on this
  // panel and had no test at all.

  it('surfaces the API error message when a toggle fails', async () => {
    // The catch branch prefers `err.message` over the generic i18n fallback,
    // so a caller sees *why* it failed rather than "Couldn't update the agent".
    mockSetEnabled.mockRejectedValueOnce(new Error('registry is read-only'));
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    fireEvent.click(screen.getAllByRole('switch')[1]);

    await waitFor(() => expect(screen.getByText('registry is read-only')).toBeInTheDocument());
    // The row must not be left stuck in its busy state after a failure.
    await waitFor(() => expect(screen.getAllByRole('switch')[1]).not.toBeDisabled());
  });

  it('falls back to the generic message when a toggle rejects a non-Error', async () => {
    // `err instanceof Error` is the branch under test: a string rejection
    // must not render "undefined" into the banner.
    mockSetEnabled.mockRejectedValueOnce('nope');
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    fireEvent.click(screen.getAllByRole('switch')[1]);

    await waitFor(() => expect(screen.getByText("Couldn't update the agent")).toBeInTheDocument());
  });

  it('deletes a custom agent and reloads the list', async () => {
    mockRemove.mockResolvedValue(true);
    renderPanel();
    await waitFor(() => expect(screen.getByText('Finance')).toBeInTheDocument());

    // Built-ins offer "Reset to default"; only the custom agent offers Delete.
    const deleteButtons = screen.getAllByRole('button', { name: /^Delete$/ });
    expect(deleteButtons).toHaveLength(1);
    fireEvent.click(deleteButtons[0]);

    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith('finance'));
    // `handleRemove` re-runs `load()` rather than patching state locally, so a
    // reset built-in comes back with its server-side defaults.
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));
  });

  it('resets a built-in agent through the same remove endpoint', async () => {
    // Built-ins are not deleted — the panel labels the action "Reset to
    // default" but routes it through `remove`, which restores the shipped
    // definition. Worth pinning: the label and the call diverge on purpose.
    mockRemove.mockResolvedValue(true);
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    const resetButtons = screen.getAllByRole('button', { name: /Reset to default/ });
    // orchestrator + researcher are both built-in.
    expect(resetButtons).toHaveLength(2);
    fireEvent.click(resetButtons[1]);

    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith('researcher'));
  });

  it('shows an error and stops reloading when a delete fails', async () => {
    mockRemove.mockRejectedValueOnce(new Error('agent is in use'));
    renderPanel();
    await waitFor(() => expect(screen.getByText('Finance')).toBeInTheDocument());

    fireEvent.click(screen.getAllByRole('button', { name: /^Delete$/ })[0]);

    await waitFor(() => expect(screen.getByText('agent is in use')).toBeInTheDocument());
    // A failed remove must NOT reload: reloading would redraw the row as if
    // nothing happened and drop the error the user needs to read.
    expect(mockList).toHaveBeenCalledTimes(1);
  });

  it('renders the orchestrator switch disabled so it cannot be toggled', async () => {
    // Scope correction, from review: an earlier version of this case claimed to
    // cover `handleToggle`'s ORCHESTRATOR_ID early-return (AgentsPanel.tsx:56)
    // by clicking the switch as well as asserting it disabled. It does not —
    // `AgentRow` renders the orchestrator's SettingsSwitch `disabled`, so the
    // click never reaches the handler and removing the guard alone would not
    // fail this test.
    //
    // My revert-proof did not catch that because the mutation removed the guard
    // AND the `disabled` prop together, so the case went red for the second
    // reason. A fault that changes two things proves neither individually.
    //
    // What this case honestly covers is the disabled control, which is the
    // user-facing protection; the handler guard behind it is defence in depth
    // and would need an enabled seam to exercise. Renamed to say so.
    renderPanel();
    await waitFor(() => expect(screen.getByText('Orchestrator')).toBeInTheDocument());

    const orchestratorSwitch = screen.getAllByRole('switch')[0];
    expect(orchestratorSwitch).toBeDisabled();
    fireEvent.click(orchestratorSwitch);

    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    expect(mockSetEnabled).not.toHaveBeenCalled();
  });
});
