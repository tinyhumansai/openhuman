import { beforeEach, describe, expect, test, vi } from 'vitest';

import { draftShareHeadline, extractResponseText } from './shareCaption';

const mocks = vi.hoisted(() => ({ callCoreRpc: vi.fn() }));

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: mocks.callCoreRpc }));

describe('extractResponseText', () => {
  test('handles a bare string', () => {
    expect(extractResponseText('hello')).toBe('hello');
  });

  test('handles the { result } envelope from RpcOutcome', () => {
    expect(extractResponseText({ result: 'hi', logs: ['x'] })).toBe('hi');
  });

  test('handles a { response } object', () => {
    expect(extractResponseText({ response: 'yo' })).toBe('yo');
  });

  test('returns empty for unknown shapes', () => {
    expect(extractResponseText({ nope: 1 })).toBe('');
    expect(extractResponseText(null)).toBe('');
  });
});

describe('draftShareHeadline', () => {
  beforeEach(() => {
    mocks.callCoreRpc.mockReset();
  });

  test('uses the sanitized LLM response on success', async () => {
    mocks.callCoreRpc.mockResolvedValue({ result: '"My agent cleared my inbox."' });
    const out = await draftShareHeadline('cleared the whole inbox in seconds', 't1');
    expect(out).toBe('My agent cleared my inbox');
    expect(mocks.callCoreRpc).toHaveBeenCalledTimes(1);
    const arg = mocks.callCoreRpc.mock.calls[0][0];
    expect(arg.method).toBe('openhuman.inference_agent_chat_simple');
    expect(arg.params.thread_id).toBe('t1');
  });

  test('falls back to a deterministic headline on RPC error', async () => {
    mocks.callCoreRpc.mockRejectedValue(new Error('no model'));
    const out = await draftShareHeadline('Summarised the quarterly report today.');
    expect(out).toBe('Summarised the quarterly report today');
  });

  test('falls back when the LLM returns empty text', async () => {
    mocks.callCoreRpc.mockResolvedValue({ result: '   ' });
    const out = await draftShareHeadline('Booked the flights and hotel.');
    expect(out).toBe('Booked the flights and hotel');
  });

  test('returns the fallback without calling RPC for empty output', async () => {
    const out = await draftShareHeadline('   ');
    expect(out).toBe('');
    expect(mocks.callCoreRpc).not.toHaveBeenCalled();
  });

  test('omits thread_id when not provided', async () => {
    mocks.callCoreRpc.mockResolvedValue({ result: 'Did a thing well' });
    await draftShareHeadline('did a thing');
    expect(mocks.callCoreRpc.mock.calls[0][0].params.thread_id).toBeUndefined();
  });
});
