import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  fetchAccessibilityStatus,
  refreshPermissionsWithRestart,
  requestAccessibilityPermission,
} from '../../../store/accessibilitySlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';

interface ScreenPermissionsStepProps {
  onNext: (accessibilityPermissionGranted: boolean) => void;
  onBack?: () => void;
}

const ScreenPermissionsStep = ({ onNext, onBack: _onBack }: ScreenPermissionsStepProps) => {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const { status, isLoading, isRequestingPermissions, isRestartingCore, lastError } =
    useAppSelector(state => state.accessibility);

  useEffect(() => {
    void dispatch(fetchAccessibilityStatus());
  }, [dispatch]);

  const accessibilityPermission = status?.permissions.accessibility ?? 'unknown';
  const isGranted = accessibilityPermission === 'granted';

  return (
    <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2">Screen & Accessibility Permissions</h1>
        <p className="opacity-70 text-sm">
          OpenHuman uses information from your screen to constantly build context about your
          workflow and assist you with desktop actions.
        </p>
      </div>

      <div className="space-y-3 mb-5">
        <div className="rounded-2xl border border-sage-500/30 bg-sage-500/10 p-3">
          <p className="text-sm font-medium mb-1">Complete Privacy</p>
          <p className="text-xs opacity-80">
            All screenshots and accessibility information gets processed locally by your local AI
            model. No data is sent to any third party or cloud.
          </p>
        </div>
        <div className="rounded-2xl border border-stone-700 bg-stone-900 p-3">
          <p className="text-xs uppercase tracking-wide opacity-60 mb-2">
            Current permission state
          </p>
          <div className="flex items-center justify-between">
            <span className="text-sm">Accessibility</span>
            <span
              className={`text-xs px-2 py-1 rounded-md border ${
                isGranted
                  ? 'bg-sage-500/20 border-sage-500/30 text-sage-300'
                  : 'bg-amber-500/20 border-amber-500/30 text-amber-300'
              }`}>
              {accessibilityPermission}
            </span>
          </div>
        </div>
      </div>

      {!isGranted ? (
        <div className="space-y-2">
          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={() => void dispatch(requestAccessibilityPermission('accessibility'))}
              disabled={isRequestingPermissions || isLoading}
              className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-60">
              {isRequestingPermissions ? 'Requesting...' : 'Request Permissions'}
            </button>
            <button
              type="button"
              onClick={() => navigate('/settings/accessibility')}
              className="w-full py-2.5 text-sm font-medium rounded-xl border border-stone-600 hover:border-stone-500 transition-colors">
              Open Accessibility
            </button>
          </div>
          <button
            type="button"
            onClick={() => void dispatch(refreshPermissionsWithRestart())}
            disabled={isRestartingCore || isLoading}
            className="w-full py-2 text-sm font-medium rounded-xl border border-stone-700 hover:border-stone-500 opacity-70 hover:opacity-100 transition-all disabled:opacity-40">
            {isRestartingCore ? 'Restarting core...' : 'Refresh Status'}
          </button>
          {(lastError || status?.permission_check_process_path) && (
            <div className="text-xs text-stone-400 text-center px-2 space-y-1">
              {lastError ? <p className="text-coral-400">{lastError}</p> : null}
              {status?.permission_check_process_path ? (
                <p className="font-mono break-all text-stone-500">
                  Grant access for: {status.permission_check_process_path}
                </p>
              ) : null}
            </div>
          )}
        </div>
      ) : (
        <button
          onClick={() => onNext(isGranted)}
          className="w-full py-2.5 btn-primary text-sm font-medium rounded-xl border transition-colors border-stone-600 hover:border-sage-500 hover:bg-sage-500/10">
          Continue
        </button>
      )}
    </div>
  );
};

export default ScreenPermissionsStep;
