import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { encodeBlobToWav } from './wavEncoder';

// jsdom doesn't ship Web Audio. We only need a thin stub that lets the
// encoder reach the WAV header + sample copy paths — the actual decode +
// resample is tested as a black box (input bytes → WAV bytes round-trip).

interface FakeAudioBuffer {
  sampleRate: number;
  length: number;
  numberOfChannels: number;
  getChannelData(c: number): Float32Array;
}

function createFakeBuffer(sampleRate: number, channels: Float32Array[]): FakeAudioBuffer {
  return {
    sampleRate,
    length: channels[0].length,
    numberOfChannels: channels.length,
    getChannelData: (c: number) => channels[c],
  };
}

describe('encodeBlobToWav', () => {
  let decodedBuffer: FakeAudioBuffer;

  beforeEach(() => {
    // Default: stereo at 48kHz so we exercise both the resample-via-render
    // path and the mono mixdown.
    decodedBuffer = createFakeBuffer(48_000, [
      new Float32Array([0, 0.5, -0.5, 1, -1]),
      new Float32Array([0, 0.5, -0.5, 1, -1]),
    ]);

    class FakeOfflineAudioContext {
      constructor(
        public numberOfChannels: number,
        public length: number,
        public sampleRate: number
      ) {}
      decodeAudioData = vi.fn(async () => decodedBuffer as unknown as AudioBuffer);
      createBufferSource() {
        return { buffer: null as AudioBuffer | null, connect: vi.fn(), start: vi.fn() };
      }
      destination = {} as AudioNode;
      startRendering = vi.fn(async () => {
        // Resampled buffer at the constructor's target sample rate (16kHz),
        // mono. Use a tiny known signal so we can assert the WAV bytes.
        return createFakeBuffer(this.sampleRate, [
          new Float32Array([0, 0.5, -0.5]),
        ]) as unknown as AudioBuffer;
      });
    }

    vi.stubGlobal('OfflineAudioContext', FakeOfflineAudioContext);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('rejects empty blobs', async () => {
    const blob = new Blob([], { type: 'audio/webm' });
    await expect(encodeBlobToWav(blob)).rejects.toThrow(/empty/);
  });

  it('produces a 16kHz mono WAV blob with a valid RIFF/WAVE header', async () => {
    const input = new Blob([new Uint8Array([1, 2, 3, 4])], { type: 'audio/webm' });
    const out = await encodeBlobToWav(input);

    expect(out.type).toBe('audio/wav');
    const buf = await out.arrayBuffer();
    const view = new DataView(buf);
    const decoder = new TextDecoder('ascii');
    expect(decoder.decode(buf.slice(0, 4))).toBe('RIFF');
    expect(decoder.decode(buf.slice(8, 12))).toBe('WAVE');
    expect(decoder.decode(buf.slice(12, 16))).toBe('fmt ');
    // PCM format = 1, mono = 1 channel, 16kHz, 16-bit
    expect(view.getUint16(20, true)).toBe(1);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(24, true)).toBe(16_000);
    expect(view.getUint16(34, true)).toBe(16);
    expect(decoder.decode(buf.slice(36, 40))).toBe('data');
  });

  it('skips the resample render pass when the source is already at 16kHz', async () => {
    decodedBuffer = createFakeBuffer(16_000, [new Float32Array([0, 0.25, -0.25])]);
    const input = new Blob([new Uint8Array([9])], { type: 'audio/wav' });
    const out = await encodeBlobToWav(input);
    // 3 samples × 2 bytes/sample + 44-byte header
    expect(out.size).toBe(44 + 6);
    const view = new DataView(await out.arrayBuffer());
    // Sample at offset 44 (first sample) should be 0
    expect(view.getInt16(44, true)).toBe(0);
    // setInt16 truncates toward zero rather than rounding, so 0.25 * 0x7fff
    // (= 8191.75) lands at 8191 in the file. Pin the truncation behavior
    // explicitly so a future "let's round" change has to flag this.
    expect(view.getInt16(46, true)).toBe(Math.trunc(0.25 * 0x7fff));
    expect(view.getInt16(48, true)).toBe(Math.trunc(-0.25 * 0x8000));
  });

  it('clamps samples that drift outside [-1, 1] from accumulator rounding', async () => {
    decodedBuffer = createFakeBuffer(16_000, [new Float32Array([1.5, -1.5])]);
    const input = new Blob([new Uint8Array([1])], { type: 'audio/wav' });
    const view = new DataView(await (await encodeBlobToWav(input)).arrayBuffer());
    expect(view.getInt16(44, true)).toBe(0x7fff); // clamped to +1
    expect(view.getInt16(46, true)).toBe(-0x8000); // clamped to -1
  });
});
