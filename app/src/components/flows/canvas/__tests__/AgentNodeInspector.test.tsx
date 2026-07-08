/**
 * Behavior tests for {@link AgentNodeInspector} — the agent-node canvas controls
 * (`agent_ref` picker + managed-tier `model` picker). The registry RPC is
 * stubbed so the component renders offline; `useT` is mocked to the key itself.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentRegistryEntry } from '../../../../services/api/agentRegistryApi';
import AgentNodeInspector from '../AgentNodeInspector';

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const AGENTS: AgentRegistryEntry[] = [
  { id: 'researcher', name: 'Researcher', description: '', source: 'default', enabled: true },
  { id: 'drafter', name: 'Drafter', description: '', source: 'custom', enabled: true },
];
const listMock = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/api/agentRegistryApi', () => ({
  agentRegistryApi: { list: listMock },
}));

describe('AgentNodeInspector', () => {
  beforeEach(() => {
    listMock.mockReset().mockResolvedValue(AGENTS);
  });

  it('lists registered agents (with an inherit default) and patches agent_ref on select', async () => {
    const onChange = vi.fn();
    render(<AgentNodeInspector config={{}} onChange={onChange} />);
    await screen.findByRole('option', { name: 'Researcher' });

    const select = screen.getByTestId('node-config-agent-ref');
    // Inherit + the two fetched agents.
    expect(within(select).getAllByRole('option')).toHaveLength(3);

    fireEvent.change(select, { target: { value: 'researcher' } });
    expect(onChange).toHaveBeenLastCalledWith({ agent_ref: 'researcher' });
  });

  it('offers the managed tiers and patches config.model with the chosen tier', () => {
    const onChange = vi.fn();
    render(<AgentNodeInspector config={{}} onChange={onChange} />);
    const model = screen.getByTestId('node-config-agent-model');
    // inherit + 4 managed tiers + custom sentinel = 6 options.
    expect(within(model).getAllByRole('option')).toHaveLength(6);
    fireEvent.change(model, { target: { value: 'chat-v1' } });
    expect(onChange).toHaveBeenLastCalledWith({ model: 'chat-v1' });
  });

  it('reveals a raw model input under Custom and patches the typed id', () => {
    const onChange = vi.fn();
    render(<AgentNodeInspector config={{}} onChange={onChange} />);
    expect(screen.queryByTestId('node-config-agent-model-custom')).not.toBeInTheDocument();
    fireEvent.change(screen.getByTestId('node-config-agent-model'), {
      target: { value: '__custom__' },
    });
    const custom = screen.getByTestId('node-config-agent-model-custom');
    fireEvent.change(custom, { target: { value: 'anthropic/claude-x' } });
    expect(onChange).toHaveBeenLastCalledWith({ model: 'anthropic/claude-x' });
  });

  it('opens the model picker in custom mode for a raw (non-tier) model id', () => {
    render(<AgentNodeInspector config={{ model: 'gpt-4o-mini' }} onChange={vi.fn()} />);
    expect(screen.getByTestId('node-config-agent-model-custom')).toHaveValue('gpt-4o-mini');
  });

  it('degrades to inherit-only when the registry fetch fails', async () => {
    listMock.mockRejectedValue(new Error('offline'));
    const onChange = vi.fn();
    render(<AgentNodeInspector config={{}} onChange={onChange} />);
    // The picker still renders (never blocks editing); only the inherit option
    // is present since no agents loaded.
    await waitFor(() => expect(listMock).toHaveBeenCalled());
    const select = screen.getByTestId('node-config-agent-ref');
    expect(within(select).getAllByRole('option')).toHaveLength(1);
  });
});
