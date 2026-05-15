import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { type FC, useEffect, useMemo, useRef, useState } from 'react';

import { useAppSelector } from '../../store/hooks';
import { selectMascotColor } from '../../store/mascotSlice';
import {
  type FrameConfig,
  FrameConfigContext,
  FrameContext,
} from '../human/Mascot/yellow/frameContext';
import { YellowMascotIdle } from '../human/Mascot/yellow/MascotIdle';

/**
 * Meet camera frame producer.
 *
 * Mounted once at app root. Listens for the shell-emitted
 * `meet-video:bus-started` / `meet-video:bus-stopped` events and, while
 * a session is active, renders a hidden Remotion-driven mascot,
 * rasterizes its SVG to a 640×480 JPEG every frame, and pushes the
 * bytes over a loopback WebSocket to the Rust frame bus
 * (`app/src-tauri/src/meet_video/frame_bus.rs`). The Rust side fans
 * each frame out to the consumer — the camera bridge inside the Meet
 * CEF webview, which paints them onto its capture canvas
 * (`canvas.captureStream(30)` → `getUserMedia` intercept).
 *
 * ## Why the mascot lives here, not in the Meet webview
 *
 * `CLAUDE.md` rules out growing JS injection into CEF child webviews.
 * The Remotion runtime + composition tree is too large to inject and
 * would run inside a third-party origin sandbox; that's a non-starter.
 * Instead the rich animation lives in our own renderer (where Remotion
 * is already a project dependency) and we ship its pixels — not its
 * code — to the Meet origin.
 *
 * ## Why XMLSerializer instead of `@remotion/player`
 *
 * Remotion's `<Player>` historically failed to start cold inside CEF
 * (see `app/src/features/human/Mascot/yellow/frameContext.tsx`); the
 * project replaced it with a local `FrameProvider` that drives ticks
 * via `requestAnimationFrame`. The compositions render to live SVG,
 * which we rasterize per frame: serialize → data URI → `<img>` decode
 * → drawImage → JPEG blob.
 */

const PRODUCER_FPS = 24; // 24 fps is plenty for "lifelike" and gives
// per-frame serialize+encode budget headroom — at 30 fps the SVG decode
// occasionally backs up on slower machines and frames pile up. The
// bridge consumer redraws its canvas at 30 fps regardless, repeating
// our latest frame between producer ticks.

// Producer renders at a *lower* resolution than the bridge canvas
// (640×480) to keep SVG rasterization cheap. The bridge cover-fits
// our 320×240 output up to 640×480, which is fine — the YellowMascot
// SVG is vector and the user is watching a small video tile in Meet
// that goes through Meet's own encoder, so source resolution is
// invisible past ~360p anyway.
//
// Empirically (instrumented in the producer diag JSON): rendering at
// 640×480 took ~1000 ms/frame on this hardware (img.decode of the
// rich SVG dominates), pinning the producer to 1 fps. Halving each
// dimension is a 4× rasterize speedup.
const FRAME_W = 320;
const FRAME_H = 240;
const JPEG_QUALITY = 0.7;

// Mascot inner-canvas dimensions. Mirrors the values YellowMascot
// passes to FrameProvider — keep in sync if those change.
const MASCOT_CANVAS = 1000;
const MASCOT_LOOP_FRAMES = PRODUCER_FPS * 6;

interface BusSession {
  requestId: string;
  port: number;
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
  const mascotColor = useAppSelector(selectMascotColor);
  const wsRef = useRef<WebSocket | null>(null);
  const wsReadyRef = useRef(false);
  const stoppedRef = useRef(false);
  const inflightRef = useRef(false);
  const sentFramesRef = useRef(0);
  // Frame counter feeding our own FrameContext below. We DON'T use the
  // shared `<FrameProvider>` wrapper because it ticks via
  // requestAnimationFrame, which Chromium throttles when the main
  // openhuman window is backgrounded behind the Meet window — the
  // mascot would freeze the moment the user clicks into Meet. The
  // worker tick below advances this state from `Date.now()` instead,
  // which keeps running regardless of focus.
  const [frame, setFrame] = useState(0);
  const startTimeRef = useRef<number | null>(null);
  const frameAtTickRef = useRef(0);

  useEffect(() => {
    stoppedRef.current = false;

    // ── Background-throttle defeater: muted autoplaying <audio> ─────
    // Chromium throttles main-thread setInterval *and* worker timers
    // when the page is backgrounded / not the key window. A page
    // that's "playing audio" (incl. silent muted audio) is exempt.
    //
    // We tried `AudioContext` first; that fails because Chromium's
    // autoplay policy starts the context in `suspended` state and
    // `resume()` only succeeds inside a user-gesture handler — which
    // never happens for the auto-launched dev meet call. Symptom:
    // pipeline ran at 24fps for ~20s, then collapsed to 1fps as soon
    // as the renderer's "playing audio" grace period expired.
    //
    // `<audio muted>` is exempt from the autoplay policy and *does*
    // start playing without a gesture, putting the page in the
    // "playing media" state Chromium uses to gate background
    // throttling. The base64'd silent WAV is ~70 bytes; loop=true
    // keeps it perpetually "playing" without ever needing a fetch.
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
    // Trigger play() explicitly — autoplay attribute alone is racy in
    // some Chromium builds; play() returns a promise that resolves
    // once the media is actually playing.
    void keepAliveAudio
      .play()
      .catch(err => console.warn('[meet-video-producer] silent audio play() failed', err));

    // ── WS connect ─────────────────────────────────────────────────────
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

    // ── Per-frame rasterize + push loop ───────────────────────────────
    // Reused across ticks. The OffscreenCanvas keeps the JPEG encode off
    // the main DOM canvas pipeline.
    const offscreen =
      typeof OffscreenCanvas !== 'undefined'
        ? new OffscreenCanvas(FRAME_W, FRAME_H)
        : (() => {
            const c = document.createElement('canvas');
            c.width = FRAME_W;
            c.height = FRAME_H;
            return c as unknown as OffscreenCanvas;
          })();
    const ctx = (offscreen as unknown as OffscreenCanvas).getContext(
      '2d'
    ) as OffscreenCanvasRenderingContext2D | null;
    if (!ctx) {
      console.warn('[meet-video-producer] no 2d ctx — aborting');
      return;
    }
    const serializer = typeof XMLSerializer !== 'undefined' ? new XMLSerializer() : null;

    const intervalMs = Math.round(1000 / PRODUCER_FPS);

    // Heartbeat from a Web Worker, NOT main-thread setInterval.
    // Background-throttling: when the meet window has focus, the main
    // openhuman window is no longer foreground, and Chromium throttles
    // main-thread setInterval to ~1Hz. Worker timers run in a separate
    // event loop and are throttled much less aggressively, which keeps
    // the producer hitting its target rate while the user is looking
    // at Meet. Inlined as a Blob URL so we don't need a separate
    // worker file in the bundler graph.
    const workerSrc =
      'let t=null;self.onmessage=(e)=>{const d=e.data||{};' +
      "if(d.cmd==='start'){clearInterval(t);t=setInterval(()=>self.postMessage('tick'),d.intervalMs);}" +
      "else if(d.cmd==='stop'){clearInterval(t);}};";
    const blob = new Blob([workerSrc], { type: 'application/javascript' });
    const workerUrl = URL.createObjectURL(blob);
    const worker = new Worker(workerUrl);

    // Diagnostic counters. Every 2s we post a JSON snapshot through
    // the WS as a text frame; the Rust side logs it as
    // `[meet-video-producer-diag]` so we can compare:
    //   - worker_ticks: how often the worker actually fires (should
    //     be ~PRODUCER_FPS regardless of focus)
    //   - encode_started / encode_completed: how many encodes ran;
    //     gap → encode is the bottleneck, not timer throttling
    //   - encode_avg_ms: per-frame encode cost
    //   - inflight_skips: how many ticks were dropped because a
    //     prior encode was still running
    let workerTicks = 0;
    let encodeStarted = 0;
    let encodeCompleted = 0;
    let encodeMsTotal = 0;
    let inflightSkips = 0;
    const diagInterval = window.setInterval(() => {
      try {
        const ws = wsRef.current;
        if (!ws || ws.readyState !== WebSocket.OPEN) return;
        const payload = JSON.stringify({
          source: 'producer',
          worker_ticks: workerTicks,
          encode_started: encodeStarted,
          encode_completed: encodeCompleted,
          encode_avg_ms: encodeCompleted > 0 ? Math.round(encodeMsTotal / encodeCompleted) : 0,
          inflight_skips: inflightSkips,
          ws_state: ws.readyState,
          frame: frameAtTickRef.current,
        });
        ws.send(payload);
      } catch (_) {
        // diagnostics best-effort; swallow to avoid breaking the worker tick.
      }
      workerTicks = 0;
      encodeStarted = 0;
      encodeCompleted = 0;
      encodeMsTotal = 0;
      inflightSkips = 0;
    }, 2000);

    const onTick = () => {
      workerTicks++;
      // Always advance the React frame so the mascot keeps animating
      // even before the WS is ready and even when the main window is
      // backgrounded. Computed from Date.now() so we're robust to the
      // worker setInterval drifting under throttling.
      if (startTimeRef.current === null) startTimeRef.current = Date.now();
      const elapsedMs = Date.now() - startTimeRef.current;
      const nextFrame = Math.floor((elapsedMs / 1000) * PRODUCER_FPS) % MASCOT_LOOP_FRAMES;
      setFrame(prev => (prev === nextFrame ? prev : nextFrame));
      frameAtTickRef.current = nextFrame;

      if (stoppedRef.current || !wsReadyRef.current) return;
      // Drop frames if a previous encode is still inflight rather than
      // letting them queue up unbounded.
      if (inflightRef.current) {
        inflightSkips++;
        return;
      }
      const host = hostRef.current;
      if (!host || !serializer) return;
      const svg = host.querySelector('svg');
      if (!svg) return;
      inflightRef.current = true;
      encodeStarted++;
      const startedAt = window.performance.now();
      void encodeAndSend(svg, serializer, ctx, ws)
        .then(ok => {
          if (ok) {
            sentFramesRef.current++;
            encodeCompleted++;
            encodeMsTotal += window.performance.now() - startedAt;
          }
        })
        .finally(() => {
          inflightRef.current = false;
        });
    };
    worker.onmessage = onTick;
    worker.postMessage({ cmd: 'start', intervalMs });

    return () => {
      stoppedRef.current = true;
      window.clearInterval(diagInterval);
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
  }, [session.port]);

  const frameConfig = useMemo<FrameConfig>(
    () => ({
      fps: PRODUCER_FPS,
      width: MASCOT_CANVAS,
      height: MASCOT_CANVAS,
      durationInFrames: MASCOT_LOOP_FRAMES,
    }),
    []
  );

  // The mascot host lives off-screen but in the layout tree so the SVG
  // gets laid out + animated normally. Fixed pixel size so the SVG
  // serialization renders at a predictable resolution.
  //
  // We bypass the shared `<YellowMascot>` wrapper because it
  // re-establishes its own rAF-based FrameProvider — which freezes
  // when the main window is backgrounded (see comment on the `frame`
  // state above). Rendering `YellowMascotIdle` directly inside our own
  // worker-driven contexts keeps the animation alive.
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
      <style>{`.mascot-producer-host svg { width: 100% !important; height: 100% !important; }`}</style>
      <div className="mascot-producer-host" style={{ width: '100%', height: '100%' }}>
        <FrameConfigContext.Provider value={frameConfig}>
          <FrameContext.Provider value={frame}>
            <YellowMascotIdle
              face="normal"
              recordingColor="#ff3b30"
              loadingColor="#ffffff"
              greeting={false}
              sleeping={false}
              mascotColor={mascotColor}
              arm="wave"
              talking={false}
              thinking={false}
            />
          </FrameContext.Provider>
        </FrameConfigContext.Provider>
      </div>
    </div>
  );
};

async function encodeAndSend(
  svg: SVGElement,
  serializer: XMLSerializer,
  ctx: OffscreenCanvasRenderingContext2D,
  ws: WebSocket
): Promise<boolean> {
  try {
    // Make sure the SVG carries width/height/xmlns so the standalone
    // data URI parses on its own (it's pulled out of the React tree).
    const clone = svg.cloneNode(true) as SVGElement;
    if (!clone.hasAttribute('xmlns')) {
      clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg');
    }
    // Force the SVG to render at our target resolution so the
    // rasterizer doesn't waste work painting a 1000×1000 surface
    // we'd downscale anyway.
    clone.setAttribute('width', `${FRAME_H}`);
    clone.setAttribute('height', `${FRAME_H}`);
    const xml = serializer.serializeToString(clone);

    // `createImageBitmap(Blob)` is significantly faster than
    // `<img>.decode()` in Chromium: it dispatches to the rasterizer
    // worker pool and skips the data-URI percent-encode roundtrip
    // (a 30–50 KB SVG was getting URL-escaped → main-thread parsed
    // every frame, which dominated the per-frame budget).
    const svgBlob = new Blob([xml], { type: 'image/svg+xml' });
    let bitmap: ImageBitmap;
    try {
      bitmap = await window.createImageBitmap(svgBlob);
    } catch (_err) {
      // Some Chromium builds reject SVG blobs in createImageBitmap;
      // fall back to the <img> decode path.
      const url = URL.createObjectURL(svgBlob);
      try {
        const img = new window.Image();
        img.decoding = 'async';
        img.src = url;
        await img.decode();
        ctx.fillStyle = '#F7F4EE';
        ctx.fillRect(0, 0, FRAME_W, FRAME_H);
        ctx.drawImage(img, 0, 0, FRAME_W, FRAME_H);
        // (skip the rest of the gradient/inset path on the fallback
        // — it's only used when createImageBitmap fails, which is
        // rare; the encode block below handles JPEG conversion.)
      } finally {
        URL.revokeObjectURL(url);
      }
      // Do the JPEG encode + send and return early.
      const oc = ctx.canvas as OffscreenCanvas;
      const blob =
        'convertToBlob' in oc
          ? await oc.convertToBlob({ type: 'image/jpeg', quality: JPEG_QUALITY })
          : await new Promise<Blob>((resolve, reject) => {
              (ctx.canvas as unknown as HTMLCanvasElement).toBlob(
                b => (b ? resolve(b) : reject(new Error('toBlob null'))),
                'image/jpeg',
                JPEG_QUALITY
              );
            });
      const buffer = await blob.arrayBuffer();
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(buffer);
        return true;
      }
      return false;
    }

    // Subtle off-yellow radial gradient — warmer center, slightly
    // darker edges. Premium-feeling backdrop without being noisy.
    const grad = ctx.createRadialGradient(
      FRAME_W / 2,
      FRAME_H / 2,
      0,
      FRAME_W / 2,
      FRAME_H / 2,
      Math.max(FRAME_W, FRAME_H) * 0.7
    );
    grad.addColorStop(0, '#FBF3D9'); // warm cream highlight
    grad.addColorStop(1, '#EFE3B8'); // soft butter edge
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, FRAME_W, FRAME_H);

    // Contain-fit (with a small inset) so the *whole* mascot lands in
    // the frame.
    const inset = 0.06; // 6% breathing room on the short axis
    const fitW = FRAME_W * (1 - 2 * inset);
    const fitH = FRAME_H * (1 - 2 * inset);
    const scale = Math.min(fitW / bitmap.width, fitH / bitmap.height);
    const dw = bitmap.width * scale;
    const dh = bitmap.height * scale;
    const dx = (FRAME_W - dw) / 2;
    const dy = (FRAME_H - dh) / 2;
    ctx.drawImage(bitmap, dx, dy, dw, dh);
    bitmap.close();

    const oc = ctx.canvas as OffscreenCanvas;
    const blob =
      'convertToBlob' in oc
        ? await oc.convertToBlob({ type: 'image/jpeg', quality: JPEG_QUALITY })
        : await new Promise<Blob>((resolve, reject) => {
            (ctx.canvas as unknown as HTMLCanvasElement).toBlob(
              b => (b ? resolve(b) : reject(new Error('toBlob null'))),
              'image/jpeg',
              JPEG_QUALITY
            );
          });
    const buffer = await blob.arrayBuffer();
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(buffer);
      return true;
    }
    return false;
  } catch (err) {
    console.warn('[meet-video-producer] encode/send failed', err);
    return false;
  }
}

export default MascotFrameProducer;
