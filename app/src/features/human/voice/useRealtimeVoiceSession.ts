import { useConversation } from '@elevenlabs/react';
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  PROACTIVE_VOICE_THREAD_ID,
  proactiveThreadPins,
} from '../../../providers/proactiveThreadPins';
import { fetchVoiceAgentSignedUrl } from '../../../services/api/voiceAgentApi';
import { socketService } from '../../../services/socketService';
import { MASCOT_VOICE_ID } from '../../../utils/config';

const log = createDebug('app:human:realtime-voice');

/**
 * Instruction prefix for "speak-back". A slow voice turn (e.g. an email summary)
 * is acknowledged aloud, finishes in the background, and its result is delivered
 * to chat AND pushed here as a `voice_speak` event. We send it back into the live
 * ElevenLabs session as a user message wrapped with this prefix so the agent reads
 * it verbatim. MUST match `VOICE_READBACK_PREFIX` in `voice/realtime_harness.rs`,
 * which uses it to avoid re-arming speak-back on the read-back turn (loop guard).
 */
export const READBACK_PREFIX =
  'Please read the following to me, word for word, and say nothing else:';

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
  /**
   * Output loudness (0..1) of the agent's voice, sampled from the SDK's
   * analyser. Drives the mascot's amplitude lip-sync — the realtime SDK owns
   * playback, so this is the only signal available to animate the mouth against.
   */
  getOutputVolume: () => number;
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
  // Answers already read aloud in THIS call, so a redelivered result is not
  // spoken twice. Scoped per call: asking the same question again in a later
  // session should of course be answered again.
  const spokenRef = useRef<Set<string>>(new Set());

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
    spokenRef.current.clear();
    // Each session is its own conversation: drop any pin from a prior session so
    // this session's deferred answers resolve to a fresh/current thread rather
    // than appending to the previous session's thread (which the Human page may
    // no longer have selected — the answer would land off-screen). The pin is
    // re-established on this session's first delivery. See
    // `resolveVisibleThreadForProactive` in ChatRuntimeProvider.
    proactiveThreadPins.delete(PROACTIVE_VOICE_THREAD_ID);
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

  // Speak-back: a slow voice turn (email/calendar summary) is acknowledged aloud,
  // finishes in the background, and the core emits its result as a `voice_speak`
  // event. While the call is still open, read it aloud by sending it back into the
  // live ElevenLabs session wrapped in the verbatim prefix (a fast read-back turn).
  // The result also lands in chat regardless (delivered core-side) — this is the
  // spoken copy. Refs keep the subscription set up once while always seeing the
  // live conversation and liveness.
  useEffect(() => {
    const handler = (payload: unknown) => {
      if (!liveRef.current) return; // call already ended — the chat copy stands alone
      const text = (payload as { full_response?: string } | undefined)?.full_response?.trim();
      if (!text) return;
      // Each read-back is a real turn the agent has to speak, so a repeat is not
      // merely redundant: it queues behind the first and pushes the session
      // towards the provider's per-turn ceiling. Redelivery of the same answer
      // (a retried turn, a reconnect) must therefore be spoken once.
      if (spokenRef.current.has(text)) {
        log('speak-back: already read this answer aloud — skipping');
        return;
      }
      spokenRef.current.add(text);
      log('speak-back: reading deferred result aloud (%d chars)', text.length);
      conversationRef.current.sendUserMessage(`${READBACK_PREFIX}\n\n${text}`);
    };
    socketService.on('voice_speak', handler);
    return () => socketService.off('voice_speak', handler);
  }, []);

  return {
    state,
    isSpeaking: conversation.isSpeaking,
    mode: conversation.mode,
    getOutputVolume: conversation.getOutputVolume,
    error,
    start,
    stop,
  };
}
