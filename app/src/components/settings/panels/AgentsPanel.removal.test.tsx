import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import AgentsPanel from './AgentsPanel';

/**
 * The destructive half of `AgentsPanel`.
 *
 * `handleRemove` (panel :75-90) was an entirely uncovered function, and it is
 * the one that deletes a registry entry. The same handler backs two differently
 * labelled buttons — "Delete" for a custom agent, "Reset" for a built-in
 * (:210-216) — so the copy is the only thing telling a user which of those two
 * things they are about to do.
 *
 * Also covers the failure arms of both `handleRemove` and `handleToggle`,
 * including their non-`Error` fallbacks onto `settings.agents.actionFailed`.
 *
 * NOT covered, deliberately: `handleToggle`'s `agent.id === ORCHESTRATOR_ID`
 * early return (:56). The orchestrator's switch is rendered `disabled`
 * (:175, `disabled={busy || isOrchestrator}`), so the guard is a second line of
 * defence that a click cannot reach. The existing suite asserts the reachable
 * half — that the switch is disabled — which is the right assertion.
 */

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

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

const mockList = vi.mocked(agentRegistryApi.list);
const mockSetEnabled = vi.mocked(agentRegistryApi.setEnabled);
const mockRemove = vi.mocked(agentRegistryApi.remove);

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

const renderPanel = () =>
  render(
    <MemoryRouter>
      <AgentsPanel />
    </MemoryRouter>
  );

/** [orchestrator, researcher (built-in), finance (custom)] */
const defaultList = () => [
  agent({ id: 'orchestrator', name: 'Orchestrator' }),
  agent({ id: 'researcher', name: 'Researcher' }),
  agent({ id: 'finance', name: 'Finance', source: 'custom' }),
];

const deleteButtons = () => screen.getAllByRole('button', { name: /delete/i });
const resetButtons = () => screen.getAllByRole('button', { name: /reset/i });

beforeEach(() => {
  vi.clearAllMocks();
  mockList.mockResolvedValue(defaultList());
  // `agentRegistryApi.remove` returns Promise<boolean> (agentRegistryApi.ts:147-154),
  // so the success fixture must resolve `true`. Resolving `undefined as never`
  // modelled a value the API cannot return, and the `as never` cast was the
  // tell — it silenced the type error that was pointing this out. It also hid
  // whether the panel handles a `false` (refused) result differently from a
  // `true` one. Caught in review by `coderabbitai`.
  mockRemove.mockResolvedValue(true);
  mockSetEnabled.mockResolvedValue(agent({ id: 'researcher', enabled: false }));
});

describe('AgentsPanel — removing an agent', () => {
  it('removes the agent the button belongs to, then reloads the list', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText('Finance')).toBeInTheDocument());
    expect(mockList).toHaveBeenCalledTimes(1);

    fireEvent.click(deleteButtons()[0]);

    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith('finance'));
    // The list must be re-fetched, not patched locally: a removal can cascade.
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));
  });

  it('resets a built-in agent through the same removal call', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    // "Reset" on a built-in is the same destructive RPC as "Delete"; only the
    // copy differs, so this pins that the right id is sent.
    fireEvent.click(resetButtons()[0]);
    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith('orchestrator'));
  });

  it('labels the action Delete for custom agents and Reset for built-ins', async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText('Finance')).toBeInTheDocument());

    // Two built-ins (orchestrator, researcher) and one custom (finance).
    expect(resetButtons()).toHaveLength(2);
    expect(deleteButtons()).toHaveLength(1);
  });

  it('surfaces the failure message when removal rejects', async () => {
    mockRemove.mockRejectedValue(new Error('agent is referenced by a flow'));
    renderPanel();
    await waitFor(() => expect(screen.getByText('Finance')).toBeInTheDocument());

    fireEvent.click(deleteButtons()[0]);
    expect(await screen.findByText(/agent is referenced by a flow/)).toBeInTheDocument();
    // A failed removal must not be reported as done.
    expect(mockList).toHaveBeenCalledTimes(1);
  });

  it('falls back to the generic copy when removal rejects with a non-Error', async () => {
    mockRemove.mockRejectedValue({ code: 500 });
    renderPanel();
    await waitFor(() => expect(screen.getByText('Finance')).toBeInTheDocument());

    fireEvent.click(deleteButtons()[0]);
    expect(await screen.findByText(/Couldn.t update the agent/)).toBeInTheDocument();
  });
});

describe('AgentsPanel — toggle failures', () => {
  it('surfaces the failure message when setEnabled rejects', async () => {
    mockSetEnabled.mockRejectedValue(new Error('registry is read-only'));
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    // Index 1 — the orchestrator's switch at index 0 is disabled.
    fireEvent.click(screen.getAllByRole('switch')[1]);
    expect(await screen.findByText(/registry is read-only/)).toBeInTheDocument();
  });

  it('falls back to the generic copy when setEnabled rejects with a non-Error', async () => {
    mockSetEnabled.mockRejectedValue('nope');
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    fireEvent.click(screen.getAllByRole('switch')[1]);
    expect(await screen.findByText(/Couldn.t update the agent/)).toBeInTheDocument();
  });

  // Regression guard rather than a mutation-proven test: this panel has no
  // optimistic update to begin with (`setAgents` runs only on success), so
  // "the row did not move" cannot be broken by a one-line change. Kept because
  // it pins that difference from AgentAccessPanel, which DOES update
  // optimistically and rolls back — the two panels must not be assumed alike.
  it('leaves the row untouched when the toggle fails', async () => {
    mockSetEnabled.mockRejectedValue(new Error('registry is read-only'));
    renderPanel();
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    const before = screen.getAllByRole('switch')[1].getAttribute('aria-checked');
    fireEvent.click(screen.getAllByRole('switch')[1]);
    await screen.findByText(/registry is read-only/);

    // No optimistic update to roll back — the switch must still reflect the
    // server state, not the click.
    expect(screen.getAllByRole('switch')[1]).toHaveAttribute('aria-checked', before as string);
  });
});
