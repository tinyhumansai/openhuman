/**
 * PttHotkeyManager
 *
 * Renderless boot-time wiring for the global push-to-talk feature:
 *   1. Registers the persisted PTT shortcut with the Tauri shell via
 *      `usePttHotkey()`.
 *   2. Owns the singleton `pttService` state machine (built in T10), wired to
 *      real audio capture (MediaRecorder), STT (voice_transcribe_bytes RPC),
 *      chat send, thread resolution, chime playback, and overlay window
 *      visibility.
 *   3. Subscribes to the Tauri events `ptt://start` / `ptt://stop` emitted by
 *      the Rust shell when the global hotkey transitions edges, and forwards
 *      them into the service.
 *
 * The service is constructed once for the AppShell's lifetime — multiple
 * mounts would create competing state machines fighting over the same mic.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import debug from 'debug';
import { useEffect, useMemo, useRef } from 'react';
import { useDispatch, useStore } from 'react-redux';

import { cancelPttAudio, finalizePttAudio, startPttAudio } from '../features/voice/pttAudio';
import { playPttChime } from '../features/voice/pttChimes';
import { createNewVoiceThread, resolveActiveThreadId } from '../features/voice/pttThread';
import { transcribePttAudio } from '../features/voice/pttTranscribe';
import { usePttHotkey } from '../hooks/usePttHotkey';
import { chatSend } from '../services/chatService';
import { createPttService } from '../services/pttService';
import type { RootState } from '../store';
import { setIsHeld } from '../store/pttSlice';
import { showPttOverlay } from '../utils/tauriCommands/ptt';

const log = debug('app:ptt:manager');

interface PttEventPayload {
  session_id: number;
}

// Stable monotonic clock for the pttService state machine. Defined at
// module scope so the useMemo factory below doesn't reference an impure
// function during render (react-hooks/purity).
const monotonicNow = (): number => Date.now();

export default function PttHotkeyManager(): null {
  // Register / unregister the configured hotkey with the Tauri shell.
  usePttHotkey();

  const dispatch = useDispatch();
  const store = useStore<RootState>();
  const unlistenRef = useRef<UnlistenFn[]>([]);

  const service = useMemo(
    () =>
      createPttService({
        audioCapture: { start: startPttAudio, finalize: finalizePttAudio, cancel: cancelPttAudio },
        transcribe: transcribePttAudio,
        sendMessage: async ({ threadId, body, speakReply, metadata }) => {
          await chatSend({
            threadId,
            message: body,
            speakReply,
            source: metadata.source,
            sessionId: metadata.session_id,
          });
        },
        resolveActiveThreadId,
        createNewVoiceThread,
        playChime: playPttChime,
        showOverlay: async (active, sessionId) => {
          // Respect the user's "show overlay" preference for the start edge,
          // but always tear it down on stop so a mid-session toggle can't leave
          // the overlay stuck visible.
          if (!active || store.getState().ptt.showOverlay) {
            await showPttOverlay(active, sessionId);
          }
        },
        getSettings: () => {
          const ptt = store.getState().ptt;
          return { speakReplies: ptt.speakReplies, showOverlay: ptt.showOverlay };
        },
        now: monotonicNow,
        // 10 s ceiling on a single PTT recording — matches the spec; if the
        // user holds the key longer the watchdog finalises so we don't keep
        // an open mic forever.
        watchdogMs: 10_000,
        // Recordings shorter than this are treated as accidental taps.
        minAudioMs: 250,
        logger: {
          debug: (msg, meta) => log(msg, meta ?? {}),
          info: (msg, meta) => log(msg, meta ?? {}),
          warn: (msg, meta) => log(msg, meta ?? {}),
        },
      }),
    // The service holds an internal state machine — recreating it across
    // store updates would orphan in-flight sessions. The closures above read
    // the latest store state on every call, so a stable identity is correct.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    []
  );

  useEffect(() => {
    let mounted = true;
    const subscribe = async () => {
      try {
        const offStart = await listen<PttEventPayload>('ptt://start', e => {
          dispatch(setIsHeld(true));
          service.onStart(e.payload.session_id).catch(err => {
            log('onStart failed', { sessionId: e.payload.session_id, err: String(err) });
          });
        });
        const offStop = await listen<PttEventPayload>('ptt://stop', e => {
          dispatch(setIsHeld(false));
          service.onStop(e.payload.session_id).catch(err => {
            log('onStop failed', { sessionId: e.payload.session_id, err: String(err) });
          });
        });
        if (!mounted) {
          offStart();
          offStop();
          return;
        }
        unlistenRef.current.push(offStart, offStop);
        log('PttHotkeyManager: listeners attached');
      } catch (err) {
        log('PttHotkeyManager: failed to attach listeners', err);
      }
    };
    void subscribe();
    return () => {
      mounted = false;
      const offs = unlistenRef.current;
      unlistenRef.current = [];
      for (const off of offs) {
        try {
          off();
        } catch (err) {
          log('PttHotkeyManager: unlisten threw', err);
        }
      }
    };
  }, [dispatch, service]);

  return null;
}
