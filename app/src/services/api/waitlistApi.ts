import createDebug from 'debug';

import type { ApiResponse } from '../../types/api';
import { apiClient } from '../apiClient';

const log = createDebug('waitlist:api');

/**
 * This runs while the app is opening, so it gets a short leash rather than the
 * client's two-minute default. Nothing downstream waits on the result.
 */
const CONFIRM_TIMEOUT_MS = 10_000;

/**
 * POST /waitlist/tasks/download/confirm — report that the desktop app was opened
 * from a tokenmaxxxing download link, which is what releases the download reward
 * on the waitlist entry that link belongs to.
 *
 * Unauthenticated on purpose. The download token *is* the credential here, and
 * whoever follows the link has usually never signed in — `requireAuth: false`
 * keeps the app's session bearer off a request that has no use for it, rather
 * than leaking it to an endpoint that does not expect one.
 *
 * Idempotent server-side, so opening the same link twice is harmless and a
 * failed attempt can simply be retried by opening it again.
 *
 * The response body is deliberately untyped beyond the envelope: the caller
 * needs to know only whether this succeeded, and pinning a shape that is still
 * being built would be a guess this module would then have to keep in step.
 */
export async function confirmWaitlistDownload(token: string): Promise<void> {
  // Length only — a download token is a credential and must not reach a log.
  log('confirming waitlist download tokenLength=%d', token.length);

  await apiClient.post<ApiResponse<unknown>>(
    '/waitlist/tasks/download/confirm',
    { token },
    { requireAuth: false, timeout: CONFIRM_TIMEOUT_MS }
  );
}
