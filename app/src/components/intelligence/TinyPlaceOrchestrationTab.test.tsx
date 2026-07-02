import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { apiClient } from '../../agentworld/AgentWorldShell';
import TinyPlaceOrchestrationTab from './TinyPlaceOrchestrationTab';

vi.mock('../../agentworld/AgentWorldShell', () => ({
  apiClient: {
    messages: { list: vi.fn() },
    inbox: { list: vi.fn() },
    orchestrationPairing: {
      list: vi.fn(),
      linkSession: vi.fn(),
      acceptRequest: vi.fn(),
      declineRequest: vi.fn(),
      blockRequest: vi.fn(),
    },
  },
}));

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const messagesListMock = vi.mocked(apiClient.messages.list);
const inboxListMock = vi.mocked(apiClient.inbox.list);
const pairingListMock = vi.mocked(apiClient.orchestrationPairing.list);
const pairingLinkMock = vi.mocked(apiClient.orchestrationPairing.linkSession);
const pairingAcceptMock = vi.mocked(apiClient.orchestrationPairing.acceptRequest);
const pairingDeclineMock = vi.mocked(apiClient.orchestrationPairing.declineRequest);
const pairingBlockMock = vi.mocked(apiClient.orchestrationPairing.blockRequest);

describe('TinyPlaceOrchestrationTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    messagesListMock.mockResolvedValue({ messages: [] });
    inboxListMock.mockResolvedValue({ items: [], unreadCount: 0, totalCount: 0 });
    pairingListMock.mockResolvedValue({
      records: [],
      contacts: { contacts: [] },
      requests: { incoming: [], outgoing: [] },
      stats: { agentId: '@openhuman', contactCount: 0, pendingIncoming: 0, pendingOutgoing: 0 },
    });
    pairingLinkMock.mockResolvedValue({
      record: {
        agentId: '@worker-new',
        status: 'pending',
        linkedAt: '2026-07-01T12:00:00.000Z',
        source: 'user_link',
      },
      remote: { agentId: '@worker-new', status: 'pending' },
    });
    pairingAcceptMock.mockResolvedValue({
      record: {
        agentId: '@worker-pending',
        status: 'linked',
        linkedAt: '2026-07-01T12:00:00.000Z',
        source: 'approved_request',
      },
      remote: { agentId: '@worker-pending', status: 'accepted' },
    });
    pairingDeclineMock.mockResolvedValue({ record: null, remote: { ok: true } });
    pairingBlockMock.mockResolvedValue({
      record: {
        agentId: '@worker-pending',
        status: 'blocked',
        linkedAt: '2026-07-01T12:00:00.000Z',
        source: 'approved_request',
      },
      remote: { agentId: '@worker-pending', status: 'blocked' },
    });
  });

  it('renders pinned master and subconscious chats before session chats', async () => {
    messagesListMock.mockResolvedValue({
      messages: [
        {
          id: 'm-master',
          from: 'human',
          to: 'master-agent',
          timestamp: '2026-07-01T12:00:00.000Z',
          deviceId: 1,
          type: 'agent-human',
          body: 'Coordinate the next worker handoff',
        },
        {
          id: 'm-subconscious',
          from: 'subconscious-loop',
          to: 'tinyplace_agent',
          timestamp: '2026-07-01T12:01:00.000Z',
          deviceId: 1,
          type: 'internal',
          body: 'Memory synthesis finished',
        },
        {
          id: 'm-session',
          from: '@worker-alpha',
          to: '@openhuman',
          timestamp: '2026-07-01T12:02:00.000Z',
          deviceId: 1,
          type: 'session',
          body: 'I asked the human master for context, then opened a worktree.',
          sessionId: 'app-session-1',
          sessionLabel: 'OpenHuman app session',
        },
      ],
    });

    render(<TinyPlaceOrchestrationTab />);

    expect(await screen.findAllByText('tinyplaceOrchestration.master.title')).toHaveLength(2);
    expect(screen.getByText('tinyplaceOrchestration.subconscious.title')).toBeInTheDocument();
    expect(screen.getByText('OpenHuman app session')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('tinyplace-chat-session:app-session-1'));

    expect(
      within(await screen.findByTestId('tinyplace-chat-messages')).getByText(
        'I asked the human master for context, then opened a worktree.'
      )
    ).toBeInTheDocument();
  });

  it('adds unread inbox sessions and marks them active', async () => {
    inboxListMock.mockResolvedValue({
      items: [
        {
          itemId: 'inbox-1',
          type: 'dm',
          status: 'unread',
          priority: 'normal',
          timestamp: '2026-07-01T12:03:00.000Z',
          subject: 'Worker update',
          summary: 'The subagent is waiting on a decision.',
          from: '@worker-beta',
        },
      ],
      unreadCount: 1,
      totalCount: 1,
    });

    render(<TinyPlaceOrchestrationTab />);

    expect(await screen.findByText('@worker-beta')).toBeInTheDocument();
    expect(screen.getByText('The subagent is waiting on a decision.')).toBeInTheDocument();
    expect(screen.getByText('1')).toBeInTheDocument();
    expect(screen.getByText('tinyplaceOrchestration.active')).toBeInTheDocument();
  });

  it('surfaces load errors and retries', async () => {
    messagesListMock.mockRejectedValueOnce(new Error('rpc failed'));

    render(<TinyPlaceOrchestrationTab />);

    expect(await screen.findByText(/tinyplaceOrchestration.failedToLoad/)).toBeInTheDocument();
    expect(screen.getByText(/rpc failed/)).toBeInTheDocument();

    fireEvent.click(screen.getByText('common.retry'));

    await waitFor(() => expect(messagesListMock).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('tinyplaceOrchestration.noMessages')).toBeInTheDocument();
  });

  it('requests a contact edge for a pasted session identity', async () => {
    render(<TinyPlaceOrchestrationTab />);

    const input = await screen.findByPlaceholderText(
      'tinyplaceOrchestration.pairing.linkPlaceholder'
    );
    fireEvent.change(input, { target: { value: '@worker-new' } });
    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.linkAction'));

    await waitFor(() => expect(pairingLinkMock).toHaveBeenCalledWith('@worker-new'));
    await waitFor(() => expect(pairingListMock).toHaveBeenCalledTimes(2));
  });

  it('surfaces incoming contact requests for explicit approval', async () => {
    pairingListMock.mockResolvedValue({
      records: [],
      contacts: { contacts: [] },
      requests: {
        incoming: [
          {
            agentId: '@worker-pending',
            status: 'pending',
            direction: 'incoming',
            contact: {
              requester: '@worker-pending',
              addressee: '@openhuman',
              status: 'pending',
              createdAt: '2026-07-01T12:00:00.000Z',
              updatedAt: '2026-07-01T12:00:00.000Z',
            },
          },
        ],
        outgoing: [],
      },
      stats: { agentId: '@openhuman', contactCount: 0, pendingIncoming: 1, pendingOutgoing: 0 },
    });

    render(<TinyPlaceOrchestrationTab />);

    expect(await screen.findByText('@worker-pending')).toBeInTheDocument();
    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.accept'));

    await waitFor(() => expect(pairingAcceptMock).toHaveBeenCalledWith('@worker-pending'));
  });
});
