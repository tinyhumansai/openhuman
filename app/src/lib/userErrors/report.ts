/**
 * Ingestion helper for user-actionable runtime errors (#3931).
 *
 * Producers (chat runtime, RPC layer, …) hand raw error signals here; this
 * classifies them and, when the signal is a recognised expected-user-state,
 * dispatches it into the panel store. It is strictly additive and defensive:
 * it NEVER throws and returns `false` for non-actionable errors, so callers can
 * drop it into existing error paths without changing their behaviour.
 */
import debug from 'debug';

import type { AppDispatch } from '../../store';
import { reportUserError } from '../../store/userErrorsSlice';
import { classifyUserActionableError, type RuntimeErrorSignal } from './classify';

const log = debug('openhuman:user-errors');

/**
 * Classify `signal` and, if user-actionable, report it to the panel store.
 * @returns `true` if an actionable error was reported, else `false`.
 */
export function ingestRuntimeErrorSignal(
  dispatch: AppDispatch,
  signal: RuntimeErrorSignal
): boolean {
  try {
    const descriptor = classifyUserActionableError(signal);
    if (!descriptor) return false;
    // Metadata-only logging: stable prefix + kind/scope/provider, never the
    // raw provider message (may carry sanitized-but-noisy upstream text).
    log(
      'actionable kind=%s scope=%s provider=%s',
      descriptor.kind,
      descriptor.scope,
      descriptor.provider ?? '-'
    );
    dispatch(reportUserError({ descriptor, at: Date.now() }));
    return true;
  } catch (err) {
    log('ingest failed: %o', err);
    return false;
  }
}
