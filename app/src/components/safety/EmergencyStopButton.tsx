import { useCallback } from 'react';
import { useDispatch, useSelector } from 'react-redux';

import { useT } from '../../lib/i18n/I18nContext';
import { emergencyStop } from '../../services/api/emergencyApi';
import { selectHalted, setHalt } from '../../store/safetySlice';

/**
 * Emergency Stop button — always-visible safety control that halts all desktop
 * automation immediately. On click it calls the core `emergency_stop` RPC and
 * reflects the halt in the Redux safety slice. If the RPC fails the halt is
 * committed locally anyway so the user always sees a response to their action.
 *
 * Hidden while automation is already halted: the `AutomationHaltedBanner`'s
 * Resume control takes over, so Stop and Resume are never shown at once.
 */
export function EmergencyStopButton() {
  const { t } = useT();
  const dispatch = useDispatch();
  const halted = useSelector(selectHalted);

  const handleClick = useCallback(async () => {
    try {
      const state = await emergencyStop();
      dispatch(setHalt({ reason: state.reason, source: state.source, since: state.engaged_at_ms }));
    } catch (err) {
      // Fail-visible: reflect intent locally even when the core is unreachable.
      dispatch(setHalt({ source: 'user' }));
      console.error('[emergency] stop failed', err);
    }
  }, [dispatch]);

  // Already halted → the halt banner (with Resume) is the active control.
  if (halted) return null;

  return (
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
  );
}
