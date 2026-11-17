import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  base64ToBytes,
  bytesToBase64,
  extensionForMime,
  formatDuration,
  readVoiceNote,
  readVoiceNoteMeta,
  storeVoiceNote,
  transcribeVoiceNote,
  voiceNoteContentPart,
} from './voiceNoteApi';

const rpc = vi.hoisted(() => ({ callCoreRpc: vi.fn() }));
const voice = vi.hoisted(() => ({ openhumanVoiceTranscribeBytes: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => rpc);
vi.mock('../../utils/tauriCommands/voice', () => voice);

describe('voiceNoteApi', () => {
  beforeEach(() => {
    rpc.callCoreRpc.mockReset();
    voice.openhumanVoiceTranscribeBytes.mockReset();
  });

  it('keeps a recording as a typed voice-note part with its media and duration', async () => {
    const part = await voiceNoteContentPart(
      'note-123',
      'audio/wav',
      new Blob([new Uint8Array([0, 1, 2])], { type: 'audio/wav' }),
      5000,
      [0.2, 0.8]
    );

    expect(part).toEqual({
      kind: 'voice_note',
      voiceNote: {
        codec: null,
        durationMs: 5000,
        waveform: [0.2, 0.8],
        pushToTalk: false,
        transcript: null,
        media: {
          opaqueId: 'note-123',
          kind: 'audio',
          contentType: 'audio/wav',
          durationMs: 5000,
          sizeBytes: 3,
          codec: null,
          fileName: null,
          height: null,
          localCachePath: null,
          path: null,
          platformMessageId: null,
          transcribed: false,
          url: null,
          width: null,
        },
      },
    });
  });

  it('round-trips bytes through base64, including large buffers', () => {
    const bytes = new Uint8Array(70000).map((_, i) => i % 251);
    expect(base64ToBytes(bytesToBase64(bytes))).toEqual(bytes);
  });

  it('formats durations as m:ss', () => {
    expect(formatDuration(undefined)).toBe('0:00');
    expect(formatDuration(12_400)).toBe('0:12');
    expect(formatDuration(125_000)).toBe('2:05');
  });

  it('maps mime types to the extension the STT route accepts', () => {
    expect(extensionForMime('audio/ogg;codecs=opus')).toBe('ogg');
    expect(extensionForMime('audio/mp4')).toBe('m4a');
    expect(extensionForMime('audio/webm')).toBe('webm');
  });

  it('reads a voice note reference from persisted metadata and rejects junk', () => {
    expect(
      readVoiceNoteMeta({
        voiceNote: {
          noteId: 'n1',
          mime: 'audio/mpeg',
          durationMs: 900,
          waveform: [0.1, 'x', 0.5],
          visemes: [{ viseme: 'aa', start_ms: 0, end_ms: 20 }, { viseme: 4 }],
        },
      })
    ).toEqual({
      noteId: 'n1',
      mime: 'audio/mpeg',
      durationMs: 900,
      waveform: [0.1, 0.5],
      visemes: [{ viseme: 'aa', start_ms: 0, end_ms: 20 }],
    });
    expect(readVoiceNoteMeta({ voiceNote: { noteId: '' } })).toBeNull();
    expect(readVoiceNoteMeta({ voiceNote: 'x' })).toBeNull();
    expect(readVoiceNoteMeta({})).toBeNull();
    expect(readVoiceNoteMeta(undefined)).toBeNull();
  });

  it('stores a recording through the core and reports the stored mime', async () => {
    rpc.callCoreRpc.mockResolvedValue({
      logs: ['voice note stored'],
      result: { note_id: 'vn_1', mime: 'audio/wav' },
    });
    const blob = new Blob([new Uint8Array([1, 2, 3])], { type: 'audio/webm' });
    await expect(storeVoiceNote('vn_1', blob)).resolves.toEqual({ mime: 'audio/wav' });
    expect(rpc.callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channel_web_voice_note_store',
      params: { note_id: 'vn_1', audio_base64: 'AQID', mime: 'audio/webm' },
    });
  });

  it('reads stored audio back as a blob, or null when the core has none', async () => {
    rpc.callCoreRpc.mockResolvedValueOnce({ audio_base64: 'AQID', mime: 'audio/mpeg' });
    const blob = await readVoiceNote('n');
    expect(blob?.type).toBe('audio/mpeg');
    expect(blob?.size).toBe(3);
    rpc.callCoreRpc.mockResolvedValueOnce({ audio_base64: null, mime: null });
    await expect(readVoiceNote('gone')).resolves.toBeNull();
  });

  it('unwraps the core Outcome envelope when reading persisted note audio', async () => {
    rpc.callCoreRpc.mockResolvedValue({
      logs: ['voice note read'],
      result: { note_id: 'n', audio_base64: 'AQID', mime: 'audio/wav' },
    });
    const blob = await readVoiceNote('n');
    expect(blob?.type).toBe('audio/wav');
    expect(blob?.size).toBe(3);
  });

  it('transcribes with the recording extension and prefers cleaned text', async () => {
    voice.openhumanVoiceTranscribeBytes.mockResolvedValue({
      text: ' hello ',
      raw_text: 'hello raw',
    });
    const text = await transcribeVoiceNote(new Blob([new Uint8Array([9])], { type: 'audio/ogg' }));
    expect(text).toBe('hello');
    expect(voice.openhumanVoiceTranscribeBytes).toHaveBeenCalledWith([9], 'ogg', undefined, false);
  });
});
