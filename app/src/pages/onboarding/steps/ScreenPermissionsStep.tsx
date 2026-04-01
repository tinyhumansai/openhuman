import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  fetchAccessibilityStatus,
  requestAccessibilityPermission,
} from '../../../store/accessibilitySlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';

interface ScreenPermissionsStepProps {
  onNext: (accessibilityPermissionGranted: boolean) => void;
  onBack?: () => void;
}

const ScreenPermissionsStep = ({ onNext, onBack }: ScreenPermissionsStepProps) => {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const { status, isLoading, isRequestingPermissions } = useAppSelector(
    state => state.accessibility
  );

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
            model. No data is sent to any third party.
          </p>
        </div>
        <div className="rounded-2xl border border-sage-500/30 bg-sage-500/10 p-3">
          <p className="text-sm font-medium mb-1">Absolutely Free</p>
          <p className="text-xs opacity-80">
            Processing uses your local AI model and hence remains free.
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

      <div className="grid grid-cols-2 gap-2 mb-4">
        <button
          type="button"
          onClick={() => void dispatch(requestAccessibilityPermission('accessibility'))}
          disabled={isRequestingPermissions || isLoading}
          className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-60">
          {isRequestingPermissions ? 'Requesting...' : 'Request Permission'}
        </button>
        <button
          type="button"
          onClick={() => navigate('/settings/accessibility')}
          className="w-full py-2.5 text-sm font-medium rounded-xl border border-stone-600 hover:border-stone-500 transition-colors">
          Open Accessibility
        </button>
      </div>

      <div className="flex gap-2">
        {onBack && (
          <button
            onClick={onBack}
            className="py-2.5 px-4 text-sm font-medium rounded-xl bg-stone-800 hover:bg-stone-700 transition-colors">
            Back
          </button>
        )}
        <button
          onClick={() => onNext(isGranted)}
          className="flex-1 py-2.5 text-sm font-medium rounded-xl bg-stone-800 hover:bg-stone-700 transition-colors">
          {isGranted ? 'Continue' : 'Continue Without Permission'}
        </button>
      </div>
    </div>
  );
};

export default ScreenPermissionsStep;
