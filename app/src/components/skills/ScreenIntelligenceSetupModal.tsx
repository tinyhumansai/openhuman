/**
 * Screen Intelligence setup/enable modal.
 *
 * Guides the user through permission grants, enables the feature,
 * and shows a success confirmation — matching the UX of third-party
 * skill setup flows (Gmail, etc.).
 */
import { useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useScreenIntelligenceState } from '../../features/screen-intelligence/useScreenIntelligenceState';
import { useT } from '../../lib/i18n/I18nContext';
import { openhumanUpdateScreenIntelligenceSettings } from '../../utils/tauriCommands';
import { settingsNavState } from '../settings/modal/settingsOverlay';
import { CheckIcon } from '../ui';
import Button from '../ui/Button';
import {
  SetupNotice,
  SetupSettingRow,
  SetupSuccess,
  SkillSetupModalShell,
} from './SkillSetupPrimitives';

// ─── Types ────────────────────────────────────────────────────────────────────

type Step = 'permissions' | 'enable' | 'success';

interface Props {
  onClose: () => void;
  /** Skip straight to manage mode when permissions are already granted. */
  initialStep?: Step;
}

// ─── Permission badge (reusable) ──────────────────────────────────────────────

const PermissionRow = ({
  label,
  value,
  onRequest,
  isRequesting,
}: {
  label: string;
  value: string;
  onRequest: () => void;
  isRequesting: boolean;
}) => {
  const { t } = useT();
  const granted = value === 'granted';
  const badgeColor = granted
    ? 'bg-sage-50 text-sage-700 border-sage-200'
    : value === 'denied'
      ? 'bg-coral-50 text-coral-700 border-coral-200'
      : 'bg-surface-subtle text-content-secondary border-line';

  return (
    <div className="flex items-center justify-between rounded-xl border border-line bg-surface px-3 py-2.5">
      <div className="flex items-center gap-2">
        {granted ? (
          <CheckIcon className="w-4 h-4 text-sage-500" />
        ) : (
          <svg
            className="w-4 h-4 text-content-faint"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <circle cx="12" cy="12" r="10" strokeWidth={2} />
          </svg>
        )}
        <span className="text-sm text-content-secondary">{label}</span>
      </div>
      {granted ? (
        <span
          className={`rounded-md border px-2 py-0.5 text-[10px] uppercase tracking-wide ${badgeColor}`}>
          {t('skills.setup.screenIntel.granted')}
        </span>
      ) : (
        <Button variant="secondary" size="xs" disabled={isRequesting} onClick={onRequest}>
          {isRequesting
            ? t('skills.setup.screenIntel.opening')
            : t('skills.setup.screenIntel.grant')}
        </Button>
      )}
    </div>
  );
};

// ─── Modal ────────────────────────────────────────────────────────────────────

export default function ScreenIntelligenceSetupModal({ onClose, initialStep }: Props) {
  const navigate = useNavigate();
  const location = useLocation();
  const { t } = useT();
  const {
    status,
    isRequestingPermissions,
    isRestartingCore,
    lastRestartSummary,
    lastError,
    requestPermission,
    refreshPermissionsWithRestart,
    refreshStatus,
  } = useScreenIntelligenceState({ loadVision: false });

  const [isEnabling, setIsEnabling] = useState(false);
  const [enableError, setEnableError] = useState<string | null>(null);

  const allGranted = useMemo(() => {
    if (!status) return false;
    return (
      status.permissions.screen_recording === 'granted' &&
      status.permissions.accessibility === 'granted' &&
      status.permissions.input_monitoring === 'granted'
    );
  }, [status]);

  const anyDenied = useMemo(() => {
    if (!status) return false;
    return (
      status.permissions.screen_recording === 'denied' ||
      status.permissions.accessibility === 'denied' ||
      status.permissions.input_monitoring === 'denied'
    );
  }, [status]);

  // Derive current step
  const [step, setStep] = useState<Step>(initialStep ?? 'permissions');

  // Auto-advance: when permissions are all granted, move past the permissions step
  useEffect(() => {
    if (step === 'permissions' && allGranted) {
      setStep('enable');
    }
  }, [step, allGranted]);

  const handleEnable = async () => {
    setIsEnabling(true);
    setEnableError(null);
    try {
      await openhumanUpdateScreenIntelligenceSettings({ enabled: true });
      await refreshStatus();
      setStep('success');
    } catch (error) {
      setEnableError(
        error instanceof Error ? error.message : t('skills.setup.screenIntel.enableError')
      );
    } finally {
      setIsEnabling(false);
    }
  };

  const handleGoToSettings = () => {
    onClose();
    navigate('/settings/screen-intelligence', settingsNavState(location));
  };

  if (status?.platform_supported === false) {
    return (
      <SkillSetupModalShell
        onClose={onClose}
        title={t('skills.setup.screenIntel.title')}
        titleId="si-setup-title"
        icon={
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M3 5h18v12H3zM8 21h8m-4-4v4"
            />
          </svg>
        }>
        <div className="space-y-4 py-2">
          <p className="text-sm text-content-secondary leading-relaxed">
            {t('skills.setup.screenIntel.macosOnly')}
          </p>
          <Button variant="secondary" size="lg" onClick={onClose} className="w-full">
            {t('common.close')}
          </Button>
        </div>
      </SkillSetupModalShell>
    );
  }

  return (
    <SkillSetupModalShell
      onClose={onClose}
      title={t('skills.setup.screenIntel.title')}
      titleId="si-setup-title"
      subtitle={
        <>
          {step === 'permissions' && t('skills.setup.screenIntel.stepPermissions')}
          {step === 'enable' && t('skills.setup.screenIntel.stepEnable')}
          {step === 'success' && t('skills.setup.screenIntel.stepSuccess')}
        </>
      }
      icon={
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M3 5h18v12H3zM8 21h8m-4-4v4"
          />
        </svg>
      }>
      {/* ─── Step 1: Permissions ─── */}
      {step === 'permissions' && (
        <div className="space-y-3">
          <p className="text-xs text-content-muted leading-relaxed">
            {t('skills.setup.screenIntel.permissionsDesc')}
          </p>

          <div className="space-y-2">
            <PermissionRow
              label={t('skills.setup.screenIntel.permScreenRecording')}
              value={status?.permissions.screen_recording ?? 'unknown'}
              onRequest={() => void requestPermission('screen_recording')}
              isRequesting={isRequestingPermissions}
            />
            <PermissionRow
              label={t('skills.setup.screenIntel.permAccessibility')}
              value={status?.permissions.accessibility ?? 'unknown'}
              onRequest={() => void requestPermission('accessibility')}
              isRequesting={isRequestingPermissions}
            />
            <PermissionRow
              label={t('skills.setup.screenIntel.permInputMonitoring')}
              value={status?.permissions.input_monitoring ?? 'unknown'}
              onRequest={() => void requestPermission('input_monitoring')}
              isRequesting={isRequestingPermissions}
            />
          </div>

          {anyDenied && (
            <SetupNotice tone="amber" className="leading-relaxed">
              <p>{t('skills.setup.screenIntel.deniedHint')}</p>
              {status?.permission_check_process_path && (
                <p className="mt-1 opacity-75 text-[10px]">
                  {t('skills.setup.screenIntel.permissionPathLabel')}{' '}
                  <span className="font-mono break-all text-content-secondary">
                    {status.permission_check_process_path}
                  </span>
                </p>
              )}
            </SetupNotice>
          )}

          {lastRestartSummary && <SetupNotice tone="sage">{lastRestartSummary}</SetupNotice>}

          {lastError && <SetupNotice tone="coral">{lastError}</SetupNotice>}

          <div className="flex items-center gap-2 pt-1">
            {anyDenied ? (
              <button
                type="button"
                onClick={() => void refreshPermissionsWithRestart()}
                disabled={isRestartingCore}
                className="flex-1 rounded-xl border border-amber-300 bg-amber-50 px-3 py-2.5 text-sm font-medium text-amber-700 hover:bg-amber-100 disabled:opacity-50 transition-colors">
                {isRestartingCore
                  ? t('skills.setup.screenIntel.restarting')
                  : t('skills.setup.screenIntel.restartRefresh')}
              </button>
            ) : (
              <Button
                variant="secondary"
                size="lg"
                onClick={() => void refreshStatus()}
                disabled={isRestartingCore}
                className="flex-1">
                {t('skills.setup.screenIntel.refreshStatus')}
              </Button>
            )}
          </div>
        </div>
      )}

      {/* ─── Step 2: Enable ─── */}
      {step === 'enable' && (
        <div className="space-y-4">
          <SetupNotice tone="sage" icon={<CheckIcon className="w-4 h-4 text-sage-500" />}>
            {t('skills.setup.screenIntel.allGranted')}
          </SetupNotice>

          <p className="text-xs text-content-muted leading-relaxed">
            {t('skills.setup.screenIntel.enableDesc')}
          </p>

          <div className="space-y-2">
            <SetupSettingRow
              label={t('skills.setup.screenIntel.captureMode')}
              value={t('skills.setup.screenIntel.captureModeValue')}
            />
            <SetupSettingRow
              label={t('skills.setup.screenIntel.visionModel')}
              value={t('common.enabled')}
            />
            <SetupSettingRow
              label={t('skills.setup.screenIntel.panicHotkey')}
              value={status?.session.panic_hotkey ?? 'Cmd+Shift+.'}
              mono
            />
          </div>

          {enableError && <SetupNotice tone="coral">{enableError}</SetupNotice>}

          <Button
            variant="primary"
            size="lg"
            onClick={() => void handleEnable()}
            disabled={isEnabling}
            className="w-full">
            {isEnabling
              ? t('skills.setup.screenIntel.enabling')
              : t('skills.setup.screenIntel.enableBtn')}
          </Button>
        </div>
      )}

      {/* ─── Step 3: Success ─── */}
      {step === 'success' && (
        <SetupSuccess
          title={t('skills.setup.screenIntel.activeTitle')}
          description={t('skills.setup.screenIntel.activeDesc')}
          settingsLabel={t('skills.setup.screenIntel.advancedSettings')}
          finishLabel={t('common.finish')}
          onSettings={handleGoToSettings}
          onFinish={onClose}
        />
      )}
    </SkillSetupModalShell>
  );
}
