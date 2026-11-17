/**
 * Gapless playback of the live agent's speech: PCM16LE mono chunks scheduled
 * back to back on one AudioContext timeline.
 *
 * The output loudness for the mascot's lip-sync is taken from the chunks
 * themselves rather than an AnalyserNode: each scheduled chunk records its RMS
 * and its slot on the context clock, and `getAmplitude()` reports the chunk
 * covering `currentTime`. That is the audio actually being heard, needs no
 * extra graph node, and is deterministic under test.
 */
import createDebug from 'debug';

const log = createDebug('app:human:live-voice');

/** Lead time before the first chunk of a burst, so it is not scheduled in the past. */
const SCHEDULE_LEAD_S = 0.04;
/**
 * Gain from RMS to the 0..1 level the lip-sync thresholds expect. Speech RMS
 * sits around 0.05–0.3 of full scale, so a plain RMS would barely open the
 * mouth; the result is clamped to 1.
 */
const AMPLITUDE_GAIN = 3;

interface ScheduledChunk {
  source: AudioBufferSourceNode;
  start: number;
  end: number;
  level: number;
}

type AudioContextCtor = new (options?: AudioContextOptions) => AudioContext;

export interface PcmPlayerOptions {
  /** Output sample rate from the session's `ready` event. */
  sampleRate: number;
  /** Notified on the playing ⇄ idle edge (drives listening/speaking). */
  onPlayingChange?: (playing: boolean) => void;
  /** Injected for tests; defaults to `window.AudioContext`. */
  createContext?: () => AudioContext;
}

/** Decode PCM16LE bytes into float32 samples in [-1, 1). */
export function pcm16ToFloat32(bytes: ArrayBuffer): Float32Array {
  const view = new DataView(bytes);
  const count = Math.floor(bytes.byteLength / 2);
  const out = new Float32Array(count);
  for (let i = 0; i < count; i += 1) out[i] = view.getInt16(i * 2, true) / 0x8000;
  return out;
}

/** Root-mean-square of a sample block (0 for an empty block). */
export function rms(samples: Float32Array): number {
  if (samples.length === 0) return 0;
  let sum = 0;
  for (let i = 0; i < samples.length; i += 1) sum += samples[i] * samples[i];
  return Math.sqrt(sum / samples.length);
}

export class PcmPlayer {
  private readonly ctx: AudioContext;
  private readonly sampleRate: number;
  private readonly onPlayingChange?: (playing: boolean) => void;
  private chunks: ScheduledChunk[] = [];
  private nextStart = 0;
  private playing = false;
  private closed = false;

  constructor(opts: PcmPlayerOptions) {
    this.sampleRate = opts.sampleRate;
    this.onPlayingChange = opts.onPlayingChange;
    this.ctx = (opts.createContext ?? PcmPlayer.defaultContext)();
    if (this.ctx.state === 'suspended') void this.ctx.resume().catch(() => undefined);
  }

  private static defaultContext(): AudioContext {
    const w = window as unknown as {
      AudioContext?: AudioContextCtor;
      webkitAudioContext?: AudioContextCtor;
    };
    const Ctor = w.AudioContext ?? w.webkitAudioContext;
    if (!Ctor) throw new Error('AudioContext is not available');
    return new Ctor();
  }

  /** Queue one chunk of agent speech right after whatever is already queued. */
  enqueue(bytes: ArrayBuffer): void {
    if (this.closed) return;
    const samples = pcm16ToFloat32(bytes);
    if (samples.length === 0) return;

    const buffer = this.ctx.createBuffer(1, samples.length, this.sampleRate);
    buffer.getChannelData(0).set(samples);
    const source = this.ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(this.ctx.destination);

    const now = this.ctx.currentTime;
    const start = Math.max(this.nextStart, now + (this.chunks.length === 0 ? SCHEDULE_LEAD_S : 0));
    const end = start + samples.length / this.sampleRate;
    const chunk: ScheduledChunk = { source, start, end, level: rms(samples) };
    this.chunks.push(chunk);
    this.nextStart = end;

    source.onended = () => this.handleEnded(chunk);
    source.start(start);
    this.setPlaying(true);
  }

  /** Drop everything queued or playing — the agent was interrupted. */
  flush(): void {
    const dropped = this.chunks.length;
    const chunks = this.chunks;
    this.chunks = [];
    this.nextStart = 0;
    for (const chunk of chunks) {
      chunk.source.onended = null;
      try {
        chunk.source.stop();
      } catch {
        // Never started or already stopped.
      }
      try {
        chunk.source.disconnect();
      } catch {
        // Already disconnected.
      }
    }
    if (dropped > 0) log('[player] flushed chunks=%d', dropped);
    this.setPlaying(false);
  }

  /** Output loudness (0..1) of the audio audible right now. */
  getAmplitude(): number {
    if (this.closed || this.chunks.length === 0) return 0;
    const now = this.ctx.currentTime;
    for (const chunk of this.chunks) {
      if (now >= chunk.start && now < chunk.end) {
        return Math.min(1, chunk.level * AMPLITUDE_GAIN);
      }
    }
    return 0;
  }

  /** Whether any chunk is queued or still playing. */
  isPlaying(): boolean {
    return this.playing;
  }

  /** Stop playback and release the AudioContext. Idempotent. */
  close(): void {
    if (this.closed) return;
    this.flush();
    this.closed = true;
    void this.ctx.close().catch(() => undefined);
  }

  private handleEnded(chunk: ScheduledChunk): void {
    const index = this.chunks.indexOf(chunk);
    if (index >= 0) this.chunks.splice(index, 1);
    try {
      chunk.source.disconnect();
    } catch {
      // Already disconnected.
    }
    if (this.chunks.length === 0) {
      this.nextStart = 0;
      this.setPlaying(false);
    }
  }

  private setPlaying(next: boolean): void {
    if (this.playing === next) return;
    this.playing = next;
    this.onPlayingChange?.(next);
  }
}
