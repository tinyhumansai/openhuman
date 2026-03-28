import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  fetchAccessibilityStatus,
  requestAccessibilityPermission,
} from '../../../store/accessibilitySlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';

interface FeaturesStepProps {
  onNext: (accessibilityPermissionGranted: boolean) => void;
}

const FeaturesStep = ({ onNext }: FeaturesStepProps) => {
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
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2">Enable Accessibility Automation</h1>
        <p className="opacity-70 text-sm">
          Allow accessibility access so OpenHuman can assist with desktop workflows and guided
          actions.
        </p>
      </div>

      <div className="rounded-2xl border border-stone-700 bg-black/30 p-4 mb-4">
        <p className="text-xs uppercase tracking-wide opacity-60 mb-2">Current permission state</p>
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

      <div className="space-y-2 mb-4">
        <button
          onClick={() => void dispatch(requestAccessibilityPermission('accessibility'))}
          disabled={isRequestingPermissions || isLoading}
          className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl disabled:opacity-60">
          {isRequestingPermissions ? 'Requesting…' : 'Request Permission'}
        </button>
        <button
          onClick={() => navigate('/settings/accessibility')}
          className="w-full py-2.5 text-sm font-medium rounded-xl border border-stone-600 hover:border-stone-500 transition-colors">
          Open Accessibility Settings
        </button>
      </div>

      <button
        onClick={() => onNext(isGranted)}
        className="w-full py-2.5 text-sm font-medium rounded-xl bg-stone-800 hover:bg-stone-700 transition-colors">
        {isGranted ? 'Continue' : 'Continue Without Permission'}
      </button>
    </div>
  );
};

export default FeaturesStep;
