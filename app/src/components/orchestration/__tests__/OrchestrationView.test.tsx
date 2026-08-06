import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import OrchestrationView from '../OrchestrationView';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// Medulla access is toggled per-test; default to granted so the view-routing
// tests exercise the live panels.
let medullaAccess = true;
vi.mock('../../../lib/orchestration/useMedullaAccess', () => ({
  useMedullaAccess: () => medullaAccess,
}));

// No live peer sessions by default, so the agent view's left rail stays hidden.
vi.mock('../../../lib/orchestration/useOrchestrationSessions', () => ({
  useContactSessions: () => ({ byContact: new Map() }),
}));

// Stub the data-backed panels so the view's tab routing is tested in isolation
// (the panels have their own unit tests).
vi.mock('../MedullaOverviewPanel', () => ({ default: () => <div data-testid="panel-medulla" /> }));
vi.mock('../AgentChatPanel', () => ({ default: () => <div data-testid="panel-agent" /> }));
vi.mock('../ConnectionsPanel', () => ({
  default: ({ onDiscover }: { onDiscover?: () => void }) => (
    <button data-testid="panel-connections" onClick={onDiscover}>
      connections
    </button>
  ),
}));
vi.mock('../DiscoverPanel', () => ({ default: () => <div data-testid="panel-discover" /> }));
vi.mock('../UsagePanel', () => ({ default: () => <div data-testid="panel-usage" /> }));
vi.mock('../OverviewPanel', () => ({ default: () => <div data-testid="panel-graph" /> }));
vi.mock('../OrchestratorTaskBoard', () => ({ default: () => <div data-testid="panel-tasks" /> }));
vi.mock('../demo/MedullaDemoChat', () => ({ default: () => <div data-testid="orch-demo-chat" /> }));
vi.mock('../demo/MedullaDemoGraph', () => ({
  default: () => <div data-testid="orch-demo-graph" />,
}));
vi.mock('../demo/MedullaDemoNetwork', () => ({
  default: () => <div data-testid="orch-demo-network" />,
}));

const BASE = '/brain?tab=orchestration';

describe('OrchestrationView (Medulla access)', () => {
  beforeEach(() => {
    medullaAccess = true;
  });

  it('defaults to the Medulla overview panel', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [BASE] });
    });
    expect(screen.getByTestId('panel-medulla')).toBeInTheDocument();
  });

  it('renders the agent chat panel from ?ov=agent', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [`${BASE}&ov=agent`] });
    });
    expect(screen.getByTestId('panel-agent')).toBeInTheDocument();
  });

  it('renders the agent graph panel from ?ov=overview', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [`${BASE}&ov=overview`] });
    });
    expect(screen.getByTestId('panel-graph')).toBeInTheDocument();
  });

  it('renders the task board from ?ov=tasks', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [`${BASE}&ov=tasks`] });
    });
    expect(screen.getByTestId('panel-tasks')).toBeInTheDocument();
  });

  it.each([
    ['connections', 'panel-connections'],
    ['discover', 'panel-discover'],
    ['usage', 'panel-usage'],
  ])('renders the %s network sub from ?ov=network&sub=%s', async (sub, testId) => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, {
        initialEntries: [`${BASE}&ov=network&sub=${sub}`],
      });
    });
    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it('switches views via the chip nav', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [BASE] });
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('orch-view-network'));
    });
    await waitFor(() => expect(screen.getByTestId('panel-connections')).toBeInTheDocument());
  });

  it('lets the connections panel jump to discover via its callback', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, {
        initialEntries: [`${BASE}&ov=network&sub=connections`],
      });
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('panel-connections'));
    });
    await waitFor(() => expect(screen.getByTestId('panel-discover')).toBeInTheDocument());
  });
});

describe('OrchestrationView scale showcase (no Medulla access)', () => {
  beforeEach(() => {
    medullaAccess = false;
  });

  it.each([
    ['agent', 'orch-demo-chat'],
    ['overview', 'orch-demo-graph'],
    ['network', 'orch-demo-network'],
  ])('renders the demo surface for ?ov=%s', async (ov, testId) => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [`${BASE}&ov=${ov}`] });
    });
    expect(screen.getByTestId(testId)).toBeInTheDocument();
  });

  it('keeps the real task board available without Medulla access', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [`${BASE}&ov=tasks`] });
    });
    expect(screen.getByTestId('panel-tasks')).toBeInTheDocument();
  });

  it('still lands on the Medulla overview by default', async () => {
    await act(async () => {
      renderWithProviders(<OrchestrationView />, { initialEntries: [BASE] });
    });
    expect(screen.getByTestId('panel-medulla')).toBeInTheDocument();
  });
});
