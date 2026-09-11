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
import { reportUserError, resolveUserError } from '../../store/userErrorsSlice';
import {
  classifyMemoryPipelineFailure,
  classifyMemoryQuarantine,
  classifyUserActionableError,
  type RuntimeErrorSignal,
  userErrorId,
} from './classify';

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

/**
 * #5324: promote the memory pipeline's typed blocking cause into the panel.
 *
 * Called from the Memory Tree status poll. The store dedupes on the
 * descriptor id, so re-reporting the same cause on every poll bumps the
 * recurrence count instead of stacking duplicate entries — which is what
 * makes it safe to call unconditionally from a polling loop.
 *
 * Same defensive contract as {@link ingestRuntimeErrorSignal}: never throws,
 * returns `false` for causes that are not user-actionable.
 *
 * @param failureCode `first_blocking_cause.code` from the status payload.
 */
export function reportMemoryPipelineFailure(
  dispatch: AppDispatch,
  failureCode: string | null | undefined
): boolean {
  try {
    const descriptor = classifyMemoryPipelineFailure(failureCode);
    if (!descriptor) return false;
    log('memory pipeline actionable kind=%s', descriptor.kind);
    dispatch(reportUserError({ descriptor, at: Date.now() }));
    return true;
  } catch (err) {
    log('memory pipeline ingest failed: %o', err);
    return false;
  }
}

/**
 * Replay a quarantine from the pipeline-status poll into the durable
 * NoticeCenter, and retire it once the store has been re-synced
 * (openhuman#5820). Never throws; returns whether a notice is active.
 */
export function reportMemoryQuarantine(
  dispatch: AppDispatch,
  quarantine: { resynced: boolean } | null | undefined
): boolean {
  try {
    const descriptor = classifyMemoryQuarantine(quarantine);
    if (descriptor) {
      log('memory quarantine active kind=%s', descriptor.kind);
      dispatch(reportUserError({ descriptor, at: Date.now() }));
      return true;
    }
    if (quarantine?.resynced) {
      dispatch(
        resolveUserError({ id: userErrorId('memory_store_corrupt', 'memory'), at: Date.now() })
      );
    }
    return false;
  } catch (err) {
    log('memory quarantine ingest failed: %o', err);
    return false;
  }
}
