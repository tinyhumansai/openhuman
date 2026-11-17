import { describe, expect, it, vi } from 'vitest';

import { FakeAudioContext, pcmBytes } from '../../../../test/liveVoiceFakes';
import { pcm16ToFloat32, PcmPlayer, rms } from './pcmPlayer';

function makePlayer(onPlayingChange = vi.fn()) {
  const ctx = new FakeAudioContext();
  const player = new PcmPlayer({
    sampleRate: 24_000,
    onPlayingChange,
    createContext: () => ctx as unknown as AudioContext,
  });
  return { ctx, player, onPlayingChange };
}

describe('pcm16ToFloat32 / rms', () => {
  it('decodes little-endian PCM16 into [-1, 1)', () => {
    const buf = new ArrayBuffer(6);
    const view = new DataView(buf);
    view.setInt16(0, 0x7fff, true);
    view.setInt16(2, -0x8000, true);
    view.setInt16(4, 0, true);
    const out = pcm16ToFloat32(buf);
    expect(out[0]).toBeCloseTo(1, 3);
    expect(out[1]).toBe(-1);
    expect(out[2]).toBe(0);
  });

  it('ignores a trailing odd byte', () => {
    expect(pcm16ToFloat32(new ArrayBuffer(5))).toHaveLength(2);
  });

  it('computes RMS, 0 for an empty block', () => {
    expect(rms(new Float32Array(0))).toBe(0);
    expect(rms(new Float32Array([0.5, -0.5]))).toBeCloseTo(0.5);
  });
});

describe('PcmPlayer', () => {
  it('schedules chunks back to back on the context clock', () => {
    const { ctx, player, onPlayingChange } = makePlayer();
    player.enqueue(pcmBytes(2400)); // 100 ms at 24 kHz
    player.enqueue(pcmBytes(2400));

    const [first, second] = ctx.sources;
    expect(first.started).toBeCloseTo(0.04);
    expect(second.started).toBeCloseTo(0.14);
    expect(player.isPlaying()).toBe(true);
    expect(onPlayingChange).toHaveBeenCalledTimes(1);
    expect(onPlayingChange).toHaveBeenLastCalledWith(true);
  });

  it('never schedules in the past after a gap', () => {
    const { ctx, player } = makePlayer();
    player.enqueue(pcmBytes(240));
    ctx.sources[0].onended?.();
    ctx.advanceTo(5);
    player.enqueue(pcmBytes(240));
    expect(ctx.sources[1].started).toBeGreaterThanOrEqual(5);
  });

  it('skips empty chunks', () => {
    const { ctx, player } = makePlayer();
    player.enqueue(new ArrayBuffer(0));
    expect(ctx.sources).toHaveLength(0);
    expect(player.isPlaying()).toBe(false);
  });

  it('reports the amplitude of the chunk audible now, 0 outside it', () => {
    const { ctx, player } = makePlayer();
    player.enqueue(pcmBytes(2400, 0.2));
    ctx.advanceTo(0.01);
    expect(player.getAmplitude()).toBe(0); // before the scheduled start
    ctx.advanceTo(0.05);
    expect(player.getAmplitude()).toBeCloseTo(0.6, 2); // 0.2 RMS × gain 3
    ctx.advanceTo(1);
    expect(player.getAmplitude()).toBe(0);
  });

  it('clamps loud chunks to 1', () => {
    const { ctx, player } = makePlayer();
    player.enqueue(pcmBytes(2400, 0.9));
    ctx.advanceTo(0.05);
    expect(player.getAmplitude()).toBe(1);
  });

  it('goes idle when the last chunk ends', () => {
    const { ctx, player, onPlayingChange } = makePlayer();
    player.enqueue(pcmBytes(240));
    player.enqueue(pcmBytes(240));
    ctx.sources[0].onended?.();
    expect(player.isPlaying()).toBe(true);
    ctx.sources[1].onended?.();
    expect(player.isPlaying()).toBe(false);
    expect(onPlayingChange).toHaveBeenLastCalledWith(false);
  });

  it('flush() stops every queued source and resets the timeline', () => {
    const { ctx, player, onPlayingChange } = makePlayer();
    player.enqueue(pcmBytes(2400));
    player.enqueue(pcmBytes(2400));
    player.flush();
    expect(ctx.sources.every(s => s.stopped)).toBe(true);
    expect(player.isPlaying()).toBe(false);
    expect(player.getAmplitude()).toBe(0);
    expect(onPlayingChange).toHaveBeenLastCalledWith(false);

    player.enqueue(pcmBytes(240));
    expect(ctx.sources[2].started).toBeCloseTo(0.04);
  });

  it('tolerates sources that throw on stop/disconnect', () => {
    const { ctx, player } = makePlayer();
    player.enqueue(pcmBytes(240));
    ctx.sources[0].stop = () => {
      throw new Error('not started');
    };
    ctx.sources[0].disconnect.mockImplementation(() => {
      throw new Error('gone');
    });
    expect(() => player.flush()).not.toThrow();
  });

  it('close() releases the context and ignores later chunks', () => {
    const { ctx, player } = makePlayer();
    player.enqueue(pcmBytes(240));
    player.close();
    player.close();
    expect(ctx.close).toHaveBeenCalledTimes(1);
    player.enqueue(pcmBytes(240));
    expect(ctx.sources).toHaveLength(1);
    expect(player.getAmplitude()).toBe(0);
  });

  it('resumes a suspended context on construction', () => {
    const ctx = new FakeAudioContext();
    ctx.state = 'suspended';
    new PcmPlayer({ sampleRate: 16_000, createContext: () => ctx as unknown as AudioContext });
    expect(ctx.resume).toHaveBeenCalled();
  });

  it('uses window.AudioContext by default', () => {
    const ctx = new FakeAudioContext();
    const constructed = vi.fn();
    vi.stubGlobal(
      'AudioContext',
      class {
        constructor() {
          constructed();
          return ctx;
        }
      }
    );
    try {
      const player = new PcmPlayer({ sampleRate: 16_000 });
      player.enqueue(pcmBytes(16));
      expect(constructed).toHaveBeenCalled();
      expect(ctx.sources).toHaveLength(1);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('throws when no AudioContext exists', () => {
    vi.stubGlobal('AudioContext', undefined);
    try {
      expect(() => new PcmPlayer({ sampleRate: 16_000 })).toThrow(/AudioContext/);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
