/**
 * pttService — push-to-talk session state machine.
 *
 * See spec: `docs/superpowers/specs/2026-06-02-global-ptt-design.md` (§ 2, § 3).
 *
 * Dependency-injected so vitest can exercise the state machine with fake
 * audio capture / fake STT / fake sendMessage. Real wiring (subscribing to
 * `ptt://*` Tauri events, the real audio_capture, etc.) happens in
 * PttHotkeyManager.tsx (T11).
 */

export type ChimeKind = 'open' | 'close' | 'error';

export interface PttSettings {
  speakReplies: boolean;
  showOverlay: boolean;
}

export interface FinalizedAudio {
  durationMs: number;
  buffer: ArrayBuffer;
}

export interface PttDeps {
  audioCapture: {
    start(opts: { sessionTag: string }): Promise<void>;
    finalize(): Promise<FinalizedAudio>;
    cancel(): Promise<void>;
  };
  transcribe(buf: ArrayBuffer): Promise<string>;
  sendMessage(args: {
    threadId: string;
    body: string;
    metadata: { source: 'ptt'; session_id: number };
    speakReply: boolean;
  }): Promise<void>;
  resolveActiveThreadId(): Promise<string | null>;
  createNewVoiceThread(): Promise<string>;
  playChime(kind: ChimeKind): Promise<void>;
  showOverlay(active: boolean, sessionId: number): Promise<void>;
  getSettings(): PttSettings;
  now(): number;
  watchdogMs: number;
  minAudioMs: number;
  logger: {
    debug(msg: string, meta?: Record<string, unknown>): void;
    info(msg: string, meta?: Record<string, unknown>): void;
    warn(msg: string, meta?: Record<string, unknown>): void;
  };
}

export interface PttService {
  onStart(sessionId: number): Promise<void>;
  onStop(sessionId: number): Promise<void>;
  cancel(reason: 'preempted' | 'mic_failure' | 'user_cancel'): Promise<void>;
}

interface ActiveSession {
  sessionId: number;
  startedAtMs: number;
  watchdogTimer: ReturnType<typeof setTimeout> | null;
  finalizedByWatchdog: boolean;
}

export function createPttService(deps: PttDeps): PttService {
  let active: ActiveSession | null = null;

  const armWatchdog = (sessionId: number) => {
    const timer = setTimeout(() => {
      if (active && active.sessionId === sessionId) {
        active.finalizedByWatchdog = true;
        deps.logger.warn('[ptt] watchdog fired — finalising session', { sessionId });
        // Fire-and-forget; the watchdog path is the same as a normal stop
        // except for the `finalizedByWatchdog` flag (used in logging only).
        void finaliseSession(sessionId, /* fromWatchdog */ true);
      }
    }, deps.watchdogMs);
    return timer;
  };

  const finaliseSession = async (sessionId: number, fromWatchdog: boolean) => {
    if (!active || active.sessionId !== sessionId) {
      // Stale finalisation — ignore.
      return;
    }

    if (active.watchdogTimer) {
      clearTimeout(active.watchdogTimer);
      active.watchdogTimer = null;
    }

    const settings = deps.getSettings();
    const session = active;
    active = null;

    let audio: FinalizedAudio;
    try {
      audio = await deps.audioCapture.finalize();
    } catch (err) {
      deps.logger.warn('[ptt] audio finalize failed', { sessionId, err: String(err) });
      await deps.playChime('error');
      await deps.showOverlay(false, sessionId);
      return;
    }

    await deps.playChime('close');
    await deps.showOverlay(false, sessionId);

    if (audio.durationMs < deps.minAudioMs) {
      deps.logger.info('[ptt] session dropped — audio shorter than minAudioMs', {
        sessionId,
        durationMs: audio.durationMs,
      });
      await deps.playChime('error');
      return;
    }

    let text = '';
    try {
      text = await deps.transcribe(audio.buffer);
    } catch (err) {
      deps.logger.warn('[ptt] transcription failed', { sessionId, err: String(err) });
      // Per spec: post the message anyway as a breadcrumb.
      text = '[Voice — transcription failed]';
    }

    const trimmed = text.trim();

    if (!trimmed) {
      deps.logger.info('[ptt] session dropped — empty transcript', { sessionId });
      await deps.playChime('error');
      return;
    }

    let threadId: string;
    try {
      const resolved = await deps.resolveActiveThreadId();
      if (!resolved) {
        threadId = await deps.createNewVoiceThread();
      } else {
        threadId = resolved;
      }
    } catch (err) {
      deps.logger.warn('[ptt] thread resolution failed — aborting commit', {
        sessionId,
        err: String(err),
      });
      await deps.playChime('error');
      return;
    }

    try {
      await deps.sendMessage({
        threadId,
        body: trimmed,
        metadata: { source: 'ptt', session_id: sessionId },
        speakReply: settings.speakReplies,
      });
    } catch (err) {
      deps.logger.warn('[ptt] sendMessage failed', { sessionId, threadId, err: String(err) });
      await deps.playChime('error');
      return;
    }

    deps.logger.info('[ptt] session committed', {
      sessionId,
      threadId,
      heldMs: deps.now() - session.startedAtMs,
      finalizedByWatchdog: fromWatchdog,
      transcriptLen: trimmed.length,
    });
  };

  return {
    async onStart(sessionId) {
      // Preempt: if another session is active, cancel it.
      if (active) {
        deps.logger.debug('[ptt] onStart while active — preempting', {
          old: active.sessionId,
          new: sessionId,
        });
        try {
          await deps.audioCapture.cancel();
        } catch (err) {
          deps.logger.warn('[ptt] cancel failed during preempt', { err: String(err) });
        }
        if (active.watchdogTimer) clearTimeout(active.watchdogTimer);
        active = null;
      }

      // Claim the slot BEFORE any awaits so concurrent onStart calls preempt
      // this in-progress session rather than racing with it.
      active = {
        sessionId,
        startedAtMs: deps.now(),
        watchdogTimer: null,
        finalizedByWatchdog: false,
      };
      const claimed = active;

      await deps.playChime('open');
      await deps.showOverlay(true, sessionId);

      // If a concurrent onStart preempted us during the awaits, our claim was
      // replaced. Stop here — the new claim owns the slot.
      if (active !== claimed) {
        return;
      }

      try {
        await deps.audioCapture.start({ sessionTag: `ptt:${sessionId}` });
      } catch (err) {
        deps.logger.warn('[ptt] audio start failed', { sessionId, err: String(err) });
        if (active === claimed) {
          active = null;
        }
        await deps.playChime('error');
        await deps.showOverlay(false, sessionId);
        return;
      }

      // Re-check after the audio.start await.
      if (active !== claimed) {
        // Concurrent preempt replaced our claim mid-flight; we already started
        // audio for an orphan session. Best-effort cancel and exit — cancellation
        // failure here is non-actionable (the orphan session is already detached).
        try {
          await deps.audioCapture.cancel();
        } catch (_) {
          // ignore: orphan-session cleanup is best-effort
        }
        return;
      }

      active.watchdogTimer = armWatchdog(sessionId);
    },

    async onStop(sessionId) {
      if (!active || active.sessionId !== sessionId) {
        deps.logger.debug('[ptt] stale onStop — ignored', { sessionId });
        return;
      }
      await finaliseSession(sessionId, /* fromWatchdog */ false);
    },

    async cancel(reason) {
      if (!active) return;
      deps.logger.info('[ptt] cancel', { sessionId: active.sessionId, reason });
      if (active.watchdogTimer) clearTimeout(active.watchdogTimer);
      const session = active;
      active = null;
      try {
        await deps.audioCapture.cancel();
      } catch (err) {
        deps.logger.warn('[ptt] cancel: audio cancel failed', { err: String(err) });
      }
      await deps.playChime('error');
      await deps.showOverlay(false, session.sessionId);
    },
  };
}
