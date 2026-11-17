import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { MemoryPolicy } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import MemorySettingsTab from './MemorySettingsTab';

const hoisted = vi.hoisted(() => ({ get: vi.fn(), set: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryPolicyGet: (...a: unknown[]) => hoisted.get(...a),
  memoryPolicySet: (...a: unknown[]) => hoisted.set(...a),
}));

const POLICY: MemoryPolicy = {
  log_conversations: true,
  recall: {
    enabled: true,
    budget_tokens: 1500,
    learnings_limit: 8,
    brain_limit: 6,
    history_limit: 6,
    team_limit: 4,
    build_beliefs_every: 20,
    pre_turn_timeout_ms: 1500,
    compaction_timeout_ms: 5000,
    build_delay_secs: 30,
  },
  root: 'user:me',
  agent_id: 'main',
  host_bound: false,
};

beforeEach(() => {
  hoisted.get.mockReset().mockResolvedValue(POLICY);
  hoisted.set.mockReset();
});

describe('MemorySettingsTab', () => {
  it('shows the recall policy and where memory is filed', async () => {
    renderWithProviders(<MemorySettingsTab />);
    expect(await screen.findByLabelText('Size limit')).toHaveValue(1500);
    expect(screen.getByLabelText('Learnings')).toHaveValue(8);
    expect(screen.getByLabelText('Brain documents')).toHaveValue(6);
    expect(screen.getByLabelText('Past conversations')).toHaveValue(6);
    expect(screen.getByLabelText('Team memory')).toHaveValue(4);
    expect(screen.getByLabelText('Build beliefs every')).toHaveValue(20);
    expect(screen.getByLabelText('Wait for memory')).toHaveValue(1500);
    expect(screen.getByTestId('memory-settings-recall')).toBeChecked();
    const identity = screen.getByTestId('memory-settings-identity');
    expect(identity).toHaveTextContent('user:me');
    expect(identity).toHaveTextContent('main');
    expect(screen.queryByTestId('memory-settings-host-bound')).not.toBeInTheDocument();
  });

  it('ends with the erase-all-memory control', async () => {
    renderWithProviders(<MemorySettingsTab />);
    expect(await screen.findByTestId('memory-erase-card')).toBeInTheDocument();
    expect(screen.getByTestId('memory-erase-open')).toHaveTextContent('Erase all memory');
  });

  it('says when the host pins the root and agent', async () => {
    hoisted.get.mockResolvedValue({ ...POLICY, host_bound: true });
    renderWithProviders(<MemorySettingsTab />);
    expect(await screen.findByTestId('memory-settings-host-bound')).toBeInTheDocument();
  });

  it('turns recall off and disables the pack limits', async () => {
    hoisted.set.mockResolvedValue({ ...POLICY, recall: { ...POLICY.recall, enabled: false } });
    renderWithProviders(<MemorySettingsTab />);
    fireEvent.click(await screen.findByTestId('memory-settings-recall'));
    await waitFor(() => expect(hoisted.set).toHaveBeenCalledWith({ recall_enabled: false }));
    await waitFor(() => expect(screen.getByLabelText('Size limit')).toBeDisabled());
    // Belief building does not depend on recall.
    expect(screen.getByLabelText('Build beliefs every')).not.toBeDisabled();
  });

  it('saves an in-range value on commit and reverts an out-of-range one', async () => {
    hoisted.set.mockResolvedValue({ ...POLICY, recall: { ...POLICY.recall, budget_tokens: 3000 } });
    renderWithProviders(<MemorySettingsTab />);
    const budget = await screen.findByLabelText('Size limit');

    fireEvent.change(budget, { target: { value: '50' } });
    fireEvent.blur(budget);
    expect(hoisted.set).not.toHaveBeenCalled();
    expect(budget).toHaveValue(1500);

    fireEvent.change(budget, { target: { value: '1500' } });
    fireEvent.blur(budget);
    expect(hoisted.set).not.toHaveBeenCalled();

    fireEvent.change(budget, { target: { value: '3000' } });
    fireEvent.blur(budget);
    await waitFor(() => expect(hoisted.set).toHaveBeenCalledWith({ budget_tokens: 3000 }));
    await waitFor(() => expect(budget).toHaveValue(3000));
  });

  it('accepts 0 to turn belief building off', async () => {
    hoisted.set.mockResolvedValue({
      ...POLICY,
      recall: { ...POLICY.recall, build_beliefs_every: 0 },
    });
    renderWithProviders(<MemorySettingsTab />);
    const every = await screen.findByLabelText('Build beliefs every');
    fireEvent.change(every, { target: { value: '0' } });
    fireEvent.blur(every);
    await waitFor(() => expect(hoisted.set).toHaveBeenCalledWith({ build_beliefs_every: 0 }));
  });

  it('shows a rejected save and restores the stored value', async () => {
    hoisted.set.mockRejectedValue(new Error('INVALID_REQUEST: team_limit out of range'));
    renderWithProviders(<MemorySettingsTab />);
    const team = await screen.findByLabelText('Team memory');
    fireEvent.change(team, { target: { value: '10' } });
    fireEvent.blur(team);
    expect(await screen.findByTestId('memory-settings-error')).toHaveTextContent('out of range');
    await waitFor(() => expect(team).toHaveValue(4));
  });

  it('shows a load error', async () => {
    hoisted.get.mockRejectedValue(new Error('MEMORY_OFF'));
    renderWithProviders(<MemorySettingsTab />);
    expect(await screen.findByTestId('memory-settings-error')).toHaveTextContent('MEMORY_OFF');
  });
});
