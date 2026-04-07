import { useEffect, useState } from 'react';

import {
  fetchAccessibilityStatus,
  refreshPermissionsWithRestart,
  requestAccessibilityPermission,
} from '../../../store/accessibilitySlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface ScreenPermissionsStepProps {
  onNext: (accessibilityPermissionGranted: boolean) => void;
  onBack?: () => void;
}

const ScreenPermissionsStep = ({ onNext, onBack: _onBack }: ScreenPermissionsStepProps) => {
  const dispatch = useAppDispatch();
  const { status, isLoading, isRequestingPermissions, isRestartingCore, lastError } =
    useAppSelector(state => state.accessibility);
  const [shouldAutoRefreshOnReturn, setShouldAutoRefreshOnReturn] = useState(false);

  useEffect(() => {
    void dispatch(fetchAccessibilityStatus());
  }, [dispatch]);

  const accessibilityPermission = status?.permissions.accessibility ?? 'unknown';
  const isGranted = accessibilityPermission === 'granted';

  useEffect(() => {
    if (!shouldAutoRefreshOnReturn) {
      return;
    }

    const refreshAfterReturn = () => {
      if (document.visibilityState !== 'visible' || isLoading || isRestartingCore || isGranted) {
        return;
      }

      setShouldAutoRefreshOnReturn(false);
      void dispatch(refreshPermissionsWithRestart());
    };

    window.addEventListener('focus', refreshAfterReturn);
    document.addEventListener('visibilitychange', refreshAfterReturn);

    return () => {
      window.removeEventListener('focus', refreshAfterReturn);
      document.removeEventListener('visibilitychange', refreshAfterReturn);
    };
  }, [dispatch, isGranted, isLoading, isRestartingCore, shouldAutoRefreshOnReturn]);

  useEffect(() => {
    if (isGranted && shouldAutoRefreshOnReturn) {
      setShouldAutoRefreshOnReturn(false);
    }
  }, [isGranted, shouldAutoRefreshOnReturn]);

  const handleRequestPermissions = () => {
    setShouldAutoRefreshOnReturn(true);
    void dispatch(requestAccessibilityPermission('accessibility'));
  };

  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2 text-stone-900">
          Screen & Accessibility Permissions
        </h1>
        <p className="text-stone-600 text-sm">
          OpenHuman uses information from your screen to constantly build context about your
          workflow and assist you with desktop actions.
        </p>
      </div>

      <div className="space-y-3 mb-5">
        <div className="rounded-2xl border border-stone-200 bg-stone-50 p-3">
          <p className="text-sm font-medium mb-1 text-stone-900">Complete Privacy</p>
          <p className="text-xs text-stone-600">
            All screenshots and accessibility information gets processed locally by your local AI
            model. No data is sent to any third party or cloud.
          </p>
        </div>
        <div className="rounded-2xl border border-stone-200 bg-white p-3">
          <p className="text-xs uppercase tracking-wide text-stone-400 mb-2">
            Current permission state
          </p>
          <div className="flex items-center justify-between">
            <span className="text-sm text-stone-900">Accessibility</span>
            <span
              className={`text-xs px-2 py-1 rounded-md border ${
                isGranted
                  ? 'bg-sage-50 border-sage-200 text-sage-600'
                  : 'bg-amber-50 border-amber-200 text-amber-600'
              }`}>
              {accessibilityPermission}
            </span>
          </div>
        </div>
      </div>

      {!isGranted && (
        <div className="space-y-2 mb-3">
          <button
            type="button"
            onClick={handleRequestPermissions}
            disabled={isRequestingPermissions || isLoading}
            className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-60">
            {isRequestingPermissions ? 'Requesting...' : 'Request Permissions'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(refreshPermissionsWithRestart())}
            disabled={isRestartingCore || isLoading}
            className="w-full py-2 text-sm font-medium rounded-xl border border-stone-200 hover:border-stone-400 text-stone-600 hover:text-stone-900 opacity-70 hover:opacity-100 transition-all disabled:opacity-40">
            {isRestartingCore ? 'Restarting core...' : 'Restart & Refresh Permissions'}
          </button>
          {(lastError || status?.permission_check_process_path) && (
            <div className="text-xs text-stone-400 text-center px-2 space-y-1">
              {shouldAutoRefreshOnReturn ? (
                <p>
                  After granting access in System Settings, return here and OpenHuman will refresh
                  automatically.
                </p>
              ) : null}
              {lastError ? <p className="text-coral-400">{lastError}</p> : null}
              {status?.permission_check_process_path ? (
                <p className="font-mono break-all text-stone-500">
                  Grant access for: {status.permission_check_process_path}
                </p>
              ) : null}
            </div>
          )}
        </div>
      )}

      <OnboardingNextButton onClick={() => onNext(isGranted)} />
    </div>
  );
};

export default ScreenPermissionsStep;
