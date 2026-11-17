/**
 * Microphone capture for the live voice agent: mic → AudioWorklet → PCM16LE
 * mono at 16 kHz, delivered as ~100 ms frames (3200 bytes).
 *
 * The processor itself is a static asset (`app/public/live-voice/
 * pcm-capture-processor.js`). It cannot be a Blob URL — the desktop CSP's
 * `script-src` has no `blob:` and worklet modules are governed by it — and it
 * cannot be a dynamic import (not allowed in `app/src`). A same-origin public
 * file satisfies `script-src 'self'` in every build.
 */
import createDebug from 'debug';

const log = createDebug('app:human:live-voice');

/** Where the processor module is served from (Vite `publicDir`). */
export const PCM_CAPTURE_WORKLET_URL = '/live-voice/pcm-capture-processor.js';
/** Name the processor registers itself under. Must match the asset. */
export const PCM_CAPTURE_PROCESSOR_NAME = 'openhuman-pcm-capture';
/** Uplink sample rate the core expects (`input_sample_rate` in `start`). */
export const LIVE_VOICE_INPUT_SAMPLE_RATE = 16_000;
/** Samples per uplink frame: 100 ms at 16 kHz. */
export const LIVE_VOICE_FRAME_SAMPLES = 1_600;

export interface PcmCapture {
  /** Stop the mic tracks, disconnect the graph and close the context. */
  stop: () => void;
  /** Silence the uplink without releasing the mic (track `enabled = false`). */
  setMuted: (muted: boolean) => void;
}

export interface StartPcmCaptureOptions {
  /** One PCM16LE frame (`LIVE_VOICE_FRAME_SAMPLES` samples). */
  onFrame: (frame: ArrayBuffer) => void;
  /** Injected for tests; defaults to `window.AudioContext`. */
  createContext?: () => AudioContext;
}

type AudioContextCtor = new (options?: AudioContextOptions) => AudioContext;

function defaultCreateContext(): AudioContext {
  const w = window as unknown as {
    AudioContext?: AudioContextCtor;
    webkitAudioContext?: AudioContextCtor;
  };
  const Ctor = w.AudioContext ?? w.webkitAudioContext;
  if (!Ctor) throw new Error('AudioContext is not available');
  return new Ctor();
}

/**
 * Open the microphone and start posting PCM16 frames.
 *
 * Rejects (after releasing anything it acquired) when the mic is denied or
 * missing, or the worklet cannot load — the caller maps that to a session
 * error rather than a half-open session.
 */
export async function startPcmCapture(opts: StartPcmCaptureOptions): Promise<PcmCapture> {
  if (!navigator.mediaDevices?.getUserMedia) {
    throw new Error('Microphone capture is not supported here');
  }
  log('[capture] requesting microphone');
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      channelCount: 1,
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
    },
  });

  let ctx: AudioContext | null = null;
  try {
    ctx = (opts.createContext ?? defaultCreateContext)();
    await ctx.audioWorklet.addModule(PCM_CAPTURE_WORKLET_URL);
    const source = ctx.createMediaStreamSource(stream);
    const node = new AudioWorkletNode(ctx, PCM_CAPTURE_PROCESSOR_NAME, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      channelCount: 1,
      processorOptions: {
        targetSampleRate: LIVE_VOICE_INPUT_SAMPLE_RATE,
        frameSamples: LIVE_VOICE_FRAME_SAMPLES,
      },
    });
    node.port.onmessage = (event: MessageEvent) => {
      if (event.data instanceof ArrayBuffer) opts.onFrame(event.data);
    };
    source.connect(node);
    // A worklet only runs while it is pulled by the destination. Its output is
    // silence (the processor writes nothing to `outputs`), so this is inaudible.
    node.connect(ctx.destination);
    if (ctx.state === 'suspended') await ctx.resume().catch(() => undefined);
    log('[capture] started context_rate=%d', ctx.sampleRate);

    const liveCtx = ctx;
    let stopped = false;
    return {
      stop: () => {
        if (stopped) return;
        stopped = true;
        node.port.onmessage = null;
        try {
          source.disconnect();
          node.disconnect();
        } catch {
          // Already disconnected — nothing to release.
        }
        for (const track of stream.getTracks()) track.stop();
        void liveCtx.close().catch(() => undefined);
        log('[capture] stopped');
      },
      setMuted: (muted: boolean) => {
        for (const track of stream.getAudioTracks()) track.enabled = !muted;
        log('[capture] muted=%s', muted);
      },
    };
  } catch (err) {
    log('[capture] failed to start the worklet graph');
    for (const track of stream.getTracks()) track.stop();
    if (ctx) void ctx.close().catch(() => undefined);
    throw err;
  }
}
