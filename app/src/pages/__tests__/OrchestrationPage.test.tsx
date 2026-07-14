import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import OrchestrationPage from '../OrchestrationPage';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// Medulla access is toggled per-test; default to granted so the shell-routing
// tests exercise the live panels.
let medullaAccess = true;
vi.mock('../../lib/orchestration/useMedullaAccess', () => ({
  useMedullaAccess: () => medullaAccess,
}));

// Stub the data-backed panels so the shell's tab routing is tested in
// isolation (the panels have their own unit tests).
vi.mock('../../components/orchestration/MedullaOverviewPanel', () => ({
  default: () => <div data-testid="panel-medulla" />,
}));
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
vi.mock('../../components/orchestration/OrchestratorTaskBoard', () => ({
  default: () => <div data-testid="panel-tasks" />,
}));

describe('OrchestrationPage shell', () => {
  beforeEach(() => {
    medullaAccess = true;
  });

  it('defaults to the Medulla overview panel', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: ['/orchestration'] });
    });
    expect(screen.getByTestId('panel-medulla')).toBeInTheDocument();
  });

  it('renders the agent chat panel from ?tab=agent', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: ['/orchestration?tab=agent'] });
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

describe('OrchestrationPage scale showcase (no Medulla access)', () => {
  beforeEach(() => {
    medullaAccess = false;
  });

  it.each([
    ['agent', 'orch-demo-chat'],
    ['overview', 'orch-demo-graph'],
    ['network', 'orch-demo-network'],
  ])('renders the demo surface for ?tab=%s', async (tab, testId) => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: [`/orchestration?tab=${tab}`] });
    });
    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it('keeps the real task board available without Medulla access', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: ['/orchestration?tab=tasks'] });
    });
    expect(screen.getByTestId('panel-tasks')).toBeInTheDocument();
  });

  it('still lands on the Medulla overview by default', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationPage />, { initialEntries: ['/orchestration'] });
    });
    expect(screen.getByTestId('panel-medulla')).toBeInTheDocument();
  });
});
