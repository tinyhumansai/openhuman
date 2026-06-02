import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';

interface ActivityLevelSettings {
  level: number;
  level_label: string;
  sync_interval_secs: number | null;
  heartbeat_enabled: boolean;
  subconscious_enabled: boolean;
  token_budget_per_cycle: number | null;
  estimated_monthly_cost_min_usd: number;
  estimated_monthly_cost_max_usd: number;
}

interface MonthlyCostSummary {
  month: string;
  total_cost_usd: number;
  total_syncs: number;
}

const LEVELS = [
  { key: 'off', value: 0 },
  { key: 'minimal', value: 1 },
  { key: 'moderate', value: 2 },
  { key: 'active', value: 3 },
  { key: 'alwaysOn', value: 4 },
] as const;

type LevelKey = (typeof LEVELS)[number]['key'];

type Status = 'idle' | 'loading' | 'saving' | 'saved' | 'error';

// These tables intentionally duplicate the backend constants in
// AgentActivityLevel::estimated_monthly_cost_range (config/schema/activity_level.rs).
// The backend only returns cost ranges for the *current* level, so we need a
// static lookup to render cost estimates for all levels simultaneously.
// A future RPC that returns ranges for all levels would allow removing these.
function getCostMin(level: number): number {
  return [0, 0.1, 1, 5, 20][level] ?? 0;
}

function getCostMax(level: number): number {
  return [0, 0.5, 5, 20, 100][level] ?? 0;
}

export default function AgentActivityPanel() {
  const { t } = useT();
  const [settings, setSettings] = useState<ActivityLevelSettings | null>(null);
  const [monthlyCost, setMonthlyCost] = useState<MonthlyCostSummary | null>(null);
  const [status, setStatus] = useState<Status>('loading');
  const [error, setError] = useState<string | null>(null);

  const loadSettings = useCallback(async () => {
    try {
      setStatus('loading');
      const [settingsResp, costResp] = await Promise.all([
        callCoreRpc<{ result: ActivityLevelSettings }>({
          method: 'openhuman.config_get_activity_level_settings',
          params: {},
        }),
        callCoreRpc<{ result: MonthlyCostSummary }>({
          method: 'openhuman.memory_sources_monthly_cost_summary',
          params: {},
        }),
      ]);
      setSettings(settingsResp.result);
      setMonthlyCost(costResp.result);
      setStatus('idle');
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setStatus('error');
    }
  }, []);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  const handleLevelChange = useCallback(async (levelKey: string) => {
    try {
      setStatus('saving');
      setError(null);
      const resp = await callCoreRpc<{ result: ActivityLevelSettings }>({
        method: 'openhuman.config_update_activity_level_settings',
        params: { level: levelKey },
      });
      setSettings(resp.result);
      setStatus('saved');
      setTimeout(() => setStatus('idle'), 2000);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setStatus('error');
    }
  }, []);

  if (status === 'loading' && !settings) {
    return (
      <div className="p-4 text-sm text-stone-500 dark:text-neutral-400">{t('common.loading')}</div>
    );
  }

  return (
    <div className="flex flex-col gap-4 p-4">
      <div>
        <h2 className="text-lg font-semibold text-stone-900 dark:text-neutral-100">
          {t('activityLevel.title')}
        </h2>
        <p className="text-xs text-stone-600 dark:text-neutral-400 mt-1">
          {t('activityLevel.description')}
        </p>
      </div>

      {monthlyCost && monthlyCost.total_cost_usd > 0 && (
        <div className="px-3 py-2 rounded-md bg-stone-100 dark:bg-neutral-800 text-sm">
          <span className="font-medium text-stone-700 dark:text-neutral-300">
            {t('activityLevel.currentMonth').replace(
              '{amount}',
              monthlyCost.total_cost_usd.toFixed(2)
            )}
          </span>
        </div>
      )}

      <div className="flex flex-col gap-2">
        {LEVELS.map(({ key, value }) => {
          const isSelected = settings?.level === value;
          const apiKey = key === 'alwaysOn' ? 'always_on' : (key as string);
          const costMin = getCostMin(value);
          const costMax = getCostMax(value);
          return (
            <button
              key={key}
              onClick={() => handleLevelChange(apiKey)}
              disabled={status === 'saving'}
              className={`w-full text-left px-4 py-3 rounded-lg border transition-colors ${
                isSelected
                  ? 'border-primary-500 bg-primary-50 dark:bg-primary-900/20'
                  : 'border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:border-stone-300 dark:hover:border-neutral-700'
              } ${status === 'saving' ? 'opacity-50' : ''}`}>
              <div className="flex items-center justify-between">
                <div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                      {t(`activityLevel.${key as LevelKey}`)}
                    </span>
                    {value === 2 && (
                      <span className="text-xs px-1.5 py-0.5 rounded bg-stone-200 dark:bg-neutral-700 text-stone-600 dark:text-neutral-400">
                        {t('activityLevel.default')}
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5">
                    {t(`activityLevel.${key as LevelKey}Desc`)}
                  </p>
                </div>
                <div className="text-xs font-mono text-stone-500 dark:text-neutral-400 shrink-0 ml-4">
                  {costMin === 0 && costMax === 0
                    ? t('activityLevel.costFree')
                    : t('activityLevel.costRange')
                        .replace('{min}', String(costMin))
                        .replace('{max}', String(costMax))}
                </div>
              </div>
            </button>
          );
        })}
      </div>

      <div role="status" aria-live="polite" aria-atomic="true" className="text-xs min-h-[1rem]">
        {status === 'saving' && (
          <span className="text-stone-500">{t('autonomy.statusSaving')}</span>
        )}
        {status === 'saved' && (
          <span className="text-sage-700 dark:text-sage-400">{t('activityLevel.saved')}</span>
        )}
        {error && <span className="text-coral-600">{error}</span>}
      </div>
    </div>
  );
}
