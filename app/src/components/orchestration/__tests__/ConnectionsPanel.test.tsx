import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ConnectionsPanel from '../ConnectionsPanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const runAction = vi.hoisted(() => vi.fn());
const pairing = vi.hoisted(() => ({
  current: {
    state: { status: 'ok' as const, snapshot: {} as Record<string, unknown> },
    reload: vi.fn(),
    runAction,
    pendingAction: null as string | null,
    actionError: null as string | null,
  },
}));
vi.mock('../../../lib/orchestration/usePairing', () => ({ usePairing: () => pairing.current }));

const blockRequest = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
const reverse = vi.hoisted(() => vi.fn().mockResolvedValue({ agents: [{ username: 'alice' }] }));
vi.mock('../../../agentworld/AgentWorldShell', () => ({
  apiClient: { orchestrationPairing: { blockRequest }, directory: { reverse } },
}));

function okState(accepted: unknown[], stats?: unknown) {
  return {
    status: 'ok' as const,
    snapshot: {
      contacts: { contacts: accepted },
      requests: { incoming: [], outgoing: [] },
      stats: stats ?? {
        agentId: 'me',
        contactCount: accepted.length,
        pendingIncoming: 0,
        pendingOutgoing: 0,
      },
    },
  };
}

describe('ConnectionsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    pairing.current = { ...pairing.current, pendingAction: null, actionError: null };
  });

  it('shows a loading state', () => {
    pairing.current = { ...pairing.current, state: { status: 'loading' } as never };
    render(<ConnectionsPanel />);
    expect(screen.getByTestId('orch-connections-loading')).toBeInTheDocument();
  });

  it('renders an empty state with a discover CTA', () => {
    pairing.current = { ...pairing.current, state: okState([]) as never };
    const onDiscover = vi.fn();
    render(<ConnectionsPanel onDiscover={onDiscover} />);
    expect(screen.getByTestId('orch-connections-empty')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('orch-connections-empty-cta'));
    expect(onDiscover).toHaveBeenCalled();
  });

  it('lists accepted contacts and blocks one', async () => {
    pairing.current = {
      ...pairing.current,
      state: okState([{ status: 'accepted', agentId: 'agent-1', contact: {} }]) as never,
    };
    render(<ConnectionsPanel />);
    expect(screen.getByTestId('orch-connection-row')).toBeInTheDocument();
    // handle resolves from the directory reverse lookup
    await waitFor(() => expect(screen.getByText('@alice')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('orch-connection-block'));
    expect(runAction).toHaveBeenCalledWith('block:agent-1', expect.any(Function));
  });
});
