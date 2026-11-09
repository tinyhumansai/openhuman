import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../services/api/threadApi';
import { chatSend } from '../services/chatService';
import { attachmentsToParts, parseAttachmentReferences, validateAndReadFile } from './attachments';

const mockCallCoreRpc = vi.fn();
vi.mock('../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
  isThreadNotFoundCoreRpcError: () => false,
}));
vi.mock('../services/socketService', () => ({
  socketService: { getSocket: () => ({ id: 'sock-1' }), on: vi.fn(), off: vi.fn() },
}));

// The core's staged form of the image: a durable workspace reference.
const DURABLE =
  '[ATTACHMENT:' +
  encodeURIComponent(
    JSON.stringify({
      path: 'uploads/t1/abc/pic.png',
      name: 'pic.png',
      mime: 'image/png',
      size_bytes: 8,
    })
  ) +
  ']';

describe('image attachment round trip (composer -> wire -> durable reference)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('sends the image as a typed part, never as a marker, and renders the core reference', async () => {
    const file = new File([Uint8Array.of(0x89, 0x50, 0x4e, 0x47)], 'pic.png', {
      type: 'image/png',
    });
    const read = await validateAndReadFile(file, 0);
    if (!('attachment' in read)) throw new Error('attachment rejected');
    const parts = attachmentsToParts([read.attachment]);

    // Core answers the append with the staged, durable content.
    mockCallCoreRpc.mockResolvedValueOnce({
      data: {
        id: 'm1',
        content: `look ${DURABLE}`,
        type: 'text',
        extraMetadata: {},
        sender: 'user',
        createdAt: '2026-10-18T00:00:00Z',
      },
    });
    const saved = await threadApi.appendMessage(
      't1',
      {
        id: 'm1',
        content: 'look',
        type: 'text',
        extraMetadata: {},
        sender: 'user',
        createdAt: '2026-10-18T00:00:00Z',
      },
      parts
    );

    const appendCall = mockCallCoreRpc.mock.calls[0][0];
    expect(appendCall.method).toBe('openhuman.threads_message_append');
    expect(appendCall.params.message.content).toBe('look');
    expect(appendCall.params.attachments).toEqual(parts);
    expect(JSON.stringify(appendCall.params)).not.toMatch(/\[(IMAGE|FILE):/);

    // The turn is started with the durable content the core returned.
    mockCallCoreRpc.mockResolvedValueOnce({
      accepted: true,
      client_id: 'sock-1',
      thread_id: 't1',
      request_id: 'request-1',
    });
    await chatSend({ threadId: 't1', message: saved.content });
    expect(mockCallCoreRpc.mock.calls[1][0].params.message).toBe(`look ${DURABLE}`);

    // Display side: the reference decodes to the original file, text stays clean.
    const shown = parseAttachmentReferences(saved.content);
    expect(shown.text).toBe('look');
    expect(shown.attachments).toEqual([
      { path: 'uploads/t1/abc/pic.png', name: 'pic.png', mime: 'image/png', size_bytes: 8 },
    ]);
  });
});
