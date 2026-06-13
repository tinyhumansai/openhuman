import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { updateMemorySource } from '../../services/memorySourcesService';

interface SyncBudgetDialogProps {
  source: {
    id: string;
    label: string;
    max_tokens_per_sync?: number | null;
    max_cost_per_sync_usd?: number | null;
    sync_depth_days?: number | null;
  };
  onClose: () => void;
  onSaved: () => void;
}

export default function SyncBudgetDialog({ source, onClose, onSaved }: SyncBudgetDialogProps) {
  const { t } = useT();
  const [maxTokens, setMaxTokens] = useState(source.max_tokens_per_sync?.toString() ?? '');
  const [maxCost, setMaxCost] = useState(source.max_cost_per_sync_usd?.toString() ?? '');
  const [depthDays, setDepthDays] = useState<string>(source.sync_depth_days?.toString() ?? '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSave = useCallback(async () => {
    setSaving(true);
    setError(null);
    try {
      await updateMemorySource(source.id, {
        max_tokens_per_sync: maxTokens ? Number(maxTokens) : undefined,
        max_cost_per_sync_usd: maxCost ? Number(maxCost) : undefined,
        sync_depth_days: depthDays ? Number(depthDays) : undefined,
      } as Parameters<typeof updateMemorySource>[1]);
      onSaved();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [source.id, maxTokens, maxCost, depthDays, onSaved, onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}>
      <div
        className="bg-white dark:bg-neutral-900 rounded-xl shadow-xl border border-stone-200 dark:border-neutral-800 w-full max-w-md mx-4 p-5"
        onClick={e => e.stopPropagation()}>
        <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100 mb-1">
          {t('syncBudget.title')}
        </h3>
        <p className="text-xs text-stone-500 dark:text-neutral-400 mb-4">{source.label}</p>

        <div className="flex flex-col gap-4">
          <div>
            <label
              htmlFor="budget-tokens"
              className="block text-sm font-medium text-stone-700 dark:text-neutral-300">
              {t('syncBudget.maxTokens')}
            </label>
            <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5 mb-1">
              {t('syncBudget.maxTokensHelp')}
            </p>
            <input
              id="budget-tokens"
              type="number"
              min={0}
              step={10000}
              value={maxTokens}
              onChange={e => setMaxTokens(e.target.value)}
              placeholder={t('syncBudget.unlimited')}
              className="w-full px-3 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-sm font-mono"
            />
          </div>

          <div>
            <label
              htmlFor="budget-cost"
              className="block text-sm font-medium text-stone-700 dark:text-neutral-300">
              {t('syncBudget.maxCost')}
            </label>
            <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5 mb-1">
              {t('syncBudget.maxCostHelp')}
            </p>
            <input
              id="budget-cost"
              type="number"
              min={0}
              step={0.01}
              value={maxCost}
              onChange={e => setMaxCost(e.target.value)}
              placeholder={t('syncBudget.unlimited')}
              className="w-full px-3 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-sm font-mono"
            />
          </div>

          <div>
            <label
              htmlFor="budget-depth"
              className="block text-sm font-medium text-stone-700 dark:text-neutral-300">
              {t('syncBudget.syncDepth')}
            </label>
            <p className="text-xs text-stone-500 dark:text-neutral-400 mt-0.5 mb-1">
              {t('syncBudget.syncDepthHelp')}
            </p>
            <select
              id="budget-depth"
              value={depthDays}
              onChange={e => setDepthDays(e.target.value)}
              className="w-full px-3 py-1.5 rounded-md border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-sm">
              <option value="">{t('syncBudget.allTime')}</option>
              <option value="7">{t('syncBudget.days7')}</option>
              <option value="30">{t('syncBudget.days30')}</option>
              <option value="90">{t('syncBudget.days90')}</option>
            </select>
          </div>
        </div>

        {error && <p className="mt-3 text-xs text-coral-600">{error}</p>}

        <div className="flex justify-end gap-2 mt-5">
          <button
            onClick={onClose}
            className="px-3 py-1.5 rounded-md text-xs font-medium text-stone-600 dark:text-neutral-400 hover:bg-stone-100 dark:hover:bg-neutral-800">
            {t('syncConfirm.cancel')}
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="px-3 py-1.5 rounded-md bg-primary-600 hover:bg-primary-500 disabled:opacity-50 text-white text-xs font-medium">
            {saving ? t('autonomy.statusSaving') : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
