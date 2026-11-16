import { describe, expect, it } from 'vitest';

import { isMissingRequiredFieldsError, sanitizeAuthError } from './composioAuthErrors';

describe('sanitizeAuthError — embedded JSON error bodies', () => {
  it('reads the message out of a prefixed 400 JSON body', () => {
    // The shape the bearer route produced once it stopped dropping the body:
    // the transport prefix, then the backend's own JSON envelope.
    const err = new Error(
      'Authorization failed: Backend returned 400 Bad Request for POST <backend>/agent-integrations/composio/authorize: ' +
        '{"success":false,"error":"No auth config found for toolkit \\"reddit\\""}'
    );
    expect(sanitizeAuthError(err)).toBe('No auth config found for toolkit "reddit"');
  });

  it('reads a raw JSON error string', () => {
    expect(sanitizeAuthError('{"error":"No auth config found for toolkit \\"reddit\\""}')).toBe(
      'No auth config found for toolkit "reddit"'
    );
  });

  it('reads a nested error.message', () => {
    expect(sanitizeAuthError('{"error":{"message":"Provider said no"}}')).toBe('Provider said no');
  });

  it('reads a top-level detail', () => {
    expect(sanitizeAuthError('{"detail":"Token expired"}')).toBe('Token expired');
  });

  it('still drops opaque JSON and falls back to the generic message', () => {
    // No readable field: the payload must not reach the UI.
    expect(sanitizeAuthError('{"foo":123}')).toBe('Authorization failed.');
    expect(sanitizeAuthError('Backend returned 400: {"foo":123}')).toBe('Authorization failed.');
  });

  it('falls back to the generic message on malformed JSON', () => {
    expect(sanitizeAuthError('Backend returned 400: {"error":')).toBe('Authorization failed.');
  });

  it('returns a safe fallback for null/undefined', () => {
    expect(sanitizeAuthError(null)).toBe('Something went wrong.');
    expect(sanitizeAuthError(undefined)).toBe('Something went wrong.');
  });

  it('keeps the missing-required-fields payload generic', () => {
    const err = new Error(
      'Backend returned 400 Bad Request: {"error":{"message":"Missing required fields","code":612,"slug":"ConnectedAccount_MissingRequiredFields"}}'
    );
    expect(isMissingRequiredFieldsError(err)).toBe(true);
    const result = sanitizeAuthError(err);
    expect(result).not.toContain('ConnectedAccount_MissingRequiredFields');
    expect(result).not.toContain('612');
    expect(result).toContain('required field');
  });
});
