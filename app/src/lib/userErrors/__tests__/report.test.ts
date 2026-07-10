import { describe, expect, it, vi } from 'vitest';

import type { AppDispatch } from '../../../store';
import { ingestRuntimeErrorSignal } from '../report';

describe('ingestRuntimeErrorSignal', () => {
  it('classifies an actionable signal and dispatches reportUserError', () => {
    const dispatch = vi.fn() as unknown as AppDispatch;
    const reported = ingestRuntimeErrorSignal(dispatch, {
      message: 'OpenRouter: requires more credits',
      scope: 'chat',
    });
    expect(reported).toBe(true);
    expect(dispatch).toHaveBeenCalledTimes(1);
    const action = (dispatch as unknown as ReturnType<typeof vi.fn>).mock.calls[0][0];
    expect(action.type).toBe('userErrors/reportUserError');
    expect(action.payload.descriptor.kind).toBe('insufficient_credits');
    expect(typeof action.payload.at).toBe('number');
  });

  it('does nothing for a non-actionable signal', () => {
    const dispatch = vi.fn() as unknown as AppDispatch;
    const reported = ingestRuntimeErrorSignal(dispatch, { message: 'Something went wrong.' });
    expect(reported).toBe(false);
    expect(dispatch).not.toHaveBeenCalled();
  });

  it('never throws even if dispatch blows up', () => {
    const dispatch = vi.fn(() => {
      throw new Error('boom');
    }) as unknown as AppDispatch;
    expect(() =>
      ingestRuntimeErrorSignal(dispatch, { message: 'Insufficient budget' })
    ).not.toThrow();
    expect(ingestRuntimeErrorSignal(dispatch, { message: 'Insufficient budget' })).toBe(false);
  });
});
