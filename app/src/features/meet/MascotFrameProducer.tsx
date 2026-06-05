import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { type FC, useCallback, useEffect, useRef, useState } from 'react';

import { RiveMascot } from '../human/Mascot';

const PRODUCER_FPS = 24;
const FRAME_W = 320;
const FRAME_H = 240;
const JPEG_QUALITY = 0.7;

interface BusSession {
  requestId: string;
  port: number;
}

export function sampleCanvasPixels(
  ctx: OffscreenCanvasRenderingContext2D,
  width: number,
  height: number
) {
  const cols = 7;
  const rows = 5;
  let sum = 0;
  let min = 255;
  let max = 0;
  let count = 0;
  let dark = 0;
  let bright = 0;

  try {
    for (let y = 0; y < rows; y++) {
      for (let x = 0; x < cols; x++) {
        const px = Math.max(0, Math.min(width - 1, Math.floor(((x + 0.5) * width) / cols)));
        const py = Math.max(0, Math.min(height - 1, Math.floor(((y + 0.5) * height) / rows)));
        const [r, g, b] = ctx.getImageData(px, py, 1, 1).data;
        const luma = Math.round(r * 0.299 + g * 0.587 + b * 0.114);
        sum += luma;
        min = Math.min(min, luma);
        max = Math.max(max, luma);
        if (luma < 8) dark++;
        if (luma > 32) bright++;
        count++;
      }
    }
    return {
      avgLuma: Math.round(sum / Math.max(1, count)),
      minLuma: min,
      maxLuma: max,
      darkSamples: dark,
      brightSamples: bright,
      sampleCount: count,
    };
  } catch (err) {
    return { error: String(err instanceof Error ? err.message : err) };
  }
}

export const MascotFrameProducer: FC = () => {
  const [session, setSession] = useState<BusSession | null>(null);

  useEffect(() => {
    let unlistenStarted: UnlistenFn | undefined;
    let unlistenStopped: UnlistenFn | undefined;
    let cancelled = false;

    listen<BusSession>('meet-video:bus-started', event => {
      const payload = event.payload;
      if (!payload || !payload.port) return;
      console.log('[meet-video-producer] bus-started', payload);
      setSession(payload);
    })
      .then(stop => {
        if (cancelled) stop();
        else unlistenStarted = stop;
      })
      .catch(() => {});

    listen<{ requestId?: string; request_id?: string }>('meet-video:bus-stopped', event => {
      console.log('[meet-video-producer] bus-stopped', event.payload);
      setSession(null);
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
    };
  }, []);

  if (!session) return null;
  return <ProducerSession key={session.requestId} session={session} />;
};

const ProducerSession: FC<{ session: BusSession }> = ({ session }) => {
  const hostRef = useRef<HTMLDivElement>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const wsReadyRef = useRef(false);
  const stoppedRef = useRef(false);
  const inflightRef = useRef(false);
  const lastDiagAtRef = useRef(0);
  const isSpeakingRef = useRef(false);
  // True while the bot is actively producing PCM into the Meet call.
  // Drives the mascot face so the mouth animates in time with the audio
  // participants hear. Source of truth is the Rust speak_pump (edge-detected
  // from the RPC poll loop). Same requestId guards against stale events from
  // a previous session bleeding into this session's mascot state.
  const [isSpeaking, setIsSpeaking] = useState(false);

  useEffect(() => {
    isSpeakingRef.current = isSpeaking;
  }, [isSpeaking]);

  const captureFrame = useCallback(async () => {
    if (stoppedRef.current || !wsReadyRef.current || inflightRef.current) return;
    const host = hostRef.current;
    if (!host) return;
    const canvas = host.querySelector('canvas');
    if (!canvas) return;

    inflightRef.current = true;
    try {
      const offscreen = new OffscreenCanvas(FRAME_W, FRAME_H);
      const ctx = offscreen.getContext('2d');
      if (!ctx) return;

      const grad = ctx.createRadialGradient(
        FRAME_W / 2,
        FRAME_H / 2,
        0,
        FRAME_W / 2,
        FRAME_H / 2,
        Math.max(FRAME_W, FRAME_H) * 0.7
      );
      grad.addColorStop(0, '#FBF3D9');
      grad.addColorStop(1, '#EFE3B8');
      ctx.fillStyle = grad;
      ctx.fillRect(0, 0, FRAME_W, FRAME_H);

      const inset = 0.06;
      const fitW = FRAME_W * (1 - 2 * inset);
      const fitH = FRAME_H * (1 - 2 * inset);
      const scale = Math.min(fitW / canvas.width, fitH / canvas.height);
      const dw = canvas.width * scale;
      const dh = canvas.height * scale;
      const dx = (FRAME_W - dw) / 2;
      const dy = (FRAME_H - dh) / 2;
      ctx.drawImage(canvas, dx, dy, dw, dh);

      const probe = sampleCanvasPixels(ctx, FRAME_W, FRAME_H);
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
              canvasWidth: canvas.width,
              canvasHeight: canvas.height,
              frameWidth: FRAME_W,
              frameHeight: FRAME_H,
              jpegBytes: blob.size,
              isSpeaking: isSpeakingRef.current,
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
    listen<{ requestId?: string; speaking?: boolean }>('meet-video:speaking-state', event => {
      const payload = event.payload;
      if (!payload) return;
      // Ignore events from a different session during teardown / restart.
      if (payload.requestId && payload.requestId !== session.requestId) return;
      setIsSpeaking(!!payload.speaking);
    })
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
        width: FRAME_H,
        height: FRAME_H,
        pointerEvents: 'none',
        opacity: 0,
      }}>
      <RiveMascot face={isSpeaking ? 'speaking' : 'idle'} size={FRAME_H} />
    </div>
  );
};

export default MascotFrameProducer;
