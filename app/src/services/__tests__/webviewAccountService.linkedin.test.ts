import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { startWebviewAccountService, stopWebviewAccountService } from '../webviewAccountService';

// ── Tauri IPC mocks ──────────────────────────────────────────────────────────

type EventHandler = (evt: { payload: unknown }) => void;
const listeners = new Map<string, EventHandler>();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  isTauri: vi.fn().mockReturnValue(true),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: EventHandler) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

// ── Service dep mocks ────────────────────────────────────────────────────────

vi.mock('../api/threadApi', () => ({ threadApi: { createNewThread: vi.fn() } }));
vi.mock('../chatService', () => ({ chatSend: vi.fn() }));
vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn().mockResolvedValue({}) }));
vi.mock('../notificationService', () => ({ ingestNotification: vi.fn() }));

// ── Helpers ──────────────────────────────────────────────────────────────────

const ACCOUNT_ID = 'acct-linkedin-test';

async function fireRecipeEvent(payload: {
  kind: string;
  account_id?: string;
  provider?: string;
  payload: Record<string, unknown>;
  ts?: number;
}): Promise<void> {
  const handler = listeners.get('webview:event');
  if (!handler) throw new Error('webview:event listener not attached');
  handler({
    payload: { account_id: ACCOUNT_ID, provider: 'linkedin', ts: Date.now(), ...payload },
  });
  // Drain microtasks + one macrotask so async persistLinkedInConversation settles.
  await new Promise(r => setTimeout(r, 0));
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('webviewAccountService — LinkedIn recipe events', () => {
  beforeEach(() => {
    listeners.clear();
    vi.clearAllMocks();
    stopWebviewAccountService();
    startWebviewAccountService();
  });

  // ── linkedin_conversation (seed / full thread) ──────────────────────────

  it('calls memory_doc_ingest with canonical key for seed conversations', async () => {
    await fireRecipeEvent({
      kind: 'linkedin_conversation',
      payload: {
        chatId: 'conv-abc',
        chatName: 'Alice',
        day: '2025-05-08',
        messages: [
          { from: 'Alice', body: 'Hello!', timestamp: 1715000000, fromMe: false },
          { from: null, body: 'Hi there', timestamp: 1715000060, fromMe: true },
        ],
        isSeed: true,
      },
    });

    expect(callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        method: 'openhuman.memory_doc_ingest',
        params: expect.objectContaining({
          namespace: `linkedin:${ACCOUNT_ID}`,
          key: 'conv-abc:2025-05-08', // canonical key — no :preview suffix
          source_type: 'linkedin-web',
          tags: expect.arrayContaining(['linkedin', 'chat-transcript', '2025-05-08']),
          metadata: expect.objectContaining({
            chat_id: 'conv-abc',
            chat_name: 'Alice',
            day: '2025-05-08',
            is_seed: true,
          }),
        }),
      })
    );
  });

  it('uses :preview key suffix for non-seed (list snippet) conversations', async () => {
    await fireRecipeEvent({
      kind: 'linkedin_conversation',
      payload: {
        chatId: 'conv-abc',
        chatName: 'Alice',
        day: '2025-05-08',
        messages: [{ from: 'Alice', body: 'Hey', timestamp: null, fromMe: false }],
        isSeed: false,
      },
    });

    expect(callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        params: expect.objectContaining({
          key: 'conv-abc:2025-05-08:preview', // :preview suffix prevents overwriting full transcript
        }),
      })
    );
  });

  it('formats transcript lines with timestamps', async () => {
    await fireRecipeEvent({
      kind: 'linkedin_conversation',
      payload: {
        chatId: 'conv-xyz',
        chatName: 'Bob',
        day: '2025-05-08',
        messages: [{ from: 'Bob', body: 'Meeting at 3?', timestamp: 1715176800, fromMe: false }],
        isSeed: true,
      },
    });

    const call = vi.mocked(callCoreRpc).mock.calls[0][0] as { params: { content: string } };
    expect(call.params.content).toContain('Bob: Meeting at 3?');
    expect(call.params.content).toContain('# LinkedIn — Bob — 2025-05-08');
  });

  it('omits "--:--" timestamp placeholder when timestamp is null', async () => {
    await fireRecipeEvent({
      kind: 'linkedin_conversation',
      payload: {
        chatId: 'conv-xyz',
        chatName: 'Bob',
        day: '2025-05-08',
        messages: [{ from: 'Bob', body: 'Hey', timestamp: null, fromMe: false }],
        isSeed: true,
      },
    });

    const call = vi.mocked(callCoreRpc).mock.calls[0][0] as { params: { content: string } };
    expect(call.params.content).toContain('[--:--] Bob: Hey');
  });

  it('labels own messages as "me"', async () => {
    await fireRecipeEvent({
      kind: 'linkedin_conversation',
      payload: {
        chatId: 'conv-xyz',
        chatName: 'Bob',
        day: '2025-05-08',
        messages: [{ from: null, body: 'Sounds good', timestamp: null, fromMe: true }],
        isSeed: true,
      },
    });

    const call = vi.mocked(callCoreRpc).mock.calls[0][0] as { params: { content: string } };
    expect(call.params.content).toContain('[--:--] me: Sounds good');
  });

  it('skips RPC call when messages array is empty', async () => {
    await fireRecipeEvent({
      kind: 'linkedin_conversation',
      payload: {
        chatId: 'conv-abc',
        chatName: 'Alice',
        day: '2025-05-08',
        messages: [],
        isSeed: true,
      },
    });

    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('falls back to chatId as chatName when chatName is null', async () => {
    await fireRecipeEvent({
      kind: 'linkedin_conversation',
      payload: {
        chatId: 'conv-no-name',
        chatName: null,
        day: '2025-05-08',
        messages: [{ from: 'X', body: 'Hi', timestamp: null, fromMe: false }],
        isSeed: true,
      },
    });

    expect(callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({
        params: expect.objectContaining({ title: 'LinkedIn · conv-no-name · 2025-05-08' }),
      })
    );
  });

  it('does not throw when callCoreRpc rejects', async () => {
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('network error'));

    await expect(
      fireRecipeEvent({
        kind: 'linkedin_conversation',
        payload: {
          chatId: 'conv-abc',
          chatName: 'Alice',
          day: '2025-05-08',
          messages: [{ from: 'Alice', body: 'Hi', timestamp: null, fromMe: false }],
          isSeed: true,
        },
      })
    ).resolves.not.toThrow();
  });

  // ── linkedin_requests ────────────────────────────────────────────────────

  it('handles linkedin_requests event without throwing', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'linkedin_requests',
        payload: {
          requests: [
            { name: 'Carol', subtitle: 'Engineer at Acme' },
            { name: 'Dave', subtitle: null },
          ],
        },
      })
    ).resolves.not.toThrow();
  });

  it('handles linkedin_requests with empty list without throwing', async () => {
    await expect(
      fireRecipeEvent({ kind: 'linkedin_requests', payload: { requests: [] } })
    ).resolves.not.toThrow();
  });

  it('drops malformed linkedin_conversation payloads before memory ingest', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'linkedin_conversation',
        payload: { chatId: 123, day: '2025-05-08', messages: [{ from: 'Alice', body: 'Hi' }] },
      })
    ).resolves.not.toThrow();

    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('drops malformed ingest payload messages before memory ingest', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'ingest',
        provider: 'whatsapp',
        payload: { messages: [{ from: 'Alice', body: 42, timestamp: 'now' }] },
      })
    ).resolves.not.toThrow();

    expect(callCoreRpc).not.toHaveBeenCalled();
  });
});
