/**
 * Global memory-sync schedule control (#3302). Presented like a backup
 * schedule: "Last synced … · Sync every …", with preset cadences (4h / 12h /
 * 24h) plus "Manual only". Reads/writes `config_*_memory_sync_settings`.
 *
 * Split out of `MemorySourcesRegistry.tsx` to keep that file under the size
 * budget.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import {
  type MemorySyncSettings,
  openhumanGetMemorySyncSettings,
  openhumanUpdateMemorySyncSettings,
} from '../../utils/tauriCommands/config';
import Button from '../ui/Button';
import { relativeTimestamp } from './memorySourcesRowHelpers';

const log = debug('intelligence:memory-sync');

/** Manual-only sentinel — stored as `sync_interval_secs = 0`. */
const MANUAL_INTERVAL_SECS = 0;
/** Preset cadences offered in the UI (seconds): 4h / 12h / 24h. */
const SYNC_INTERVAL_PRESETS_SECS = [14_400, 43_200, 86_400];

/** Human label for a cadence ("Every 4h" / "Every 30m" / "Manual only"). */
function intervalChipLabel(secs: number, t: (k: string) => string): string {
  if (secs === MANUAL_INTERVAL_SECS) return t('memorySyncInterval.manual');
  if (secs % 3600 === 0) {
    return t('memorySyncInterval.everyHours').replace('{h}', String(secs / 3600));
  }
  return t('memorySyncInterval.everyMinutes').replace('{m}', String(Math.round(secs / 60)));
}

interface MemorySyncScheduleProps {
  lastSyncMs: number | null;
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

export function MemorySyncSchedule({ lastSyncMs, onToast }: MemorySyncScheduleProps) {
  const { t } = useT();
  const [settings, setSettings] = useState<MemorySyncSettings | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let active = true;
    const loadSettings = async () => {
      try {
        const resp = await openhumanGetMemorySyncSettings();
        if (active) setSettings(resp.result);
      } catch (err) {
        log('get settings failed: %O', err);
      }
    };
    void loadSettings();
    return () => {
      active = false;
    };
  }, []);

  const handleSelect = useCallback(
    async (secs: number) => {
      setSaving(true);
      try {
        const resp = await openhumanUpdateMemorySyncSettings({ sync_interval_secs: secs });
        setSettings(resp.result);
      } catch (err) {
        onToast?.({
          type: 'error',
          title: t('memorySyncInterval.saveFailed'),
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setSaving(false);
      }
    },
    [onToast, t]
  );

  if (!settings) return null;

  const lastSync = relativeTimestamp(lastSyncMs, t);
  const currentLabel = settings.is_manual
    ? t('memorySyncInterval.manual')
    : intervalChipLabel(settings.selected_secs, t);
  // Option list: backend presets (fall back to the local defaults) + Manual.
  const presetSecs =
    settings.presets && settings.presets.length > 0 ? settings.presets : SYNC_INTERVAL_PRESETS_SECS;
  const options = [...presetSecs, MANUAL_INTERVAL_SECS];
  const selectedSecs = settings.is_manual ? MANUAL_INTERVAL_SECS : settings.selected_secs;

  return (
    <div
      className="mb-3 rounded-md border border-line bg-surface-muted p-3"
      data-testid="memory-sync-schedule">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <p className="text-xs font-semibold text-content-secondary">
            {t('memorySyncInterval.title')}
          </p>
          <p className="mt-0.5 text-xs text-content-muted">
            <span>
              {t('memorySyncInterval.lastSynced')} {lastSync ?? t('memorySyncInterval.never')}
            </span>
            <span aria-hidden="true"> · </span>
            <span data-testid="memory-sync-current">{currentLabel}</span>
          </p>
        </div>
        <div
          role="radiogroup"
          aria-label={t('memorySyncInterval.title')}
          className="flex flex-wrap items-center gap-1.5">
          {options.map(secs => {
            const isSelected = secs === selectedSecs;
            return (
              <Button
                key={secs}
                variant="secondary"
                size="xs"
                role="radio"
                aria-checked={isSelected}
                disabled={saving}
                onClick={() => void handleSelect(secs)}
                data-testid={`memory-sync-preset-${secs}`}
                className={
                  isSelected
                    ? 'border-primary-500 bg-primary-50 text-primary-700 dark:border-primary-500/50 dark:bg-primary-500/10 dark:text-primary-300'
                    : 'text-content-secondary'
                }>
                {intervalChipLabel(secs, t)}
              </Button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
