import { describe, expect, it } from 'vitest';

import { parseWorkerThreadRef } from './workerThreadRef';

describe('parseWorkerThreadRef', () => {
  it('extracts the envelope and surrounding prose for a well-formed payload', () => {
    const input =
      'Spawned worker thread `worker-abc` for the delegated task. ' +
      'Continue from a brief summary in this thread instead of relaying the entire run.\n\n' +
      '[worker_thread_ref]\n' +
      '{"thread_id":"worker-abc","label":"worker","agent_id":"researcher",' +
      '"task_id":"sub-1","elapsed_ms":120,"iterations":3}\n' +
      '[/worker_thread_ref]';

    const parsed = parseWorkerThreadRef(input);
    expect(parsed).not.toBeNull();
    expect(parsed!.before).toContain('Spawned worker thread');
    expect(parsed!.after).toBe('');
    expect(parsed!.ref).toEqual({
      threadId: 'worker-abc',
      label: 'worker',
      agentId: 'researcher',
      taskId: 'sub-1',
      elapsedMs: 120,
      iterations: 3,
    });
  });

  it('returns null when no envelope is present', () => {
    expect(parseWorkerThreadRef('plain tool result with no card')).toBeNull();
  });

  it('returns null when the envelope payload is not valid JSON', () => {
    const input = '[worker_thread_ref]\nnot really json\n[/worker_thread_ref]';
    expect(parseWorkerThreadRef(input)).toBeNull();
  });

  it('returns null when thread_id is missing or blank', () => {
    const input = '[worker_thread_ref]\n{"label":"worker"}\n[/worker_thread_ref]';
    expect(parseWorkerThreadRef(input)).toBeNull();
  });

  it('falls back to "worker" when label is missing', () => {
    const input = '[worker_thread_ref]\n{"thread_id":"worker-x"}\n[/worker_thread_ref]';
    const parsed = parseWorkerThreadRef(input);
    expect(parsed!.ref.label).toBe('worker');
    expect(parsed!.ref.threadId).toBe('worker-x');
    expect(parsed!.ref.agentId).toBeUndefined();
  });

  it('handles null and empty input safely', () => {
    expect(parseWorkerThreadRef(null)).toBeNull();
    expect(parseWorkerThreadRef(undefined)).toBeNull();
    expect(parseWorkerThreadRef('')).toBeNull();
  });
});
