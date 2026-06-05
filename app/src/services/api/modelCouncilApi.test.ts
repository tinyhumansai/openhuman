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

  it('passes through an object with a result field but no logs (not a real envelope)', () => {
    // Only `{ result, logs }` (logs array present) is treated as the CLI
    // envelope. A bare `{ result }` is returned unchanged, not unwrapped.
    const notAnEnvelope = { result: RESULT } as unknown;
    expect(unwrapCouncilEnvelope(notAnEnvelope)).toEqual({ result: RESULT });
  });

  it('does NOT unwrap a null result (guards against { result: null, logs } crashing the UI)', () => {
    // A partial-error envelope with a null result must not be unwrapped to
    // `null` — that would crash the component on `result.members`. The guard
    // returns the payload as-is so the caller surfaces it as a malformed shape.
    const withNull = { result: null, logs: ['boom'] } as unknown;
    expect(unwrapCouncilEnvelope(withNull)).toEqual({ result: null, logs: ['boom'] });
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

describe('modelCouncilApi.answerMember', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls openhuman.model_council_answer_member with a long timeout', async () => {
    const member = { model: 'reasoning-v1', response: 'Paris.', error: null };
    mockCallCoreRpc.mockResolvedValueOnce({ result: member, logs: ['answered'] });

    await expect(
      modelCouncilApi.answerMember({
        question: 'What is the capital of France?',
        model: 'reasoning-v1',
        temperature: 0.2,
      })
    ).resolves.toEqual(member);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.model_council_answer_member',
      params: {
        question: 'What is the capital of France?',
        model: 'reasoning-v1',
        temperature: 0.2,
      },
      timeoutMs: 180_000,
    });
  });
});

describe('modelCouncilApi.synthesizeCouncil', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls openhuman.model_council_synthesize and unwraps the result', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: RESULT, logs: ['synthesized'] });

    await expect(
      modelCouncilApi.synthesizeCouncil({
        question: 'What is the capital of France?',
        members: RESULT.members,
        chair_model: 'chair-model',
        temperature: 0.1,
      })
    ).resolves.toEqual(RESULT);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.model_council_synthesize',
      params: {
        question: 'What is the capital of France?',
        members: RESULT.members,
        chair_model: 'chair-model',
        temperature: 0.1,
      },
      timeoutMs: 180_000,
    });
  });
});
