/**
 * Regression tests for parseRecipePayload validation (PR #2036).
 *
 * Exercises the meet_captions, meet_call_ended, ingest (valid path), and
 * notify branches added in this PR — covering the paths that the malformed-
 * payload tests in webviewAccountService.linkedin.test.ts intentionally skip.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { ingestNotification } from '../notificationService';
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
vi.mock('../../utils/tauriCommands/config', () => ({
  openhumanGetMeetSettings: vi.fn().mockResolvedValue({ result: {}, logs: [] }),
}));

// ── Helpers ──────────────────────────────────────────────────────────────────

const ACCOUNT_ID = 'acct-validation-test';

async function fireRecipeEvent(evt: {
  kind: string;
  provider?: string;
  payload: Record<string, unknown>;
  ts?: number;
}): Promise<void> {
  const handler = listeners.get('webview:event');
  if (!handler) throw new Error('webview:event listener not attached');
  handler({
    payload: { account_id: ACCOUNT_ID, provider: 'test-provider', ts: Date.now(), ...evt },
  });
  await new Promise(r => setTimeout(r, 0));
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('webviewAccountService — recipe event payload validation', () => {
  beforeEach(() => {
    listeners.clear();
    vi.clearAllMocks();
    stopWebviewAccountService();
    startWebviewAccountService();
    vi.mocked(ingestNotification).mockResolvedValue({ id: 'notif-1' });
    vi.mocked(callCoreRpc).mockResolvedValue({} as never);
  });

  // ── meet_captions ──────────────────────────────────────────────────────────

  it('handles meet_captions with valid payload without throwing', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'meet_captions',
        payload: {
          code: 'abc-defg-hij',
          captions: [{ speaker: 'Alice', text: 'Hello there' }],
          ts: Date.now(),
        },
      })
    ).resolves.not.toThrow();
  });

  it('drops malformed meet_captions payload (invalid code type) silently', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'meet_captions',
        // code must be a string; number should fail schema validation
        payload: { code: 99, captions: [], ts: Date.now() },
      })
    ).resolves.not.toThrow();
  });

  // ── meet_call_ended ────────────────────────────────────────────────────────

  it('handles meet_call_ended with valid payload without throwing', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'meet_call_ended',
        payload: { code: 'abc-defg-hij', endedAt: Date.now(), reason: 'hangup' },
      })
    ).resolves.not.toThrow();
  });

  it('drops malformed meet_call_ended payload (missing required endedAt) silently', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'meet_call_ended',
        // endedAt is required; omitting it (or wrong type) should fail validation
        payload: { code: 'abc-defg-hij', endedAt: 'not-a-number' },
      })
    ).resolves.not.toThrow();
  });

  // ── ingest (valid path) ────────────────────────────────────────────────────

  it('dispatches messages and calls memory ingest on valid ingest payload', async () => {
    await fireRecipeEvent({
      kind: 'ingest',
      provider: 'slack',
      payload: {
        messages: [
          { id: 'msg-1', from: 'Alice', body: 'Hello!', unread: 1 },
          { id: 'msg-2', from: null, body: 'World', unread: 0 },
        ],
        unread: 1,
        snapshotKey: 'channel-C123',
      },
      ts: 1000,
    });

    // persistIngestToMemory fires callCoreRpc for non-whatsapp providers
    expect(callCoreRpc).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.memory_doc_ingest' })
    );
  });

  it('skips memory ingest when ingest payload has no messages', async () => {
    await fireRecipeEvent({
      kind: 'ingest',
      provider: 'slack',
      payload: { messages: [], unread: 0 },
      ts: 2000,
    });

    expect(callCoreRpc).not.toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.memory_doc_ingest' })
    );
  });

  // ── notify ─────────────────────────────────────────────────────────────────

  it('calls ingestNotification for valid notify payload with title and body', async () => {
    await fireRecipeEvent({
      kind: 'notify',
      payload: { title: 'New message', body: 'You have a new notification' },
    });

    expect(ingestNotification).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'New message', body: 'You have a new notification' })
    );
  });

  it('skips ingestNotification when notify payload has no title or body', async () => {
    await fireRecipeEvent({ kind: 'notify', payload: { title: '', body: '' } });

    expect(ingestNotification).not.toHaveBeenCalled();
  });

  it('drops malformed notify payload (invalid schema) silently', async () => {
    await expect(
      fireRecipeEvent({
        kind: 'notify',
        // title must be string; number should fail schema validation
        payload: { title: 123, body: 'hello' },
      })
    ).resolves.not.toThrow();

    expect(ingestNotification).not.toHaveBeenCalled();
  });
});
