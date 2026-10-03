/*
 * Pure formatting helpers + small presentational primitives shared by the
 * background-loop controls (loop map + usage ledger section).
 */
import type { ComposioConnection } from '../../../../lib/composio/types';
import type { CreditTransaction } from '../../../../services/api/creditsApi';

export const USD = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 4,
  maximumFractionDigits: 6,
});

export const WEEK_MINUTES = 7 * 24 * 60;
export const COMPOSIO_PERIODIC_TICK_MINUTES = 20;

export const formatUsd = (value: number): string => {
  const safe = Number.isFinite(value) ? value : 0;
  if (safe > 0 && safe < 0.000001) return `<${USD.format(0.000001)}`;
  return USD.format(safe);
};

export const spendAmount = (tx: CreditTransaction): number => {
  const amount = Number(tx.amountUsd);
  return Number.isFinite(amount) ? Math.abs(amount) : 0;
};

export const formatCount = (value: number): string =>
  new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 }).format(
    Number.isFinite(value) ? value : 0
  );

export const formatDateTime = (value: string | null | undefined): string => {
  if (!value) return 'n/a';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'n/a';
  return date.toLocaleString([], {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
};

export const activeConnection = (connection: ComposioConnection): boolean => {
  const status = connection.status.toUpperCase();
  return status === 'ACTIVE' || status === 'CONNECTED';
};

export function summarizeSpendByAction(
  transactions: CreditTransaction[]
): Array<[string, number, number]> {
  const byAction = new Map<string, { count: number; total: number }>();
  for (const tx of transactions) {
    if (tx.type !== 'SPEND') continue;
    const key = tx.action || 'SPEND';
    const prev = byAction.get(key) ?? { count: 0, total: 0 };
    prev.count += 1;
    prev.total += spendAmount(tx);
    byAction.set(key, prev);
  }
  return Array.from(byAction.entries())
    .map(([action, value]) => [action, value.count, value.total] as [string, number, number])
    .sort((a, b) => b[2] - a[2])
    .slice(0, 4);
}

export function summarizeSpendByHour(transactions: CreditTransaction[]): Array<[string, number]> {
  const byHour = new Map<string, number>();
  for (const tx of transactions) {
    if (tx.type !== 'SPEND') continue;
    const date = new Date(tx.createdAt);
    if (Number.isNaN(date.getTime())) continue;
    date.setMinutes(0, 0, 0);
    const key = date.toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric' });
    byHour.set(key, (byHour.get(key) ?? 0) + spendAmount(tx));
  }
  return Array.from(byHour.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, 4);
}

export function summarizeSpendSample(transactions: CreditTransaction[]) {
  const rows = transactions
    .filter(tx => tx.type === 'SPEND')
    .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());
  const total = rows.reduce((sum, tx) => sum + spendAmount(tx), 0);
  const avgRowUsd = rows.length > 0 ? total / rows.length : 0;
  const times = rows
    .map(tx => new Date(tx.createdAt).getTime())
    .filter(time => !Number.isNaN(time))
    .sort((a, b) => a - b);
  const sampleHours =
    times.length >= 2 ? Math.max((times[times.length - 1] - times[0]) / 3_600_000, 1 / 60) : 0;
  const spendPerHour = sampleHours > 0 ? total / sampleHours : 0;
  const rowsPerHour = sampleHours > 0 ? rows.length / sampleHours : 0;
  return { rows, total, avgRowUsd, sampleHours, spendPerHour, rowsPerHour };
}

export const MetricTile = ({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail?: string;
}) => (
  <div className="min-w-0 overflow-hidden rounded-lg bg-surface-muted px-3 py-2.5">
    <div className="truncate text-xs text-content-muted">{label}</div>
    <div className="mt-0.5 truncate text-lg font-semibold tabular-nums text-content">{value}</div>
    {detail ? <div className="mt-0.5 truncate text-xs text-content-faint">{detail}</div> : null}
  </div>
);

export const FormulaRow = ({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail: string;
}) => (
  <div className="flex min-w-0 items-center justify-between gap-4 px-4 py-3">
    <div className="min-w-0">
      <div className="truncate text-sm font-medium text-content">{label}</div>
      <div className="mt-0.5 truncate text-xs text-content-muted">{detail}</div>
    </div>
    <span className="shrink-0 font-mono text-sm tabular-nums text-content">{value}</span>
  </div>
);
