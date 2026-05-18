import { useT } from '../../../../lib/i18n/I18nContext';
import type { AccessibilityPermissionKind } from '../../../../utils/tauriCommands';

interface PermissionsBadgeProps {
  label: string;
  value: string;
}

const PermissionBadge = ({ label, value }: PermissionsBadgeProps) => {
  const colorClass =
    value === 'granted'
      ? 'bg-green-50 dark:bg-green-500/10 text-green-700 dark:text-green-300 border-green-200 dark:border-green-500/30'
      : value === 'denied'
        ? 'bg-red-50 dark:bg-red-500/10 text-red-700 dark:text-red-300 border-red-200 dark:border-red-500/30'
        : 'bg-stone-100 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 border-stone-200 dark:border-neutral-800';

  return (
    <div className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3">
      <span className="text-sm text-stone-700 dark:text-neutral-200">{label}</span>
      <span className={`rounded-md border px-2 py-1 text-xs uppercase tracking-wide ${colorClass}`}>
        {value}
      </span>
    </div>
  );
};

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
    <section className="space-y-3">
      <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
        {t('settings.screenIntel.permissions.title')}
      </h3>
      <PermissionBadge
        label={t('settings.screenIntel.permissions.screenRecording')}
        value={screenRecording}
      />
      <PermissionBadge
        label={t('settings.screenIntel.permissions.accessibility')}
        value={accessibility}
      />
      <PermissionBadge
        label={t('settings.screenIntel.permissions.inputMonitoring')}
        value={inputMonitoring}
      />

      {anyPermissionDenied && (
        <div className="rounded-xl border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-300 space-y-1">
          <p>{t('settings.screenIntel.permissions.grantHint')}</p>
          {permissionCheckProcessPath ? (
            <p className="opacity-75 text-xs">
              {t('settings.screenIntel.permissions.macosAppliesPrivacy')}{' '}
              <span className="font-mono break-all text-stone-600 dark:text-neutral-300">
                {permissionCheckProcessPath}
              </span>
            </p>
          ) : null}
        </div>
      )}

      {lastRestartSummary ? (
        <div className="rounded-xl border border-green-300 dark:border-green-500/40 bg-green-50 dark:bg-green-500/10 p-3 text-sm text-green-700 dark:text-green-300">
          {lastRestartSummary}
        </div>
      ) : null}

      <button
        type="button"
        onClick={() => void requestPermission('screen_recording')}
        disabled={isRequestingPermissions || isRestartingCore}
        className="mt-1 rounded-lg border border-primary-400 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-sm text-primary-700 dark:text-primary-300 disabled:opacity-50">
        {isRequestingPermissions
          ? t('settings.screenIntel.permissions.requesting')
          : t('settings.screenIntel.permissions.requestScreenRecording')}
      </button>
      <button
        type="button"
        onClick={() => void requestPermission('accessibility')}
        disabled={isRequestingPermissions || isRestartingCore}
        className="rounded-lg border border-primary-400 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-sm text-primary-700 dark:text-primary-300 disabled:opacity-50">
        {isRequestingPermissions
          ? t('settings.screenIntel.permissions.requesting')
          : t('settings.screenIntel.permissions.requestAccessibility')}
      </button>
      <button
        type="button"
        onClick={() => void requestPermission('input_monitoring')}
        disabled={isRequestingPermissions || isRestartingCore}
        className="rounded-lg border border-primary-400 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-sm text-primary-700 dark:text-primary-300 disabled:opacity-50">
        {isRequestingPermissions
          ? t('settings.screenIntel.permissions.requesting')
          : t('settings.screenIntel.permissions.openInputMonitoring')}
      </button>
      {anyPermissionDenied ? (
        <button
          type="button"
          onClick={() => void refreshPermissionsWithRestart()}
          disabled={isRestartingCore || isLoading}
          className="rounded-lg border border-amber-400 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-300 disabled:opacity-50">
          {isRestartingCore
            ? t('settings.screenIntel.permissions.restartingCore')
            : t('settings.screenIntel.permissions.restartRefresh')}
        </button>
      ) : (
        <button
          type="button"
          onClick={() => void refreshStatus()}
          disabled={isLoading || isRestartingCore}
          className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2 text-sm text-stone-700 dark:text-neutral-200 disabled:opacity-50">
          {isLoading
            ? t('settings.screenIntel.permissions.refreshing')
            : t('settings.screenIntel.permissions.refreshStatus')}
        </button>
      )}
    </section>
  );
};

export default PermissionsSection;
