import { useCallback } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { emergencyResume } from '../../services/api/emergencyApi';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { clearHalt, selectHalted, selectHaltReason } from '../../store/safetySlice';

/**
 * AutomationHaltedBanner — renders at the top of main content when automation
 * is halted via the emergency stop. Provides a Resume button to lift the halt.
 *
 * The `finally` block in `onResume` ensures the UI clears the halt locally even
 * if the core resume RPC fails, so the user is never stuck in a halted state
 * they cannot escape from without a restart.
 */
export function AutomationHaltedBanner() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const halted = useAppSelector(selectHalted);
  const reason = useAppSelector(selectHaltReason);

  const onResume = useCallback(async () => {
    console.debug('[emergency] resume requested (source=user)');
    try {
      await emergencyResume();
      console.debug('[emergency] resume confirmed by core');
    } catch (err) {
      console.error('[emergency] resume failed — clearing halt locally anyway', err);
    } finally {
      dispatch(clearHalt());
    }
  }, [dispatch]);

  if (!halted) return null;

  return (
    <div
      role="alert"
      data-analytics-id="automation-halted-banner"
      className="flex items-center justify-between gap-3 px-4 py-2.5 bg-[var(--color-coral-50,#fdf2f2)] border-b border-[var(--color-coral-200,#f5c6c6)] text-[var(--color-coral-900,#7c2d2d)]">
      <div className="flex items-center gap-2 min-w-0">
        <strong className="shrink-0 font-semibold">{t('safety.haltedTitle')}</strong>
        <span className="truncate text-sm text-[var(--color-coral-700,#b94040)]">
          {reason ?? t('safety.haltedBody')}
        </span>
      </div>
      <button
        type="button"
        data-analytics-id="emergency-resume"
        onClick={() => void onResume()}
        className="shrink-0 rounded-md px-3 py-1 text-sm font-medium border border-[var(--color-coral-400,#d97373)] hover:bg-[var(--color-coral-100,#fce8e8)] transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-coral-500,#e05c5c)]">
        {t('safety.resume')}
      </button>
    </div>
  );
}
