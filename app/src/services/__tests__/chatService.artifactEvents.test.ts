import { beforeEach, describe, expect, it, vi } from 'vitest';

import { aiRegenerate, subscribeChatEvents } from '../chatService';
import { callCoreRpc } from '../coreRpcClient';
import { socketService } from '../socketService';

vi.mock('../socketService', () => ({ socketService: { getSocket: vi.fn() } }));
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
    for (const handler of handlers.get(event) ?? []) {
      handler(payload);
    }
  };
  return { id: 'socket-1', on, off, emit };
}

describe('chatService — artifact_ready / artifact_failed handlers (#2779)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('subscribes to artifact events under canonical snake_case names', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    subscribeChatEvents({ onArtifactReady: () => {}, onArtifactFailed: () => {} });

    const events = socket.on.mock.calls.map(call => call[0]);
    expect(events).toEqual(['artifact_ready', 'artifact_failed']);
  });

  it('flattens the wire envelope into a typed ArtifactReadyEvent', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactReady = vi.fn();

    subscribeChatEvents({ onArtifactReady });

    socket.emit('artifact_ready', {
      thread_id: 'thread-1',
      client_id: 'web-x',
      args: {
        artifact_id: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        path: 'a-1/deck.pptx',
        size_bytes: 4096,
      },
    });

    expect(onArtifactReady).toHaveBeenCalledTimes(1);
    expect(onArtifactReady).toHaveBeenCalledWith({
      thread_id: 'thread-1',
      client_id: 'web-x',
      artifact_id: 'a-1',
      kind: 'presentation',
      title: 'Deck',
      workspace_dir: '/workspace',
      path: 'a-1/deck.pptx',
      size_bytes: 4096,
    });
  });

  it('drops an artifact_ready payload missing required fields', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactReady = vi.fn();

    subscribeChatEvents({ onArtifactReady });

    // No args at all → skipped.
    socket.emit('artifact_ready', { thread_id: 'thread-1' });
    // Missing path → skipped.
    socket.emit('artifact_ready', {
      thread_id: 'thread-1',
      args: { artifact_id: 'a-1', kind: 'document', title: 'Doc', size_bytes: 1 },
    });
    // Missing size_bytes → skipped.
    socket.emit('artifact_ready', {
      thread_id: 'thread-1',
      args: { artifact_id: 'a-1', kind: 'document', title: 'Doc', path: 'a-1/doc.pdf' },
    });
    // Missing artifact_id → skipped.
    socket.emit('artifact_ready', {
      thread_id: 'thread-1',
      args: { kind: 'document', title: 'Doc', path: 'a-1/doc.pdf', size_bytes: 1 },
    });

    expect(onArtifactReady).not.toHaveBeenCalled();
  });

  it('rejects artifact_ready with an unknown kind (not in the allowlist)', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactReady = vi.fn();

    subscribeChatEvents({ onArtifactReady });

    socket.emit('artifact_ready', {
      thread_id: 'thread-1',
      args: {
        artifact_id: 'a-1',
        // not in {presentation, document, image, other}
        kind: 'spreadsheet',
        title: 'Deck',
        path: 'a-1/deck.pptx',
        size_bytes: 4096,
      },
    });

    expect(onArtifactReady).not.toHaveBeenCalled();
  });

  it('accepts every allowlisted kind for artifact_ready', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactReady = vi.fn();

    subscribeChatEvents({ onArtifactReady });

    for (const kind of ['presentation', 'document', 'image', 'other'] as const) {
      socket.emit('artifact_ready', {
        thread_id: 'thread-1',
        args: {
          artifact_id: `a-${kind}`,
          kind,
          title: 'X',
          workspace_dir: '/workspace',
          path: `a-${kind}/file`,
          size_bytes: 1,
        },
      });
    }

    expect(onArtifactReady).toHaveBeenCalledTimes(4);
    expect(onArtifactReady.mock.calls.map(c => (c[0] as { kind: string }).kind)).toEqual([
      'presentation',
      'document',
      'image',
      'other',
    ]);
  });

  it('flattens artifact_failed into a typed ArtifactFailedEvent', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactFailed = vi.fn();

    subscribeChatEvents({ onArtifactFailed });

    socket.emit('artifact_failed', {
      thread_id: 'thread-1',
      client_id: 'web-y',
      args: {
        artifact_id: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        error: 'engine failed: validation rejected slides[0]',
      },
    });

    expect(onArtifactFailed).toHaveBeenCalledTimes(1);
    expect(onArtifactFailed).toHaveBeenCalledWith({
      thread_id: 'thread-1',
      client_id: 'web-y',
      artifact_id: 'a-1',
      kind: 'presentation',
      title: 'Deck',
      workspace_dir: '/workspace',
      error: 'engine failed: validation rejected slides[0]',
    });
  });

  it('drops an artifact_failed payload missing required fields', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactFailed = vi.fn();

    subscribeChatEvents({ onArtifactFailed });

    // Missing error
    socket.emit('artifact_failed', {
      thread_id: 'thread-1',
      args: { artifact_id: 'a-1', kind: 'presentation', title: 'Deck' },
    });
    // Missing artifact_id
    socket.emit('artifact_failed', {
      thread_id: 'thread-1',
      args: { kind: 'presentation', title: 'Deck', error: 'x' },
    });
    // Missing title
    socket.emit('artifact_failed', {
      thread_id: 'thread-1',
      args: { artifact_id: 'a-1', kind: 'presentation', error: 'x' },
    });

    expect(onArtifactFailed).not.toHaveBeenCalled();
  });

  it('rejects artifact_failed with an unknown kind', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactFailed = vi.fn();

    subscribeChatEvents({ onArtifactFailed });

    socket.emit('artifact_failed', {
      thread_id: 'thread-1',
      args: { artifact_id: 'a-1', kind: 'video', title: 'X', error: 'boom' },
    });

    expect(onArtifactFailed).not.toHaveBeenCalled();
  });

  it('preserves the full error string on the dispatched event even when huge', () => {
    // The handler caps only the LOG line, not the dispatched payload.
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactFailed = vi.fn();

    subscribeChatEvents({ onArtifactFailed });

    const huge = 'x'.repeat(500);
    socket.emit('artifact_failed', {
      thread_id: 'thread-1',
      args: {
        artifact_id: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        error: huge,
      },
    });

    expect(onArtifactFailed).toHaveBeenCalledTimes(1);
    const event = onArtifactFailed.mock.calls[0]![0] as { error: string };
    expect(event.error).toHaveLength(500);
    expect(event.error).toBe(huge);
  });

  it('removes both artifact handlers on cleanup', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    const cleanup = subscribeChatEvents({ onArtifactReady: () => {}, onArtifactFailed: () => {} });
    cleanup();

    const offEvents = socket.off.mock.calls.map(call => call[0]);
    expect(offEvents).toEqual(['artifact_ready', 'artifact_failed']);
  });
});

describe('chatService — artifact_pending handler (#3162)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('subscribes to artifact_pending under its canonical snake_case name', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);

    subscribeChatEvents({ onArtifactPending: () => {} });

    const events = socket.on.mock.calls.map(call => call[0]);
    expect(events).toEqual(['artifact_pending']);
  });

  it('flattens the wire envelope into a typed ArtifactPendingEvent', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactPending = vi.fn();

    subscribeChatEvents({ onArtifactPending });

    socket.emit('artifact_pending', {
      thread_id: 'thread-1',
      client_id: 'web-x',
      args: {
        artifact_id: 'a-1',
        kind: 'presentation',
        title: 'Deck',
        workspace_dir: '/workspace',
        path: 'a-1/deck.pptx',
      },
    });

    expect(onArtifactPending).toHaveBeenCalledTimes(1);
    expect(onArtifactPending.mock.calls[0]![0]).toEqual({
      thread_id: 'thread-1',
      client_id: 'web-x',
      artifact_id: 'a-1',
      kind: 'presentation',
      title: 'Deck',
      workspace_dir: '/workspace',
      path: 'a-1/deck.pptx',
    });
  });

  it('drops a pending payload missing required args', () => {
    const socket = createMockSocket();
    vi.mocked(socketService.getSocket).mockReturnValue(socket as never);
    const onArtifactPending = vi.fn();

    subscribeChatEvents({ onArtifactPending });

    // Missing `path` → malformed, skipped.
    socket.emit('artifact_pending', {
      thread_id: 'thread-1',
      args: { artifact_id: 'a-1', kind: 'presentation', title: 'Deck', workspace_dir: '/ws' },
    });

    expect(onArtifactPending).not.toHaveBeenCalled();
  });
});

describe('aiRegenerate (#3162)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('calls ai_regenerate with the socket client_id for routing', async () => {
    vi.mocked(socketService.getSocket).mockReturnValue({ id: 'socket-9' } as never);
    vi.mocked(callCoreRpc).mockResolvedValueOnce({} as never);

    const ok = await aiRegenerate('a-1', 'thread-1');

    expect(ok).toBe(true);
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.ai_regenerate',
      params: { artifact_id: 'a-1', thread_id: 'thread-1', client_id: 'socket-9' },
    });
  });

  it('throws when the socket is not connected', async () => {
    vi.mocked(socketService.getSocket).mockReturnValue(undefined as never);
    await expect(aiRegenerate('a-1', 'thread-1')).rejects.toThrow('Socket not connected');
    expect(callCoreRpc).not.toHaveBeenCalled();
  });
});
