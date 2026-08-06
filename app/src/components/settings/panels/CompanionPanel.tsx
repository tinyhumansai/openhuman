import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type {
  CompanionConfig,
  CompanionSessionStatus,
  StartCompanionSessionResult,
  StopCompanionSessionResult,
} from '../../../store/companionSlice';
import { useAppSelector } from '../../../store/hooks';
import { isTauri } from '../../../utils/tauriCommands/common';
import Button from '../../ui/Button';
import { SettingsRow, SettingsSection, SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const CompanionPanel = () => {
  const { t } = useT();
  const companionState = useAppSelector(state => state.companion.state);

  const [status, setStatus] = useState<CompanionSessionStatus | null>(null);
  const [config, setConfig] = useState<CompanionConfig | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchStatus = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const result = await invoke<CompanionSessionStatus>('companion_status');
      setStatus(result);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const fetchConfig = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const result = await invoke<CompanionConfig>('companion_config_get');
      setConfig(result);
    } catch {
      // Config fetch is best-effort — defaults shown if unavailable.
    }
  }, []);

  useEffect(() => {
    const load = async () => {
      setIsLoading(true);
      await Promise.all([fetchStatus(), fetchConfig()]);
      setIsLoading(false);
    };
    void load();
  }, [fetchStatus, fetchConfig]);

  // Poll status while panel is open.
  useEffect(() => {
    const id = window.setInterval(() => void fetchStatus(), 3000);
    return () => window.clearInterval(id);
  }, [fetchStatus]);

  const handleStart = async () => {
    setIsStarting(true);
    setError(null);
    try {
      await invoke<StartCompanionSessionResult>('companion_start_session', { consent: true });
      try {
        await invoke<void>('register_companion_hotkey', {
          shortcut: config?.hotkey ?? 'ctrl+space',
        });
      } catch (registrationError) {
        // A session without its advertised shortcut cannot be used. Roll it
        // back so the panel never reports a live but unreachable companion.
        await invoke<StopCompanionSessionResult>('companion_stop_session').catch(() => undefined);
        throw registrationError;
      }
      await fetchStatus();
    } catch (err) {
      await fetchStatus();
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsStarting(false);
    }
  };

  const handleStop = async () => {
    setIsStopping(true);
    setError(null);
    try {
      await invoke<StopCompanionSessionResult>('companion_stop_session');
      await fetchStatus();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsStopping(false);
    }
  };

  const sessionActive = status?.active ?? false;

  return (
    <SettingsPanel description={t('settings.assistant.desktopCompanionDesc')}>
      {/* Session status + controls */}
      <SettingsSection>
        <SettingsRow
          label={t('settings.companion.session')}
          description={
            isLoading
              ? t('common.loading')
              : sessionActive
                ? `${t('settings.companion.activeLabel')} — ${companionState}`
                : t('settings.companion.inactiveStatus')
          }
          control={
            sessionActive ? (
              <Button
                type="button"
                variant="secondary"
                tone="danger"
                size="sm"
                onClick={handleStop}
                disabled={isStopping}>
                {isStopping
                  ? t('settings.companion.stopping')
                  : t('settings.companion.stopSession')}
              </Button>
            ) : (
              <Button
                type="button"
                variant="primary"
                size="sm"
                onClick={handleStart}
                disabled={isStarting || isLoading}>
                {isStarting
                  ? t('settings.companion.starting')
                  : t('settings.companion.startSession')}
              </Button>
            )
          }
        />
      </SettingsSection>

      {/* Session details */}
      {sessionActive && status && (
        <SettingsSection>
          <div className="px-4 py-3 text-xs text-content-secondary space-y-1">
            <p>
              {t('settings.companion.sessionId')}:{' '}
              <span className="font-mono">{status.session_id?.slice(0, 8)}…</span>
            </p>
            <p>
              {t('settings.companion.turns')}: {status.turn_count}
            </p>
            {status.remaining_ms != null && (
              <p>
                {t('settings.companion.remaining')}: {Math.floor(status.remaining_ms / 60000)}m{' '}
                {Math.floor((status.remaining_ms % 60000) / 1000)}s
              </p>
            )}
          </div>
        </SettingsSection>
      )}

      {/* Config */}
      {config && (
        <SettingsSection title={t('settings.companion.configuration')}>
          <SettingsRow
            label={t('settings.companion.hotkey')}
            control={
              <span className="rounded bg-surface-subtle px-2 py-0.5 font-mono text-xs text-content-secondary">
                {config.hotkey}
              </span>
            }
          />
          <SettingsRow
            label={t('settings.companion.activationMode')}
            control={<span className="text-xs text-content-muted">{config.activation_mode}</span>}
          />
          <SettingsRow
            label={t('settings.companion.sessionTtl')}
            control={<span className="text-xs text-content-muted">{config.ttl_secs}s</span>}
          />
        </SettingsSection>
      )}

      {/* Error */}
      <SettingsStatusLine
        saving={false}
        savedNote={null}
        error={error}
        savingLabel={t('settings.agentAccess.saving')}
      />
    </SettingsPanel>
  );
};

export default CompanionPanel;
