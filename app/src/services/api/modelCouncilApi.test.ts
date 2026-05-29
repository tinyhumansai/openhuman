import { beforeEach, describe, expect, it, vi } from 'vitest';

import { modelCouncilApi, type ModelCouncilResult, unwrapCouncilEnvelope } from './modelCouncilApi';

const mockCallCoreRpc = vi.fn();
vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const RESULT: ModelCouncilResult = {
  question: 'What is the capital of France?',
  members: [
    { model: 'model-a', response: 'Paris.', error: null },
    { model: 'model-b', response: null, error: 'rate limited' },
  ],
  chair_model: 'chair-model',
  synthesis: 'Both agree the capital is Paris (one seat failed).',
};

describe('unwrapCouncilEnvelope', () => {
  it('unwraps the { result, logs } CLI envelope', () => {
    expect(unwrapCouncilEnvelope({ result: RESULT, logs: ['done'] })).toEqual(RESULT);
  });

  it('passes a bare result through unchanged', () => {
    expect(unwrapCouncilEnvelope(RESULT)).toEqual(RESULT);
  });

  it('does not mistake a result that happens to have a result field but no logs', () => {
    const bare = { ...RESULT } as unknown;
    expect(unwrapCouncilEnvelope(bare)).toEqual(RESULT);
  });
});

describe('modelCouncilApi.runCouncil', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls openhuman.model_council_run with the params + a long timeout', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: RESULT, logs: ['ok'] });
    const out = await modelCouncilApi.runCouncil({
      question: 'What is the capital of France?',
      member_models: ['model-a', 'model-b'],
      chair_model: 'chair-model',
      temperature: 0.4,
    });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.model_council_run',
      params: {
        question: 'What is the capital of France?',
        member_models: ['model-a', 'model-b'],
        chair_model: 'chair-model',
        temperature: 0.4,
      },
      timeoutMs: 180_000,
    });
    expect(out).toEqual(RESULT);
  });

  it('returns the unwrapped result when the core wraps it in an envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: RESULT, logs: ['done'] });
    const out = await modelCouncilApi.runCouncil({
      question: 'q',
      member_models: ['a'],
      chair_model: 'c',
    });
    expect(out.members).toHaveLength(2);
    expect(out.synthesis).toContain('Paris');
  });

  it('returns a bare result unchanged when no envelope is present', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(RESULT);
    const out = await modelCouncilApi.runCouncil({
      question: 'q',
      member_models: ['a'],
      chair_model: 'c',
    });
    expect(out).toEqual(RESULT);
  });

  it('propagates errors from the RPC layer', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('all member models failed'));
    await expect(
      modelCouncilApi.runCouncil({ question: 'q', member_models: ['a'], chair_model: 'c' })
    ).rejects.toThrow('all member models failed');
  });
});
