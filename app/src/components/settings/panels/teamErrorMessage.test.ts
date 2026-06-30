import { describe, expect, it } from 'vitest';

import { CoreRpcError } from '../../../services/coreRpcClient';
import { teamErrorMessage } from './teamErrorMessage';

const FALLBACK = 'Something went wrong';

describe('teamErrorMessage', () => {
  it('prefers a structured reason from CoreRpcError.data', () => {
    const err = new CoreRpcError('GET /teams failed (403 Forbidden)', 'unknown', undefined, {
      message: 'Not a member of this team',
    });
    expect(teamErrorMessage(err, FALLBACK)).toBe('Not a member of this team');
  });

  it('lifts the human field out of a JSON body in CoreRpcError.message', () => {
    const err = new CoreRpcError(
      'POST /teams/join failed (400 Bad Request): {"error":"Invite expired"}',
      'unknown'
    );
    expect(teamErrorMessage(err, FALLBACK)).toBe('Invite expired');
  });

  it('returns a clean message when there is no RPC prefix', () => {
    const err = new CoreRpcError('Session expired. Please log in again.', 'auth_expired');
    expect(teamErrorMessage(err, FALLBACK)).toBe('Session expired. Please log in again.');
  });

  it('falls back when the body is a raw HTML error page', () => {
    const err = new CoreRpcError(
      'POST /teams failed (404 Not Found): <!DOCTYPE html><html><body>Not Found</body></html>',
      'unknown'
    );
    expect(teamErrorMessage(err, FALLBACK)).toBe(FALLBACK);
  });

  it('falls back when a JSON body has no human-readable field', () => {
    const err = new CoreRpcError(
      'POST /teams/join failed (400 Bad Request): {"code":"E_BAD"}',
      'unknown'
    );
    expect(teamErrorMessage(err, FALLBACK)).toBe(FALLBACK);
  });

  it('falls back when nothing remains after the RPC prefix', () => {
    const err = new CoreRpcError(
      'DELETE /teams/t1 failed (500 Internal Server Error): ',
      'unknown'
    );
    expect(teamErrorMessage(err, FALLBACK)).toBe(FALLBACK);
  });

  it('handles the legacy plain { error } rejection shape', () => {
    expect(teamErrorMessage({ error: 'Team limit reached' }, FALLBACK)).toBe('Team limit reached');
  });

  it('surfaces a bare Error message', () => {
    expect(teamErrorMessage(new Error('boom'), FALLBACK)).toBe('boom');
  });

  it('falls back for non-object rejections', () => {
    expect(teamErrorMessage(null, FALLBACK)).toBe(FALLBACK);
    expect(teamErrorMessage(undefined, FALLBACK)).toBe(FALLBACK);
    expect(teamErrorMessage('nope', FALLBACK)).toBe(FALLBACK);
  });

  it('caps an overly long reason', () => {
    const long = 'x'.repeat(500);
    const out = teamErrorMessage({ error: long }, FALLBACK);
    expect(out.length).toBeLessThanOrEqual(200);
    expect(out.endsWith('…')).toBe(true);
  });
});
