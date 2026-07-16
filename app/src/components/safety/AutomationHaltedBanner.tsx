import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { emergencyResume } from '../../services/api/emergencyApi';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { clearHalt, selectHalted, selectHaltReason } from '../../store/safetySlice';

/**
 * AutomationHaltedBanner — renders at the top of main content when automation
 * is halted via the emergency stop. Provides a Resume button to lift the halt.
 *
 * The Redux `clearHalt` only fires on a confirmed resume from the core. If the
 * `emergency_resume` RPC fails (timeout, auth, core unavailable), the halt is
 * preserved locally and a visible retry message is shown — because the core is
 * still halted and clearing the banner would silently re-enable the Stop button
 * while every external-effect action remained blocked. The authoritative source
 * of truth is the core; the `automation_halt` socket broadcast will also clear
 * the state if the resume succeeds server-side after an in-flight RPC failure.
 */
export function AutomationHaltedBanner() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const halted = useAppSelector(selectHalted);
  const reason = useAppSelector(selectHaltReason);
  const [resumeFailed, setResumeFailed] = useState(false);

  const onResume = useCallback(async () => {
    setResumeFailed(false);
    console.debug('[emergency] resume requested (source=user)');
    try {
      await emergencyResume();
      console.debug('[emergency] resume confirmed by core');
      // Only clear locally on a CONFIRMED resume. On failure the core is still
      // halted, so clearing here would give a false "safe to run" signal.
      dispatch(clearHalt());
    } catch (err) {
      console.error('[emergency] resume FAILED — halt preserved locally, retry required', err);
      setResumeFailed(true);
    }
  }, [dispatch]);

  // The banner is mounted permanently (it only returns null when not halted), so
  // `resumeFailed` would otherwise leak across halt cycles: a failed resume, then
  // an external socket-driven clear, then a fresh halt would show a stale "could
  // not resume" retry indicator the user never triggered. Reset it whenever the
  // halt lifts so each new cycle starts clean.
  useEffect(() => {
    if (!halted) setResumeFailed(false);
  }, [halted]);

  if (!halted) return null;

  // `sticky top-0 z-40` keeps the halt banner (and its Resume button) visible
  // and reachable ABOVE the provider WebviewHost overlay (absolute inset-0 z-30,
  // rendered as a sibling below); otherwise an active provider account fully
  // covers the safety banner. Stays below the settings modal portal (z-50),
  // matching the app's documented stacking convention.
  return (
    <div
      role="alert"
      data-analytics-id="automation-halted-banner"
      className="sticky top-0 z-40 flex items-center justify-between gap-3 px-4 py-2.5 bg-[var(--color-coral-50,#fdf2f2)] border-b border-[var(--color-coral-200,#f5c6c6)] text-[var(--color-coral-900,#7c2d2d)]">
      <div className="flex items-center gap-2 min-w-0">
        <strong className="shrink-0 font-semibold">{t('safety.haltedTitle')}</strong>
        <span className="truncate text-sm text-[var(--color-coral-700,#b94040)]">
          {reason || t('safety.haltedBody')}
        </span>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {resumeFailed && (
          <span
            role="status"
            aria-label={t('safety.resumeFailed')}
            data-analytics-id="emergency-resume-failed"
            className="rounded-md bg-[var(--color-coral-100,#fce8e8)] px-2 py-1 text-xs font-medium text-[var(--color-coral-800,#8f3a3a)]">
            {t('safety.resumeFailed')}
          </span>
        )}
        <button
          type="button"
          data-analytics-id="emergency-resume"
          onClick={() => void onResume()}
          className="rounded-md px-3 py-1 text-sm font-medium border border-[var(--color-coral-400,#d97373)] hover:bg-[var(--color-coral-100,#fce8e8)] transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-coral-500,#e05c5c)]">
          {t('safety.resume')}
        </button>
      </div>
    </div>
  );
}
