import { beforeEach, describe, expect, it, vi } from 'vitest';

import { createPttService, type PttDeps } from '../pttService';

function makeDeps(overrides: Partial<PttDeps> = {}): PttDeps {
  return {
    audioCapture: {
      start: vi.fn().mockResolvedValue(undefined),
      finalize: vi.fn().mockResolvedValue({ durationMs: 1500, buffer: new ArrayBuffer(0) }),
      cancel: vi.fn().mockResolvedValue(undefined),
    },
    transcribe: vi.fn().mockResolvedValue('hello world'),
    sendMessage: vi.fn().mockResolvedValue(undefined),
    resolveActiveThreadId: vi.fn().mockResolvedValue('thread-active'),
    createNewVoiceThread: vi.fn().mockResolvedValue('thread-new'),
    playChime: vi.fn().mockResolvedValue(undefined),
    showOverlay: vi.fn().mockResolvedValue(undefined),
    getSettings: () => ({ speakReplies: true, showOverlay: true }),
    now: () => 1_700_000_000_000,
    watchdogMs: 10_000,
    minAudioMs: 250,
    logger: { debug: vi.fn(), info: vi.fn(), warn: vi.fn() },
    ...overrides,
  };
}

describe('pttService state machine', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  it('happy path: start → stop sends the transcript to the active thread with speakReply', async () => {
    const deps = makeDeps();
    const svc = createPttService(deps);

    await svc.onStart(1);
    expect(deps.audioCapture.start).toHaveBeenCalledWith({ sessionTag: 'ptt:1' });
    expect(deps.playChime).toHaveBeenCalledWith('open');
    expect(deps.showOverlay).toHaveBeenCalledWith(true, 1);

    await svc.onStop(1);
    expect(deps.audioCapture.finalize).toHaveBeenCalled();
    expect(deps.playChime).toHaveBeenCalledWith('close');
    expect(deps.showOverlay).toHaveBeenCalledWith(false, 1);
    expect(deps.transcribe).toHaveBeenCalled();
    expect(deps.sendMessage).toHaveBeenCalledWith({
      threadId: 'thread-active',
      body: 'hello world',
      metadata: { source: 'ptt', session_id: 1 },
      speakReply: true,
    });
  });

  it('falls back to a new "Voice" thread when no active thread exists', async () => {
    const deps = makeDeps({ resolveActiveThreadId: vi.fn().mockResolvedValue(null) });
    const svc = createPttService(deps);

    await svc.onStart(2);
    await svc.onStop(2);

    expect(deps.createNewVoiceThread).toHaveBeenCalled();
    expect(deps.sendMessage).toHaveBeenCalledWith(
      expect.objectContaining({ threadId: 'thread-new' })
    );
  });

  it('drops the session and plays the error chime when audio is shorter than minAudioMs', async () => {
    const deps = makeDeps({
      audioCapture: {
        start: vi.fn().mockResolvedValue(undefined),
        finalize: vi.fn().mockResolvedValue({ durationMs: 100, buffer: new ArrayBuffer(0) }),
        cancel: vi.fn().mockResolvedValue(undefined),
      },
    });
    const svc = createPttService(deps);

    await svc.onStart(3);
    await svc.onStop(3);

    expect(deps.transcribe).not.toHaveBeenCalled();
    expect(deps.sendMessage).not.toHaveBeenCalled();
    expect(deps.playChime).toHaveBeenCalledWith('error');
  });

  it('drops the session when the transcript is empty', async () => {
    const deps = makeDeps({ transcribe: vi.fn().mockResolvedValue('   ') });
    const svc = createPttService(deps);

    await svc.onStart(4);
    await svc.onStop(4);

    expect(deps.sendMessage).not.toHaveBeenCalled();
    expect(deps.playChime).toHaveBeenCalledWith('error');
  });

  it('watchdog finalises the session after watchdogMs even if onStop never arrives', async () => {
    const deps = makeDeps();
    const svc = createPttService(deps);

    await svc.onStart(5);

    // Advance fake time past the watchdog.
    await vi.advanceTimersByTimeAsync(11_000);

    expect(deps.audioCapture.finalize).toHaveBeenCalled();
    expect(deps.sendMessage).toHaveBeenCalledWith(
      expect.objectContaining({ metadata: expect.objectContaining({ session_id: 5 }) })
    );
  });

  it('second onStart while a session is active preempts the first', async () => {
    const deps = makeDeps();
    const svc = createPttService(deps);

    await svc.onStart(6);
    await svc.onStart(7);

    expect(deps.audioCapture.cancel).toHaveBeenCalled();
    expect(deps.audioCapture.start).toHaveBeenLastCalledWith({ sessionTag: 'ptt:7' });
  });

  it('honours the speakReplies setting when forwarding to sendMessage', async () => {
    const deps = makeDeps({ getSettings: () => ({ speakReplies: false, showOverlay: true }) });
    const svc = createPttService(deps);

    await svc.onStart(8);
    await svc.onStop(8);

    expect(deps.sendMessage).toHaveBeenCalledWith(expect.objectContaining({ speakReply: false }));
  });

  it('mismatched session_id on onStop is ignored', async () => {
    const deps = makeDeps();
    const svc = createPttService(deps);

    await svc.onStart(9);
    await svc.onStop(999); // stale stop event

    expect(deps.audioCapture.finalize).not.toHaveBeenCalled();
  });

  it('cancel("user_cancel") aborts an active session without sending a message', async () => {
    const deps = makeDeps();
    const svc = createPttService(deps);

    await svc.onStart(10);
    await svc.cancel('user_cancel');

    expect(deps.audioCapture.cancel).toHaveBeenCalled();
    expect(deps.playChime).toHaveBeenCalledWith('error');
    expect(deps.showOverlay).toHaveBeenLastCalledWith(false, 10);
    expect(deps.sendMessage).not.toHaveBeenCalled();
  });

  it('plays error chime and bails if audioCapture.start throws', async () => {
    const deps = makeDeps({
      audioCapture: {
        start: vi.fn().mockRejectedValue(new Error('mic denied')),
        finalize: vi.fn().mockResolvedValue({ durationMs: 1500, buffer: new ArrayBuffer(0) }),
        cancel: vi.fn().mockResolvedValue(undefined),
      },
    });
    const svc = createPttService(deps);

    await svc.onStart(11);

    expect(deps.playChime).toHaveBeenCalledWith('open');
    expect(deps.playChime).toHaveBeenCalledWith('error');
    expect(deps.showOverlay).toHaveBeenLastCalledWith(false, 11);
    // The session never armed — onStop should be a no-op.
    await svc.onStop(11);
    expect(deps.audioCapture.finalize).not.toHaveBeenCalled();
    expect(deps.sendMessage).not.toHaveBeenCalled();
  });

  it('posts a "[Voice — transcription failed]" breadcrumb when transcribe throws', async () => {
    const deps = makeDeps({ transcribe: vi.fn().mockRejectedValue(new Error('stt timeout')) });
    const svc = createPttService(deps);

    await svc.onStart(12);
    await svc.onStop(12);

    expect(deps.sendMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        body: '[Voice — transcription failed]',
        metadata: { source: 'ptt', session_id: 12 },
      })
    );
  });
});
