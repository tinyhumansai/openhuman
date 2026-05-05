import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { transcribeCloud } from './sttClient';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('transcribeCloud', () => {
  beforeEach(() => {
    (callCoreRpc as ReturnType<typeof vi.fn>).mockReset();
  });
  it('routes through openhuman.voice_cloud_transcribe with base64 + mime', async () => {
    const mock = callCoreRpc as ReturnType<typeof vi.fn>;
    mock.mockResolvedValueOnce({ text: 'hello there' });
    const blob = new Blob([new Uint8Array([1, 2, 3, 4, 5])], { type: 'audio/webm;codecs=opus' });

    const text = await transcribeCloud(blob);

    expect(text).toBe('hello there');
    expect(mock).toHaveBeenCalledTimes(1);
    const call = mock.mock.calls[0][0] as {
      method: string;
      params: { audio_base64: string; mime_type: string; file_name: string };
    };
    expect(call.method).toBe('openhuman.voice_cloud_transcribe');
    // `audio/webm;codecs=opus` should collapse to the bare type the backend
    // allow-list accepts.
    expect(call.params.mime_type).toBe('audio/webm');
    expect(call.params.file_name).toBe('audio.webm');
    expect(call.params.audio_base64).toBe(btoa('\x01\x02\x03\x04\x05'));
  });

  it('rejects empty blobs without hitting the core', async () => {
    const mock = callCoreRpc as ReturnType<typeof vi.fn>;
    const blob = new Blob([], { type: 'audio/webm' });
    await expect(transcribeCloud(blob)).rejects.toThrow(/empty/);
    expect(mock).not.toHaveBeenCalled();
  });

  it('forwards the optional model + language hints', async () => {
    const mock = callCoreRpc as ReturnType<typeof vi.fn>;
    mock.mockResolvedValueOnce({ text: 'hi' });
    const blob = new Blob([new Uint8Array([9])], { type: 'audio/webm' });

    await transcribeCloud(blob, { model: 'scribe_v1', language: 'en' });
    const params = mock.mock.calls[0][0].params as Record<string, unknown>;
    expect(params.model).toBe('scribe_v1');
    expect(params.language).toBe('en');
  });

  it('trims whitespace off the returned transcript', async () => {
    const mock = callCoreRpc as ReturnType<typeof vi.fn>;
    mock.mockResolvedValueOnce({ text: '  spacey  ' });
    const blob = new Blob([new Uint8Array([1])], { type: 'audio/webm' });
    expect(await transcribeCloud(blob)).toBe('spacey');
  });
});
