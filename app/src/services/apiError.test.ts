import { describe, expect, it } from 'vitest';

import { messageForApiError } from './apiError';

describe('messageForApiError', () => {
  it('uses the message from an Error', () => {
    expect(messageForApiError(new Error('Connection lost'), 'Try again.')).toBe('Connection lost');
  });

  it('uses the string error from an apiClient rejection', () => {
    expect(
      messageForApiError({ success: false, error: 'You have already reported this.' }, 'Try again.')
    ).toBe('You have already reported this.');
  });

  it.each([
    new Error(''),
    { success: false, error: '' },
    { success: false, error: '   ' },
    { success: false, error: { reason: 'Not safe to display' } },
    { success: false },
    null,
  ])('uses the fallback for a blank or missing display message: %j', error => {
    expect(messageForApiError(error, 'Try again.')).toBe('Try again.');
  });
});
