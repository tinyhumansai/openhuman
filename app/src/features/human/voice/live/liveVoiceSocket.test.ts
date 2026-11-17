import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FakeWebSocket } from '../../../../test/liveVoiceFakes';
import {
  buildLiveVoiceUrl,
  LiveVoiceSocket,
  parseLiveVoiceEvent,
  resolveLiveVoiceUrl,
} from './liveVoiceSocket';

vi.mock('../../../../services/coreRpcClient', () => ({
  getCoreHttpBaseUrl: vi.fn(async () => 'http://127.0.0.1:7788'),
  getCoreRpcToken: vi.fn(async () => 'tok en'),
  resolveCoreSocketEndpoint: vi.fn(async (baseUrl: string) => ({ baseUrl, path: '/socket.io/' })),
}));

const START = {
  type: 'start' as const,
  provider: null,
  thread_id: 't-1',
  input_sample_rate: 16_000,
};

describe('buildLiveVoiceUrl', () => {
  it('maps http → ws and carries the bearer as ?token=', () => {
    expect(buildLiveVoiceUrl('http://127.0.0.1:7788', 'a b')).toBe(
      'ws://127.0.0.1:7788/ws/live-voice?token=a+b'
    );
  });

  it('maps https → wss and drops any path/query/hash', () => {
    expect(buildLiveVoiceUrl('https://core.example/rpc?x=1#y', null)).toBe(
      'wss://core.example/ws/live-voice'
    );
  });

  it('resolves against the running core', async () => {
    await expect(resolveLiveVoiceUrl()).resolves.toBe(
      'ws://127.0.0.1:7788/ws/live-voice?token=tok+en'
    );
  });

  it('uses the shell relay for private-LAN HTTP cores', async () => {
    const client = await import('../../../../services/coreRpcClient');
    vi.mocked(client.resolveCoreSocketEndpoint).mockResolvedValueOnce({
      baseUrl: 'http://127.0.0.1:40000',
      path: '/secret/socket.io/',
      liveVoicePath: '/secret/ws/live-voice',
      transports: ['websocket'],
    });
    await expect(resolveLiveVoiceUrl()).resolves.toBe(
      'ws://127.0.0.1:40000/secret/ws/live-voice?token=tok+en'
    );
  });
});

describe('parseLiveVoiceEvent', () => {
  it('accepts known event types', () => {
    expect(parseLiveVoiceEvent('{"type":"turn_complete"}')).toEqual({ type: 'turn_complete' });
  });

  it.each(['not json', '42', 'null', '{"type":"bogus"}', '{"kind":"ready"}'])('rejects %s', raw => {
    expect(parseLiveVoiceEvent(raw)).toBeNull();
  });
});

describe('LiveVoiceSocket', () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.stubGlobal('WebSocket', FakeWebSocket);
  });
  afterEach(() => vi.unstubAllGlobals());

  const open = () => {
    const handlers = { onOpen: vi.fn(), onEvent: vi.fn(), onAudio: vi.fn(), onClose: vi.fn() };
    const socket = new LiveVoiceSocket('ws://core/ws/live-voice', START, handlers);
    const ws = FakeWebSocket.latest();
    return { socket, ws, handlers };
  };

  it('sends the start frame first, with binary type arraybuffer', () => {
    const { ws, handlers } = open();
    expect(ws.binaryType).toBe('arraybuffer');
    ws.open();
    expect(ws.jsonFrames()[0]).toEqual(START);
    expect(handlers.onOpen).toHaveBeenCalled();
  });

  it('routes binary to onAudio and JSON events to onEvent', () => {
    const { ws, handlers } = open();
    ws.open();
    const pcm = new ArrayBuffer(4);
    ws.emitRaw(pcm);
    ws.emit({ type: 'interrupted' });
    ws.emitRaw('garbage');
    ws.emitRaw(12);
    expect(handlers.onAudio).toHaveBeenCalledWith(pcm);
    expect(handlers.onEvent).toHaveBeenCalledTimes(1);
    expect(handlers.onEvent).toHaveBeenCalledWith({ type: 'interrupted' });
  });

  it('sends audio, text, interrupt and stop only while open', () => {
    const { socket, ws } = open();
    socket.sendAudio(new ArrayBuffer(2));
    socket.sendText('early');
    expect(ws.sent).toHaveLength(0);
    expect(socket.isOpen).toBe(false);

    ws.open();
    const frame = new ArrayBuffer(3200);
    socket.sendAudio(frame);
    socket.sendText('hi');
    socket.interrupt();
    expect(ws.sent[1]).toBe(frame);
    expect(ws.jsonFrames().slice(1)).toEqual([{ type: 'text', text: 'hi' }, { type: 'interrupt' }]);

    socket.stop();
    expect(ws.jsonFrames().at(-1)).toEqual({ type: 'stop' });
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
  });

  it('reports a clean close when we closed it, unclean for a drop', () => {
    const a = open();
    a.ws.open();
    a.socket.close();
    a.socket.close();
    expect(a.handlers.onClose).toHaveBeenCalledTimes(1);
    expect(a.handlers.onClose.mock.calls[0][0]).toMatchObject({ code: 1000, clean: true });

    const b = open();
    b.ws.open();
    b.ws.onerror?.();
    b.ws.drop(1006);
    expect(b.handlers.onClose.mock.calls[0][0]).toMatchObject({ code: 1006, clean: false });
  });
});
