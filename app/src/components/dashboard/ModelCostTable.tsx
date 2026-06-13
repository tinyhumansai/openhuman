import type { CostDashboardModelStats } from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import { formatCurrency, formatTokens } from './formatCurrency';

export interface ModelCostTableProps {
  models: CostDashboardModelStats[];
  currency: string;
}

const PROVIDER_PALETTE: Record<string, string> = {
  anthropic:
    'bg-[#D97757]/15 text-[#D97757] dark:bg-[#D97757]/20 dark:text-[#F5A584] ring-[#D97757]/30',
  openai: 'bg-sage-500/15 text-sage-700 dark:bg-sage-500/20 dark:text-sage-300 ring-sage-500/30',
  google:
    'bg-ocean-500/15 text-ocean-700 dark:bg-ocean-500/20 dark:text-ocean-300 ring-ocean-500/30',
  fireworks:
    'bg-coral-500/15 text-coral-700 dark:bg-coral-500/20 dark:text-coral-300 ring-coral-500/30',
  groq: 'bg-amber-500/15 text-amber-700 dark:bg-amber-500/20 dark:text-amber-300 ring-amber-500/30',
};

const PROVIDER_FALLBACK =
  'bg-stone-200 text-stone-700 dark:bg-neutral-800 dark:text-neutral-300 ring-stone-300 dark:ring-neutral-700';

function providerChipClass(provider: string | null): string {
  if (!provider) return PROVIDER_FALLBACK;
  return PROVIDER_PALETTE[provider.toLowerCase()] ?? PROVIDER_FALLBACK;
}

const ModelCostTable = ({ models, currency }: ModelCostTableProps) => {
  const { t } = useT();
  if (models.length === 0) {
    return (
      <div
        data-testid="model-cost-table-empty"
        className="text-xs text-stone-500 dark:text-neutral-400 italic py-2">
        {t('settings.costDashboard.noModels')}
      </div>
    );
  }

  return (
    <div className="overflow-x-auto -mx-1" data-testid="model-cost-table">
      <table className="w-full text-xs">
        <thead>
          <tr className="text-left text-[10px] uppercase tracking-wide text-stone-500 dark:text-neutral-400 border-b border-stone-200 dark:border-neutral-800">
            <Th>{t('settings.costDashboard.model')}</Th>
            <Th>{t('settings.costDashboard.provider')}</Th>
            <Th align="right">{t('settings.costDashboard.tokens')}</Th>
            <Th align="right">{t('settings.costDashboard.requests')}</Th>
            <Th align="right">{t('settings.costDashboard.cost')}</Th>
            <Th align="right">{t('settings.costDashboard.percentOfTotal')}</Th>
          </tr>
        </thead>
        <tbody>
          {models.map(row => {
            const modelName = row.model.includes('/')
              ? row.model.split('/').slice(1).join('/')
              : row.model;
            const sharePct = Math.max(0, Math.min(100, row.percent_of_total));
            return (
              <tr
                key={row.model}
                data-testid={`model-row-${row.model}`}
                className="group border-b border-stone-100 dark:border-neutral-800/60 last:border-0 hover:bg-stone-50/60 dark:hover:bg-neutral-800/40 transition-colors">
                <Td>
                  <div
                    className="font-medium text-stone-800 dark:text-neutral-100 truncate max-w-[16rem]"
                    title={row.model}>
                    {modelName}
                  </div>
                </Td>
                <Td>
                  <span
                    className={`inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium ring-1 ring-inset ${providerChipClass(row.provider)}`}>
                    {row.provider ?? t('settings.costDashboard.unknownProvider')}
                  </span>
                </Td>
                <Td align="right">
                  <span className="tabular-nums text-stone-700 dark:text-neutral-200">
                    {formatTokens(row.total_tokens)}
                  </span>
                </Td>
                <Td align="right">
                  <span className="tabular-nums text-stone-700 dark:text-neutral-200">
                    {row.request_count}
                  </span>
                </Td>
                <Td align="right">
                  <span className="tabular-nums font-medium text-stone-900 dark:text-neutral-50">
                    {formatCurrency(row.cost_usd, currency)}
                  </span>
                </Td>
                <Td align="right">
                  <div className="flex items-center justify-end gap-2">
                    <div
                      aria-hidden
                      className="h-1 w-12 rounded-full bg-stone-200 dark:bg-neutral-800 overflow-hidden">
                      <div
                        className="h-full rounded-full bg-ocean-500"
                        style={{ width: `${sharePct}%` }}
                      />
                    </div>
                    <span className="tabular-nums w-10 text-right text-stone-600 dark:text-neutral-300">
                      {`${sharePct.toFixed(1)}%`}
                    </span>
                  </div>
                </Td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
};

interface CellProps {
  children: React.ReactNode;
  align?: 'left' | 'right';
}

const Th = ({ children, align = 'left' }: CellProps) => (
  <th className={`py-2 px-2 font-medium ${align === 'right' ? 'text-right' : 'text-left'}`}>
    {children}
  </th>
);

const Td = ({ children, align = 'left' }: CellProps) => (
  <td className={`py-2 px-2 ${align === 'right' ? 'text-right' : 'text-left'}`}>{children}</td>
);

export default ModelCostTable;
