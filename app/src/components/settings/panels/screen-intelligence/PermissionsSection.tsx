import {
  fetchAccessibilityStatus,
  refreshPermissionsWithRestart,
  requestAccessibilityPermission,
} from '../../../../store/accessibilitySlice';
import { useAppDispatch } from '../../../../store/hooks';

interface PermissionsBadgeProps {
  label: string;
  value: string;
}

const PermissionBadge = ({ label, value }: PermissionsBadgeProps) => {
  const colorClass =
    value === 'granted'
      ? 'bg-green-50 text-green-700 border-green-200'
      : value === 'denied'
        ? 'bg-red-50 text-red-700 border-red-200'
        : 'bg-stone-100 text-stone-600 border-stone-200';

  return (
    <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-white p-3">
      <span className="text-sm text-stone-700">{label}</span>
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
  permissionCheckProcessPath: string | null | undefined;
  isRequestingPermissions: boolean;
  isRestartingCore: boolean;
  isLoading: boolean;
}

const PermissionsSection = ({
  screenRecording,
  accessibility,
  inputMonitoring,
  anyPermissionDenied,
  permissionCheckProcessPath,
  isRequestingPermissions,
  isRestartingCore,
  isLoading,
}: PermissionsSectionProps) => {
  const dispatch = useAppDispatch();

  return (
    <section className="space-y-3">
      <h3 className="text-sm font-semibold text-stone-900">Permissions</h3>
      <PermissionBadge label="Screen Recording" value={screenRecording} />
      <PermissionBadge label="Accessibility" value={accessibility} />
      <PermissionBadge label="Input Monitoring" value={inputMonitoring} />

      {anyPermissionDenied && (
        <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700 space-y-1">
          <p>
            After granting in System Settings, click &ldquo;Restart &amp; Refresh Permissions&rdquo;
            so a new core process picks up the grants.
          </p>
          {permissionCheckProcessPath ? (
            <p className="opacity-75 text-xs">
              macOS applies privacy to this executable:{' '}
              <span className="font-mono break-all text-stone-600">
                {permissionCheckProcessPath}
              </span>
            </p>
          ) : null}
        </div>
      )}

      <button
        type="button"
        onClick={() => void dispatch(requestAccessibilityPermission('screen_recording'))}
        disabled={isRequestingPermissions || isRestartingCore}
        className="mt-1 rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
        {isRequestingPermissions ? 'Requesting…' : 'Request Screen Recording'}
      </button>
      <button
        type="button"
        onClick={() => void dispatch(requestAccessibilityPermission('accessibility'))}
        disabled={isRequestingPermissions || isRestartingCore}
        className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
        {isRequestingPermissions ? 'Requesting…' : 'Request Accessibility'}
      </button>
      <button
        type="button"
        onClick={() => void dispatch(requestAccessibilityPermission('input_monitoring'))}
        disabled={isRequestingPermissions || isRestartingCore}
        className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
        {isRequestingPermissions ? 'Requesting…' : 'Open Input Monitoring'}
      </button>
      {anyPermissionDenied ? (
        <button
          type="button"
          onClick={() => void dispatch(refreshPermissionsWithRestart())}
          disabled={isRestartingCore || isLoading}
          className="rounded-lg border border-amber-400 bg-amber-50 px-3 py-2 text-sm text-amber-700 disabled:opacity-50">
          {isRestartingCore ? 'Restarting core…' : 'Restart & Refresh Permissions'}
        </button>
      ) : (
        <button
          type="button"
          onClick={() => void dispatch(fetchAccessibilityStatus())}
          disabled={isLoading || isRestartingCore}
          className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-2 text-sm text-stone-700 disabled:opacity-50">
          {isLoading ? 'Refreshing…' : 'Refresh Status'}
        </button>
      )}
    </section>
  );
};

export default PermissionsSection;
