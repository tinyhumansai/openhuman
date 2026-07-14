import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import DiscoverPanel from '../DiscoverPanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const runAction = vi.hoisted(() => vi.fn());
const pairing = vi.hoisted(() => ({
  current: {
    state: { status: 'ok' as const, snapshot: { requests: { incoming: [], outgoing: [] } } },
    reload: vi.fn(),
    runAction,
    pendingAction: null as string | null,
    actionError: null as string | null,
  },
}));
vi.mock('../../../lib/orchestration/usePairing', () => ({ usePairing: () => pairing.current }));

const selfIdentity = vi.hoisted(() => vi.fn());
const relayInfo = vi.hoisted(() => vi.fn().mockResolvedValue({ baseUrl: 'x', network: 'prod' }));
vi.mock('../../../lib/orchestration/orchestrationClient', () => ({
  orchestrationClient: { selfIdentity, relayInfo },
}));

const linkSession = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
const acceptRequest = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
const reverse = vi.hoisted(() => vi.fn().mockResolvedValue({ agents: [] }));
vi.mock('../../../agentworld/AgentWorldShell', () => ({
  apiClient: {
    orchestrationPairing: { linkSession, acceptRequest, declineRequest: vi.fn() },
    directory: { reverse },
  },
}));

describe('DiscoverPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    selfIdentity.mockResolvedValue({
      agentId: 'me',
      handles: [],
      discoverable: false,
      cardPublished: false,
      keyPublished: false,
    });
    pairing.current = {
      ...pairing.current,
      state: { status: 'ok', snapshot: { requests: { incoming: [], outgoing: [] } } } as never,
      pendingAction: null,
      actionError: null,
    };
  });

  it('shows the discoverability guide when not discoverable', async () => {
    render(<DiscoverPanel />);
    await waitFor(() => expect(screen.getByTestId('orch-discover-guide')).toBeInTheDocument());
    expect(screen.getByTestId('orch-discover-no-requests')).toBeInTheDocument();
  });

  it('submits a link request', async () => {
    render(<DiscoverPanel />);
    fireEvent.change(screen.getByTestId('orch-discover-link-input'), {
      target: { value: 'agent-9' },
    });
    fireEvent.submit(screen.getByTestId('orch-discover-link-input').closest('form')!);
    expect(runAction).toHaveBeenCalledWith('link:agent-9', expect.any(Function));
  });

  it('accepts an inbound request', async () => {
    pairing.current = {
      ...pairing.current,
      state: {
        status: 'ok',
        snapshot: {
          requests: {
            incoming: [{ status: 'pending', agentId: 'agent-7', contact: {} }],
            outgoing: [],
          },
        },
      } as never,
    };
    render(<DiscoverPanel />);
    await waitFor(() =>
      expect(screen.getByTestId('orch-discover-request-row')).toBeInTheDocument()
    );
    fireEvent.click(screen.getByTestId('orch-discover-accept'));
    expect(runAction).toHaveBeenCalledWith('accept:agent-7', expect.any(Function));
  });
});
