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
      sessionsCreate: vi.fn(),
      messagesList: vi.fn(),
      sendMasterMessage: vi.fn(),
      markRead: vi.fn(),
      status: vi.fn(),
    },
  };
});

vi.mock('../../services/socketService', () => {
  return { socketService: { on: vi.fn(), off: vi.fn(), getSocket: vi.fn(() => null) } };
});

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

vi.mock('../../utils/tauriCommands/subconscious', () => ({
  subconsciousTrigger: vi.fn(async () => ({ result: { triggered: true }, logs: [] })),
}));

const sessionsListMock = vi.mocked(orchestrationClient.sessionsList);
const sessionsCreateMock = vi.mocked(orchestrationClient.sessionsCreate);
const messagesListMock = vi.mocked(orchestrationClient.messagesList);
const sendMasterMock = vi.mocked(orchestrationClient.sendMasterMessage);
const markReadMock = vi.mocked(orchestrationClient.markRead);
const statusMock = vi.mocked(orchestrationClient.status);

const pairingListMock = vi.mocked(apiClient.orchestrationPairing.list);
const pairingLinkMock = vi.mocked(apiClient.orchestrationPairing.linkSession);
const pairingAcceptMock = vi.mocked(apiClient.orchestrationPairing.acceptRequest);
const pairingDeclineMock = vi.mocked(apiClient.orchestrationPairing.declineRequest);
const pairingBlockMock = vi.mocked(apiClient.orchestrationPairing.blockRequest);

const socketOnMock = vi.mocked(socketService.on);

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
    status: 'idle' as const,
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
    status: 'idle' as const,
  },
];

describe('TinyPlaceOrchestrationTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionsListMock.mockResolvedValue({ sessions: [...PINNED_SESSIONS] });
    sessionsCreateMock.mockResolvedValue({
      session: {
        sessionId: 'new-sess',
        agentId: '@peer',
        source: 'user_created',
        chatKind: 'session',
        lastMessageAt: '2026-07-04T00:00:00.000Z',
        unread: 0,
        active: true,
        pinned: false,
        status: 'idle' as const,
      },
    });
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

  it('steering header shows the directive and Run review triggers the tinyplace kind', async () => {
    const { subconsciousTrigger } = await import('../../utils/tauriCommands/subconscious');
    statusMock.mockResolvedValue({
      steering: {
        text: 'prioritize the billing migration',
        createdAt: '2026-07-04T00:00:00.000Z',
        expiresAfterCycles: 12,
      },
      lastTickAt: 1_700_000_000,
    });

    render(<TinyPlaceOrchestrationTab />);

    // Open the pinned Subconscious window.
    fireEvent.click(await screen.findByTestId('tinyplace-chat-subconscious'));

    const header = await screen.findByTestId('tinyplace-steering-header');
    expect(within(header).getByText('prioritize the billing migration')).toBeInTheDocument();

    fireEvent.click(within(header).getByText('tinyplaceOrchestration.steeringHeader.runReview'));
    await waitFor(() => expect(subconsciousTrigger).toHaveBeenCalledWith('tinyplace'));
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
          status: 'idle' as const,
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
          status: 'idle' as const,
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
          status: 'idle' as const,
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
    // Listeners are registered through socketService.on (which queues until the
    // socket exists), so capture the handler there rather than on a raw socket.
    socketOnMock.mockImplementation(((event: string, cb: (payload: unknown) => void) => {
      if (event === 'orchestration:message') handler = cb;
    }) as never);

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

  it('falls back to the contact requester address when agentId is absent', async () => {
    // The relay's /contacts/requests payload does not always populate the
    // top-level agentId; the counterpart address lives in contact.requester.
    const rawAddress = '3icjiLXhn6BMv43MsHjpKKxm7hEYBk7R5rvNXB1HUk7g';
    pairingListMock.mockResolvedValue({
      records: [],
      contacts: { contacts: [] },
      requests: {
        incoming: [
          {
            agentId: '',
            status: 'pending',
            direction: 'incoming',
            contact: {
              requester: rawAddress,
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

    expect(await screen.findByText(rawAddress)).toBeInTheDocument();
    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.accept'));

    await waitFor(() => expect(pairingAcceptMock).toHaveBeenCalledWith(rawAddress));
  });

  it('lists accepted contacts (address resolved from the contact record)', async () => {
    const rawAddress = '3icjiLXhn6BMv43MsHjpKKxm7hEYBk7R5rvNXB1HUk7g';
    pairingListMock.mockResolvedValue({
      records: [],
      contacts: {
        contacts: [
          {
            agentId: '',
            status: 'accepted',
            direction: 'incoming',
            contact: {
              requester: rawAddress,
              addressee: '@openhuman',
              status: 'accepted',
              createdAt: '2026-07-01T12:00:00.000Z',
              updatedAt: '2026-07-01T12:00:00.000Z',
            },
          },
        ],
      },
      requests: { incoming: [], outgoing: [] },
      stats: { agentId: '@openhuman', contactCount: 1, pendingIncoming: 0, pendingOutgoing: 0 },
    });

    render(<TinyPlaceOrchestrationTab />);

    // The accepted contact appears in the Contacts list, not just the count.
    expect(await screen.findByText(rawAddress)).toBeInTheDocument();
    expect(screen.getByText('tinyplaceOrchestration.contacts')).toBeInTheDocument();
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

  const ACCEPTED_CONTACT_ADDRESS = '3icjiLXhn6BMv43MsHjpKKxm7hEYBk7R5rvNXB1HUk7g';
  const acceptedContactSnapshot = () => ({
    records: [],
    contacts: {
      contacts: [
        {
          agentId: ACCEPTED_CONTACT_ADDRESS,
          status: 'accepted' as const,
          direction: 'incoming' as const,
          contact: {
            requester: ACCEPTED_CONTACT_ADDRESS,
            addressee: '@openhuman',
            status: 'accepted' as const,
            createdAt: '2026-07-01T12:00:00.000Z',
            updatedAt: '2026-07-01T12:00:00.000Z',
          },
        },
      ],
    },
    requests: { incoming: [], outgoing: [] },
    stats: { agentId: '@openhuman', contactCount: 1, pendingIncoming: 0, pendingOutgoing: 0 },
  });

  it('creates a new session under an expanded contact', async () => {
    pairingListMock.mockResolvedValue(acceptedContactSnapshot());

    render(<TinyPlaceOrchestrationTab />);

    // Expand the contact row (exposes state to assistive tech), then create.
    const contactToggle = await screen.findByTestId(
      `tinyplace-contact-${ACCEPTED_CONTACT_ADDRESS}`
    );
    expect(contactToggle).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(contactToggle);
    expect(contactToggle).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(await screen.findByTestId(`tinyplace-new-session-${ACCEPTED_CONTACT_ADDRESS}`));

    await waitFor(() =>
      expect(sessionsCreateMock).toHaveBeenCalledWith({ agentId: ACCEPTED_CONTACT_ADDRESS })
    );
  });

  it('threads a composed message under the selected session', async () => {
    pairingListMock.mockResolvedValue(acceptedContactSnapshot());
    sessionsListMock.mockResolvedValue({
      sessions: [
        ...PINNED_SESSIONS,
        {
          sessionId: 'sess-x',
          agentId: ACCEPTED_CONTACT_ADDRESS,
          source: 'user_created',
          label: 'Design review',
          chatKind: 'session',
          lastMessageAt: '2026-07-01T12:02:00.000Z',
          unread: 0,
          active: true,
          pinned: false,
          status: 'idle' as const,
        },
      ],
    });

    render(<TinyPlaceOrchestrationTab />);

    // Expand the contact, open its nested session, then send.
    fireEvent.click(await screen.findByTestId(`tinyplace-contact-${ACCEPTED_CONTACT_ADDRESS}`));
    fireEvent.click(await screen.findByTestId('tinyplace-chat-sess-x'));

    const input = await screen.findByTestId('tinyplace-master-composer-input');
    fireEvent.change(input, { target: { value: 'ping under session' } });
    fireEvent.click(screen.getByTestId('tinyplace-master-composer-send'));

    await waitFor(() =>
      expect(sendMasterMock).toHaveBeenCalledWith({
        body: 'ping under session',
        recipient: ACCEPTED_CONTACT_ADDRESS,
        sessionId: 'sess-x',
      })
    );
  });
});
