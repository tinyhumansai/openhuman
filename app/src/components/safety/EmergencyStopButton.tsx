import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { emergencyStop } from '../../services/api/emergencyApi';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { selectHalted, setHalt } from '../../store/safetySlice';

/**
 * Emergency Stop button — always-visible safety control that halts all desktop
 * automation immediately. On click it calls the core `emergency_stop` RPC and
 * reflects the halt in the Redux safety slice.
 *
 * On RPC failure it does NOT mark the halt locally (that would falsely signal a
 * stop that did not happen) — instead it surfaces a visible, retryable error so
 * the operator knows the kill switch did not engage.
 *
 * Hidden while automation is already halted: the `AutomationHaltedBanner`'s
 * Resume control takes over, so Stop and Resume are never shown at once.
 */
export function EmergencyStopButton() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const halted = useAppSelector(selectHalted);
  const [failed, setFailed] = useState(false);

  const handleClick = useCallback(async () => {
    setFailed(false);
    console.debug('[emergency] stop requested (source=user)');
    try {
      const state = await emergencyStop();
      console.debug('[emergency] stop confirmed by core', {
        engaged: state.engaged,
        source: state.source,
      });
      dispatch(setHalt({ reason: state.reason, source: state.source, since: state.engaged_at_ms }));
    } catch (err) {
      // Do NOT mark halted locally on failure: if the RPC did not succeed the
      // core is not actually halted, and showing the halted banner would give a
      // false sense of safety. Surface a visible, retryable error instead so the
      // operator knows the stop did not go through; a confirmed halt only
      // appears from a successful response or the `automation_halt` broadcast.
      setFailed(true);
      console.error('[emergency] stop FAILED — core NOT halted, retry required', err);
    }
  }, [dispatch]);

  // Already halted → the halt banner (with Resume) is the active control.
  if (halted) return null;

  return (
    <div className="flex items-center gap-2">
      {failed && (
        <span
          role="alert"
          data-analytics-id="emergency-stop-failed"
          className="rounded-md bg-[var(--color-coral-50,#fdf2f2)] px-2 py-1 text-xs font-medium text-[var(--color-coral-800,#8f3a3a)] shadow-sm">
          {t('safety.stopFailed')}
        </span>
      )}
      <button
        type="button"
        data-analytics-id="emergency-stop"
        onClick={() => void handleClick()}
        className="flex items-center gap-1.5 rounded-full px-3 py-1.5 text-sm font-semibold shadow-md bg-[var(--color-coral-500,#e05c5c)] text-white hover:bg-[var(--color-coral-600,#c94f4f)] transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-coral-500,#e05c5c)]"
        aria-label={t('safety.emergencyStop')}>
        <svg aria-hidden="true" viewBox="0 0 16 16" className="h-3 w-3" fill="currentColor">
          <rect x="3" y="3" width="10" height="10" rx="2" />
        </svg>
        {t('safety.emergencyStop')}
      </button>
    </div>
  );
}
