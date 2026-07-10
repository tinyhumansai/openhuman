import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { updateMemorySource } from '../../services/memorySourcesService';
import Button from '../ui/Button';

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
        className="bg-surface rounded-xl shadow-xl border border-line w-full max-w-md mx-4 p-5"
        onClick={e => e.stopPropagation()}>
        <h3 className="text-base font-semibold text-content mb-1">{t('syncBudget.title')}</h3>
        <p className="text-xs text-content-muted mb-4">{source.label}</p>

        <div className="flex flex-col gap-4">
          <div>
            <label
              htmlFor="budget-tokens"
              className="block text-sm font-medium text-content-secondary">
              {t('syncBudget.maxTokens')}
            </label>
            <p className="text-xs text-content-muted mt-0.5 mb-1">
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
              className="w-full px-3 py-1.5 rounded-md border border-line bg-surface text-sm font-mono"
            />
          </div>

          <div>
            <label
              htmlFor="budget-cost"
              className="block text-sm font-medium text-content-secondary">
              {t('syncBudget.maxCost')}
            </label>
            <p className="text-xs text-content-muted mt-0.5 mb-1">{t('syncBudget.maxCostHelp')}</p>
            <input
              id="budget-cost"
              type="number"
              min={0}
              step={0.01}
              value={maxCost}
              onChange={e => setMaxCost(e.target.value)}
              placeholder={t('syncBudget.unlimited')}
              className="w-full px-3 py-1.5 rounded-md border border-line bg-surface text-sm font-mono"
            />
          </div>

          <div>
            <label
              htmlFor="budget-depth"
              className="block text-sm font-medium text-content-secondary">
              {t('syncBudget.syncDepth')}
            </label>
            <p className="text-xs text-content-muted mt-0.5 mb-1">
              {t('syncBudget.syncDepthHelp')}
            </p>
            <select
              id="budget-depth"
              value={depthDays}
              onChange={e => setDepthDays(e.target.value)}
              className="w-full px-3 py-1.5 rounded-md border border-line bg-surface text-sm">
              <option value="">{t('syncBudget.allTime')}</option>
              <option value="7">{t('syncBudget.days7')}</option>
              <option value="30">{t('syncBudget.days30')}</option>
              <option value="90">{t('syncBudget.days90')}</option>
            </select>
          </div>
        </div>

        {error && <p className="mt-3 text-xs text-coral-600">{error}</p>}

        <div className="flex justify-end gap-2 mt-5">
          <Button variant="tertiary" size="sm" onClick={onClose}>
            {t('syncConfirm.cancel')}
          </Button>
          <Button variant="primary" size="sm" onClick={handleSave} disabled={saving}>
            {saving ? t('autonomy.statusSaving') : t('common.save')}
          </Button>
        </div>
      </div>
    </div>
  );
}
