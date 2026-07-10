import { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';
import Button from '../ui/Button';

interface SyncEstimate {
  item_count: number;
  estimated_tokens: number;
  estimated_cost_usd: number;
  budget_max_cost_usd: number | null;
  budget_max_tokens: number | null;
}

interface SyncConfirmDialogProps {
  sourceId: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function SyncConfirmDialog({
  sourceId,
  onConfirm,
  onCancel,
}: SyncConfirmDialogProps) {
  const { t } = useT();
  const [estimate, setEstimate] = useState<SyncEstimate | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setEstimate(null);
    setError(null);
    (async () => {
      try {
        const resp = await callCoreRpc<{ result: SyncEstimate }>({
          method: 'openhuman.memory_sources_estimate_sync_cost',
          params: { source_id: sourceId },
        });
        if (!cancelled) setEstimate(resp.result);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [sourceId]);

  const tokenStr = estimate
    ? estimate.estimated_tokens > 1000
      ? `${Math.round(estimate.estimated_tokens / 1000)}k`
      : String(estimate.estimated_tokens)
    : '';

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onCancel}>
      <div
        className="bg-surface rounded-xl shadow-xl border border-line w-full max-w-sm mx-4 p-5"
        onClick={e => e.stopPropagation()}>
        <h3 className="text-base font-semibold text-content mb-3">{t('syncConfirm.title')}</h3>

        {!estimate && !error && (
          <p className="text-sm text-content-muted">{t('syncConfirm.estimating')}</p>
        )}

        {error && <p className="text-sm text-coral-600">{error}</p>}

        {estimate && (
          <div className="flex flex-col gap-2">
            <p className="text-sm text-content-secondary">
              {t('syncConfirm.message')
                .replace('{items}', String(estimate.item_count))
                .replace('{tokens}', tokenStr)
                .replace('{cost}', estimate.estimated_cost_usd.toFixed(4))}
            </p>
            {estimate.budget_max_cost_usd != null && (
              <p className="text-xs text-content-muted">
                {t('syncConfirm.budgetNote').replace(
                  '{max}',
                  estimate.budget_max_cost_usd.toFixed(2)
                )}
              </p>
            )}
          </div>
        )}

        <div className="flex justify-end gap-2 mt-5">
          <Button variant="tertiary" size="sm" onClick={onCancel}>
            {t('syncConfirm.cancel')}
          </Button>
          <Button variant="primary" size="sm" onClick={onConfirm} disabled={!estimate}>
            {t('syncConfirm.proceed')}
          </Button>
        </div>
      </div>
    </div>
  );
}
