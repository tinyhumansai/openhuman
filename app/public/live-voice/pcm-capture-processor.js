/**
 * AudioWorklet processor for the live voice agent's microphone uplink.
 *
 * Served as a static asset (not bundled) because the desktop CSP allows
 * `script-src 'self'` but not `blob:`, so a Blob-URL worklet would be refused
 * and a dynamic import is not allowed in `app/src`. Loaded by
 * `app/src/features/human/voice/live/pcmCaptureWorklet.ts`.
 *
 * Input: the mic as float32 at the AudioContext rate (`sampleRate` global).
 * Output: `port.postMessage(ArrayBuffer)` — PCM16 little-endian mono at
 * `processorOptions.targetSampleRate` (16 kHz), one message per
 * `processorOptions.frameSamples` samples (1600 = 100 ms).
 *
 * Resampling is a box filter (average of the input samples that fall in each
 * output slot) for downsampling, which doubles as a cheap anti-alias filter,
 * and linear interpolation when the context already runs at or below the
 * target rate.
 */
/* global sampleRate, registerProcessor, AudioWorkletProcessor */
class PcmCaptureProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const opts = (options && options.processorOptions) || {};
    this.targetRate = opts.targetSampleRate || 16000;
    this.frameSamples = opts.frameSamples || 1600;
    this.ratio = sampleRate / this.targetRate;
    this.frame = new Int16Array(this.frameSamples);
    this.frameFill = 0;
    // Fractional read position carried across render quanta so the output
    // rate is exact rather than drifting by a rounding error every 128 frames.
    this.position = 0;
    this.carry = new Float32Array(0);
  }

  pushSample(value) {
    const clamped = value > 1 ? 1 : value < -1 ? -1 : value;
    this.frame[this.frameFill] = clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff;
    this.frameFill += 1;
    if (this.frameFill === this.frameSamples) {
      const out = this.frame.buffer.slice(0);
      this.port.postMessage(out, [out]);
      this.frameFill = 0;
    }
  }

  process(inputs) {
    const input = inputs[0];
    const channel = input && input[0];
    if (!channel || channel.length === 0) return true;

    // Join the unread tail of the previous quantum with this one.
    const samples = new Float32Array(this.carry.length + channel.length);
    samples.set(this.carry, 0);
    samples.set(channel, this.carry.length);

    const ratio = this.ratio;
    let pos = this.position;
    if (ratio >= 1) {
      while (pos + ratio <= samples.length) {
        const start = Math.floor(pos);
        const end = Math.floor(pos + ratio);
        let sum = 0;
        for (let i = start; i < end; i += 1) sum += samples[i];
        this.pushSample(end > start ? sum / (end - start) : samples[start]);
        pos += ratio;
      }
    } else {
      while (pos + 1 < samples.length) {
        const i = Math.floor(pos);
        const frac = pos - i;
        this.pushSample(samples[i] + (samples[i + 1] - samples[i]) * frac);
        pos += ratio;
      }
    }

    const consumed = Math.floor(pos);
    this.carry = samples.slice(consumed);
    this.position = pos - consumed;
    return true;
  }
}

registerProcessor('openhuman-pcm-capture', PcmCaptureProcessor);
