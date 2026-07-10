import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import OrchestrationPage from '../OrchestrationPage';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// Stub the four data-backed panels so the shell's tab routing is tested in
// isolation (the panels have their own unit tests).
vi.mock('../../components/orchestration/AgentChatPanel', () => ({
  default: () => <div data-testid="panel-agent" />,
}));
vi.mock('../../components/orchestration/ConnectionsPanel', () => ({
  default: ({ onDiscover }: { onDiscover?: () => void }) => (
    <button data-testid="panel-connections" onClick={onDiscover}>
      connections
    </button>
  ),
}));
vi.mock('../../components/orchestration/DiscoverPanel', () => ({
  default: () => <div data-testid="panel-discover" />,
}));
vi.mock('../../components/orchestration/UsagePanel', () => ({
  default: () => <div data-testid="panel-usage" />,
}));

describe('OrchestrationPage shell', () => {
  it('defaults to the agent chat panel', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: ['/orchestration'] });
    });
    expect(screen.getByTestId('panel-agent')).toBeInTheDocument();
  });

  it.each([
    ['connections', 'panel-connections'],
    ['discover', 'panel-discover'],
    ['usage', 'panel-usage'],
  ])('renders the %s panel from ?tab=%s', async (tab, testId) => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: [`/orchestration?tab=${tab}`] });
    });
    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it('projects a sub-nav that switches tabs', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: ['/orchestration'] });
    });
    // Sub-nav renders via the sidebar portal once the outlet mounts. `usage` is
    // now a chip sub of the `network` tab, so the top-level nav exposes `network`.
    const networkNav = await screen.findByTestId('two-pane-nav-network');
    await act(async () => {
      fireEvent.click(networkNav);
    });
    await waitFor(() => expect(screen.getByTestId('panel-connections')).toBeInTheDocument());
  });

  it('lets the connections panel jump to discover via its callback', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, {
        initialEntries: ['/orchestration?tab=connections'],
      });
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('panel-connections'));
    });
    await waitFor(() => expect(screen.getByTestId('panel-discover')).toBeInTheDocument());
  });
});
