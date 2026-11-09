/**
 * Connections → Desktop Control. Local desktop automation setup: a master
 * switch card with the live status chip, the OS permissions it needs, pending
 * desktop-action approvals, and an access check. Permission truth comes from
 * the core.
 */
import {
  Activity,
  Check,
  ExternalLink,
  type LucideIcon,
  Monitor,
  PersonStanding,
  RefreshCw,
  ScreenShare,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc, isLocalDesktopHost } from '../../services/coreRpcClient';
import { openUrl } from '../../utils/openUrl';
import SettingsTabbedPage from '../settings/layout/SettingsTabbedPage';
import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  Card,
  CenteredLoadingState,
  Field,
  Switch,
  TileGrid,
} from '../ui';
import { Spinner } from '../ui/icons';

export interface DesktopStatus {
  supported: boolean;
  enabled: boolean;
  platform: string;
  module_state: string;
  accessibility: string;
  screen_recording: string;
  jev_ready: boolean;
  approvals_enabled?: boolean;
  reason?: string;
}

interface DesktopProbe {
  ok: boolean;
  app_count?: number;
  reason?: string;
}

interface DesktopPending {
  confirmation_id: string;
  app: string;
  operation: string;
  target_name: string | null;
  action_summary: string;
  reason: string;
  expires_at: string;
  approved: boolean;
}

type PermissionKind = 'accessibility' | 'screen_recording';

function permissionSettingsUrl(kind: PermissionKind, platform: string): string | null {
  if (platform === 'macos') {
    return `x-apple.systempreferences:com.apple.preference.security?Privacy_${kind === 'accessibility' ? 'Accessibility' : 'ScreenCapture'}`;
  }
  // Windows has no per-app accessibility or screen-recording permission toggle
  // analogous to macOS TCC. Opening the generic Privacy settings page sends
  // the user to a dead end, so return null to suppress the button.
  return null;
}

export interface DesktopConnectionPageProps {
  /** Render only the body, for hosting inside the Computer panel's chip tabs. */
  embedded?: boolean;
}

export default function DesktopConnectionPage({
  embedded = false,
}: DesktopConnectionPageProps = {}) {
  const { t } = useT();
  const [localHost, setLocalHost] = useState<boolean | null>(null);
  const [status, setStatus] = useState<DesktopStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pendingError, setPendingError] = useState<string | null>(null);
  const [probe, setProbe] = useState<DesktopProbe | null>(null);
  const [pending, setPending] = useState<DesktopPending[]>([]);

  useEffect(() => {
    let active = true;
    void isLocalDesktopHost().then(local => {
      if (active) setLocalHost(local);
    });
    return () => {
      active = false;
    };
  }, []);

  const refreshPending = useCallback(async () => {
    try {
      const entries = await callCoreRpc<DesktopPending[]>({ method: 'openhuman.desktop_pending' });
      setPending(entries.filter(entry => !entry.approved));
      setPendingError(null);
    } catch {
      setPendingError(t('desktop.pendingUnavailable'));
    }
  }, [t]);

  const refresh = useCallback(async () => {
    if (localHost === null) return;
    setLoading(true);
    setError(null);
    try {
      setStatus(
        localHost
          ? await callCoreRpc<DesktopStatus>({ method: 'openhuman.desktop_status' })
          : {
              supported: false,
              enabled: false,
              platform: 'remote',
              module_state: 'unavailable',
              accessibility: 'not_required',
              screen_recording: 'not_required',
              jev_ready: false,
            }
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, [localHost]);

  useEffect(() => {
    void Promise.resolve().then(refresh);
  }, [refresh]);

  useEffect(() => {
    if (!status?.enabled || !status.approvals_enabled) return;
    void Promise.resolve().then(refreshPending);
    const timer = window.setInterval(() => void refreshPending(), 5000);
    return () => window.clearInterval(timer);
  }, [status?.enabled, status?.approvals_enabled, refreshPending]);

  const setEnabled = async () => {
    if (!localHost || !status || busy) return;
    setBusy(true);
    setError(null);
    setProbe(null);
    try {
      const next = await callCoreRpc<DesktopStatus>({
        method: 'openhuman.desktop_set_enabled',
        params: { enabled: !status.enabled },
      });
      setStatus(next);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const runProbe = async () => {
    setBusy(true);
    setError(null);
    setProbe(null);
    try {
      setProbe(await callCoreRpc<DesktopProbe>({ method: 'openhuman.desktop_probe' }));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const decide = async (confirmationId: string, approve: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await callCoreRpc({
        method: 'openhuman.desktop_confirm',
        params: { confirmation_id: confirmationId, approve },
      });
      await refreshPending();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const openSettings = async (kind: PermissionKind) => {
    if (!status) return;
    const url = permissionSettingsUrl(kind, status.platform);
    if (!url) return;
    try {
      await openUrl(url);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const permissionState = (value: string) =>
    value === 'granted'
      ? { variant: 'success' as const, label: t('desktop.permissionState.granted') }
      : value === 'not_required'
        ? { variant: 'success' as const, label: t('desktop.permissionState.notRequired') }
        : value === 'denied'
          ? { variant: 'danger' as const, label: t('desktop.permissionState.denied') }
          : { variant: 'neutral' as const, label: t('desktop.permissionState.unknown') };

  const permissionRow = (kind: PermissionKind, value: string) => {
    const Icon = kind === 'accessibility' ? PersonStanding : ScreenShare;
    const url = status && permissionSettingsUrl(kind, status.platform);
    const state = permissionState(value);
    const needsAction = value !== 'granted' && value !== 'not_required' && url;
    return (
      <div
        key={kind}
        className="flex flex-wrap items-center gap-3 px-4 py-3"
        data-testid={`desktop-permission-${kind}`}>
        <IconTile icon={Icon} />
        <p className="min-w-0 flex-1 text-sm font-medium text-content">
          {t(`desktop.permission.${kind}`)}
        </p>
        <Badge variant={state.variant}>{state.label}</Badge>
        {needsAction && (
          <Button
            variant="secondary"
            size="sm"
            trailingIcon={<ExternalLink className="h-3.5 w-3.5" aria-hidden />}
            onClick={() => void openSettings(kind)}>
            {t('desktop.openSettings')}
          </Button>
        )}
      </div>
    );
  };

  const ready =
    !!status &&
    status.enabled &&
    status.module_state === 'ready' &&
    (status.accessibility === 'granted' || status.accessibility === 'not_required') &&
    status.jev_ready;
  const statusBadge = status?.supported ? (
    <Badge
      variant={!status.enabled ? 'neutral' : ready ? 'success' : 'warning'}
      data-testid="desktop-status-badge">
      {!status.enabled
        ? t('common.disabled')
        : status.module_state === 'failed'
          ? t('desktop.statusModuleFailed')
          : ready
            ? t('channels.status.connected')
            : t('desktop.statusSetupNeeded')}
    </Badge>
  ) : null;

  const refreshButton = (
    <Button
      variant="secondary"
      size="sm"
      leadingIcon={<RefreshCw className="h-3.5 w-3.5" aria-hidden />}
      onClick={() => {
        void refresh();
        if (status?.enabled && status.approvals_enabled) void refreshPending();
      }}
      disabled={loading}>
      {t('common.refresh')}
    </Button>
  );

  const body = (
    <div className="space-y-4" data-testid="desktop-connection-page">
      {!embedded && (
        <Alert variant="warning" density="compact" role={undefined}>
          <AlertDescription>{t('connections.earlyAlphaNotice')}</AlertDescription>
        </Alert>
      )}

      {loading && !status && <CenteredLoadingState label={t('common.loading')} />}

      {status && !status.supported && (
        <Alert variant="info" density="compact">
          <AlertDescription>
            {t(
              status.platform === 'windows' ? 'desktop.unsupportedWindows' : 'desktop.unsupported'
            )}
          </AlertDescription>
        </Alert>
      )}

      {/* ── Master switch: on/off, where it stands, and why ────────────── */}
      {status?.supported && (
        <Card data-testid="desktop-status-card">
          <div className="flex items-center gap-3 p-4">
            <IconTile icon={Monitor} active={status.enabled} size="lg" />
            <div className="min-w-0 flex-1">
              <label
                htmlFor="desktop-enabled-switch"
                className="flex flex-wrap items-center gap-2 text-sm font-semibold text-content">
                {t('desktop.enableLabel')}
                {statusBadge}
              </label>
              <p className="mt-0.5 text-xs text-content-muted">{t('desktop.localOnly')}</p>
            </div>
            <Switch
              id="desktop-enabled-switch"
              checked={status.enabled}
              disabled={busy}
              onCheckedChange={() => void setEnabled()}
              aria-label={status.enabled ? t('common.disable') : t('common.enable')}
            />
          </div>
          {(status.reason || (status.enabled && !ready)) && (
            <div className="px-4 py-3 text-xs text-content-muted">
              {status.reason ??
                (status.module_state === 'failed'
                  ? t('desktop.moduleUnavailable')
                  : t('desktop.enabledPending'))}
            </div>
          )}
        </Card>
      )}

      {status?.supported && status.enabled && (
        <>
          {/* ── Pending approvals ──────────────────────────────────────── */}
          {status.approvals_enabled && pending.length > 0 && (
            <Card
              title={t('chat.approval.title')}
              headerRight={<Badge variant="warning">{pending.length}</Badge>}
              data-testid="desktop-pending-card">
              {pending.map(entry => (
                <div
                  key={entry.confirmation_id}
                  className="flex flex-wrap items-center gap-3 px-4 py-3">
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium text-content">
                      {entry.target_name
                        ? t('desktop.approvalSummary')
                            .replace(
                              '{operation}',
                              t(`desktop.action.${entry.operation.toLowerCase()}`, entry.operation)
                            )
                            .replace('{target}', entry.target_name)
                            .replace('{app}', entry.app)
                        : t('common.notAvailable')}
                    </p>
                    <p className="mt-0.5 text-xs text-content-muted">{entry.reason}</p>
                  </div>
                  <div className="flex shrink-0 gap-2">
                    <Button
                      size="sm"
                      variant="secondary"
                      leadingIcon={<X className="h-3.5 w-3.5" aria-hidden />}
                      disabled={busy}
                      onClick={() => void decide(entry.confirmation_id, false)}>
                      {t('chat.approval.deny')}
                    </Button>
                    <Button
                      size="sm"
                      leadingIcon={<Check className="h-3.5 w-3.5" aria-hidden />}
                      disabled={busy || !entry.action_summary || !entry.target_name}
                      onClick={() => void decide(entry.confirmation_id, true)}>
                      {t('chat.approval.approve')}
                    </Button>
                  </div>
                </div>
              ))}
            </Card>
          )}
        </>
      )}

      {/* Permissions and the access check are short cards: side by side. */}
      {status?.supported && (
        <TileGrid columns={2}>
          <Card
            title={t('desktop.permissions')}
            description={t('desktop.captureNote')}
            className="h-full">
            {permissionRow('accessibility', status.accessibility)}
            {permissionRow('screen_recording', status.screen_recording)}
          </Card>
          {/* ── Access check ───────────────────────────────────────── */}
          {status.enabled && (
            <Card title={t('desktop.checkHeading')} className="h-full">
              <Field
                label={t('desktop.checkLabel')}
                description={t('desktop.testDescription')}
                control={
                  <Button
                    variant="secondary"
                    size="sm"
                    leadingIcon={
                      busy ? <Spinner /> : <Activity className="h-3.5 w-3.5" aria-hidden />
                    }
                    onClick={() => void runProbe()}
                    disabled={busy}>
                    {t('desktop.testButton')}
                  </Button>
                }
              />
              {probe && (
                <div className="p-4">
                  <Alert variant={probe.ok ? 'success' : 'warning'} density="compact" role="status">
                    <AlertDescription>
                      {probe.ok
                        ? t('desktop.testPassed')
                        : (probe.reason ?? t('desktop.testFailed'))}
                    </AlertDescription>
                  </Alert>
                </div>
              )}
            </Card>
          )}
        </TileGrid>
      )}

      {error && (
        <Alert variant="destructive" density="compact">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      {pendingError && (
        <Alert variant="destructive" density="compact">
          <AlertDescription>{pendingError}</AlertDescription>
        </Alert>
      )}
    </div>
  );

  if (embedded) {
    return (
      <div className="space-y-4">
        {status?.supported && <div className="flex justify-end">{refreshButton}</div>}
        {body}
      </div>
    );
  }

  return (
    <SettingsTabbedPage
      title={t('desktop.title')}
      description={t('desktop.description')}
      headerAction={refreshButton}>
      {body}
    </SettingsTabbedPage>
  );
}

/** Square icon tile, filled with the primary colour when `active`. */
function IconTile({
  icon: Icon,
  active = false,
  size = 'md',
}: {
  icon: LucideIcon;
  active?: boolean;
  size?: 'md' | 'lg';
}) {
  return (
    <span
      className={cn(
        'flex shrink-0 items-center justify-center rounded-lg',
        size === 'lg' ? 'h-10 w-10' : 'h-8 w-8',
        active ? 'bg-primary-500 text-content-inverted' : 'bg-surface-muted text-content-secondary'
      )}>
      <Icon className={size === 'lg' ? 'h-5 w-5' : 'h-4 w-4'} aria-hidden />
    </span>
  );
}
