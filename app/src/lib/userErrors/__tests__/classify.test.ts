import { describe, expect, it } from 'vitest';

import { classifyUserActionableError, userErrorId } from '../classify';

const BUDGET_MSG = 'OpenHuman API error (400): Insufficient budget';
const CREDITS_MSG = 'OpenRouter: this request requires more credits';
const BALANCE_MSG = 'HTTP 402: account is out of balance';
const GENERIC_MSG = 'Something went wrong. Please try again.';

describe('classifyUserActionableError', () => {
  it('classifies managed-budget exhaustion (USER_INSUFFICIENT_CREDITS)', () => {
    const a = classifyUserActionableError({ message: BUDGET_MSG });
    expect(a?.kind).toBe('budget_exceeded');
    expect(a?.action).toBe('open_billing');
    expect(a?.titleKey).toBe('userErrors.budgetExceeded.title');

    const b = classifyUserActionableError({ errorType: 'USER_INSUFFICIENT_CREDITS' });
    expect(b?.kind).toBe('budget_exceeded');
  });

  it('classifies BYO provider out-of-credits (402 / requires more credits)', () => {
    const a = classifyUserActionableError({ message: CREDITS_MSG });
    expect(a?.kind).toBe('insufficient_credits');
    expect(a?.action).toBe('open_provider_settings');
    expect(a?.titleKey).toBe('userErrors.insufficientCredits.title');

    const b = classifyUserActionableError({ message: BALANCE_MSG });
    expect(b?.kind).toBe('insufficient_credits');
  });

  it('prefers managed-budget over BYO-credits when text says "insufficient budget"', () => {
    // "insufficient budget" contains "insufficient" but must not be misread as
    // the BYO-credits case.
    const a = classifyUserActionableError({ message: 'insufficient budget for this request' });
    expect(a?.kind).toBe('budget_exceeded');
  });

  it('returns null for generic / non-actionable errors and empty input', () => {
    expect(classifyUserActionableError({ message: GENERIC_MSG })).toBeNull();
    expect(classifyUserActionableError({ message: '', errorType: 'inference' })).toBeNull();
    expect(classifyUserActionableError({})).toBeNull();
  });

  it('defaults scope to chat and carries provider into a stable dedupe id', () => {
    const a = classifyUserActionableError({
      message: 'requires more credits',
      provider: 'openrouter',
    });
    expect(a?.scope).toBe('chat');
    expect(a?.id).toBe(userErrorId('insufficient_credits', 'chat', 'openrouter'));
  });
});
