import { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../services/coreRpcClient';
import { ConfirmDialog } from '../ui';

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
    <ConfirmDialog
      title={t('syncConfirm.title')}
      body={
        <>
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
        </>
      }
      confirmLabel={t('syncConfirm.proceed')}
      cancelLabel={t('syncConfirm.cancel')}
      confirmDisabled={!estimate}
      destructive={false}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />
  );
}
