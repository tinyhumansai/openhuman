import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FakeAudioContext } from '../../../../test/liveVoiceFakes';
import {
  LIVE_VOICE_FRAME_SAMPLES,
  LIVE_VOICE_INPUT_SAMPLE_RATE,
  PCM_CAPTURE_PROCESSOR_NAME,
  PCM_CAPTURE_WORKLET_URL,
  startPcmCapture,
} from './pcmCaptureWorklet';

// ── the static processor asset ────────────────────────────────────────────────

function findAsset(): string {
  const rel = `public${PCM_CAPTURE_WORKLET_URL}`;
  let dir = process.cwd();
  for (;;) {
    for (const candidate of [resolve(dir, rel), resolve(dir, 'app', rel)]) {
      if (existsSync(candidate)) return candidate;
    }
    const parent = dirname(dir);
    if (parent === dir) throw new Error(`could not locate ${rel}`);
    dir = parent;
  }
}

type ProcessorInstance = {
  port: { postMessage: (data: ArrayBuffer) => void };
  process: (inputs: Float32Array[][]) => boolean;
};

/** Evaluate the processor source in a fake AudioWorkletGlobalScope. */
function loadProcessor(contextRate: number) {
  const source = readFileSync(findAsset(), 'utf8');
  const registered: Record<string, new (opts: unknown) => ProcessorInstance> = {};
  class AudioWorkletProcessor {
    port = { postMessage: vi.fn() };
  }
  // Evaluates the shipped asset in a fake AudioWorkletGlobalScope.
  new Function('AudioWorkletProcessor', 'registerProcessor', 'sampleRate', source)(
    AudioWorkletProcessor,
    (name: string, ctor: new (opts: unknown) => ProcessorInstance) => {
      registered[name] = ctor;
    },
    contextRate
  );
  return registered;
}

function frames(proc: ProcessorInstance): ArrayBuffer[] {
  return (proc.port.postMessage as ReturnType<typeof vi.fn>).mock.calls.map(
    c => c[0] as ArrayBuffer
  );
}

describe('pcm-capture-processor asset', () => {
  it('registers under the name the client uses', () => {
    expect(Object.keys(loadProcessor(48_000))).toEqual([PCM_CAPTURE_PROCESSOR_NAME]);
  });

  it('downsamples 48 kHz to 16 kHz PCM16 in 100 ms frames', () => {
    const Ctor = loadProcessor(48_000)[PCM_CAPTURE_PROCESSOR_NAME];
    const proc = new Ctor({ processorOptions: { targetSampleRate: 16_000, frameSamples: 1_600 } });
    // 100 ms of 48 kHz audio in 128-sample render quanta, constant 0.5.
    const quantum = new Float32Array(128).fill(0.5);
    for (let i = 0; i < Math.ceil(4_800 / 128); i += 1) {
      expect(proc.process([[quantum]])).toBe(true);
    }
    const out = frames(proc);
    expect(out).toHaveLength(1);
    expect(out[0].byteLength).toBe(3_200);
    const view = new DataView(out[0]);
    expect(view.getInt16(0, true)).toBe(Math.floor(0.5 * 0x7fff));
    expect(view.getInt16(3_198, true)).toBe(Math.floor(0.5 * 0x7fff));
  });

  it('clamps out-of-range samples and encodes negatives', () => {
    const Ctor = loadProcessor(16_000)[PCM_CAPTURE_PROCESSOR_NAME];
    const proc = new Ctor({ processorOptions: { frameSamples: 4 } });
    proc.process([[new Float32Array([2, -2, -0.5, 0])]]);
    const view = new DataView(frames(proc)[0]);
    expect(view.getInt16(0, true)).toBe(0x7fff);
    expect(view.getInt16(2, true)).toBe(-0x8000);
    expect(view.getInt16(4, true)).toBe(-0x4000);
    expect(view.getInt16(6, true)).toBe(0);
  });

  it('interpolates up when the context runs below the target rate', () => {
    const Ctor = loadProcessor(8_000)[PCM_CAPTURE_PROCESSOR_NAME];
    const proc = new Ctor({ processorOptions: { targetSampleRate: 16_000, frameSamples: 4 } });
    proc.process([[new Float32Array([0, 0.5, 0.5, 0.5])]]);
    const view = new DataView(frames(proc)[0]);
    expect(view.getInt16(2, true)).toBe(Math.floor(0.25 * 0x7fff));
  });

  it('keeps running with no input connected', () => {
    const Ctor = loadProcessor(48_000)[PCM_CAPTURE_PROCESSOR_NAME];
    const proc = new Ctor(undefined);
    expect(proc.process([])).toBe(true);
    expect(proc.process([[new Float32Array(0)]])).toBe(true);
  });
});

// ── startPcmCapture ───────────────────────────────────────────────────────────

class FakeWorkletNode {
  static last: FakeWorkletNode | null = null;
  port: { onmessage: ((e: MessageEvent) => void) | null } = { onmessage: null };
  connect = vi.fn();
  disconnect = vi.fn();
  constructor(
    public ctx: unknown,
    public name: string,
    public options: { processorOptions: Record<string, number> }
  ) {
    FakeWorkletNode.last = this;
  }
}

function fakeStream() {
  const track = { stop: vi.fn(), enabled: true };
  return { track, stream: { getTracks: () => [track], getAudioTracks: () => [track] } };
}

describe('startPcmCapture', () => {
  let getUserMedia: ReturnType<typeof vi.fn>;
  let mic: ReturnType<typeof fakeStream>;

  beforeEach(() => {
    mic = fakeStream();
    getUserMedia = vi.fn(async () => mic.stream);
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia },
    });
    vi.stubGlobal('AudioWorkletNode', FakeWorkletNode);
  });
  afterEach(() => vi.unstubAllGlobals());

  it('opens the mic with voice processing and posts worklet frames', async () => {
    const ctx = new FakeAudioContext();
    ctx.state = 'suspended';
    const onFrame = vi.fn();
    const capture = await startPcmCapture({
      onFrame,
      createContext: () => ctx as unknown as AudioContext,
    });

    expect(getUserMedia).toHaveBeenCalledWith({
      audio: expect.objectContaining({
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      }),
    });
    expect(ctx.audioWorklet.addModule).toHaveBeenCalledWith(PCM_CAPTURE_WORKLET_URL);
    expect(ctx.resume).toHaveBeenCalled();
    const node = FakeWorkletNode.last!;
    expect(node.name).toBe(PCM_CAPTURE_PROCESSOR_NAME);
    expect(node.options.processorOptions).toEqual({
      targetSampleRate: LIVE_VOICE_INPUT_SAMPLE_RATE,
      frameSamples: LIVE_VOICE_FRAME_SAMPLES,
    });

    const frame = new ArrayBuffer(3_200);
    node.port.onmessage?.({ data: frame } as MessageEvent);
    node.port.onmessage?.({ data: 'noise' } as MessageEvent);
    expect(onFrame).toHaveBeenCalledTimes(1);
    expect(onFrame).toHaveBeenCalledWith(frame);

    capture.setMuted(true);
    expect(mic.track.enabled).toBe(false);
    capture.setMuted(false);
    expect(mic.track.enabled).toBe(true);

    capture.stop();
    capture.stop();
    expect(mic.track.stop).toHaveBeenCalledTimes(1);
    expect(ctx.close).toHaveBeenCalledTimes(1);
    expect(node.port.onmessage).toBeNull();
  });

  it('releases the mic when the worklet fails to load', async () => {
    const ctx = new FakeAudioContext();
    ctx.audioWorklet.addModule.mockRejectedValueOnce(new Error('blocked'));
    await expect(
      startPcmCapture({ onFrame: vi.fn(), createContext: () => ctx as unknown as AudioContext })
    ).rejects.toThrow('blocked');
    expect(mic.track.stop).toHaveBeenCalled();
    expect(ctx.close).toHaveBeenCalled();
  });

  it('propagates a denied microphone', async () => {
    getUserMedia.mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'));
    await expect(startPcmCapture({ onFrame: vi.fn() })).rejects.toThrow('denied');
  });

  it('rejects where getUserMedia does not exist', async () => {
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: undefined });
    await expect(startPcmCapture({ onFrame: vi.fn() })).rejects.toThrow(/not supported/);
  });

  it('fails cleanly without an AudioContext implementation', async () => {
    vi.stubGlobal('AudioContext', undefined);
    await expect(startPcmCapture({ onFrame: vi.fn() })).rejects.toThrow(/AudioContext/);
    expect(mic.track.stop).toHaveBeenCalled();
  });
});
