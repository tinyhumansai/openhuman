import { useConversation } from '@elevenlabs/react';
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { fetchVoiceAgentSignedUrl } from '../../../services/api/voiceAgentApi';
import { MASCOT_VOICE_ID } from '../../../utils/config';

const log = createDebug('app:human:realtime-voice');

/**
 * Lifecycle of a realtime ElevenLabs Agents voice session (#5399).
 * `idle → connecting → active → idle`, or `→ error`.
 */
export type RealtimeSessionState = 'idle' | 'connecting' | 'active' | 'error';

export interface RealtimeVoiceSession {
  state: RealtimeSessionState;
  /** True while the agent is speaking (drives the mascot's speaking pose). */
  isSpeaking: boolean;
  /** ElevenLabs turn mode; `listening` while the user speaks. */
  mode: 'speaking' | 'listening';
  error: string | null;
  /** Fetch a signed URL and open the WebSocket session. Idempotent while busy. */
  start: () => Promise<void>;
  stop: () => void;
}

/**
 * Drives a realtime voice-agent session with `@elevenlabs/react`. Must be used
 * inside a `ConversationProvider` (see `RealtimeVoiceControls`). Uses the
 * WebSocket connection type so the per-audio-event character `alignment` is
 * available for mascot lip-sync.
 */
export function useRealtimeVoiceSession(opts?: { voiceId?: string }): RealtimeVoiceSession {
  const [state, setState] = useState<RealtimeSessionState>('idle');
  const [error, setError] = useState<string | null>(null);
  const startingRef = useRef(false);
  // Tracks whether a session is live so the unmount teardown only ends a real
  // session, and so the cleanup closure isn't tied to a stale `state`.
  const liveRef = useRef(false);

  const conversation = useConversation({
    onConnect: () => {
      liveRef.current = true;
      log('connected');
      setState('active');
    },
    onDisconnect: () => {
      liveRef.current = false;
      log('disconnected');
      setState('idle');
    },
    // Errors from the SDK (mic denied, invalid/expired signed URL, WS handshake)
    // arrive here — startSession itself returns void — so this is the single
    // failure sink. Surface the message to the user (their own error) but log
    // only a stable category, never the raw provider text.
    onError: (message: string) => {
      liveRef.current = false;
      log('session error');
      setError(message);
      setState('error');
    },
  });

  // Keep a ref to the live conversation controls so the unmount effect can tear
  // the session down without re-running on every render.
  const conversationRef = useRef(conversation);
  conversationRef.current = conversation;

  const start = useCallback(async () => {
    if (startingRef.current || state === 'active' || state === 'connecting') return;
    startingRef.current = true;
    setError(null);
    setState('connecting');
    log('start: requesting signed url');
    try {
      const { signedUrl, userToken } = await fetchVoiceAgentSignedUrl();
      log('start: signed url acquired, opening session');
      // `userId` is the identity binding the backend relay verifies (#5399).
      //
      // It rides the conversation-init event as `user_id`, but the provider does
      // not put it on the Custom-LLM request body: a live capture of
      // `POST /voice-agent/chat/completions` carried only
      // [messages, model, max_tokens, stream, stream_options, temperature, tools],
      // so every relayed turn was rejected for having no identity.
      //
      // `customLlmExtraBody` does reach that request — forwarded under an
      // `elevenlabs_extra_body` key rather than merged into the top level, which
      // is where the relay looks for it. `userId` stays for provider-side
      // attribution.
      conversation.startSession({
        signedUrl,
        connectionType: 'websocket',
        userId: userToken,
        customLlmExtraBody: { user: userToken },
        overrides: { tts: { voiceId: opts?.voiceId ?? MASCOT_VOICE_ID } },
      });
    } catch (err) {
      // Only the signed-url fetch rejects here; classify without leaking text.
      log('start failed: signed url request rejected');
      setError(err instanceof Error ? err.message : 'failed to start voice session');
      setState('error');
    } finally {
      startingRef.current = false;
    }
  }, [conversation, opts?.voiceId, state]);

  const stop = useCallback(() => {
    log('stop requested');
    conversation.endSession();
    liveRef.current = false;
    setState('idle');
  }, [conversation]);

  // Tear the session down if the component unmounts mid-call (e.g. the user
  // navigates away or switches voice mode) so the WebSocket and mic are released.
  useEffect(
    () => () => {
      if (liveRef.current) {
        log('unmount teardown: ending live session');
        conversationRef.current.endSession();
        liveRef.current = false;
      }
    },
    []
  );

  return {
    state,
    isSpeaking: conversation.isSpeaking,
    mode: conversation.mode,
    error,
    start,
    stop,
  };
}
