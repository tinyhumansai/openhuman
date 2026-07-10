/**
 * NotchApp
 *
 * Standalone React root rendered inside the native macOS NSPanel that floats
 * at the top-centre of the primary screen (see `app/src-tauri/src/notch_window.rs`).
 *
 * The panel has no Tauri IPC bridge (WKWebView outside the CEF runtime). The
 * Rust host injects the core base URL via `evaluateJavaScript` once
 * `OPENHUMAN_CORE_RPC_URL` is set by `CoreProcessHandle`, dispatching:
 *   `window.__OPENHUMAN_NOTCH_CORE_URL__`  (global)
 *   `notch:core-url` CustomEvent            (for late mounts)
 *
 * This component connects to the core over Socket.IO — identical to
 * `OverlayApp` — and renders a pill that expands from the notch area when
 * voice is active or the agent is performing an action.
 *
 * Events handled:
 *   dictation:toggle          voice recording started / stopped
 *   dictation:transcription   final transcript text
 *   companion:state_changed   agent lifecycle (thinking, speaking, …)
 *   overlay:attention         core broadcast message
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { Socket } from 'socket.io-client';

import { useT } from '../lib/i18n/I18nContext';
import { connectCoreSocket } from '../services/coreSocket';

// ── Types ─────────────────────────────────────────────────────────────────────

// 'ready' is the always-visible idle baseline (shows "Ready"); the pill never
// fully disappears so the user always knows the listener's status.
type NotchMode = 'ready' | 'listening' | 'transcribing' | 'thinking' | 'speaking' | 'attention';

interface NotchState {
  mode: NotchMode;
  text: string;
}

interface DictationTogglePayload {
  type?: string;
}
interface DictationTranscriptionPayload {
  text?: string;
}
interface CompanionStatePayload {
  state?: string;
  message?: string;
}
interface AttentionPayload {
  message?: string;
  ttl_ms?: number;
}

// ── Constants ─────────────────────────────────────────────────────────────────

const LINGER_MS = 1800;
const DEFAULT_TTL_MS = 6000;

// ── Waveform bars (voice activity animation) ──────────────────────────────────

function WaveformBars() {
  return (
    <div className="flex items-center gap-[3px]" aria-hidden="true">
      {[0, 1, 2, 3, 4].map(i => (
        <span
          key={i}
          className="w-[3px] rounded-full bg-surface/90"
          style={{
            height: `${10 + (i % 3) * 4}px`,
            animation: `notch-bar 0.9s ease-in-out infinite`,
            animationDelay: `${i * 0.12}s`,
          }}
        />
      ))}
    </div>
  );
}

// ── Spinner dots ──────────────────────────────────────────────────────────────

function SpinnerDots() {
  return (
    <div className="flex items-center gap-[4px]" aria-hidden="true">
      {[0, 1, 2].map(i => (
        <span
          key={i}
          className="h-[5px] w-[5px] rounded-full bg-surface/80"
          style={{
            animation: `notch-dot 1.2s ease-in-out infinite`,
            animationDelay: `${i * 0.2}s`,
          }}
        />
      ))}
    </div>
  );
}

// ── Icon glyph ────────────────────────────────────────────────────────────────

function ModeIcon({ mode }: { mode: NotchMode }) {
  // Steady green dot when idle/ready — calm "I'm listening for the wake word".
  if (mode === 'ready') return <span className="h-2 w-2 rounded-full bg-emerald-400/90" />;
  if (mode === 'listening') return <WaveformBars />;
  if (mode === 'transcribing' || mode === 'thinking') return <SpinnerDots />;
  if (mode === 'speaking') {
    return (
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        <path
          d="M8 1.5a3 3 0 0 1 3 3v4a3 3 0 0 1-6 0v-4a3 3 0 0 1 3-3z"
          fill="rgba(255,255,255,0.9)"
        />
        <path
          d="M3.5 7.5A4.5 4.5 0 0 0 8 12a4.5 4.5 0 0 0 4.5-4.5"
          stroke="rgba(255,255,255,0.9)"
          strokeWidth="1.2"
          strokeLinecap="round"
        />
        <line
          x1="8"
          y1="12"
          x2="8"
          y2="14.5"
          stroke="rgba(255,255,255,0.9)"
          strokeWidth="1.2"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  // attention / fallback
  return <span className="h-2 w-2 rounded-full bg-blue-400" />;
}

// ── Main component ────────────────────────────────────────────────────────────

export default function NotchApp() {
  const { t } = useT();
  const [state, setState] = useState<NotchState>({ mode: 'ready', text: '' });
  const dismissRef = useRef<number | null>(null);
  const socketRef = useRef<Socket | null>(null);

  const clearDismiss = useCallback(() => {
    if (dismissRef.current !== null) {
      window.clearTimeout(dismissRef.current);
      dismissRef.current = null;
    }
  }, []);

  const scheduleDismiss = useCallback(
    (ms: number) => {
      clearDismiss();
      dismissRef.current = window.setTimeout(() => {
        // Fall back to the always-visible "Ready" baseline, never invisible.
        setState({ mode: 'ready', text: '' });
        dismissRef.current = null;
      }, ms);
    },
    [clearDismiss]
  );

  // ── Socket.IO connection ────────────────────────────────────────────────────

  const connectSocket = useCallback(
    (baseUrl: string) => {
      if (socketRef.current?.connected) return;
      if (socketRef.current) {
        socketRef.current.disconnect();
      }

      let disposed = false;
      void (async () => {
        try {
          const socket = await connectCoreSocket({
            getBaseUrl: async () => baseUrl,
            isDisposed: () => disposed,
          });
          if (!socket || disposed) return;
          socketRef.current = socket;

          socket.on('dictation:toggle', (payload: DictationTogglePayload) => {
            const type = payload?.type ?? 'pressed';
            console.debug(`[notch] dictation:toggle type=${type}`);
            if (type === 'pressed') {
              clearDismiss();
              setState({ mode: 'listening', text: t('notch.listening', 'Listening…') });
            } else if (type === 'released') {
              scheduleDismiss(LINGER_MS);
            }
          });

          socket.on('dictation:transcription', (payload: DictationTranscriptionPayload) => {
            const text = payload?.text?.trim();
            if (!text) return;
            console.debug(`[notch] dictation:transcription chars=${text.length}`);
            clearDismiss();
            setState({
              mode: 'transcribing',
              text: text.length > 60 ? `${text.slice(0, 57)}…` : text,
            });
            scheduleDismiss(LINGER_MS);
          });

          socket.on('companion:state_changed', (payload: CompanionStatePayload) => {
            const agentState = payload?.state ?? 'idle';
            console.debug(`[notch] companion:state_changed state=${agentState}`);

            if (agentState === 'idle') {
              scheduleDismiss(0);
              return;
            }
            clearDismiss();

            const modeMap: Partial<Record<string, NotchMode>> = {
              listening: 'listening',
              thinking: 'thinking',
              speaking: 'speaking',
            };
            const textMap: Partial<Record<string, string>> = {
              listening: t('notch.listening', 'Listening…'),
              thinking: t('notch.processing', 'Processing…'),
              speaking: t('notch.speaking', 'Speaking…'),
            };

            setState({
              mode: modeMap[agentState] ?? 'thinking',
              text: textMap[agentState] ?? agentState,
            });
          });

          socket.on('overlay:attention', (payload: AttentionPayload) => {
            const message = payload?.message?.trim();
            if (!message) return;
            console.debug(`[notch] overlay:attention chars=${message.length}`);
            clearDismiss();
            // The voice listener uses two reserved status words to drive the
            // pill: "Listening" (capturing speech) and "Processing" (running a
            // command). Map them to the matching icon; everything else is a
            // generic attention message.
            const lower = message.toLowerCase();
            const mode: NotchMode =
              lower === 'listening'
                ? 'listening'
                : lower === 'processing'
                  ? 'thinking'
                  : 'attention';
            setState({ mode, text: message.length > 60 ? `${message.slice(0, 57)}…` : message });
            scheduleDismiss(payload?.ttl_ms ?? DEFAULT_TTL_MS);
          });

          socket.connect();
          console.debug('[notch] socket connected', socket.id);
        } catch (err) {
          console.warn('[notch] failed to connect socket', err);
        }
      })();

      return () => {
        disposed = true;
      };
    },
    [t, clearDismiss, scheduleDismiss]
  );

  // ── Core URL bootstrap ──────────────────────────────────────────────────────

  useEffect(() => {
    // Track the in-flight connect's disposer so an unmount (or a new core-url)
    // cancels a still-resolving connectCoreSocket — otherwise the async branch
    // could attach listeners / setState after teardown.
    let disposePendingConnect: (() => void) | undefined;

    // Check if Rust already injected the URL before this component mounted.
    const preloaded = (window as { __OPENHUMAN_NOTCH_CORE_URL__?: string })
      .__OPENHUMAN_NOTCH_CORE_URL__;
    if (preloaded) {
      disposePendingConnect = connectSocket(preloaded);
    }

    // Also listen for the event (fires when core becomes ready after mount).
    const handler = (e: CustomEvent<{ url: string }>) => {
      if (e.detail?.url) {
        disposePendingConnect?.();
        disposePendingConnect = connectSocket(e.detail.url);
      }
    };
    window.addEventListener('notch:core-url', handler as EventListener);

    return () => {
      window.removeEventListener('notch:core-url', handler as EventListener);
      disposePendingConnect?.();
      socketRef.current?.disconnect();
      socketRef.current = null;
      clearDismiss();
    };
  }, [connectSocket, clearDismiss]);

  // ── Render ──────────────────────────────────────────────────────────────────

  const { mode, text } = state;

  // The pill is ALWAYS visible so the user can always see the listener status:
  // Ready (idle) · Listening (capturing speech) · Processing (running a command).
  const label = text || (mode === 'ready' ? t('notch.ready', 'Ready') : '');

  const pillBg =
    mode === 'speaking'
      ? 'bg-[rgba(10,40,10,0.92)]'
      : mode === 'ready'
        ? 'bg-[rgba(10,10,10,0.72)]' // dimmer when idle
        : 'bg-[rgba(10,10,10,0.92)]';

  return (
    <div className="flex h-screen w-screen items-start justify-center bg-transparent pt-[10px]">
      <div
        className={`flex select-none items-center gap-2 rounded-full px-4 py-[7px] shadow-lg ${pillBg}`}
        style={{
          animation: 'notch-pill-in 220ms cubic-bezier(0.34, 1.56, 0.64, 1)',
          backdropFilter: 'blur(12px)',
          WebkitBackdropFilter: 'blur(12px)',
        }}>
        <ModeIcon mode={mode} />
        {label && (
          <span className="max-w-[260px] truncate text-[13px] font-medium leading-none tracking-[-0.01em] text-white/95">
            {label}
          </span>
        )}
      </div>
    </div>
  );
}
