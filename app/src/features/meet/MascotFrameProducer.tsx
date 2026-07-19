import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { type FC, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useSelector } from 'react-redux';

import {
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
  selectSecondaryMascotId,
} from '../../store/mascotSlice';
import {
  getMascotPalette,
  hexToArgbInt,
  ManifestRiveMascot,
  type MascotFace,
  type MascotManifestEntry,
  RiveMascot,
} from '../human/Mascot';
import { findMascot } from '../human/Mascot/manifest/manifestService';
import { useMascotManifest } from '../human/Mascot/manifest/useMascotManifest';
import {
  drawMascotInCell,
  FRAME_H,
  FRAME_H_DUAL,
  FRAME_W,
  FRAME_W_DUAL,
  MASCOT_INSET,
  sampleCanvasPixels,
} from './mascotFrameCompositor';
import { type ActiveMascotSlot, type MeetingPhase, useMeetingMascots } from './useMeetingMascots';

const PRODUCER_FPS = 24;
const JPEG_QUALITY = 0.7;

/**
 * How long the mascot(s) wave hello on join before settling into the live
 * `active` face state (ms). Matches the greeting the participants see so the
 * first frames read as "the bot is saying hi" rather than a cold stare.
 */
const GREETING_MS = 2500;
/**
 * Teardown grace after `bus-stopped` (ms): keep the session mounted — WS +
 * worker alive — long enough for the goodbye wave to stream before the frame
 * pipeline is torn down. Without this the last thing the call sees is a hard
 * cut mid-pose instead of a wave.
 */
const SIGNOFF_MS = 1500;

interface BusSession {
  requestId: string;
  port: number;
}

// Re-export from the compositor for back-compat with existing importers
// (the producer test imports `sampleCanvasPixels` from here). The
// implementation moved to mascotFrameCompositor.ts (issue #4277).
export { sampleCanvasPixels };

export const MascotFrameProducer: FC = () => {
  const [session, setSession] = useState<BusSession | null>(null);
  // Meeting lifecycle phase, owned here so it survives across the brief
  // sign-off grace where `session` is still set but the bus has stopped.
  const [phase, setPhase] = useState<MeetingPhase>('greeting');
  // Set when `bus-stopped` fires; the session is cleared SIGNOFF_MS later so
  // the goodbye wave can stream. A ref so the timers don't re-arm on render.
  const signoffTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let unlistenStarted: UnlistenFn | undefined;
    let unlistenStopped: UnlistenFn | undefined;
    let cancelled = false;

    listen<BusSession>('meet-video:bus-started', event => {
      const payload = event.payload;
      if (!payload || !payload.port) return;
      console.log('[meet-video-producer] bus-started', payload);
      if (signoffTimerRef.current) {
        clearTimeout(signoffTimerRef.current);
        signoffTimerRef.current = null;
      }
      setPhase('greeting');
      setSession(payload);
    })
      .then(stop => {
        if (cancelled) stop();
        else unlistenStarted = stop;
      })
      .catch(() => {});

    listen<{ requestId?: string; request_id?: string }>('meet-video:bus-stopped', event => {
      console.log('[meet-video-producer] bus-stopped', event.payload);
      // Enter the goodbye wave and keep the pipeline alive for SIGNOFF_MS so
      // the wave frames actually reach the call before we clear the session.
      setPhase('signoff');
      if (signoffTimerRef.current) clearTimeout(signoffTimerRef.current);
      signoffTimerRef.current = setTimeout(() => {
        console.log('[meet-video-producer] sign-off grace elapsed, clearing session');
        signoffTimerRef.current = null;
        setSession(null);
      }, SIGNOFF_MS);
    })
      .then(stop => {
        if (cancelled) stop();
        else unlistenStopped = stop;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      if (unlistenStarted) unlistenStarted();
      if (unlistenStopped) unlistenStopped();
      if (signoffTimerRef.current) {
        clearTimeout(signoffTimerRef.current);
        signoffTimerRef.current = null;
      }
    };
  }, []);

  // Advance greeting → active after GREETING_MS, per active session. Bound to
  // requestId so a fresh session restarts the greeting.
  useEffect(() => {
    if (!session || phase !== 'greeting') return;
    const id = setTimeout(() => {
      console.log('[meet-video-producer] greeting elapsed → active', session.requestId);
      setPhase('active');
    }, GREETING_MS);
    return () => clearTimeout(id);
  }, [session, phase]);

  if (!session) return null;
  return <ProducerSession key={session.requestId} session={session} phase={phase} />;
};

const ProducerSession: FC<{ session: BusSession; phase: MeetingPhase }> = ({ session, phase }) => {
  const hostRef = useRef<HTMLDivElement>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const wsReadyRef = useRef(false);
  const stoppedRef = useRef(false);
  const inflightRef = useRef(false);
  const lastDiagAtRef = useRef(0);
  // True while the bot is actively producing PCM into the Meet call.
  // Drives the mascot face so the mouth animates in time with the audio
  // participants hear. Source of truth is the Rust speak_pump (edge-detected
  // from the RPC poll loop). Same requestId guards against stale events from
  // a previous session bleeding into this session's mascot state.
  const [isSpeaking, setIsSpeaking] = useState(false);
  // Which mascot slot is speaking the current audio (issue #4277). Single-
  // mascot calls always report slot 0. Read off the speaking-state event.
  const [activeMascotSlot, setActiveMascotSlot] = useState<ActiveMascotSlot>(0);

  // Per-slot render state (which mascot + which face) for this tick.
  const render = useMeetingMascots({ speaking: isSpeaking, activeMascotSlot, phase });
  const { dualEnabled } = render;

  // Resolve the manifest entries for both slots. The primary follows the
  // user's selection (same resolution the Human page uses); the secondary is
  // looked up by its explicit id so the frame honors both selections and no
  // longer always shows the bundled default (fixes the pre-existing single-
  // mascot bug where the meeting camera ignored `selectedMascotId`).
  const { manifest, entry: primaryEntry } = useMascotManifest();
  const secondaryMascotId = useSelector(selectSecondaryMascotId);
  const secondaryEntry: MascotManifestEntry | null =
    dualEnabled && manifest ? (findMascot(manifest, secondaryMascotId) ?? null) : null;

  // Mascot body colors, mirroring HumanPage so the meeting mascot matches the
  // one on the Human stage. Per-mascot colors are out of scope (#4277) — both
  // slots share the single selected color.
  const mascotColor = useSelector(selectMascotColor);
  const customPrimary = useSelector(selectCustomPrimaryColor);
  const customSecondary = useSelector(selectCustomSecondaryColor);
  const palette = getMascotPalette(mascotColor);
  const primaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customPrimary : palette.bodyFill),
    [mascotColor, customPrimary, palette]
  );
  const secondaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customSecondary : palette.neckShadowColor),
    [mascotColor, customSecondary, palette]
  );

  const isSpeakingRef = useRef(isSpeaking);
  useEffect(() => {
    isSpeakingRef.current = isSpeaking;
  }, [isSpeaking]);

  // `dualEnabled` and `activeMascotSlot` are read inside captureFrame via refs
  // so a speaker switch (activeMascotSlot 0↔1) or a dual-mode toggle does NOT
  // change captureFrame's identity — otherwise the WS/worker effect below
  // (which depends on captureFrame) would tear down and rebuild the socket +
  // frame worker on every alternation. captureFrame stays keyed to the session
  // only, matching the original single-mascot behavior.
  const dualEnabledRef = useRef(dualEnabled);
  useEffect(() => {
    dualEnabledRef.current = dualEnabled;
  }, [dualEnabled]);
  const activeMascotSlotRef = useRef(activeMascotSlot);
  useEffect(() => {
    activeMascotSlotRef.current = activeMascotSlot;
  }, [activeMascotSlot]);

  const captureFrame = useCallback(async () => {
    if (stoppedRef.current || !wsReadyRef.current || inflightRef.current) return;
    const host = hostRef.current;
    if (!host) return;
    const dualEnabledNow = dualEnabledRef.current;
    // Look the source canvases up by slot so a face change (which re-renders
    // the mascot) never changes which canvas we sample.
    const primaryCanvas = host.querySelector<HTMLCanvasElement>(
      '[data-mascot-slot="primary"] canvas'
    );
    if (!primaryCanvas) return;
    const secondaryCanvas = dualEnabledNow
      ? host.querySelector<HTMLCanvasElement>('[data-mascot-slot="secondary"] canvas')
      : null;

    const frameW = dualEnabledNow ? FRAME_W_DUAL : FRAME_W;
    // The dual frame is taller (16:9) than the single frame so the fake-camera
    // bridge's cover-scale fills the 1280×720 canvas without cropping — see
    // FRAME_H_DUAL in mascotFrameCompositor.ts.
    const frameH = dualEnabledNow ? FRAME_H_DUAL : FRAME_H;

    inflightRef.current = true;
    try {
      const offscreen = new OffscreenCanvas(frameW, frameH);
      const ctx = offscreen.getContext('2d');
      if (!ctx) return;

      const grad = ctx.createRadialGradient(
        frameW / 2,
        frameH / 2,
        0,
        frameW / 2,
        frameH / 2,
        Math.max(frameW, frameH) * 0.7
      );
      grad.addColorStop(0, '#FBF3D9');
      grad.addColorStop(1, '#EFE3B8');
      ctx.fillStyle = grad;
      ctx.fillRect(0, 0, frameW, frameH);

      if (dualEnabledNow && secondaryCanvas) {
        // Two half-cells: [0..half] primary, [half..frameW] secondary.
        const half = frameW / 2;
        drawMascotInCell(
          ctx,
          primaryCanvas,
          0,
          0,
          half,
          frameH,
          MASCOT_INSET,
          primaryCanvas.width,
          primaryCanvas.height
        );
        drawMascotInCell(
          ctx,
          secondaryCanvas,
          half,
          0,
          half,
          frameH,
          MASCOT_INSET,
          secondaryCanvas.width,
          secondaryCanvas.height
        );
      } else {
        // Single-cell draw. This also covers the dual-but-secondary-not-yet-
        // mounted tick (a 2.2MB mascot still decoding): rather than emit a
        // black half we draw the primary across the whole frame for this tick.
        drawMascotInCell(
          ctx,
          primaryCanvas,
          0,
          0,
          frameW,
          frameH,
          MASCOT_INSET,
          primaryCanvas.width,
          primaryCanvas.height
        );
      }

      const probe = sampleCanvasPixels(ctx, frameW, frameH);
      const blob = await offscreen.convertToBlob({ type: 'image/jpeg', quality: JPEG_QUALITY });
      const buffer = await blob.arrayBuffer();
      const ws = wsRef.current;
      if (ws && ws.readyState === WebSocket.OPEN) {
        const now = Date.now();
        if (now - lastDiagAtRef.current > 2000) {
          lastDiagAtRef.current = now;
          ws.send(
            JSON.stringify({
              kind: 'producer-pixel-probe',
              requestId: session.requestId,
              canvasWidth: primaryCanvas.width,
              canvasHeight: primaryCanvas.height,
              frameWidth: frameW,
              frameHeight: frameH,
              jpegBytes: blob.size,
              isSpeaking: isSpeakingRef.current,
              // Dual-mascot diagnostics (issue #4277): whether we drew two
              // cells this tick, and which slot the audio is on.
              dualEnabled: dualEnabledNow,
              secondaryMounted: dualEnabledNow ? Boolean(secondaryCanvas) : undefined,
              activeMascotSlot: activeMascotSlotRef.current,
              probe,
            })
          );
        }
        ws.send(buffer);
      }
    } catch (err) {
      console.warn('[meet-video-producer] capture failed', err);
    } finally {
      inflightRef.current = false;
    }
  }, [session.requestId]);

  useEffect(() => {
    stoppedRef.current = false;

    const SILENT_WAV =
      'data:audio/wav;base64,UklGRigAAABXQVZFZm10IBIAAAABAAEAQB8AAEAfAAABAAgAAABmYWN0BAAAAAAAAABkYXRhAAAAAA==';
    const keepAliveAudio = document.createElement('audio');
    keepAliveAudio.muted = true;
    keepAliveAudio.loop = true;
    keepAliveAudio.autoplay = true;
    keepAliveAudio.preload = 'auto';
    keepAliveAudio.src = SILENT_WAV;
    keepAliveAudio.style.display = 'none';
    document.body.appendChild(keepAliveAudio);
    void keepAliveAudio
      .play()
      .catch(err => console.warn('[meet-video-producer] silent audio play() failed', err));

    const url = `ws://127.0.0.1:${session.port}`;
    let ws: WebSocket;
    try {
      ws = new WebSocket(url);
    } catch (err) {
      console.warn('[meet-video-producer] ws ctor failed', err);
      return;
    }
    ws.binaryType = 'arraybuffer';
    wsRef.current = ws;
    ws.onopen = () => {
      wsReadyRef.current = true;
      console.log('[meet-video-producer] ws connected', url);
    };
    ws.onclose = () => {
      wsReadyRef.current = false;
      console.log('[meet-video-producer] ws closed');
    };
    ws.onerror = err => {
      console.warn('[meet-video-producer] ws error', err);
    };

    const intervalMs = Math.round(1000 / PRODUCER_FPS);
    const workerSrc =
      'let t=null;self.onmessage=(e)=>{const d=e.data||{};' +
      "if(d.cmd==='start'){clearInterval(t);t=setInterval(()=>self.postMessage('tick'),d.intervalMs);}" +
      "else if(d.cmd==='stop'){clearInterval(t);}};";
    const blob = new Blob([workerSrc], { type: 'application/javascript' });
    const workerUrl = URL.createObjectURL(blob);
    const worker = new Worker(workerUrl);

    worker.onmessage = () => {
      void captureFrame();
    };
    worker.postMessage({ cmd: 'start', intervalMs });

    // Subscribe to the speak_pump's speaking-state edge events so the
    // mascot face toggles in sync with the audio participants hear. Done
    // inside this effect so the listener lifetime is bound to the same
    // session — a remount tears it down with the rest of the pipeline.
    let unlistenSpeaking: UnlistenFn | undefined;
    let speakingListenerCancelled = false;
    listen<{ requestId?: string; speaking?: boolean; activeMascotSlot?: number }>(
      'meet-video:speaking-state',
      event => {
        const payload = event.payload;
        if (!payload) return;
        // Ignore events from a different session during teardown / restart.
        if (payload.requestId && payload.requestId !== session.requestId) return;
        setIsSpeaking(!!payload.speaking);
        // `activeMascotSlot` names which mascot is speaking this audio (0|1);
        // default to slot 0 for single-mascot / older core builds that omit it.
        setActiveMascotSlot(payload.activeMascotSlot === 1 ? 1 : 0);
      }
    )
      .then(stop => {
        if (speakingListenerCancelled) stop();
        else unlistenSpeaking = stop;
      })
      .catch(err => console.debug('[meet-video-producer] speaking-state listen failed', err));

    return () => {
      stoppedRef.current = true;
      speakingListenerCancelled = true;
      if (unlistenSpeaking) unlistenSpeaking();
      try {
        worker.postMessage({ cmd: 'stop' });
        worker.terminate();
      } catch (err) {
        console.debug('[meet-video-producer] worker stop failed', err);
      }
      URL.revokeObjectURL(workerUrl);
      try {
        ws.close();
      } catch (err) {
        console.debug('[meet-video-producer] ws close failed', err);
      }
      try {
        keepAliveAudio.pause();
        keepAliveAudio.remove();
      } catch (err) {
        console.debug('[meet-video-producer] audio teardown failed', err);
      }
      wsRef.current = null;
      wsReadyRef.current = false;
    };
  }, [session.port, session.requestId, captureFrame]);

  return (
    <div
      ref={hostRef}
      aria-hidden="true"
      style={{
        position: 'fixed',
        left: '-99999px',
        top: 0,
        width: dualEnabled ? FRAME_H * 2 : FRAME_H,
        height: FRAME_H,
        pointerEvents: 'none',
        opacity: 0,
      }}>
      {/* Slot 0 (primary). Stable key per mascot id so a face change updates in
          place instead of remounting the Rive/WebGL context. */}
      <div data-mascot-slot="primary" style={{ width: FRAME_H, height: FRAME_H }}>
        <MascotStage
          entry={primaryEntry}
          face={render.primary.face}
          primaryColor={primaryColor}
          secondaryColor={secondaryColor}
        />
      </div>
      {/* Slot 1 (secondary) — only mounted when a distinct second mascot is
          enabled. Its 2.2MB asset may still be decoding for the first frames;
          captureFrame falls back to a single-cell draw until its canvas exists. */}
      {dualEnabled && render.secondary && (
        <div data-mascot-slot="secondary" style={{ width: FRAME_H, height: FRAME_H }}>
          <MascotStage
            entry={secondaryEntry}
            face={render.secondary.face}
            primaryColor={primaryColor}
            secondaryColor={secondaryColor}
          />
        </div>
      )}
    </div>
  );
};

/**
 * Render one mascot slot. Prefers the manifest mascot (honoring the user's
 * selection) and falls back to the bundled `RiveMascot` while the manifest is
 * still resolving so the frame never blanks. Keyed by mascot id upstream so a
 * *selection* change remounts, while a *face* change updates in place.
 */
const MascotStage: FC<{
  entry: MascotManifestEntry | null;
  face: MascotFace;
  primaryColor: number;
  secondaryColor: number;
}> = ({ entry, face, primaryColor, secondaryColor }) => {
  if (entry) {
    return (
      <ManifestRiveMascot
        key={entry.id}
        entry={entry}
        face={face}
        size={FRAME_H}
        primaryColor={primaryColor}
        secondaryColor={secondaryColor}
      />
    );
  }
  return (
    <RiveMascot
      face={face}
      size={FRAME_H}
      primaryColor={primaryColor}
      secondaryColor={secondaryColor}
    />
  );
};

export default MascotFrameProducer;
