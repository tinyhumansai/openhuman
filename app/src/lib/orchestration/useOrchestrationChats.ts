/**
 * Data hook backing the TinyPlace orchestration Brain tab.
 *
 * Owns everything that talks to the core:
 * - sessions list (pinned master + subconscious + app sessions)
 * - per-chat message loads (lazy, for the selected chat)
 * - the master composer send (optimistic append)
 * - read receipts (`mark_read` on open)
 * - orchestration status (active steering directive)
 * - live refetch on the `orchestration:message` socket event
 *
 * The component stays presentational: it renders the `ChatWindow[]` view model
 * this hook produces and calls the returned actions.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { socketService } from '../../services/socketService';
import {
  orchestrationClient,
  type OrchestrationMessage,
  type OrchestrationMessageEvent,
  type OrchestrationStatus,
  PaymentRequiredError,
  type SessionSummary,
} from './orchestrationClient';

const MESSAGE_LIMIT = 100;

export const MASTER_CHAT_KEY = 'master';
export const SUBCONSCIOUS_CHAT_KEY = 'subconscious';

export type ChatKind = 'master' | 'subconscious' | 'session';

export interface ChatMessage {
  id: string;
  from: string;
  body: string;
  timestamp: string;
  encrypted: boolean;
}

export interface ChatWindow {
  id: string;
  kind: ChatKind;
  title: string;
  subtitle: string;
  preview: string;
  lastTimestamp: string | null;
  unread: number;
  active: boolean;
  pinned: boolean;
  peerAgentId: string | null;
  messages: ChatMessage[];
}

type Translate = (key: string) => string;

export type SessionsState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'payment_required' }
  | { status: 'ok' };

export type MessagesPaneState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ok' };

/** RPC `chat` key for a summary: sessions key on their id, pinned on their kind. */
export function chatKeyForSession(summary: SessionSummary): string {
  return summary.chatKind === 'session' ? summary.sessionId : summary.chatKind;
}

function truncate(text: string, length = 96): string {
  if (text.length <= length) return text;
  return `${text.slice(0, length - 1)}…`;
}

function mapMessage(message: OrchestrationMessage): ChatMessage {
  return {
    id: message.id,
    from: message.role?.trim() || message.agentId || '',
    body: message.body,
    timestamp: message.timestamp,
    encrypted: false,
  };
}

function messageTime(timestamp: string): number {
  const parsed = Date.parse(timestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}

function sortMessages(messages: ChatMessage[]): ChatMessage[] {
  return messages.slice().sort((a, b) => messageTime(a.timestamp) - messageTime(b.timestamp));
}

function pinnedTitle(kind: ChatKind, t: Translate): string {
  return kind === 'subconscious'
    ? t('tinyplaceOrchestration.subconscious.title')
    : t('tinyplaceOrchestration.master.title');
}

function pinnedSubtitle(kind: ChatKind, t: Translate): string {
  return kind === 'subconscious'
    ? t('tinyplaceOrchestration.subconscious.subtitle')
    : t('tinyplaceOrchestration.master.subtitle');
}

function pinnedPreview(kind: ChatKind, t: Translate): string {
  return kind === 'subconscious'
    ? t('tinyplaceOrchestration.subconscious.preview')
    : t('tinyplaceOrchestration.master.preview');
}

function buildChatWindow(
  summary: SessionSummary,
  messages: ChatMessage[] | undefined,
  t: Translate
): ChatWindow {
  const key = chatKeyForSession(summary);
  const loaded = messages ?? [];
  const last = loaded[loaded.length - 1];
  const preview = last
    ? truncate(last.body)
    : summary.pinned
      ? pinnedPreview(summary.chatKind, t)
      : '';
  return {
    id: key,
    kind: summary.chatKind,
    title: summary.pinned
      ? pinnedTitle(summary.chatKind, t)
      : summary.label?.trim() || summary.sessionId,
    subtitle: summary.pinned
      ? pinnedSubtitle(summary.chatKind, t)
      : summary.workspace?.trim() ||
        summary.source?.trim() ||
        t('tinyplaceOrchestration.session.subtitle'),
    preview,
    lastTimestamp: summary.lastMessageAt || last?.timestamp || null,
    unread: summary.unread,
    active: summary.active,
    pinned: summary.pinned,
    peerAgentId: summary.chatKind === 'session' ? summary.agentId || null : null,
    messages: loaded,
  };
}

export interface UseOrchestrationChatsResult {
  sessionsState: SessionsState;
  messagesState: MessagesPaneState;
  chats: ChatWindow[];
  selectedId: string;
  selected: ChatWindow | undefined;
  status: OrchestrationStatus | null;
  masterError: string | null;
  selectChat: (chatKey: string) => void;
  refresh: () => Promise<void>;
  sendMaster: (body: string) => Promise<boolean>;
}

export function useOrchestrationChats(t: Translate): UseOrchestrationChatsResult {
  const [sessionsState, setSessionsState] = useState<SessionsState>({ status: 'loading' });
  const [messagesState, setMessagesState] = useState<MessagesPaneState>({ status: 'idle' });
  const [summaries, setSummaries] = useState<SessionSummary[]>([]);
  const [messagesByChat, setMessagesByChat] = useState<Record<string, ChatMessage[]>>({});
  const [status, setStatus] = useState<OrchestrationStatus | null>(null);
  const [selectedId, setSelectedId] = useState<string>(MASTER_CHAT_KEY);
  const [masterError, setMasterError] = useState<string | null>(null);
  const mountedRef = useRef(true);
  // Track the selected chat for socket handlers without re-subscribing on every change.
  const selectedIdRef = useRef(selectedId);
  selectedIdRef.current = selectedId;

  const loadMessages = useCallback(async (chatKey: string) => {
    setMessagesState({ status: 'loading' });
    try {
      const result = await orchestrationClient.messagesList({
        chat: chatKey,
        limit: MESSAGE_LIMIT,
      });
      if (!mountedRef.current) return;
      setMessagesByChat(prev => ({
        ...prev,
        [chatKey]: sortMessages(result.messages.map(mapMessage)),
      }));
      setMessagesState({ status: 'ok' });
    } catch (error) {
      if (!mountedRef.current) return;
      if (error instanceof PaymentRequiredError) {
        setSessionsState({ status: 'payment_required' });
        setMessagesState({ status: 'idle' });
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      setMessagesState({ status: 'error', message });
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const next = await orchestrationClient.status();
      if (mountedRef.current) setStatus(next);
    } catch {
      // Status is advisory only — never block the chat surface on it.
    }
  }, []);

  const loadSessions = useCallback(async (): Promise<SessionSummary[]> => {
    const result = await orchestrationClient.sessionsList();
    if (mountedRef.current) setSummaries(result.sessions);
    return result.sessions;
  }, []);

  const markRead = useCallback(async (chatKey: string) => {
    try {
      await orchestrationClient.markRead(chatKey);
      if (!mountedRef.current) return;
      // Optimistically clear the local unread badge; a refetch reconciles.
      setSummaries(prev =>
        prev.map(s => (chatKeyForSession(s) === chatKey ? { ...s, unread: 0 } : s))
      );
    } catch {
      // Read receipts are best-effort; a failure must not break the pane.
    }
  }, []);

  const refresh = useCallback(async () => {
    setSessionsState({ status: 'loading' });
    try {
      await Promise.all([loadSessions(), refreshStatus()]);
      if (!mountedRef.current) return;
      setSessionsState({ status: 'ok' });
      await loadMessages(selectedIdRef.current);
    } catch (error) {
      if (!mountedRef.current) return;
      if (error instanceof PaymentRequiredError) {
        setSessionsState({ status: 'payment_required' });
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      setSessionsState({ status: 'error', message });
    }
  }, [loadSessions, refreshStatus, loadMessages]);

  const selectChat = useCallback(
    (chatKey: string) => {
      if (chatKey === selectedIdRef.current) return;
      setSelectedId(chatKey);
      setMasterError(null);
      void loadMessages(chatKey);
      void markRead(chatKey);
    },
    [loadMessages, markRead]
  );

  const sendMaster = useCallback(
    async (rawBody: string): Promise<boolean> => {
      const body = rawBody.trim();
      if (!body) return false;
      setMasterError(null);
      const optimistic: ChatMessage = {
        id: `optimistic:${Date.now()}`,
        from: t('tinyplaceOrchestration.master.you'),
        body,
        timestamp: new Date().toISOString(),
        encrypted: false,
      };
      setMessagesByChat(prev => ({
        ...prev,
        [MASTER_CHAT_KEY]: sortMessages([...(prev[MASTER_CHAT_KEY] ?? []), optimistic]),
      }));
      try {
        await orchestrationClient.sendMasterMessage({ body });
        if (!mountedRef.current) return true;
        // Reconcile against the authoritative server state.
        void loadMessages(MASTER_CHAT_KEY);
        void loadSessions();
        return true;
      } catch (error) {
        if (!mountedRef.current) return false;
        // Roll the optimistic message back out.
        setMessagesByChat(prev => ({
          ...prev,
          [MASTER_CHAT_KEY]: (prev[MASTER_CHAT_KEY] ?? []).filter(m => m.id !== optimistic.id),
        }));
        const message = error instanceof Error ? error.message : String(error);
        setMasterError(message);
        return false;
      }
    },
    [loadMessages, loadSessions, t]
  );

  // Initial load + mark the default (master) chat read.
  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => {
      void refresh().then(() => {
        if (mountedRef.current) void markRead(selectedIdRef.current);
      });
    }, 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
    // Run once on mount; refresh/markRead are stable useCallback refs.
  }, [refresh, markRead]);

  // Live updates: refetch the affected chat + sessions list on new messages.
  useEffect(() => {
    const socket = socketService.getSocket();
    if (!socket) return;
    const handler = (payload: unknown) => {
      const event = payload as OrchestrationMessageEvent | null;
      if (!event || typeof event !== 'object') return;
      const affected = event.chatKind === 'session' ? event.sessionId : event.chatKind;
      void loadSessions();
      void refreshStatus();
      if (affected && affected === selectedIdRef.current) {
        void loadMessages(affected);
      }
    };
    socket.on('orchestration:message', handler);
    socket.on('orchestration_message', handler);
    return () => {
      socket.off('orchestration:message', handler);
      socket.off('orchestration_message', handler);
    };
  }, [loadSessions, loadMessages, refreshStatus]);

  const chats = useMemo(() => {
    return summaries.map(summary =>
      buildChatWindow(summary, messagesByChat[chatKeyForSession(summary)], t)
    );
  }, [summaries, messagesByChat, t]);

  const resolvedSelectedId = chats.some(chat => chat.id === selectedId)
    ? selectedId
    : (chats[0]?.id ?? MASTER_CHAT_KEY);
  const selected = chats.find(chat => chat.id === resolvedSelectedId);

  return {
    sessionsState,
    messagesState,
    chats,
    selectedId: resolvedSelectedId,
    selected,
    status,
    masterError,
    selectChat,
    refresh,
    sendMaster,
  };
}
