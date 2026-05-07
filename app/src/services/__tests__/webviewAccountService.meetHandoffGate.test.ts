/**
 * Privacy gate regression tests for issue #1299.
 *
 * Verifies that `maybeHandoffToOrchestrator` only invokes the orchestrator
 * (creating a fresh chat thread + sending the transcript prompt) when the
 * user has explicitly opted in via the `meet.auto_orchestrator_handoff`
 * setting. Default-OFF must skip both `threadApi.createNewThread` and
 * `chatSend` entirely. RPC failures fail closed (no handoff).
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { __testInternals } from '../webviewAccountService';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  isTauri: vi.fn().mockReturnValue(true),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../notificationService', () => ({ ingestNotification: vi.fn() }));

const createNewThreadMock = vi.fn();
vi.mock('../api/threadApi', () => ({
  threadApi: { createNewThread: (...args: unknown[]) => createNewThreadMock(...args) },
}));

const chatSendMock = vi.fn();
vi.mock('../chatService', () => ({ chatSend: (...args: unknown[]) => chatSendMock(...args) }));

const getMeetSettingsMock = vi.fn();
vi.mock('../../utils/tauriCommands/config', () => ({
  openhumanGetMeetSettings: () => getMeetSettingsMock(),
}));

interface MockMeetingSession {
  code: string;
  startedAt: number;
  snapshots: never[];
}

function makeSession(): MockMeetingSession {
  return { code: 'abc-defg-hij', startedAt: Date.now() - 60_000, snapshots: [] };
}

describe('maybeHandoffToOrchestrator (#1299 privacy gate)', () => {
  beforeEach(() => {
    createNewThreadMock.mockReset();
    chatSendMock.mockReset();
    getMeetSettingsMock.mockReset();
    createNewThreadMock.mockResolvedValue({ id: 'thread-1' });
    chatSendMock.mockResolvedValue(undefined);
  });

  it('skips handoff when auto_orchestrator_handoff is false', async () => {
    getMeetSettingsMock.mockResolvedValue({
      result: { auto_orchestrator_handoff: false },
      logs: [],
    });

    await __testInternals.maybeHandoffToOrchestrator(
      'acct-test',
      // The function only reads `code` and `startedAt`. Ts cast is enough
      // for a structural mock — full MeetingSession is heavier than needed.
      makeSession() as unknown as Parameters<typeof __testInternals.maybeHandoffToOrchestrator>[1],
      Date.now(),
      '## Transcript\n[10:00:00] Alice: hello',
      new Set(['Alice'])
    );

    expect(createNewThreadMock).not.toHaveBeenCalled();
    expect(chatSendMock).not.toHaveBeenCalled();
  });

  it('skips handoff when settings field is missing', async () => {
    getMeetSettingsMock.mockResolvedValue({ result: {}, logs: [] });

    await __testInternals.maybeHandoffToOrchestrator(
      'acct-test',
      makeSession() as unknown as Parameters<typeof __testInternals.maybeHandoffToOrchestrator>[1],
      Date.now(),
      '## Transcript',
      new Set()
    );

    expect(createNewThreadMock).not.toHaveBeenCalled();
    expect(chatSendMock).not.toHaveBeenCalled();
  });

  it('fails closed (no handoff) when settings RPC throws', async () => {
    getMeetSettingsMock.mockRejectedValue(new Error('core rpc down'));

    await __testInternals.maybeHandoffToOrchestrator(
      'acct-test',
      makeSession() as unknown as Parameters<typeof __testInternals.maybeHandoffToOrchestrator>[1],
      Date.now(),
      '## Transcript',
      new Set()
    );

    expect(createNewThreadMock).not.toHaveBeenCalled();
    expect(chatSendMock).not.toHaveBeenCalled();
  });

  it('fires handoff when auto_orchestrator_handoff is true', async () => {
    getMeetSettingsMock.mockResolvedValue({
      result: { auto_orchestrator_handoff: true },
      logs: [],
    });

    await __testInternals.maybeHandoffToOrchestrator(
      'acct-test',
      makeSession() as unknown as Parameters<typeof __testInternals.maybeHandoffToOrchestrator>[1],
      Date.now(),
      '## Transcript\n[10:00:00] Alice: hello',
      new Set(['Alice'])
    );

    expect(createNewThreadMock).toHaveBeenCalledTimes(1);
    expect(chatSendMock).toHaveBeenCalledTimes(1);
    const sendArgs = chatSendMock.mock.calls[0][0];
    expect(sendArgs.threadId).toBe('thread-1');
    expect(sendArgs.message).toContain('## Transcript');
  });
});
