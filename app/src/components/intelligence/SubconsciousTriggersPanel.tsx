import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  setSubconsciousTriggersEnabled,
  subconsciousTriggersStatus,
  type SubconsciousTriggersStatus,
} from '../../utils/tauriCommands/subconscious';
import Button from '../ui/Button';

const cardClass = 'rounded-lg border border-line bg-surface p-4';

/**
 * Debug / manage panel for the event-driven subconscious trigger pipeline.
 * Surfaces the `subconscious_triggers.status` RPC: whether the pipeline is
 * enabled, the effective mode, the promotion budget, and live orchestrator
 * runtime state (running flag + pending queue depth). Provides an
 * enable/disable toggle (via `heartbeat_settings_set`). Polls every 5s.
 *
 * Works over any core transport (Tauri or cloud/tunnel) — `callCoreRpc`
 * resolves the transport, so there is no `isTauri()` gate here.
 */
export default function SubconsciousTriggersPanel() {
  const { t } = useT();
  const [status, setStatus] = useState<SubconsciousTriggersStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [toggling, setToggling] = useState(false);
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const res = await subconsciousTriggersStatus();
      setStatus(res.result ?? null);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
      inFlight.current = false;
    }
  }, []);

  const toggle = useCallback(async () => {
    if (toggling || !status) return;
    setToggling(true);
    try {
      await setSubconsciousTriggersEnabled(!status.triggers_enabled);
      setError(null);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setToggling(false);
    }
  }, [toggling, status, refresh]);

  useEffect(() => {
    // refresh() only setStates asynchronously (after an await); the initial
    // poll + 5s interval mirror the useSubconscious pattern.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refresh();
    const id = setInterval(() => void refresh(), 5000);
    return () => clearInterval(id);
  }, [refresh]);

  return (
    <div className={cardClass} data-testid="subconscious-triggers-panel">
      <div className="mb-3 flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold text-content">{t('subconsciousTriggers.title')}</h3>
          <p className="text-xs text-content-muted">{t('subconsciousTriggers.subtitle')}</p>
        </div>
        <div className="flex items-center gap-2">
          {status && (
            <Button
              variant={status.triggers_enabled ? 'secondary' : 'primary'}
              size="xs"
              onClick={() => void toggle()}
              disabled={toggling}
              data-testid="subconscious-triggers-toggle">
              {status.triggers_enabled
                ? t('subconsciousTriggers.disable')
                : t('subconsciousTriggers.enable')}
            </Button>
          )}
          <Button
            variant="secondary"
            size="xs"
            onClick={() => void refresh()}
            data-testid="subconscious-triggers-refresh">
            {t('common.refresh')}
          </Button>
        </div>
      </div>

      {loading && !status ? (
        <p className="text-xs text-content-muted" data-testid="subconscious-triggers-loading">
          {t('common.loading')}
        </p>
      ) : error ? (
        <p
          className="text-xs text-coral-600 dark:text-coral-400"
          data-testid="subconscious-triggers-error">
          {t('common.error')}: {error}
        </p>
      ) : status ? (
        <div className="space-y-2" data-testid="subconscious-triggers-status">
          <StatusRow
            testid="row-pipeline"
            label={t('subconsciousTriggers.pipeline')}
            value={status.triggers_enabled ? t('common.enabled') : t('common.disabled')}
            tone={status.triggers_enabled ? 'good' : 'muted'}
          />
          <StatusRow testid="row-mode" label={t('subconsciousTriggers.mode')} value={status.mode} />
          <StatusRow
            testid="row-orchestrator"
            label={t('subconsciousTriggers.orchestrator')}
            value={
              status.orchestrator_running
                ? t('subconsciousTriggers.running')
                : t('subconsciousTriggers.stopped')
            }
            tone={status.orchestrator_running ? 'good' : 'muted'}
          />
          <StatusRow
            testid="row-promotions"
            label={t('subconsciousTriggers.promotionsPerHour')}
            value={String(status.max_promotions_per_hour)}
          />
          <StatusRow
            testid="row-queue"
            label={t('subconsciousTriggers.queueDepth')}
            value={status.queue_depth === null ? '—' : String(status.queue_depth)}
          />
          <StatusRow
            testid="row-orchestrator-thread"
            label={t('subconsciousTriggers.orchestratorThread')}
            value={status.orchestrator_thread_id}
            mono
          />
          <StatusRow
            testid="row-user-thread"
            label={t('subconsciousTriggers.userThread')}
            value={status.user_thread_id}
            mono
          />

          {!status.triggers_enabled && (
            <p
              className="pt-1 text-xs text-content-muted"
              data-testid="subconscious-triggers-disabled-hint">
              {t('subconsciousTriggers.disabledHint')}
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}

interface StatusRowProps {
  label: string;
  value: string;
  tone?: 'default' | 'good' | 'muted';
  mono?: boolean;
  testid?: string;
}

function StatusRow({ label, value, tone = 'default', mono = false, testid }: StatusRowProps) {
  const toneClass =
    tone === 'good'
      ? 'text-sage-600 dark:text-sage-400'
      : tone === 'muted'
        ? 'text-content-faint'
        : 'text-content';
  return (
    <div className="flex items-center justify-between gap-3 text-xs" data-testid={testid}>
      <span className="text-content-muted">{label}</span>
      <span className={`${toneClass} ${mono ? 'font-mono' : 'font-medium'} truncate`}>{value}</span>
    </div>
  );
}
