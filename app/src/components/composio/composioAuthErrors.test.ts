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

  it('keeps plain authorization messages intact', () => {
    expect(sanitizeAuthError('Permission denied')).toBe('Permission denied');
  });

  it('preserves ${...} placeholders in a readable message', () => {
    // The brace trim must not mistake the `{` in a placeholder for JSON, or
    // the guidance the user needs disappears.
    expect(sanitizeAuthError('{"error":"Set client_id=${REDDIT_CLIENT_ID} and retry."}')).toBe(
      'Set client_id=${REDDIT_CLIENT_ID} and retry.'
    );
  });

  it('strips backend URLs from extracted messages', () => {
    expect(
      sanitizeAuthError(
        '{"error":"Request failed for POST https://api.tinyhumans.ai/agent-integrations/composio/authorize"}'
      )
    ).toBe('Request failed for POST <backend>');
  });

  it('does not return serialized JSON from an extracted message', () => {
    expect(
      sanitizeAuthError('{"error":"{\\"code\\":500,\\"internal\\":\\"provider-debug\\"}"}')
    ).toBe('Authorization failed.');
  });

  it('does not surface an internal message from a 5xx body', () => {
    // A server-side failure is not actionable and its body can carry backend
    // internals, so it stays behind the raw-JSON guard.
    const err = new Error(
      'Authorization failed: Backend returned 500 Internal Server Error for POST <backend>/agent-integrations/composio/authorize: ' +
        '{"error":{"message":"internal server error payload","code":500}}'
    );
    const result = sanitizeAuthError(err);
    expect(result).not.toContain('internal server error payload');
    expect(result).not.toContain('"message"');
  });

  it('does not surface an internal message from an HTTP 5xx prefix body', () => {
    const err = new Error(
      'HTTP 500: {"error":{"message":"internal server error payload","code":500}}'
    );
    const result = sanitizeAuthError(err);
    expect(result).not.toContain('internal server error payload');
    expect(result).not.toContain('"message"');
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
