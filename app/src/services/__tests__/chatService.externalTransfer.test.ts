import { beforeEach, describe, expect, it, vi } from 'vitest';

import { subscribeChatEvents } from '../chatService';
import { socketService } from '../socketService';

vi.mock('../socketService', () => ({
  socketService: { getSocket: vi.fn(), on: vi.fn(), off: vi.fn() },
}));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

type Handler = (...args: unknown[]) => void;

function createMockSocket() {
  const handlers = new Map<string, Handler[]>();
  const on = vi.fn((event: string, cb: Handler) => {
    const existing = handlers.get(event) ?? [];
    existing.push(cb);
    handlers.set(event, existing);
  });
  const off = vi.fn((event: string, cb: Handler) => {
    const existing = handlers.get(event) ?? [];
    handlers.set(
      event,
      existing.filter(handler => handler !== cb)
    );
  });
  const emit = (event: string, payload: unknown) => {
    for (const handler of handlers.get(event) ?? []) handler(payload);
  };
  return { id: 'socket-1', on, off, emit };
}

function bindMockSocket(socket: ReturnType<typeof createMockSocket>) {
  vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
  vi.mocked(socketService.on).mockImplementation((event, cb) => socket.on(event, cb as Handler));
  vi.mocked(socketService.off).mockImplementation((event, cb) => socket.off(event, cb as Handler));
}

describe('chatService — external_transfer_pending handler (#4437 / S3)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('subscribes under the canonical snake_case event name', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);

    subscribeChatEvents({ onExternalTransferPending: () => {} });

    const events = socket.on.mock.calls.map(call => call[0]);
    expect(events).toEqual(['external_transfer_pending']);
  });

  it('flattens the wire envelope into a typed ExternalTransferPendingEvent', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onExternalTransferPending = vi.fn();

    subscribeChatEvents({ onExternalTransferPending });

    socket.emit('external_transfer_pending', {
      thread_id: 'thread-1',
      client_id: 'web-x',
      args: {
        provider_slug: 'openai',
        service: 'OpenAI',
        is_external: true,
        reason: 'inference',
        data_kinds: ['prompt', 'tool_arguments'],
        risk_level: 'unknown',
        risk_categories: [],
      },
    });

    expect(onExternalTransferPending).toHaveBeenCalledTimes(1);
    expect(onExternalTransferPending).toHaveBeenCalledWith({
      thread_id: 'thread-1',
      client_id: 'web-x',
      provider_slug: 'openai',
      service: 'OpenAI',
      is_external: true,
      reason: 'inference',
      data_kinds: ['prompt', 'tool_arguments'],
      risk_level: 'unknown',
      risk_categories: [],
    });
  });

  it('accepts an empty data_kinds array (metadata-only transfer)', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onExternalTransferPending = vi.fn();

    subscribeChatEvents({ onExternalTransferPending });

    socket.emit('external_transfer_pending', {
      thread_id: 'thread-1',
      args: {
        provider_slug: 'composio',
        service: 'Gmail',
        is_external: true,
        reason: 'integration',
        data_kinds: [],
        risk_level: 'unknown',
        risk_categories: [],
      },
    });

    expect(onExternalTransferPending).toHaveBeenCalledTimes(1);
    expect(onExternalTransferPending.mock.calls[0]![0]).toMatchObject({
      service: 'Gmail',
      data_kinds: [],
    });
  });

  it('defaults is_external to true and risk_level to unknown when absent/invalid', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onExternalTransferPending = vi.fn();

    subscribeChatEvents({ onExternalTransferPending });

    socket.emit('external_transfer_pending', {
      thread_id: 'thread-1',
      args: {
        provider_slug: 'openai',
        service: 'OpenAI',
        // is_external omitted
        reason: 'inference',
        data_kinds: ['prompt'],
        risk_level: 'not-a-real-level',
        // risk_categories omitted
      },
    });

    const event = onExternalTransferPending.mock.calls[0]![0] as {
      is_external: boolean;
      risk_level: string;
      risk_categories: string[];
    };
    expect(event.is_external).toBe(true);
    expect(event.risk_level).toBe('unknown');
    expect(event.risk_categories).toEqual([]);
  });

  it('drops payloads missing load-bearing fields', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);
    const onExternalTransferPending = vi.fn();

    subscribeChatEvents({ onExternalTransferPending });

    // No args → bad envelope.
    socket.emit('external_transfer_pending', { thread_id: 'thread-1' });
    // Missing provider_slug.
    socket.emit('external_transfer_pending', {
      thread_id: 'thread-1',
      args: { service: 'OpenAI', reason: 'inference', data_kinds: [] },
    });
    // Missing service.
    socket.emit('external_transfer_pending', {
      thread_id: 'thread-1',
      args: { provider_slug: 'openai', reason: 'inference', data_kinds: [] },
    });
    // Missing reason.
    socket.emit('external_transfer_pending', {
      thread_id: 'thread-1',
      args: { provider_slug: 'openai', service: 'OpenAI', data_kinds: [] },
    });
    // data_kinds not an array.
    socket.emit('external_transfer_pending', {
      thread_id: 'thread-1',
      args: {
        provider_slug: 'openai',
        service: 'OpenAI',
        reason: 'inference',
        data_kinds: 'prompt',
      },
    });
    // Missing thread_id → bad envelope.
    socket.emit('external_transfer_pending', {
      args: { provider_slug: 'openai', service: 'OpenAI', reason: 'inference', data_kinds: [] },
    });

    expect(onExternalTransferPending).not.toHaveBeenCalled();
  });

  it('removes the handler on cleanup', () => {
    const socket = createMockSocket();
    bindMockSocket(socket);

    const cleanup = subscribeChatEvents({ onExternalTransferPending: () => {} });
    cleanup();

    const offEvents = socket.off.mock.calls.map(call => call[0]);
    expect(offEvents).toEqual(['external_transfer_pending']);
  });
});
