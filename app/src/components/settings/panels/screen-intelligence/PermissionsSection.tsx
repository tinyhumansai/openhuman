import { useT } from '../../../../lib/i18n/I18nContext';
import type { AccessibilityPermissionKind } from '../../../../utils/tauriCommands';
import Button from '../../../ui/Button';
import {
  SettingsBadge,
  type SettingsBadgeVariant,
  SettingsRow,
  SettingsSection,
} from '../../controls';

const badgeVariant = (value: string): SettingsBadgeVariant =>
  value === 'granted' ? 'success' : value === 'denied' ? 'danger' : 'neutral';

interface PermissionsSectionProps {
  screenRecording: string;
  accessibility: string;
  inputMonitoring: string;
  anyPermissionDenied: boolean;
  lastRestartSummary: string | null;
  permissionCheckProcessPath: string | null | undefined;
  isRequestingPermissions: boolean;
  isRestartingCore: boolean;
  isLoading: boolean;
  requestPermission: (permission: AccessibilityPermissionKind) => Promise<unknown>;
  refreshPermissionsWithRestart: () => Promise<unknown>;
  refreshStatus: () => Promise<unknown>;
}

const PermissionsSection = ({
  screenRecording,
  accessibility,
  inputMonitoring,
  anyPermissionDenied,
  lastRestartSummary,
  permissionCheckProcessPath,
  isRequestingPermissions,
  isRestartingCore,
  isLoading,
  requestPermission,
  refreshPermissionsWithRestart,
  refreshStatus,
}: PermissionsSectionProps) => {
  const { t } = useT();
  return (
    <SettingsSection title={t('settings.screenIntel.permissions.title')}>
      <SettingsRow
        label={t('settings.screenIntel.permissions.screenRecording')}
        control={
          <SettingsBadge variant={badgeVariant(screenRecording)}>{screenRecording}</SettingsBadge>
        }
      />
      <SettingsRow
        label={t('settings.screenIntel.permissions.accessibility')}
        control={
          <SettingsBadge variant={badgeVariant(accessibility)}>{accessibility}</SettingsBadge>
        }
      />
      <SettingsRow
        label={t('settings.screenIntel.permissions.inputMonitoring')}
        control={
          <SettingsBadge variant={badgeVariant(inputMonitoring)}>{inputMonitoring}</SettingsBadge>
        }
      />

      {anyPermissionDenied && (
        <div className="px-4 py-3">
          <div className="space-y-1 rounded-xl border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-300">
            <p>{t('settings.screenIntel.permissions.grantHint')}</p>
            {permissionCheckProcessPath ? (
              <p className="text-xs opacity-75">
                {t('settings.screenIntel.permissions.macosAppliesPrivacy')}{' '}
                <span className="break-all font-mono text-content-secondary">
                  {permissionCheckProcessPath}
                </span>
              </p>
            ) : null}
          </div>
        </div>
      )}

      {lastRestartSummary ? (
        <div className="px-4 py-3">
          <div className="rounded-xl border border-green-300 dark:border-green-500/40 bg-green-50 dark:bg-green-500/10 p-3 text-sm text-green-700 dark:text-green-300">
            {lastRestartSummary}
          </div>
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2 px-4 py-3">
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void requestPermission('screen_recording')}
          disabled={isRequestingPermissions || isRestartingCore}>
          {isRequestingPermissions
            ? t('settings.screenIntel.permissions.requesting')
            : t('settings.screenIntel.permissions.requestScreenRecording')}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void requestPermission('accessibility')}
          disabled={isRequestingPermissions || isRestartingCore}>
          {isRequestingPermissions
            ? t('settings.screenIntel.permissions.requesting')
            : t('settings.screenIntel.permissions.requestAccessibility')}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void requestPermission('input_monitoring')}
          disabled={isRequestingPermissions || isRestartingCore}>
          {isRequestingPermissions
            ? t('settings.screenIntel.permissions.requesting')
            : t('settings.screenIntel.permissions.openInputMonitoring')}
        </Button>
        {anyPermissionDenied ? (
          <Button
            variant="secondary"
            tone="danger"
            size="sm"
            onClick={() => void refreshPermissionsWithRestart()}
            disabled={isRestartingCore || isLoading}>
            {isRestartingCore
              ? t('settings.screenIntel.permissions.restartingCore')
              : t('settings.screenIntel.permissions.restartRefresh')}
          </Button>
        ) : (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void refreshStatus()}
            disabled={isLoading || isRestartingCore}>
            {isLoading
              ? t('settings.screenIntel.permissions.refreshing')
              : t('settings.screenIntel.permissions.refreshStatus')}
          </Button>
        )}
      </div>
    </SettingsSection>
  );
};

export default PermissionsSection;
