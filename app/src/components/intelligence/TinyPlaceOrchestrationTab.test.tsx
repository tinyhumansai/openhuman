import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { apiClient } from '../../agentworld/AgentWorldShell';
import { orchestrationClient } from '../../lib/orchestration/orchestrationClient';
import { socketService } from '../../services/socketService';
import TinyPlaceOrchestrationTab from './TinyPlaceOrchestrationTab';

vi.mock('../../agentworld/AgentWorldShell', () => ({
  apiClient: {
    orchestrationPairing: {
      list: vi.fn(),
      linkSession: vi.fn(),
      acceptRequest: vi.fn(),
      declineRequest: vi.fn(),
      blockRequest: vi.fn(),
    },
  },
}));

vi.mock('../../lib/orchestration/orchestrationClient', async importOriginal => {
  const actual =
    await importOriginal<typeof import('../../lib/orchestration/orchestrationClient')>();
  return {
    ...actual,
    orchestrationClient: {
      sessionsList: vi.fn(),
      messagesList: vi.fn(),
      sendMasterMessage: vi.fn(),
      markRead: vi.fn(),
      status: vi.fn(),
    },
  };
});

vi.mock('../../services/socketService', () => {
  const socket = { on: vi.fn(), off: vi.fn() };
  return { socketService: { getSocket: vi.fn(() => socket) } };
});

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const sessionsListMock = vi.mocked(orchestrationClient.sessionsList);
const messagesListMock = vi.mocked(orchestrationClient.messagesList);
const sendMasterMock = vi.mocked(orchestrationClient.sendMasterMessage);
const markReadMock = vi.mocked(orchestrationClient.markRead);
const statusMock = vi.mocked(orchestrationClient.status);

const pairingListMock = vi.mocked(apiClient.orchestrationPairing.list);
const pairingLinkMock = vi.mocked(apiClient.orchestrationPairing.linkSession);
const pairingAcceptMock = vi.mocked(apiClient.orchestrationPairing.acceptRequest);
const pairingDeclineMock = vi.mocked(apiClient.orchestrationPairing.declineRequest);
const pairingBlockMock = vi.mocked(apiClient.orchestrationPairing.blockRequest);

const getSocketMock = vi.mocked(socketService.getSocket);

const PINNED_SESSIONS = [
  {
    sessionId: 'master',
    agentId: '@openhuman',
    source: 'core',
    chatKind: 'master' as const,
    lastMessageAt: '2026-07-01T12:00:00.000Z',
    unread: 0,
    active: true,
    pinned: true,
  },
  {
    sessionId: 'subconscious',
    agentId: '@openhuman',
    source: 'core',
    chatKind: 'subconscious' as const,
    lastMessageAt: '2026-07-01T12:01:00.000Z',
    unread: 0,
    active: true,
    pinned: true,
  },
];

describe('TinyPlaceOrchestrationTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionsListMock.mockResolvedValue({ sessions: [...PINNED_SESSIONS] });
    messagesListMock.mockResolvedValue({ messages: [] });
    sendMasterMock.mockResolvedValue({ ok: true, messageId: 'm-1' });
    markReadMock.mockResolvedValue({ ok: true });
    statusMock.mockResolvedValue({});
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

  it('renders pinned master and subconscious chats plus app sessions', async () => {
    sessionsListMock.mockResolvedValue({
      sessions: [
        ...PINNED_SESSIONS,
        {
          sessionId: 'app-session-1',
          agentId: '@worker-alpha',
          source: 'openhuman-app',
          label: 'OpenHuman app session',
          chatKind: 'session',
          lastMessageAt: '2026-07-01T12:02:00.000Z',
          unread: 0,
          active: true,
          pinned: false,
        },
      ],
    });

    render(<TinyPlaceOrchestrationTab />);

    // Pinned master appears twice: in the list button and the main header.
    expect(await screen.findAllByText('tinyplaceOrchestration.master.title')).toHaveLength(2);
    expect(screen.getByText('tinyplaceOrchestration.subconscious.title')).toBeInTheDocument();
    expect(screen.getByText('OpenHuman app session')).toBeInTheDocument();
  });

  it('loads and renders messages for the opened chat', async () => {
    sessionsListMock.mockResolvedValue({
      sessions: [
        ...PINNED_SESSIONS,
        {
          sessionId: 'app-session-1',
          agentId: '@worker-alpha',
          source: 'openhuman-app',
          label: 'OpenHuman app session',
          chatKind: 'session',
          lastMessageAt: '2026-07-01T12:02:00.000Z',
          unread: 0,
          active: true,
          pinned: false,
        },
      ],
    });
    messagesListMock.mockImplementation(async ({ chat }) => {
      if (chat === 'app-session-1') {
        return {
          messages: [
            {
              id: 'm-session',
              agentId: '@worker-alpha',
              sessionId: 'app-session-1',
              chatKind: 'session' as const,
              role: '@worker-alpha',
              body: 'I opened a worktree and asked the master for context.',
              timestamp: '2026-07-01T12:02:00.000Z',
              seq: 1,
            },
          ],
        };
      }
      return { messages: [] };
    });

    render(<TinyPlaceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('tinyplace-chat-app-session-1'));

    expect(
      within(await screen.findByTestId('tinyplace-chat-messages')).getByText(
        'I opened a worktree and asked the master for context.'
      )
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(messagesListMock).toHaveBeenCalledWith(
        expect.objectContaining({ chat: 'app-session-1' })
      )
    );
  });

  it('marks a chat read when it is opened', async () => {
    sessionsListMock.mockResolvedValue({
      sessions: [
        ...PINNED_SESSIONS,
        {
          sessionId: 'app-session-1',
          agentId: '@worker-alpha',
          source: 'openhuman-app',
          label: 'OpenHuman app session',
          chatKind: 'session',
          lastMessageAt: '2026-07-01T12:02:00.000Z',
          unread: 3,
          active: true,
          pinned: false,
        },
      ],
    });

    render(<TinyPlaceOrchestrationTab />);

    fireEvent.click(await screen.findByTestId('tinyplace-chat-app-session-1'));

    await waitFor(() => expect(markReadMock).toHaveBeenCalledWith('app-session-1'));
  });

  it('sends a master message and optimistically appends it', async () => {
    // Hold the send promise open so the optimistic append is observable before
    // the success reconcile refetch replaces it.
    let resolveSend: (() => void) | undefined;
    sendMasterMock.mockImplementation(
      () =>
        new Promise(res => {
          resolveSend = () => res({ ok: true, messageId: 'm-1' });
        })
    );

    render(<TinyPlaceOrchestrationTab />);

    const input = await screen.findByTestId('tinyplace-master-composer-input');
    fireEvent.change(input, { target: { value: 'Coordinate the next handoff' } });
    fireEvent.click(screen.getByTestId('tinyplace-master-composer-send'));

    // Optimistic append renders immediately in the message pane (send pending).
    expect(
      within(await screen.findByTestId('tinyplace-chat-messages')).getByText(
        'Coordinate the next handoff'
      )
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(sendMasterMock).toHaveBeenCalledWith({ body: 'Coordinate the next handoff' })
    );

    resolveSend?.();

    // Input clears on success.
    await waitFor(() => expect((input as HTMLInputElement).value).toBe(''));
  });

  it('refetches when an orchestration:message socket event fires', async () => {
    let handler: ((payload: unknown) => void) | undefined;
    const socket = {
      on: vi.fn((event: string, cb: (payload: unknown) => void) => {
        if (event === 'orchestration:message') handler = cb;
      }),
      off: vi.fn(),
    };
    getSocketMock.mockReturnValue(socket as never);

    render(<TinyPlaceOrchestrationTab />);

    await waitFor(() => expect(sessionsListMock).toHaveBeenCalled());
    const initialCalls = sessionsListMock.mock.calls.length;
    expect(handler).toBeDefined();

    handler?.({ agentId: '@worker-alpha', sessionId: 'master', chatKind: 'master' });

    await waitFor(() => expect(sessionsListMock.mock.calls.length).toBeGreaterThan(initialCalls));
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

  it('surfaces load errors and retries', async () => {
    sessionsListMock.mockRejectedValueOnce(new Error('rpc failed'));

    render(<TinyPlaceOrchestrationTab />);

    expect(await screen.findByText(/tinyplaceOrchestration.failedToLoad/)).toBeInTheDocument();
    expect(screen.getByText(/rpc failed/)).toBeInTheDocument();

    fireEvent.click(screen.getByText('common.retry'));

    await waitFor(() => expect(sessionsListMock).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('tinyplaceOrchestration.noMessages')).toBeInTheDocument();
  });
});
