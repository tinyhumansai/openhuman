import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../../lib/orchestration/orchestrationClient';
import AgentChatPanel from '../AgentChatPanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const selectChat = vi.hoisted(() => vi.fn());
const sendMessage = vi.hoisted(() => vi.fn().mockResolvedValue(true));
const chatsApi = vi.hoisted(() => ({
  current: {
    sessionsState: { status: 'ok' as const },
    messagesState: { status: 'ok' as const },
    chats: [
      { id: 'master', title: 'Master', subtitle: 'you', unread: 0, messages: [] as unknown[] },
      {
        id: 'subconscious',
        title: 'Subconscious',
        subtitle: 'loop',
        unread: 0,
        messages: [] as unknown[],
      },
    ],
    selectedId: 'master',
    selected: { id: 'master', title: 'Master', messages: [] as unknown[] },
    status: null as unknown,
    masterError: null as string | null,
    selectChat,
    refresh: vi.fn(),
    sendMessage,
  },
}));
vi.mock('../../../lib/orchestration/useOrchestrationChats', () => ({
  MASTER_CHAT_KEY: 'master',
  SUBCONSCIOUS_CHAT_KEY: 'subconscious',
  useOrchestrationChats: () => chatsApi.current,
}));

const subconsciousTrigger = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock('../../../utils/tauriCommands/subconscious', () => ({ subconsciousTrigger }));

const sendMasterMessage = vi.hoisted(() => vi.fn().mockResolvedValue({ ok: true, messageId: 'm' }));
vi.mock('../../../lib/orchestration/orchestrationClient', async orig => ({
  ...(await orig<typeof import('../../../lib/orchestration/orchestrationClient')>()),
  orchestrationClient: { sendMasterMessage },
}));

const contactSessions = vi.hoisted(() => ({ current: [] as SessionSummary[] }));
const transcript = vi.hoisted(() => ({
  current: { state: { status: 'ok' as const }, messages: [] as unknown[], refresh: vi.fn() },
}));
vi.mock('../../../lib/orchestration/useOrchestrationSessions', () => ({
  useContactSessions: () => ({
    state: { status: 'ok' },
    sessions: contactSessions.current,
    byContact: new Map(),
    refresh: vi.fn(),
  }),
  useSessionTranscript: () => transcript.current,
}));

const pinged: SessionSummary = {
  sessionId: 's-auth',
  agentId: '@peer',
  source: 'claude',
  status: 'waiting-approval',
  chatKind: 'session',
  lastMessageAt: '2026-07-08T00:00:00Z',
  unread: 0,
  active: true,
  pinned: false,
  label: 'auth-fix',
  messageCount: 3,
};

describe('AgentChatPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    contactSessions.current = [];
    chatsApi.current = {
      ...chatsApi.current,
      selectedId: 'master',
      selected: { id: 'master', title: 'Master', messages: [] },
      masterError: null,
    };
  });

  it('renders the thread rail and switches conversation', () => {
    render(<AgentChatPanel />);
    expect(screen.getByTestId('orch-agent-tab-master')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('orch-agent-tab-subconscious'));
    expect(selectChat).toHaveBeenCalledWith('subconscious');
  });

  it('sends a master message from the composer', async () => {
    render(<AgentChatPanel />);
    fireEvent.change(screen.getByTestId('orch-agent-composer-input'), { target: { value: 'go' } });
    fireEvent.click(screen.getByTestId('orch-agent-composer-send'));
    await waitFor(() =>
      expect(sendMessage).toHaveBeenCalledWith(expect.objectContaining({ id: 'master' }), 'go')
    );
  });

  it('shows the steering header + runs a review on the subconscious thread', () => {
    chatsApi.current = {
      ...chatsApi.current,
      selectedId: 'subconscious',
      selected: { id: 'subconscious', title: 'Subconscious', messages: [] },
    };
    render(<AgentChatPanel />);
    expect(screen.getByTestId('orch-agent-steering-header')).toBeInTheDocument();
    fireEvent.click(screen.getByText('tinyplaceOrchestration.steeringHeader.runReview'));
    expect(subconsciousTrigger).toHaveBeenCalledWith('tinyplace');
  });

  it('opens a session side-tab from a View-session card and replies (no auto-open)', async () => {
    contactSessions.current = [pinged];
    render(<AgentChatPanel />);
    expect(screen.queryByTestId('orch-agent-session-drawer')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('orch-agent-view-session-s-auth'));
    expect(screen.getByTestId('orch-agent-session-drawer')).toBeInTheDocument();

    fireEvent.change(screen.getByTestId('orch-agent-drawer-reply'), { target: { value: 'hi' } });
    fireEvent.click(
      screen.getByTestId('orch-agent-session-drawer').querySelector('button[type="submit"]')!
    );
    await waitFor(() =>
      expect(sendMasterMessage).toHaveBeenCalledWith({
        body: 'hi',
        recipient: '@peer',
        sessionId: 's-auth',
      })
    );

    fireEvent.click(screen.getByTestId('orch-agent-drawer-close'));
    expect(screen.queryByTestId('orch-agent-session-drawer')).not.toBeInTheDocument();
  });

  it('surfaces a drawer reply failure', async () => {
    contactSessions.current = [pinged];
    sendMasterMessage.mockRejectedValueOnce(new Error('boom'));
    render(<AgentChatPanel />);
    fireEvent.click(screen.getByTestId('orch-agent-view-session-s-auth'));
    fireEvent.change(screen.getByTestId('orch-agent-drawer-reply'), { target: { value: 'hi' } });
    fireEvent.click(
      screen.getByTestId('orch-agent-session-drawer').querySelector('button[type="submit"]')!
    );
    expect(await screen.findByTestId('orch-agent-drawer-reply-error')).toHaveTextContent('boom');
  });

  it('shows an error state when the transcript fails to load', () => {
    chatsApi.current = {
      ...chatsApi.current,
      messagesState: { status: 'error', message: 'load failed' } as never,
    };
    render(<AgentChatPanel />);
    expect(screen.getByText(/load failed/)).toBeInTheDocument();
  });
});
