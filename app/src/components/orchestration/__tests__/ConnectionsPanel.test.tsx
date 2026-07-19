import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../../lib/orchestration/orchestrationClient';
import ConnectionsPanel from '../ConnectionsPanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const runAction = vi.hoisted(() =>
  vi.fn(async (_id: string, fn: () => Promise<unknown>) => {
    await fn();
  })
);
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

const acceptRequest = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
const declineRequest = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
const reverse = vi.hoisted(() => vi.fn().mockResolvedValue({ agents: [{ username: 'alice' }] }));
vi.mock('../../../agentworld/AgentWorldShell', () => ({
  apiClient: {
    orchestrationPairing: { acceptRequest, declineRequest, blockRequest: vi.fn() },
    directory: { reverse },
  },
}));

const sessionsCreate = vi.hoisted(() => vi.fn());
const sendMasterMessage = vi.hoisted(() => vi.fn().mockResolvedValue({ ok: true, messageId: 'm' }));
vi.mock('../../../lib/orchestration/orchestrationClient', async orig => ({
  ...(await orig<typeof import('../../../lib/orchestration/orchestrationClient')>()),
  orchestrationClient: { sessionsCreate, sendMasterMessage },
}));

const sessionsHook = vi.hoisted(() => ({
  byContact: new Map<string, SessionSummary[]>(),
  refresh: vi.fn(),
}));
const transcriptHook = vi.hoisted(() => ({
  current: { state: { status: 'ok' as const }, messages: [] as unknown[], refresh: vi.fn() },
}));
vi.mock('../../../lib/orchestration/useOrchestrationSessions', () => ({
  useContactSessions: () => ({
    state: { status: 'ok' },
    sessions: [...sessionsHook.byContact.values()].flat(),
    byContact: sessionsHook.byContact,
    refresh: sessionsHook.refresh,
  }),
  useSessionTranscript: () => transcriptHook.current,
}));

const ADDR = '3icjiLXhn6BMv43MsHjpKKxm7hEYBk7R5rvNXB1HUk7g';

function session(over: Partial<SessionSummary>): SessionSummary {
  return {
    sessionId: 's1',
    agentId: ADDR,
    source: 'claude',
    status: 'waiting-approval',
    chatKind: 'session',
    lastMessageAt: '2026-07-08T00:00:00Z',
    unread: 0,
    active: true,
    pinned: false,
    label: 'auth-fix',
    messageCount: 5,
    ...over,
  };
}

function okState(accepted: unknown[], incoming: unknown[] = []) {
  return {
    status: 'ok' as const,
    snapshot: {
      contacts: { contacts: accepted },
      requests: { incoming, outgoing: [] },
      stats: {
        agentId: 'me',
        contactCount: accepted.length,
        pendingIncoming: incoming.length,
        pendingOutgoing: 0,
      },
    },
  };
}

describe('ConnectionsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionsHook.byContact = new Map();
    transcriptHook.current = { state: { status: 'ok' }, messages: [], refresh: vi.fn() };
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

  it('accepts and declines an incoming request', () => {
    const req = { agentId: ADDR, status: 'pending', direction: 'incoming' };
    pairing.current = { ...pairing.current, state: okState([], [req]) as never };
    render(<ConnectionsPanel />);
    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.accept'));
    expect(acceptRequest).toHaveBeenCalledWith(ADDR);
    fireEvent.click(screen.getByText('tinyplaceOrchestration.pairing.decline'));
    expect(declineRequest).toHaveBeenCalledWith(ADDR);
  });

  it('expands a contact to reveal its sessions, opens one, and replies', async () => {
    const contact = { agentId: ADDR, status: 'accepted', direction: 'outgoing' };
    sessionsHook.byContact.set(ADDR, [session({})]);
    pairing.current = { ...pairing.current, state: okState([contact]) as never };
    render(<ConnectionsPanel />);

    fireEvent.click(screen.getByTestId(`orch-connection-${ADDR}`).querySelector('button')!);
    const sessionRow = await screen.findByTestId('orch-session-s1');
    expect(sessionRow).toHaveTextContent('auth-fix');

    fireEvent.click(sessionRow);
    expect(screen.getByTestId('orch-session-view')).toBeInTheDocument();

    fireEvent.change(screen.getByTestId('orch-session-reply-input'), { target: { value: 'hi' } });
    fireEvent.click(screen.getByTestId('orch-session-reply-send'));
    await waitFor(() =>
      expect(sendMasterMessage).toHaveBeenCalledWith({
        body: 'hi',
        recipient: ADDR,
        sessionId: 's1',
      })
    );

    fireEvent.click(screen.getByTestId('orch-session-back'));
    expect(screen.getByTestId('orch-connections-panel')).toBeInTheDocument();
  });

  it('routes a runtime tool-approval decision back as an allow reply', async () => {
    const contact = { agentId: ADDR, status: 'accepted', direction: 'outgoing' };
    sessionsHook.byContact.set(ADDR, [session({})]);
    pairing.current = { ...pairing.current, state: okState([contact]) as never };
    transcriptHook.current = {
      state: { status: 'ok' },
      messages: [
        {
          id: 'ap',
          from: 'agent',
          body: 'gh pr status',
          timestamp: '2026-07-08T00:00:00Z',
          encrypted: false,
          eventKind: 'approval_request',
          toolName: 'shell',
        },
      ],
      refresh: vi.fn(),
    };
    render(<ConnectionsPanel />);
    fireEvent.click(screen.getByTestId(`orch-connection-${ADDR}`).querySelector('button')!);
    fireEvent.click(await screen.findByTestId('orch-session-s1'));
    fireEvent.click(screen.getByText('chat.approval.approve'));
    await waitFor(() =>
      expect(sendMasterMessage).toHaveBeenCalledWith({
        body: 'allow',
        recipient: ADDR,
        sessionId: 's1',
      })
    );
  });

  it('surfaces a reply send failure instead of swallowing it', async () => {
    const contact = { agentId: ADDR, status: 'accepted', direction: 'outgoing' };
    sessionsHook.byContact.set(ADDR, [session({})]);
    sendMasterMessage.mockRejectedValueOnce(new Error('relay down'));
    pairing.current = { ...pairing.current, state: okState([contact]) as never };
    render(<ConnectionsPanel />);
    fireEvent.click(screen.getByTestId(`orch-connection-${ADDR}`).querySelector('button')!);
    fireEvent.click(await screen.findByTestId('orch-session-s1'));
    fireEvent.change(screen.getByTestId('orch-session-reply-input'), { target: { value: 'hi' } });
    fireEvent.click(screen.getByTestId('orch-session-reply-send'));
    expect(await screen.findByTestId('orch-session-reply-error')).toHaveTextContent('relay down');
  });

  it('creates a new session under a contact', async () => {
    const contact = { agentId: ADDR, status: 'accepted', direction: 'outgoing' };
    sessionsCreate.mockResolvedValue({ session: session({ sessionId: 's-new' }) });
    pairing.current = { ...pairing.current, state: okState([contact]) as never };
    render(<ConnectionsPanel />);
    fireEvent.click(screen.getByTestId(`orch-connection-${ADDR}`).querySelector('button')!);
    fireEvent.click(await screen.findByTestId(`orch-new-session-${ADDR}`));
    await waitFor(() => expect(sessionsCreate).toHaveBeenCalledWith({ agentId: ADDR }));
  });
});
