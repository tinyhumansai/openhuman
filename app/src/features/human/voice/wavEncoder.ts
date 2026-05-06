/**
 * Re-encode a recorded audio blob (any container the browser's
 * `decodeAudioData` understands — WebM/Opus, MP4/AAC, OGG, …) to a
 * **16kHz mono 16-bit PCM WAV** blob.
 *
 * Why this exists: the hosted STT upstream (GMI Whisper) rejects
 * Opus-in-WebM payloads with "Invalid JSON payload", and Chromium-based
 * runtimes (including the CEF webview Tauri ships) don't reliably support
 * `MediaRecorder` with MP4. WAV at Whisper's native 16kHz is the most
 * portable thing we can hand the backend without standing up an ffmpeg
 * dependency in the desktop app.
 *
 * Implementation: `OfflineAudioContext` decodes + resamples in one pass,
 * then we mix to mono and write a standard RIFF/WAVE header in front of
 * the 16-bit little-endian samples. Synchronous after the decode promise
 * resolves so we can pipe it straight into the STT client.
 */

const TARGET_SAMPLE_RATE = 16_000;

export async function encodeBlobToWav(blob: Blob): Promise<Blob> {
  if (!blob || blob.size === 0) {
    throw new Error('audio blob is empty');
  }
  const arrayBuffer = await blob.arrayBuffer();
  // `decodeAudioData` consumes the buffer, so use a copy if the caller
  // happens to reuse `blob` afterwards.
  const decoded = await decodeToBuffer(arrayBuffer.slice(0));
  const mono = mixDownToMono(decoded);
  const wav = buildWav(mono, TARGET_SAMPLE_RATE);
  return new Blob([wav], { type: 'audio/wav' });
}

/**
 * Decode arbitrary compressed audio into an `AudioBuffer` at
 * `TARGET_SAMPLE_RATE`. Uses `OfflineAudioContext` so the resample
 * happens during decode rather than via a separate render step.
 */
async function decodeToBuffer(arrayBuffer: ArrayBuffer): Promise<AudioBuffer> {
  // OfflineAudioContext requires concrete length/channels up front, but
  // `decodeAudioData` returns a buffer at the source rate. Trick: decode
  // with a throwaway `AudioContext`, then render through an OfflineAC at
  // 16kHz to perform the resample.
  const tmp = new OfflineAudioContext(1, 1, TARGET_SAMPLE_RATE);
  const decoded = await tmp.decodeAudioData(arrayBuffer);

  if (decoded.sampleRate === TARGET_SAMPLE_RATE) {
    return decoded;
  }

  const offline = new OfflineAudioContext(
    decoded.numberOfChannels,
    Math.ceil((decoded.length * TARGET_SAMPLE_RATE) / decoded.sampleRate),
    TARGET_SAMPLE_RATE
  );
  const source = offline.createBufferSource();
  source.buffer = decoded;
  source.connect(offline.destination);
  source.start(0);
  return offline.startRendering();
}

function mixDownToMono(buffer: AudioBuffer): Float32Array {
  if (buffer.numberOfChannels === 1) {
    return buffer.getChannelData(0);
  }
  const length = buffer.length;
  const mono = new Float32Array(length);
  const channels: Float32Array[] = [];
  for (let c = 0; c < buffer.numberOfChannels; c++) {
    channels.push(buffer.getChannelData(c));
  }
  for (let i = 0; i < length; i++) {
    let sum = 0;
    for (let c = 0; c < channels.length; c++) sum += channels[c][i];
    mono[i] = sum / channels.length;
  }
  return mono;
}

function buildWav(samples: Float32Array, sampleRate: number): ArrayBuffer {
  const bytesPerSample = 2; // 16-bit PCM
  const numChannels = 1;
  const dataBytes = samples.length * bytesPerSample;
  const buffer = new ArrayBuffer(44 + dataBytes);
  const view = new DataView(buffer);

  // RIFF chunk descriptor
  writeString(view, 0, 'RIFF');
  view.setUint32(4, 36 + dataBytes, true);
  writeString(view, 8, 'WAVE');

  // fmt sub-chunk (PCM)
  writeString(view, 12, 'fmt ');
  view.setUint32(16, 16, true); // sub-chunk size
  view.setUint16(20, 1, true); // PCM format
  view.setUint16(22, numChannels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * numChannels * bytesPerSample, true); // byte rate
  view.setUint16(32, numChannels * bytesPerSample, true); // block align
  view.setUint16(34, bytesPerSample * 8, true); // bits per sample

  // data sub-chunk
  writeString(view, 36, 'data');
  view.setUint32(40, dataBytes, true);

  let offset = 44;
  for (let i = 0; i < samples.length; i++, offset += 2) {
    // Clamp + scale to signed 16-bit. Reverse-clipping protects against
    // floats slightly outside [-1, 1] from accumulator rounding.
    const s = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(offset, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }

  return buffer;
}

function writeString(view: DataView, offset: number, value: string) {
  for (let i = 0; i < value.length; i++) {
    view.setUint8(offset + i, value.charCodeAt(i));
  }
}
