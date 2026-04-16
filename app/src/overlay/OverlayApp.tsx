/**
 * OverlayApp
 *
 * Standalone React root rendered inside the Tauri `overlay` window (see
 * `app/src-tauri/tauri.conf.json`). The overlay lives in its own WebView
 * and cannot share Redux state with the main window, so it reacts to
 * signals from the Rust core over a dedicated, unauthenticated Socket.IO
 * connection (same pattern as `useDictationHotkey`).
 *
 * The overlay activates in two cases:
 *
 *   1. **STT / dictation** — when the user presses the dictation hotkey.
 *      The core emits `dictation:toggle` with `{type: "pressed" | "released"}`
 *      and `dictation:transcription` with `{text}`. "Pressed" opens the
 *      overlay into STT mode; "released" (or the final transcription)
 *      dismisses it.
 *
 *   2. **Attention message** — when the core (subconscious loop, heartbeat,
 *      …) publishes an `OverlayAttentionEvent` via
 *      `openhuman::overlay::publish_attention(...)`. The bridge in
 *      `core::socketio` forwards this as an `overlay:attention` event.
 *      The bubble auto-dismisses after its ttl.
 *
 * There is **no** demo loop — the overlay is entirely event-driven.
 */
import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  currentMonitor,
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
} from '@tauri-apps/api/window';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { io, Socket } from 'socket.io-client';

import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';
import { callCoreRpc } from '../services/coreRpcClient';
import { CORE_RPC_URL } from '../utils/config';

const OVERLAY_IDLE_WIDTH = 50;
const OVERLAY_IDLE_HEIGHT = 50;
const OVERLAY_ACTIVE_WIDTH = 224;
const OVERLAY_ACTIVE_HEIGHT = 208;
const OVERLAY_IDLE_MARGIN = 10;
const OVERLAY_ACTIVE_MARGIN = 20;
const OVERLAY_IDLE_OPACITY = 0.6;

const OVERLAY_POSITION_KEY = 'overlay-position';

/** Default auto-dismiss for an attention bubble when no ttl is supplied. */
const DEFAULT_ATTENTION_TTL_MS = 6000;
/** Grace period after STT `released` before returning to idle, giving the
 *  final transcription time to arrive and the user a moment to read it. */
const STT_RELEASE_LINGER_MS = 1500;
/** Placeholder bubble text while waiting for the first transcription. */
const STT_LISTENING_PLACEHOLDER = '"Listening…"';
let lastPollDebugTs = 0;

// ── State model ──────────────────────────────────────────────────────────

type OverlayMode = 'idle' | 'stt' | 'attention';
type BubbleTone = 'neutral' | 'accent' | 'success';

interface OverlayBubble {
  id: string;
  text: string;
  tone: BubbleTone;
  compact?: boolean;
}

// ── Socket payload types ─────────────────────────────────────────────────

interface DictationTogglePayload {
  type?: string;
  hotkey?: string;
  activation_mode?: string;
}

interface DictationTranscriptionPayload {
  text?: string;
}

interface OverlayAttentionPayload {
  id?: string;
  message?: string;
  tone?: BubbleTone;
  ttl_ms?: number;
  source?: string;
}

// ── Helpers ──────────────────────────────────────────────────────────────

function bubbleToneClass(tone: BubbleTone) {
  switch (tone) {
    case 'accent':
      return 'bg-blue-700 text-white';
    case 'success':
      return 'bg-emerald-500 text-emerald-950';
    default:
      return 'bg-slate-700 text-white';
  }
}

/** Resolve the core process base URL (without /rpc suffix) for Socket.IO.
 *  Mirrors `useDictationHotkey.resolveCoreSocketUrl`. */
async function resolveCoreSocketUrl(): Promise<string> {
  let rpcUrl = CORE_RPC_URL;
  if (isTauri()) {
    try {
      const url = await invoke<string>('core_rpc_url');
      if (url) rpcUrl = String(url);
    } catch {
      // fall through to default
    }
  }
  const trimmed = rpcUrl.trim().replace(/\/+$/, '');
  return trimmed.endsWith('/rpc') ? trimmed.slice(0, -4) : trimmed;
}

// ── Bubble chip with typewriter animation ────────────────────────────────

function OverlayBubbleChip({ bubble }: { bubble: OverlayBubble }) {
  // Reset the typewriter on every new bubble identity via `key` at the
  // call site — that avoids a cascading setState inside this effect.
  const [displayedText, setDisplayedText] = useState('');
  const indexRef = useRef(0);

  useEffect(() => {
    if (!bubble.text) {
      return () => {
        indexRef.current = 0;
      };
    }

    const intervalId = window.setInterval(
      () => {
        indexRef.current += 1;
        setDisplayedText(bubble.text.slice(0, indexRef.current));
        if (indexRef.current >= bubble.text.length) {
          window.clearInterval(intervalId);
        }
      },
      bubble.compact ? 28 : 32
    );

    return () => {
      window.clearInterval(intervalId);
      indexRef.current = 0;
    };
  }, [bubble.compact, bubble.text]);

  return (
    <div
      className={`max-w-[184px] rounded-[18px] px-3 py-2 text-right transition-all duration-200 ${bubbleToneClass(bubble.tone)} ${bubble.compact ? 'text-[12px] leading-[1.35]' : 'text-[13px] leading-[1.45]'}`}>
      {displayedText || ' '}
    </div>
  );
}

// ── Main overlay root ────────────────────────────────────────────────────

export default function OverlayApp() {
  const [mode, setMode] = useState<OverlayMode>('idle');
  const [bubble, setBubble] = useState<OverlayBubble | null>(null);
  const [isHovered, setIsHovered] = useState(false);

  /** Timer that returns the overlay to idle after a ttl (attention) or a
   *  grace period (stt release). We clear it whenever the mode changes. */
  const dismissTimerRef = useRef<number | null>(null);

  const clearDismissTimer = useCallback(() => {
    if (dismissTimerRef.current !== null) {
      window.clearTimeout(dismissTimerRef.current);
      dismissTimerRef.current = null;
    }
  }, []);

  const scheduleDismiss = useCallback(
    (ms: number) => {
      clearDismissTimer();
      dismissTimerRef.current = window.setTimeout(() => {
        console.debug('[overlay] auto-dismiss → idle');
        setMode('idle');
        setBubble(null);
        dismissTimerRef.current = null;
      }, ms);
    },
    [clearDismissTimer]
  );

  const goIdle = useCallback(() => {
    clearDismissTimer();
    setMode('idle');
    setBubble(null);
  }, [clearDismissTimer]);

  /** Click handler for the orb: idle → bring main window to front; active → dismiss bubble. */
  const handleOrbClick = useCallback(() => {
    if (mode === 'idle') {
      console.debug('[overlay] orb clicked while idle — activating main window');
      invoke('activate_main_window').catch(err => {
        console.error('[overlay] failed to activate main window:', err);
      });
    } else {
      goIdle();
    }
  }, [mode, goIdle]);

  // ── Dictation: pressed / released ──────────────────────────────────────
  const handleDictationToggle = useCallback(
    (payload: DictationTogglePayload) => {
      const type = payload?.type ?? 'pressed';
      console.debug(`[overlay] dictation:toggle type=${type}`);

      if (type === 'pressed') {
        clearDismissTimer();
        setMode('stt');
        setBubble({
          id: `stt-${Date.now()}`,
          text: STT_LISTENING_PLACEHOLDER,
          tone: 'accent',
          compact: true,
        });
        return;
      }

      if (type === 'released') {
        // Linger briefly so any final transcription arriving shortly after
        // has a chance to land in the bubble before we go idle.
        scheduleDismiss(STT_RELEASE_LINGER_MS);
      }
    },
    [clearDismissTimer, scheduleDismiss]
  );

  // ── Dictation: final transcription text ────────────────────────────────
  const handleDictationTranscription = useCallback(
    (payload: DictationTranscriptionPayload) => {
      const text = payload?.text?.trim();
      if (!text) return;
      console.debug(`[overlay] dictation:transcription chars=${text.length}`);

      setMode('stt');
      setBubble({
        id: `stt-final-${Date.now()}`,
        text: `"${text}"`,
        tone: 'accent',
        compact: true,
      });
      // Show the result briefly then dismiss, regardless of hotkey state.
      scheduleDismiss(STT_RELEASE_LINGER_MS);
    },
    [scheduleDismiss]
  );

  // ── Attention from subconscious / core ─────────────────────────────────
  const handleAttention = useCallback(
    (payload: OverlayAttentionPayload) => {
      const message = payload?.message?.trim();
      if (!message) {
        console.debug('[overlay] attention event with empty message — ignoring');
        return;
      }
      console.debug(
        `[overlay] attention source=${payload?.source ?? 'unknown'} tone=${payload?.tone ?? 'neutral'} chars=${message.length}`
      );

      const ttl =
        typeof payload?.ttl_ms === 'number' && payload.ttl_ms > 0
          ? payload.ttl_ms
          : DEFAULT_ATTENTION_TTL_MS;

      setMode('attention');
      setBubble({
        id: payload?.id ?? `attention-${Date.now()}`,
        text: `"${message}"`,
        // Match the Rust-side `OverlayAttentionTone::default()` (Neutral)
        // so missing/legacy payloads render as the neutral slate bubble.
        tone: payload?.tone ?? 'neutral',
      });
      scheduleDismiss(ttl);
    },
    [scheduleDismiss]
  );

  // ── Socket.IO subscription lifecycle ───────────────────────────────────
  useEffect(() => {
    let socket: Socket | null = null;
    let disposed = false;

    const connect = async () => {
      try {
        const baseUrl = await resolveCoreSocketUrl();
        if (disposed) return;

        console.debug(`[overlay] connecting to core socket at ${baseUrl}`);
        socket = io(baseUrl, {
          path: '/socket.io/',
          transports: ['websocket', 'polling'],
          reconnection: true,
          reconnectionDelay: 2000,
          reconnectionAttempts: Infinity,
          forceNew: true,
        });

        socket.on('connect', () => {
          console.debug('[overlay] socket connected', socket?.id);
        });

        socket.on('connect_error', (err: Error) => {
          console.debug('[overlay] socket connect error:', err.message);
        });

        socket.on('disconnect', (reason: string) => {
          console.debug('[overlay] socket disconnected:', reason);
        });

        // Core emits each event under both colon and underscore forms
        // (see `emit_with_aliases` in `src/core/socketio.rs`). Subscribe
        // only to the canonical colon-delimited form so each signal fires
        // the handler exactly once.
        socket.on('dictation:toggle', handleDictationToggle);
        socket.on('dictation:transcription', handleDictationTranscription);
        socket.on('overlay:attention', handleAttention);

        socket.connect();
      } catch (err) {
        console.warn('[overlay] failed to open core socket', err);
      }
    };

    void connect();

    return () => {
      disposed = true;
      if (socket) {
        socket.disconnect();
        socket = null;
      }
      clearDismissTimer();
    };
  }, [clearDismissTimer, handleAttention, handleDictationToggle, handleDictationTranscription]);

  // ── Poll voice server status as fallback sync ─────────────────────────
  // Socket events are the primary state driver, but if an event is missed
  // (reconnect, brief disconnect) the overlay can get stuck. Polling the
  // actual server state every 2s corrects any drift.
  const modeRef = useRef(mode);
  useEffect(() => {
    modeRef.current = mode;
  }, [mode]);

  useEffect(() => {
    let disposed = false;

    const poll = async () => {
      try {
        const res = await callCoreRpc<{
          state: string;
          hotkey: string;
          activation_mode: string;
          transcription_count: number;
          last_error: string | null;
        }>({ method: 'openhuman.voice_server_status' });

        if (disposed) return;

        const serverState = res.state; // 'stopped' | 'idle' | 'recording' | 'transcribing'
        const currentMode = modeRef.current;

        // Server is actively recording/transcribing but overlay is idle → show stt
        if (
          (serverState === 'recording' || serverState === 'transcribing') &&
          currentMode !== 'stt'
        ) {
          console.debug(
            `[overlay] poll sync: server=${serverState}, overlay=${currentMode} → activating stt`
          );
          clearDismissTimer();
          setMode('stt');
          setBubble({
            id: `stt-poll-${Date.now()}`,
            text: serverState === 'transcribing' ? '"Transcribing…"' : STT_LISTENING_PLACEHOLDER,
            tone: 'accent',
            compact: true,
          });
        }

        // Server is idle/stopped but overlay thinks it's in stt → dismiss
        if ((serverState === 'idle' || serverState === 'stopped') && currentMode === 'stt') {
          console.debug(`[overlay] poll sync: server=${serverState}, overlay=stt → dismissing`);
          goIdle();
        }
      } catch (err) {
        if (process.env.NODE_ENV !== 'production') {
          const now = Date.now();
          if (now - lastPollDebugTs > 5000) {
            lastPollDebugTs = now;
            console.debug('[overlay] RPC poll failed', err);
          }
        }
      }
    };

    void poll();
    const id = window.setInterval(() => void poll(), 2000);
    return () => {
      disposed = true;
      window.clearInterval(id);
    };
  }, [clearDismissTimer, goIdle]);

  // ── Window framing: resize / reposition on mode change ────────────────
  const status: 'idle' | 'active' = mode === 'idle' ? 'idle' : 'active';
  const userDraggedRef = useRef(false);

  /** Save the current window position to localStorage after a drag. */
  const persistPosition = useCallback(async () => {
    try {
      const appWindow = getCurrentWindow();
      const pos = await appWindow.outerPosition();
      localStorage.setItem(OVERLAY_POSITION_KEY, JSON.stringify({ x: pos.x, y: pos.y }));
      userDraggedRef.current = true;
    } catch {
      // position read failed — ignore
    }
  }, []);

  /** Reset saved position so the overlay snaps back to the default corner. */
  const resetPosition = useCallback(() => {
    localStorage.removeItem(OVERLAY_POSITION_KEY);
    userDraggedRef.current = false;
  }, []);

  // NSPanel (non-activating overlay) doesn't deliver synthesized `click`
  // events to the webview, and calling `startDragging()` eagerly on
  // mouse-down blocks `mouseup` from firing. We instead arm the drag only
  // after the pointer moves past a small threshold, so a pure click fires
  // `mouseup` normally and we can activate the main window there.
  const pressRef = useRef<{ x: number; y: number; dragStarted: boolean } | null>(null);
  const CLICK_SLOP_PX = 4;

  /** Record mouse-down position; defer drag until the pointer actually moves. */
  const handleDragStart = useCallback((e: React.MouseEvent) => {
    if (e.button !== 0) return;
    pressRef.current = { x: e.screenX, y: e.screenY, dragStarted: false };
  }, []);

  /** If pointer moves past the slop, escalate into a native window drag. */
  const handleMouseMove = useCallback(
    async (e: React.MouseEvent) => {
      const press = pressRef.current;
      if (!press || press.dragStarted) return;
      const dx = Math.abs(e.screenX - press.x);
      const dy = Math.abs(e.screenY - press.y);
      if (dx <= CLICK_SLOP_PX && dy <= CLICK_SLOP_PX) return;
      press.dragStarted = true;
      try {
        const appWindow = getCurrentWindow();
        await appWindow.startDragging();
        void persistPosition();
      } catch {
        // startDragging can fail if not supported — fall through silently
      }
    },
    [persistPosition]
  );

  /**
   * On mouse-up, treat as a click if no drag was initiated. Emulates
   * `onClick` for the non-activating panel.
   */
  const handleMouseUp = useCallback(
    (e: React.MouseEvent) => {
      const press = pressRef.current;
      pressRef.current = null;
      if (e.button !== 0 || !press) return;
      if (!press.dragStarted) {
        console.debug('[overlay] orb mouseup → click');
        handleOrbClick();
      }
    },
    [handleOrbClick]
  );

  useEffect(() => {
    const appWindow = getCurrentWindow();
    const isActive = status === 'active';
    const width = isActive ? OVERLAY_ACTIVE_WIDTH : OVERLAY_IDLE_WIDTH;
    const height = isActive ? OVERLAY_ACTIVE_HEIGHT : OVERLAY_IDLE_HEIGHT;
    const margin = isActive ? OVERLAY_ACTIVE_MARGIN : OVERLAY_IDLE_MARGIN;
    const size = new LogicalSize(width, height);

    const updateWindowFrame = async () => {
      // Remove all size constraints first, then set the new size, then
      // re-apply constraints. This avoids the ordering problem where the
      // old min/max clamps the new size.
      try {
        await appWindow.setMinSize(null);
      } catch {
        /* ignore */
      }
      try {
        await appWindow.setMaxSize(null);
      } catch {
        /* ignore */
      }
      try {
        await appWindow.setSize(size);
      } catch (error) {
        console.warn('[overlay] failed to resize overlay window', error);
      }
      console.debug(`[overlay] resized to ${width}x${height} (active=${isActive})`);
      // Lock to exact size so the user can't accidentally resize
      try {
        await appWindow.setMinSize(size);
      } catch {
        /* ignore */
      }
      try {
        await appWindow.setMaxSize(size);
      } catch {
        /* ignore */
      }

      // Restore saved position from a previous drag
      const saved = localStorage.getItem(OVERLAY_POSITION_KEY);
      if (saved) {
        try {
          const { x, y } = JSON.parse(saved) as { x: number; y: number };
          await appWindow.setPosition(new LogicalPosition(x, y));
          userDraggedRef.current = true;
          return;
        } catch {
          localStorage.removeItem(OVERLAY_POSITION_KEY);
        }
      }

      if (userDraggedRef.current) {
        return;
      }

      // Default: pin to bottom-right corner
      try {
        const monitor = await currentMonitor();
        if (!monitor) {
          console.warn('[overlay] could not resolve current monitor for positioning');
          return;
        }

        const x = monitor.workArea.position.x + monitor.workArea.size.width - width - margin;
        const y = monitor.workArea.position.y + monitor.workArea.size.height - height - margin;
        await appWindow.setPosition(new LogicalPosition(x, y));
      } catch (error) {
        console.warn('[overlay] failed to pin overlay bottom-right after resize', error);
      }
    };

    void updateWindowFrame();
  }, [status]);

  // ── Render ────────────────────────────────────────────────────────────
  const orbClassName = useMemo(() => {
    if (status === 'active') {
      return 'border-blue-950 bg-blue-700';
    }
    return 'border-slate-950 bg-slate-800';
  }, [status]);
  const tetrahedronInverted = status === 'active';
  const orbSizeClassName = status === 'active' ? 'h-[52px] w-[52px]' : 'h-[40px] w-[40px]';
  const orbCanvasClassName = status === 'active' ? 'h-[92%] w-[92%]' : 'h-[88%] w-[88%]';
  const orbStyle =
    status === 'idle' ? { opacity: isHovered ? 1 : OVERLAY_IDLE_OPACITY } : undefined;

  return (
    <div className="flex h-screen w-screen items-end justify-end bg-transparent px-0 py-0">
      <div
        className={`relative flex select-none flex-col items-end ${status === 'active' ? 'gap-3' : 'gap-0'}`}>
        {status === 'active' && bubble && (
          <div className="max-w-[184px]">
            <div className="animate-[overlay-bubble-in_220ms_ease-out]">
              <OverlayBubbleChip key={bubble.id} bubble={bubble} />
            </div>
          </div>
        )}

        <div className="relative">
          <button
            type="button"
            aria-label={
              mode === 'stt'
                ? 'Voice input active'
                : mode === 'attention'
                  ? 'Attention message'
                  : 'OpenHuman overlay'
            }
            onMouseDown={handleDragStart}
            onMouseMove={handleMouseMove}
            onMouseUp={handleMouseUp}
            onDoubleClick={resetPosition}
            onMouseEnter={() => {
              setIsHovered(true);
            }}
            onMouseLeave={() => {
              setIsHovered(false);
            }}
            className={`group relative flex cursor-grab items-center justify-center overflow-hidden rounded-full border transition-all duration-200 active:cursor-grabbing ${orbClassName} ${orbSizeClassName}`}
            style={orbStyle}
            title="Drag to move · Double-click to reset position">
            <div
              className={`pointer-events-none opacity-95 transition-transform duration-300 group-hover:scale-105 ${orbCanvasClassName}`}>
              <RotatingTetrahedronCanvas inverted={tetrahedronInverted} />
            </div>
          </button>
        </div>
      </div>
    </div>
  );
}
