import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentDefinitionDisplay } from '../../services/api/agentLibraryApi';
import AgentsLibraryPanel from './AgentsLibraryPanel';

const mockListDefinitions = vi.fn();

vi.mock('../../services/api/agentLibraryApi', () => ({
  agentLibraryApi: { listDefinitions: (...args: unknown[]) => mockListDefinitions(...args) },
}));

function agent(overrides: Partial<AgentDefinitionDisplay> = {}): AgentDefinitionDisplay {
  return {
    id: 'researcher',
    display_name: 'Researcher',
    when_to_use: 'Use for focused research.',
    tier: 'worker',
    model: { kind: 'hint', value: 'reasoning' },
    direct_tool_count: 1,
    direct_tool_names: ['web_search'],
    uses_wildcard_tools: false,
    subagent_ids: [],
    includes_profile: false,
    includes_memory_md: false,
    includes_memory_context: false,
    can_run_as_user_facing_worker: true,
    write_capable: false,
    source: 'builtin',
    ...overrides,
  };
}

describe('AgentsLibraryPanel', () => {
  beforeEach(() => {
    mockListDefinitions.mockReset();
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  });

  it('shows loading then an empty state', async () => {
    mockListDefinitions.mockResolvedValueOnce([]);
    render(<AgentsLibraryPanel onRunAgentTask={vi.fn()} />);

    expect(screen.getByText(/loading agents/i)).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByText(/no runnable agents/i)).toBeInTheDocument();
    });
  });

  it('shows a load error', async () => {
    mockListDefinitions.mockRejectedValueOnce(new Error('registry unavailable'));
    render(<AgentsLibraryPanel onRunAgentTask={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/registry unavailable/i)).toBeInTheDocument();
    });
  });

  it('renders safe metadata and filters non-runnable chat agents', async () => {
    mockListDefinitions.mockResolvedValueOnce([
      agent({ model: { kind: 'inherit' } }),
      agent({
        id: 'orchestrator',
        display_name: 'Orchestrator',
        tier: 'chat',
        can_run_as_user_facing_worker: false,
      }),
    ]);
    render(<AgentsLibraryPanel onRunAgentTask={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('Researcher')).toBeInTheDocument();
    });
    expect(screen.queryByText('Orchestrator')).not.toBeInTheDocument();
    expect(screen.getByText('Inherit')).toBeInTheDocument();
    expect(screen.getByText('Read-only')).toBeInTheDocument();
    expect(screen.getByText('1 tool')).toBeInTheDocument();
  });

  it('runs a one-off task for the selected agent', async () => {
    mockListDefinitions.mockResolvedValueOnce([agent()]);
    const onRun = vi.fn().mockResolvedValue(undefined);
    render(<AgentsLibraryPanel onRunAgentTask={onRun} />);

    await waitFor(() => {
      expect(screen.getByText('Researcher')).toBeInTheDocument();
    });
    fireEvent.change(screen.getByPlaceholderText(/task for this agent/i), {
      target: { value: 'Find current docs' },
    });
    fireEvent.click(screen.getByRole('button', { name: /run task/i }));

    await waitFor(() => expect(onRun).toHaveBeenCalledTimes(1));
    expect(onRun).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'researcher' }),
      'Find current docs'
    );
  });

  it('copies an agent id', async () => {
    mockListDefinitions.mockResolvedValueOnce([agent()]);
    render(<AgentsLibraryPanel onRunAgentTask={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('Researcher')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /copy id/i }));
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('researcher');
  });

  it('does not show copied when clipboard write is unavailable', async () => {
    mockListDefinitions.mockResolvedValueOnce([agent()]);
    Object.assign(navigator, { clipboard: {} });
    render(<AgentsLibraryPanel onRunAgentTask={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('Researcher')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /copy id/i }));
    expect(screen.queryByRole('button', { name: /copied/i })).not.toBeInTheDocument();
  });
});
