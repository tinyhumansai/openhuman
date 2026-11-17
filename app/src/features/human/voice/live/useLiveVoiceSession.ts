/**
 * React hook driving one live voice-agent session against the core's
 * `/ws/live-voice` socket.
 *
 * `idle → connecting → listening ⇄ speaking → idle`, or `→ error` from any of
 * them. The mic is opened only after the core answers `ready`, so a session the
 * core refuses never prompts for the microphone.
 *
 * The core persists every FINAL transcript into the open thread itself, so this
 * hook never appends chat messages — it re-loads the thread's messages after
 * each final transcript and when the session closes.
 *
 * Logs (namespace `app:human:live-voice`) carry state edges, provider ids,
 * sizes and error codes — never transcript text.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { trackAnalyticsEvent } from '../../../../components/analytics';
import { socketService } from '../../../../services/socketService';
import { useAppDispatch } from '../../../../store/hooks';
import { loadThreadMessages } from '../../../../store/threadSlice';
import { type LiveVoiceEvent, LiveVoiceSocket, resolveLiveVoiceUrl } from './liveVoiceSocket';
import {
  LIVE_VOICE_INPUT_SAMPLE_RATE,
  type PcmCapture,
  startPcmCapture,
} from './pcmCaptureWorklet';
import { PcmPlayer } from './pcmPlayer';
import { READBACK_PREFIX } from './readbackPrefix';

const log = createDebug('app:human:live-voice');

/** Final caption lines kept for display. */
const MAX_CAPTIONS = 20;

export type LiveVoiceState = 'idle' | 'connecting' | 'listening' | 'speaking' | 'error';

export interface LiveVoiceCaption {
  role: 'user' | 'agent';
  text: string;
}

export interface LiveVoiceToolCall {
  callId: string;
  name: string;
  status: 'running' | 'ok' | 'failed' | 'cancelled';
}

export interface LiveVoiceError {
  code: string;
  message: string;
  fatal: boolean;
}

export interface LiveVoiceStartOptions {
  /** Provider id; omitted/null = the user's default. */
  provider?: string | null;
  /** Thread the session talks into; null lets the core pick. */
  threadId?: string | null;
}

export interface LiveVoiceSession {
  state: LiveVoiceState;
  /** Provider that answered `ready`, or null before then. */
  provider: string | null;
  /** Thread the core bound the session to. */
  threadId: string | null;
  muted: boolean;
  /** Finalised caption lines, oldest first. */
  captions: LiveVoiceCaption[];
  /** The utterance in progress per role (replaced, not appended, until final). */
  partial: { user: string | null; agent: string | null };
  toolCalls: LiveVoiceToolCall[];
  error: LiveVoiceError | null;
  /** True between `start()` and teardown. */
  active: boolean;
  start: (opts?: LiveVoiceStartOptions) => Promise<void>;
  stop: () => void;
  sendText: (text: string) => void;
  interrupt: () => void;
  toggleMute: () => void;
  setMuted: (muted: boolean) => void;
  /** Output loudness (0..1) of the agent's voice right now, for lip-sync. */
  getOutputVolume: () => number;
}

interface LiveResources {
  socket: LiveVoiceSocket | null;
  capture: PcmCapture | null;
  player: PcmPlayer | null;
  ready: boolean;
  provider: string | null;
  threadId: string | null;
  tracked: boolean;
}

const emptyResources = (): LiveResources => ({
  socket: null,
  capture: null,
  player: null,
  ready: false,
  provider: null,
  threadId: null,
  tracked: false,
});

export function useLiveVoiceSession(): LiveVoiceSession {
  const dispatch = useAppDispatch();
  const [state, setState] = useState<LiveVoiceState>('idle');
  const [provider, setProvider] = useState<string | null>(null);
  const [threadId, setThreadId] = useState<string | null>(null);
  const [muted, setMutedState] = useState(false);
  const [captions, setCaptions] = useState<LiveVoiceCaption[]>([]);
  const [partial, setPartial] = useState<{ user: string | null; agent: string | null }>({
    user: null,
    agent: null,
  });
  const [toolCalls, setToolCalls] = useState<LiveVoiceToolCall[]>([]);
  const [error, setError] = useState<LiveVoiceError | null>(null);

  // Bumped on every start and teardown; async work from an older session
  // compares against it and bails instead of touching the new one.
  const genRef = useRef(0);
  const resRef = useRef<LiveResources>(emptyResources());
  const mutedRef = useRef(false);
  const mountedRef = useRef(true);
  const spokenRef = useRef<Set<string>>(new Set());
  const [active, setActive] = useState(false);

  const refreshThread = useCallback(
    (id: string | null) => {
      if (!id) return;
      log('[thread] refreshing persisted voice transcript');
      void dispatch(loadThreadMessages(id));
    },
    [dispatch]
  );

  /** Release everything the current session holds. Safe to call repeatedly. */
  const teardown = useCallback(
    (reason: string, notifyCore: boolean) => {
      genRef.current += 1;
      const res = resRef.current;
      resRef.current = emptyResources();
      if (res.socket) {
        if (notifyCore) res.socket.stop();
        else res.socket.close();
      }
      res.capture?.stop();
      res.player?.close();
      if (res.tracked) {
        trackAnalyticsEvent('live_voice_session_ended', { provider: res.provider ?? 'unknown' });
      }
      if (res.socket || res.capture || res.player) {
        log('[session] teardown reason=%s', reason);
        refreshThread(res.threadId);
      }
      if (mountedRef.current) setActive(false);
    },
    [refreshThread]
  );

  const fail = useCallback(
    (err: LiveVoiceError) => {
      log('[session] error code=%s fatal=%s', err.code, err.fatal);
      teardown(`error:${err.code}`, true);
      if (!mountedRef.current) return;
      setError(err);
      setState('error');
      setPartial({ user: null, agent: null });
    },
    [teardown]
  );

  const handleEvent = useCallback(
    (gen: number, event: LiveVoiceEvent) => {
      if (gen !== genRef.current) return;
      const res = resRef.current;
      switch (event.type) {
        case 'ready': {
          log(
            '[session] ready provider=%s output_rate=%d',
            event.provider,
            event.output_sample_rate
          );
          res.ready = true;
          res.provider = event.provider;
          res.threadId = event.thread_id ?? res.threadId;
          setProvider(event.provider);
          setThreadId(res.threadId);
          res.player = new PcmPlayer({
            sampleRate: event.output_sample_rate,
            onPlayingChange: playing => {
              if (gen !== genRef.current) return;
              setState(playing ? 'speaking' : 'listening');
            },
          });
          if (!res.tracked) {
            res.tracked = true;
            trackAnalyticsEvent('live_voice_session_started', { provider: event.provider });
          }
          startPcmCapture({
            onFrame: frame => {
              if (gen !== genRef.current || mutedRef.current) return;
              resRef.current.socket?.sendAudio(frame);
            },
          })
            .then(capture => {
              if (gen !== genRef.current) {
                capture.stop();
                return;
              }
              resRef.current.capture = capture;
              capture.setMuted(mutedRef.current);
              setState(current => (current === 'connecting' ? 'listening' : current));
            })
            .catch((err: unknown) => {
              if (gen !== genRef.current) return;
              const name = err instanceof DOMException ? err.name : 'unknown';
              log('[capture] failed name=%s', name);
              fail({
                code: name === 'NotAllowedError' ? 'mic_denied' : 'mic_unavailable',
                message: err instanceof Error ? err.message : String(err),
                fatal: true,
              });
            });
          return;
        }
        case 'transcript': {
          if (event.final) {
            log('[transcript] final role=%s chars=%d', event.role, event.text.length);
            setPartial(prev => ({ ...prev, [event.role]: null }));
            if (event.text.trim()) {
              setCaptions(prev =>
                [...prev, { role: event.role, text: event.text }].slice(-MAX_CAPTIONS)
              );
            }
            refreshThread(res.threadId);
          } else {
            setPartial(prev => ({ ...prev, [event.role]: event.text }));
          }
          return;
        }
        case 'tool_started':
          log('[tool] started name=%s', event.name);
          setToolCalls(prev => [
            ...prev.filter(call => call.callId !== event.call_id),
            { callId: event.call_id, name: event.name, status: 'running' },
          ]);
          return;
        case 'tool_finished': {
          const status: LiveVoiceToolCall['status'] = event.cancelled
            ? 'cancelled'
            : event.ok
              ? 'ok'
              : 'failed';
          log('[tool] finished name=%s status=%s', event.name, status);
          setToolCalls(prev => {
            const known = prev.some(call => call.callId === event.call_id);
            return known
              ? prev.map(call => (call.callId === event.call_id ? { ...call, status } : call))
              : [...prev, { callId: event.call_id, name: event.name, status }];
          });
          return;
        }
        case 'interrupted':
          log('[session] interrupted — flushing playback');
          res.player?.flush();
          setPartial(prev => ({ ...prev, agent: null }));
          return;
        case 'turn_complete':
          setToolCalls(prev => prev.filter(call => call.status === 'running'));
          return;
        case 'error':
          if (event.fatal) {
            fail({ code: event.code, message: event.message, fatal: true });
          } else {
            log('[session] non-fatal error code=%s', event.code);
            setError({ code: event.code, message: event.message, fatal: false });
          }
          return;
        case 'closed':
          log('[session] closed by core reason=%s', event.reason);
          teardown('core_closed', false);
          setState('idle');
          setPartial({ user: null, agent: null });
          return;
      }
    },
    [fail, refreshThread, teardown]
  );

  const start = useCallback(
    async (opts?: LiveVoiceStartOptions) => {
      if (resRef.current.socket) {
        log('[session] start ignored — already active');
        return;
      }
      teardown('restart', false);
      const gen = genRef.current;
      setActive(true);
      setState('connecting');
      setError(null);
      setCaptions([]);
      setPartial({ user: null, agent: null });
      setToolCalls([]);
      setProvider(null);
      spokenRef.current.clear();
      resRef.current.threadId = opts?.threadId ?? null;
      setThreadId(opts?.threadId ?? null);
      log(
        '[session] start provider=%s has_thread=%s',
        opts?.provider ?? 'default',
        !!opts?.threadId
      );

      let url: string;
      try {
        url = await resolveLiveVoiceUrl();
      } catch (err) {
        if (gen !== genRef.current) return;
        fail({
          code: 'core_unreachable',
          message: err instanceof Error ? err.message : String(err),
          fatal: true,
        });
        return;
      }
      if (gen !== genRef.current) return;

      const clientId = socketService.getSocket()?.id;
      const socket = new LiveVoiceSocket(
        url,
        {
          type: 'start',
          provider: opts?.provider ?? null,
          thread_id: opts?.threadId ?? null,
          input_sample_rate: LIVE_VOICE_INPUT_SAMPLE_RATE,
          ...(clientId ? { client_id: clientId } : {}),
        },
        {
          onEvent: event => handleEvent(gen, event),
          onAudio: pcm => {
            if (gen !== genRef.current) return;
            resRef.current.player?.enqueue(pcm);
          },
          onClose: ({ code }) => {
            if (gen !== genRef.current) return;
            const wasReady = resRef.current.ready;
            log('[session] socket closed code=%d ready=%s', code, wasReady);
            if (wasReady) {
              teardown('socket_closed', false);
              setState('idle');
              setPartial({ user: null, agent: null });
            } else {
              fail({
                code: 'connection_failed',
                message: `live voice connection closed (${code})`,
                fatal: true,
              });
            }
          },
        }
      );
      resRef.current.socket = socket;
    },
    [fail, handleEvent, teardown]
  );

  const stop = useCallback(() => {
    log('[session] stop requested');
    teardown('user_stop', true);
    setState('idle');
    setPartial({ user: null, agent: null });
    setToolCalls([]);
  }, [teardown]);

  const sendText = useCallback((text: string) => {
    const trimmed = text.trim();
    if (!trimmed || !resRef.current.ready) return;
    log('[session] send text chars=%d', trimmed.length);
    resRef.current.socket?.sendText(trimmed);
  }, []);

  const interrupt = useCallback(() => {
    const res = resRef.current;
    if (!res.socket) return;
    log('[session] interrupt requested');
    res.player?.flush();
    res.socket.interrupt();
  }, []);

  const setMuted = useCallback((next: boolean) => {
    mutedRef.current = next;
    resRef.current.capture?.setMuted(next);
    setMutedState(next);
  }, []);

  const toggleMute = useCallback(() => setMuted(!mutedRef.current), [setMuted]);

  const getOutputVolume = useCallback(() => resRef.current.player?.getAmplitude() ?? 0, []);

  // Full teardown on unmount: socket, mic tracks and both AudioContexts.
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      teardown('unmount', true);
    };
  }, [teardown]);

  // Speak-back: a slow voice turn finishes in the background and the core emits
  // its result as `voice_speak`. While the session is live, ask the agent to read
  // it aloud verbatim. Each answer is spoken once per session.
  useEffect(() => {
    const handler = (payload: unknown) => {
      if (!resRef.current.ready) return;
      const text = (payload as { full_response?: string } | undefined)?.full_response?.trim();
      if (!text) return;
      if (spokenRef.current.has(text)) {
        log('[speak-back] already read this answer aloud — skipping');
        return;
      }
      spokenRef.current.add(text);
      log('[speak-back] reading deferred result aloud chars=%d', text.length);
      resRef.current.socket?.sendText(`${READBACK_PREFIX}\n\n${text}`);
    };
    socketService.on('voice_speak', handler);
    return () => socketService.off('voice_speak', handler);
  }, []);

  return {
    state,
    provider,
    threadId,
    muted,
    captions,
    partial,
    toolCalls,
    error,
    active,
    start,
    stop,
    sendText,
    interrupt,
    toggleMute,
    setMuted,
    getOutputVolume,
  };
}
