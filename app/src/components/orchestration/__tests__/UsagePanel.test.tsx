import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import UsagePanel from '../UsagePanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const getBalance = vi.hoisted(() => vi.fn());
const getTeamUsage = vi.hoisted(() => vi.fn());
vi.mock('../../../services/api/creditsApi', () => ({ creditsApi: { getBalance, getTeamUsage } }));

const getTokenjuiceSavings = vi.hoisted(() => vi.fn());
vi.mock('../../../utils/tauriCommands/tokenjuice', () => ({ getTokenjuiceSavings }));

const list = vi.hoisted(() => vi.fn());
vi.mock('../../../agentworld/AgentWorldShell', () => ({
  apiClient: { orchestrationPairing: { list } },
}));

describe('UsagePanel', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders every stat tile when all sources succeed', async () => {
    getBalance.mockResolvedValue({ promotionBalanceUsd: 5, teamTopupUsd: 10 });
    getTeamUsage.mockResolvedValue({
      cycleSpentUsd: 3.5,
      cycleBudgetUsd: 20,
      insights: { totals: { inferenceCalls: 42, integrationCalls: 7 } },
    });
    getTokenjuiceSavings.mockResolvedValue({ total: { tokensSaved: 1234, costSavedUsd: 0.9 } });
    list.mockResolvedValue({ stats: { contactCount: 3 }, contacts: { contacts: [] } });

    render(<UsagePanel />);
    await waitFor(() => expect(screen.getByTestId('orch-usage-panel')).toBeInTheDocument());
    expect(screen.getByTestId('orch-usage-connections')).toHaveTextContent('3');
    expect(screen.getByTestId('orch-usage-balance')).toHaveTextContent('$15.00');
    expect(screen.getByTestId('orch-usage-cycle-spend')).toHaveTextContent('$3.50');
    expect(screen.getByTestId('orch-usage-inference-calls')).toHaveTextContent('42');
    expect(screen.getByTestId('orch-usage-tokens-saved')).toHaveTextContent('1,234');
  });

  it('degrades unavailable sources to a dash without blanking the page', async () => {
    getBalance.mockRejectedValue(new Error('billing offline'));
    getTeamUsage.mockRejectedValue(new Error('billing offline'));
    getTokenjuiceSavings.mockRejectedValue(new Error('no core'));
    list.mockRejectedValue(new Error('no relay'));

    render(<UsagePanel />);
    await waitFor(() => expect(screen.getByTestId('orch-usage-panel')).toBeInTheDocument());
    expect(screen.getByTestId('orch-usage-balance')).toHaveTextContent('—');
    expect(screen.getByTestId('orch-usage-connections')).toHaveTextContent('—');
  });
});
