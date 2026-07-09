/**
 * Hooks backing the Connections + Agent-chat session surfaces.
 *
 * - {@link useContactSessions}: the sessions list grouped by contact agent id
 *   (the `sessionsByContact` map the roster/accordion needs), live-refreshed on
 *   the `orchestration:message` socket event.
 * - {@link useSessionTranscript}: the message transcript for one session
 *   (lazy-loaded, socket-refreshed), mapped to {@link ChatMessage} for the
 *   shared `SessionTranscript` renderer.
 *
 * Kept separate from `useOrchestrationChats` (which owns the pinned master /
 * subconscious chat surface) so a panel pulls in only what it needs.
 */
import debugFactory from 'debug';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { socketService } from '../../services/socketService';
import {
  orchestrationClient,
  type OrchestrationMessage,
  type OrchestrationMessageEvent,
  PaymentRequiredError,
  type SessionSummary,
} from './orchestrationClient';
import type { ChatMessage } from './useOrchestrationChats';

const debug = debugFactory('orchestration:sessions');

const TRANSCRIPT_LIMIT = 100;

export type SessionsState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'payment_required' }
  | { status: 'ok' };

export interface UseContactSessionsResult {
  state: SessionsState;
  /** All non-pinned session windows. */
  sessions: SessionSummary[];
  /** Sessions grouped by their peer contact agent id. */
  byContact: Map<string, SessionSummary[]>;
  refresh: () => Promise<void>;
}

function groupByContact(sessions: SessionSummary[]): Map<string, SessionSummary[]> {
  const map = new Map<string, SessionSummary[]>();
  for (const session of sessions) {
    if (session.chatKind !== 'session' || !session.agentId) continue;
    const list = map.get(session.agentId) ?? [];
    list.push(session);
    map.set(session.agentId, list);
  }
  return map;
}

export function useContactSessions(): UseContactSessionsResult {
  const [state, setState] = useState<SessionsState>({ status: 'loading' });
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const mountedRef = useRef(true);

  const refresh = useCallback(async () => {
    debug('[orchestration:sessions] contact-sessions refresh: entry');
    try {
      const { sessions: rows } = await orchestrationClient.sessionsList();
      if (!mountedRef.current) return;
      const sessionRows = rows.filter(s => s.chatKind === 'session');
      debug('[orchestration:sessions] contact-sessions refresh: ok count=%d', sessionRows.length);
      setSessions(sessionRows);
      setState({ status: 'ok' });
    } catch (error) {
      if (!mountedRef.current) return;
      if (error instanceof PaymentRequiredError) {
        debug('[orchestration:sessions] contact-sessions refresh: payment_required');
        setState({ status: 'payment_required' });
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      debug('[orchestration:sessions] contact-sessions refresh: error %s', message);
      setState({ status: 'error', message });
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => void refresh(), 0);
    const onMessage = (): void => {
      debug('[orchestration:sessions] socket refresh (contact sessions)');
      void refresh();
    };
    socketService.on('orchestration:message', onMessage);
    socketService.on('orchestration_message', onMessage);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
      socketService.off('orchestration:message', onMessage);
      socketService.off('orchestration_message', onMessage);
    };
  }, [refresh]);

  const byContact = useMemo(() => groupByContact(sessions), [sessions]);
  return { state, sessions, byContact, refresh };
}

/** OrchestrationMessage → ChatMessage view-model row. */
export function mapTranscriptMessage(message: OrchestrationMessage): ChatMessage {
  return {
    id: message.id,
    from: message.role?.trim() || message.agentId || '',
    body: message.body,
    timestamp: message.timestamp,
    encrypted: false,
    ...(message.eventKind ? { eventKind: message.eventKind } : {}),
    ...(message.toolName ? { toolName: message.toolName } : {}),
    ...(message.callId ? { callId: message.callId } : {}),
    ...(message.ok !== undefined ? { ok: message.ok } : {}),
    ...(message.isError !== undefined ? { isError: message.isError } : {}),
    ...(message.exitCode !== undefined ? { exitCode: message.exitCode } : {}),
  };
}

export type TranscriptState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ok' };

export interface UseSessionTranscriptResult {
  state: TranscriptState;
  messages: ChatMessage[];
  refresh: () => Promise<void>;
}

/** Load + live-refresh one session's transcript. Pass `null` to load nothing. */
export function useSessionTranscript(sessionId: string | null): UseSessionTranscriptResult {
  const [state, setState] = useState<TranscriptState>({ status: 'idle' });
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const mountedRef = useRef(true);
  // Monotonic request token: only the newest in-flight load may apply its
  // result, so switching `sessionId` can never overwrite state with a slower
  // response for the PREVIOUS session (the shared `mountedRef` alone can't
  // guard this — the new effect re-sets it to true before the stale request
  // resolves).
  const reqRef = useRef(0);

  const refresh = useCallback(async () => {
    if (!sessionId) {
      setMessages([]);
      setState({ status: 'idle' });
      return;
    }
    const reqId = ++reqRef.current;
    const target = sessionId;
    debug('[orchestration:sessions] transcript load: entry session=%s req=%d', target, reqId);
    setState(prev => (prev.status === 'ok' ? prev : { status: 'loading' }));
    try {
      const { messages: rows } = await orchestrationClient.messagesList({
        chat: target,
        limit: TRANSCRIPT_LIMIT,
      });
      // Drop a stale response (a newer load started, or we unmounted).
      if (!mountedRef.current || reqRef.current !== reqId) {
        debug(
          '[orchestration:sessions] transcript load: dropped stale session=%s req=%d',
          target,
          reqId
        );
        return;
      }
      debug(
        '[orchestration:sessions] transcript load: ok session=%s count=%d',
        target,
        rows.length
      );
      setMessages(rows.map(mapTranscriptMessage));
      setState({ status: 'ok' });
    } catch (error) {
      if (!mountedRef.current || reqRef.current !== reqId) return;
      const message = error instanceof Error ? error.message : String(error);
      debug('[orchestration:sessions] transcript load: error session=%s %s', target, message);
      setState({ status: 'error', message });
    }
  }, [sessionId]);

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => void refresh(), 0);
    const onMessage = (payload: unknown): void => {
      const event = payload as OrchestrationMessageEvent | null;
      const affected = event && event.chatKind === 'session' ? event.sessionId : null;
      if (affected && affected === sessionId) {
        debug('[orchestration:sessions] socket refresh (transcript) session=%s', sessionId);
        void refresh();
      }
    };
    socketService.on('orchestration:message', onMessage);
    socketService.on('orchestration_message', onMessage);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
      socketService.off('orchestration:message', onMessage);
      socketService.off('orchestration_message', onMessage);
    };
  }, [refresh, sessionId]);

  return { state, messages, refresh };
}
