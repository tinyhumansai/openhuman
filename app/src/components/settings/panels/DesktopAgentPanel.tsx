import { useCallback, useEffect, useState } from 'react';

import {
  fetchScreenIntelligenceStatus,
  refreshScreenIntelligencePermissionsWithRestart,
  requestScreenIntelligencePermission,
} from '../../../features/screen-intelligence/api';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AccessibilityPermissionKind,
  type AccessibilityPermissionState,
  type AccessibilityStatus,
  openhumanGetAutonomySettings,
  openhumanGetVoiceServerSettings,
  openhumanUpdateAutonomySettings,
  openhumanUpdateVoiceServerSettings,
  syncNotchVisibility,
} from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import {
  SettingsBadge,
  type SettingsBadgeVariant,
  SettingsRow,
  SettingsSection,
  SettingsSwitch,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

/**
 * Desktop Agent setup panel — Settings → Features → Desktop Agent.
 *
 * Two things in one place, no new backend:
 *  1. Check + grant the four OS permissions the desktop agent needs (Microphone,
 *     Accessibility, Screen Recording, Input Monitoring) — reuses the existing
 *     screen-intelligence permission RPCs.
 *  2. "Let the agent act without asking" — a seamless-mode toggle that grants Full
 *     access and auto-approves the desktop-control tools so the agent runs them
 *     without an in-app approval card (reuses the autonomy RPCs).
 */

/**
 * The four permissions the desktop agent depends on, in setup order.
 *
 * Accessibility / Screen Recording / Input Monitoring intentionally reuse the
 * existing `settings.screenIntel.permissions.*` labels (same OS permissions as the
 * Screen Intelligence panel) to avoid duplicate translations; only Microphone needs
 * a desktop-agent-specific key.
 */
const PERMISSIONS: ReadonlyArray<{ kind: AccessibilityPermissionKind; labelKey: string }> = [
  { kind: 'microphone', labelKey: 'settings.desktopAgent.microphone' },
  { kind: 'accessibility', labelKey: 'settings.screenIntel.permissions.accessibility' },
  { kind: 'screen_recording', labelKey: 'settings.screenIntel.permissions.screenRecording' },
  { kind: 'input_monitoring', labelKey: 'settings.screenIntel.permissions.inputMonitoring' },
];

/**
 * Desktop-control tools auto-approved by seamless mode so the agent actuates apps
 * without an approval card. Deliberately excludes shell / file / network / install
 * tools — those keep prompting. Matched by exact tool name in `autonomy.auto_approve`.
 */
const SEAMLESS_TOOLS = ['automate', 'ax_interact', 'launch_app', 'keyboard', 'mouse'] as const;

const STATE_VARIANT: Record<AccessibilityPermissionState, SettingsBadgeVariant> = {
  granted: 'success',
  denied: 'danger',
  unknown: 'warning',
  unsupported: 'neutral',
};

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const stateFor = (
  status: AccessibilityStatus | null,
  kind: AccessibilityPermissionKind
): AccessibilityPermissionState => status?.permissions?.[kind] ?? 'unknown';

const DesktopAgentPanel = () => {
  const { t } = useT();

  const [status, setStatus] = useState<AccessibilityStatus | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [requestingKind, setRequestingKind] = useState<AccessibilityPermissionKind | null>(null);
  const [isRestarting, setIsRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restartSummary, setRestartSummary] = useState<string | null>(null);

  // Seamless mode (auto-approve allowlist). Null until the autonomy settings load.
  const [autoApprove, setAutoApprove] = useState<string[] | null>(null);
  const [isUpdatingSeamless, setIsUpdatingSeamless] = useState(false);

  // Always-on listening (relocated from the Voice panel). Null until loaded.
  const [alwaysOn, setAlwaysOn] = useState<boolean | null>(null);
  const [isUpdatingAlwaysOn, setIsUpdatingAlwaysOn] = useState(false);

  const refresh = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      setStatus(await fetchScreenIntelligenceStatus());
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setIsLoading(false);
    }
  }, []);

  const loadAutonomy = useCallback(async () => {
    setError(null);
    try {
      const resp = await openhumanGetAutonomySettings();
      setAutoApprove(resp.result.auto_approve ?? []);
    } catch (err) {
      setError(errorMessage(err));
    }
  }, []);

  const loadVoice = useCallback(async () => {
    setError(null);
    try {
      const resp = await openhumanGetVoiceServerSettings();
      setAlwaysOn(resp.result.always_on_enabled);
    } catch (err) {
      setError(errorMessage(err));
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh();
    void loadAutonomy();
    void loadVoice();
  }, [refresh, loadAutonomy, loadVoice]);

  const toggleAlwaysOn = useCallback(
    async (next: boolean) => {
      // Guard against rapid re-toggles racing and committing out of order.
      if (isUpdatingAlwaysOn) return;
      const previous = alwaysOn;
      setIsUpdatingAlwaysOn(true);
      setAlwaysOn(next); // optimistic flip
      setError(null);
      try {
        await openhumanUpdateVoiceServerSettings({ always_on_enabled: next });
      } catch (err) {
        // Persist failed — revert the optimistic flip and surface the error.
        setAlwaysOn(previous);
        setError(errorMessage(err));
        setIsUpdatingAlwaysOn(false);
        return;
      }
      try {
        // The notch pill is the always-on listening HUD: show on, hide off.
        // Persistence already succeeded, so keep the UI state even if this fails.
        await syncNotchVisibility(next);
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        setIsUpdatingAlwaysOn(false);
      }
    },
    [alwaysOn, isUpdatingAlwaysOn]
  );

  const seamlessOn =
    autoApprove !== null && SEAMLESS_TOOLS.every(tool => autoApprove.includes(tool));

  const toggleSeamless = useCallback(
    async (next: boolean) => {
      const current = autoApprove ?? [];
      setIsUpdatingSeamless(true);
      setError(null);
      try {
        if (next) {
          // Grant Full access (satisfies app_control_enabled) + auto-approve the
          // desktop tools + drop the task-plan approval prompt.
          const merged = Array.from(new Set([...current, ...SEAMLESS_TOOLS]));
          await openhumanUpdateAutonomySettings({
            level: 'full',
            require_task_plan_approval: false,
            auto_approve: merged,
          });
        } else {
          // Remove only the desktop tools + restore the plan-approval prompt. Leave
          // the autonomy tier as-is (managed in Settings → Agent Access).
          const seamlessSet = new Set<string>(SEAMLESS_TOOLS);
          const pruned = current.filter(tool => !seamlessSet.has(tool));
          await openhumanUpdateAutonomySettings({
            require_task_plan_approval: true,
            auto_approve: pruned,
          });
        }
        await loadAutonomy();
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        setIsUpdatingSeamless(false);
      }
    },
    [autoApprove, loadAutonomy]
  );

  const grant = useCallback(async (kind: AccessibilityPermissionKind) => {
    setRequestingKind(kind);
    setError(null);
    setRestartSummary(null);
    try {
      setStatus(await requestScreenIntelligencePermission(kind));
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setRequestingKind(null);
    }
  }, []);

  const restartAndRecheck = useCallback(async () => {
    setIsRestarting(true);
    setError(null);
    setRestartSummary(null);
    try {
      const result = await refreshScreenIntelligencePermissionsWithRestart(status);
      setStatus(result.status);
      setRestartSummary(result.restartSummary);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setIsRestarting(false);
    }
  }, [status]);

  const busy = isLoading || isRestarting || requestingKind !== null;
  const actionable = PERMISSIONS.map(p => stateFor(status, p.kind)).filter(
    s => s === 'denied' || s === 'unknown'
  );
  const allGranted =
    status !== null &&
    PERMISSIONS.every(p => {
      const s = stateFor(status, p.kind);
      return s === 'granted' || s === 'unsupported';
    });

  return (
    <SettingsPanel>
      <div
        data-testid="desktop-agent-beta-notice"
        className="rounded-xl border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-300">
        {t('settings.desktopAgent.beta')}
      </div>
      <p className="text-sm text-content-secondary">{t('settings.desktopAgent.description')}</p>

      <SettingsSection title={t('settings.screenIntel.permissions.title')}>
        {PERMISSIONS.map(({ kind, labelKey }) => {
          const state = stateFor(status, kind);
          const unsupported = state === 'unsupported';
          const canGrant = state === 'denied' || state === 'unknown';
          return (
            <SettingsRow
              key={kind}
              data-testid={`desktop-agent-perm-${kind}`}
              label={t(labelKey)}
              control={
                <div className="flex items-center gap-2">
                  <SettingsBadge variant={STATE_VARIANT[state]}>{state}</SettingsBadge>
                  {unsupported ? (
                    <span className="text-xs text-content-muted">
                      {t('settings.desktopAgent.notRequiredOnOs')}
                    </span>
                  ) : canGrant ? (
                    <Button
                      variant="secondary"
                      size="sm"
                      data-testid={`desktop-agent-grant-${kind}`}
                      onClick={() => void grant(kind)}
                      disabled={busy}>
                      {requestingKind === kind
                        ? t('settings.screenIntel.permissions.requesting')
                        : t('settings.desktopAgent.grant')}
                    </Button>
                  ) : null}
                </div>
              }
            />
          );
        })}
      </SettingsSection>

      <SettingsSection title={t('settings.desktopAgent.seamless.title')}>
        <SettingsRow
          htmlFor="desktop-agent-seamless"
          label={t('settings.desktopAgent.seamless.label')}
          description={t('settings.desktopAgent.seamless.description')}
          control={
            <SettingsSwitch
              id="desktop-agent-seamless"
              data-testid="desktop-agent-seamless-toggle"
              checked={seamlessOn}
              disabled={autoApprove === null || isUpdatingSeamless}
              onCheckedChange={next => void toggleSeamless(next)}
              aria-label={t('settings.desktopAgent.seamless.label')}
            />
          }
        />
      </SettingsSection>
      <p className="text-xs text-content-muted -mt-2">{t('settings.desktopAgent.seamless.note')}</p>

      <SettingsSection title={t('voice.debug.alwaysOn')}>
        <SettingsRow
          htmlFor="desktop-agent-always-on"
          label={t('voice.debug.alwaysOn')}
          description={t('voice.debug.alwaysOnDesc')}
          control={
            <SettingsSwitch
              id="desktop-agent-always-on"
              data-testid="voice-always-on-toggle"
              checked={alwaysOn ?? false}
              disabled={alwaysOn === null || isUpdatingAlwaysOn}
              onCheckedChange={next => void toggleAlwaysOn(next)}
              aria-label={t('voice.debug.alwaysOn')}
            />
          }
        />
      </SettingsSection>

      <div
        data-testid="desktop-agent-wake-word-hint"
        className="-mt-2 flex items-center gap-3 rounded-xl border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3.5 py-2.5">
        <svg
          className="h-5 w-5 flex-shrink-0 text-primary-500 dark:text-primary-300"
          fill="none"
          stroke="currentColor"
          strokeWidth={1.8}
          viewBox="0 0 24 24"
          aria-hidden="true">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M12 18.75a6 6 0 006-6v-1.5m-6 7.5a6 6 0 01-6-6v-1.5m6 7.5v3.75m-3.75 0h7.5M12 15.75a3 3 0 01-3-3V4.5a3 3 0 116 0v8.25a3 3 0 01-3 3z"
          />
        </svg>
        <p className="text-xs leading-relaxed text-primary-800 dark:text-primary-200">
          {t('settings.desktopAgent.wakeWordHint')}
        </p>
      </div>

      {allGranted ? (
        <div
          data-testid="desktop-agent-all-granted"
          className="rounded-xl border border-sage-300 dark:border-sage-500/40 bg-sage-50 dark:bg-sage-500/10 p-3 text-sm text-sage-700 dark:text-sage-300">
          {t('settings.desktopAgent.allGranted')}
        </div>
      ) : (
        <div className="rounded-xl border border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-300">
          {t('settings.screenIntel.permissions.grantHint')}
          {status?.permission_check_process_path ? (
            <p className="opacity-75 text-xs mt-1">
              {t('settings.screenIntel.permissions.macosAppliesPrivacy')}{' '}
              <span className="font-mono break-all text-content-secondary">
                {status.permission_check_process_path}
              </span>
            </p>
          ) : null}
        </div>
      )}

      {restartSummary ? (
        <div className="rounded-xl border border-sage-300 dark:border-sage-500/40 bg-sage-50 dark:bg-sage-500/10 p-3 text-sm text-sage-700 dark:text-sage-300">
          {restartSummary}
        </div>
      ) : null}

      {error ? (
        <div
          data-testid="desktop-agent-error"
          className="rounded-xl border border-coral-300 dark:border-coral-500/40 bg-coral-50 dark:bg-coral-500/10 p-3 text-sm text-coral-600 dark:text-coral-300">
          {error}
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="secondary"
          size="md"
          data-testid="desktop-agent-recheck"
          onClick={() => void refresh()}
          disabled={busy}>
          {isLoading
            ? t('settings.screenIntel.permissions.refreshing')
            : t('settings.desktopAgent.recheck')}
        </Button>
        {actionable.length > 0 ? (
          <button
            type="button"
            data-testid="desktop-agent-restart"
            onClick={() => void restartAndRecheck()}
            disabled={busy}
            className="rounded-lg border border-amber-400 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-300 disabled:opacity-50">
            {isRestarting
              ? t('settings.screenIntel.permissions.restartingCore')
              : t('settings.desktopAgent.restartAndRecheck')}
          </button>
        ) : null}
      </div>
    </SettingsPanel>
  );
};

export default DesktopAgentPanel;
