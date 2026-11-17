import { RefreshCw } from 'lucide-react';
import { type ReactNode, useMemo, useState } from 'react';

import { type CostUsageRecord, useCostUsageLog } from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import {
  Badge,
  Button,
  DataTable,
  type DataTableColumn,
  EmptyState,
  NativeSelect,
  StatusLine,
} from '../ui';
import { formatCurrency, formatTokens } from './formatCurrency';

const ALL = '';
const PERIODS = [1, 7, 30, 90, 365];

/**
 * Two-line timestamp — date over time — in the user's locale, matching the
 * Approval history table; an unparseable value is shown raw.
 */
const DateTimeCell = ({ value }: { value: string }) => {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return <>{value}</>;
  return (
    <span className="flex flex-col leading-tight" title={date.toLocaleString()}>
      <span className="text-content">{date.toLocaleDateString()}</span>
      <span className="text-xs text-content-muted">
        {date.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}
      </span>
    </span>
  );
};

/** Detailed local usage records. Filters apply to the fetched, bounded window. */
const UsageLogPanel = () => {
  const { t } = useT();
  const [days, setDays] = useState(30);
  const [category, setCategory] = useState(ALL);
  const [provider, setProvider] = useState(ALL);
  const [query, setQuery] = useState('');
  const [source, setSource] = useState(ALL);
  const { data, isLoading, isFetching, error, refetch } = useCostUsageLog({ days, limit: 1000 });

  const categories = useMemo(
    () => [...new Set(data?.records.map(record => record.category) ?? [])].sort(),
    [data]
  );
  const providers = useMemo(
    () => [...new Set(data?.records.map(record => record.provider ?? '') ?? [])].sort(),
    [data]
  );
  const records = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return (data?.records ?? []).filter(
      record =>
        (category === ALL || record.category === category) &&
        (provider === ALL || (record.provider ?? '') === provider) &&
        (source === ALL || record.cost_source === source) &&
        (!needle ||
          record.model.toLocaleLowerCase().includes(needle) ||
          record.session_id.toLocaleLowerCase().includes(needle))
    );
  }, [data, category, provider, source, query]);
  const filteredCost = records.reduce((sum, record) => sum + record.cost_usd, 0);
  const currency = data?.currency ?? 'USD';

  const columns: DataTableColumn<CostUsageRecord>[] = [
    {
      id: 'when',
      header: t('settings.costDashboard.when'),
      className: 'w-px whitespace-nowrap tabular-nums',
      cell: record => <DateTimeCell value={record.timestamp} />,
    },
    {
      id: 'model',
      header: t('settings.costDashboard.model'),
      // `w-full max-w-0` is what lets a table cell truncate instead of growing.
      className: 'w-full max-w-0',
      cell: record => (
        <div className="min-w-0">
          <p className="truncate font-medium text-content" title={record.model}>
            {record.model}
          </p>
          <p className="truncate text-xs text-content-muted">
            {record.provider ?? t('settings.costDashboard.unknownProvider')}
          </p>
        </div>
      ),
    },
    {
      id: 'category',
      header: t('settings.costDashboard.category'),
      className: 'w-px whitespace-nowrap',
      cell: record => <Badge>{record.category}</Badge>,
    },
    {
      id: 'input',
      header: t('settings.costDashboard.inputTokens'),
      align: 'right',
      className: 'w-px whitespace-nowrap tabular-nums text-content-secondary',
      cell: record => formatTokens(record.input_tokens),
    },
    {
      id: 'output',
      header: t('settings.costDashboard.outputTokens'),
      align: 'right',
      className: 'w-px whitespace-nowrap tabular-nums text-content-secondary',
      cell: record => formatTokens(record.output_tokens),
    },
    {
      id: 'cost',
      header: t('settings.costDashboard.cost'),
      align: 'right',
      className: 'w-px whitespace-nowrap tabular-nums font-medium text-content',
      cell: record =>
        record.cost_source === 'unknown' ? '—' : formatCurrency(record.cost_usd, currency),
    },
    {
      id: 'source',
      header: t('settings.costDashboard.costSource'),
      className: 'w-px whitespace-nowrap',
      cell: record =>
        record.cost_source === 'provider_charged' ? (
          <Badge variant="success">{t('settings.costDashboard.providerCharged')}</Badge>
        ) : record.cost_source === 'unknown' ? (
          '—'
        ) : (
          <Badge>{t('settings.costDashboard.estimated')}</Badge>
        ),
    },
    {
      id: 'session',
      header: t('settings.costDashboard.session'),
      className: 'w-px whitespace-nowrap font-mono text-xs text-content-muted',
      cell: record => <span title={record.session_id}>{record.session_id.slice(0, 8)}</span>,
    },
  ];

  return (
    // Fills the Usage page's non-scrolling tab body; only the rows scroll.
    <div className="flex h-full min-h-0 flex-col" data-testid="usage-log-panel">
      <DataTable<CostUsageRecord>
        title={t('settings.costDashboard.usageLog')}
        description={t('settings.costDashboard.usageLogHint')
          .replace('{days}', String(days))
          .replace('{limit}', '1000')}
        actions={
          <>
            {data && (
              <span className="text-xs tabular-nums text-content-muted">
                {t('settings.costDashboard.filteredTotal')
                  .replace('{shown}', String(records.length))
                  .replace('{loaded}', String(data.records.length))
                  .replace('{cost}', formatCurrency(filteredCost, data.currency))}
              </span>
            )}
            <Button
              type="button"
              variant="secondary"
              size="sm"
              leadingIcon={
                <RefreshCw
                  className={`h-3.5 w-3.5 ${isFetching ? 'animate-spin' : ''}`}
                  aria-hidden
                />
              }
              onClick={() => void refetch()}
              disabled={isFetching}>
              {t('settings.costDashboard.refresh')}
            </Button>
          </>
        }
        columns={columns}
        rows={records}
        rowKey={record => record.id}
        ariaLabel={t('settings.costDashboard.usageLog')}
        pagination={{ pageSize: 25, pageSizeOptions: [10, 25, 50, 100] }}
        toolbarTop={
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
            <FilterSelect
              label={t('settings.costDashboard.period')}
              value={String(days)}
              onChange={value => setDays(Number(value))}>
              {PERIODS.map(value => (
                <option key={value} value={value}>
                  {t('settings.costDashboard.periodDays').replace('{days}', String(value))}
                </option>
              ))}
            </FilterSelect>
            <FilterSelect
              label={t('settings.costDashboard.category')}
              value={category}
              onChange={setCategory}>
              <option value="">{t('settings.costDashboard.all')}</option>
              {categories.map(value => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </FilterSelect>
            <FilterSelect
              label={t('settings.costDashboard.provider')}
              value={provider}
              onChange={setProvider}>
              <option value="">{t('settings.costDashboard.all')}</option>
              {providers.filter(Boolean).map(value => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </FilterSelect>
            <FilterSelect
              label={t('settings.costDashboard.costSource')}
              value={source}
              onChange={setSource}>
              <option value="">{t('settings.costDashboard.all')}</option>
              <option value="estimated">{t('settings.costDashboard.estimated')}</option>
              <option value="provider_charged">
                {t('settings.costDashboard.providerCharged')}
              </option>
            </FilterSelect>
          </div>
        }
        search={{
          value: query,
          onChange: setQuery,
          placeholder: t('settings.costDashboard.searchModelSession'),
          ariaLabel: t('settings.costDashboard.searchModelSession'),
        }}
        loading={!data && isLoading}
        loadingLabel={t('settings.costDashboard.loading')}
        error={error ? <StatusLine saving={false} error={error} savingLabel="" /> : undefined}
        empty={
          data ? (
            <EmptyState className="p-0" label={t('settings.costDashboard.noUsageLog')} />
          ) : undefined
        }
      />
    </div>
  );
};

/** A compact labelled select for the table toolbar; the label is its accessible name. */
const FilterSelect = ({
  label,
  value,
  onChange,
  children,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  children: ReactNode;
}) => (
  <label className="flex shrink-0 items-center gap-1.5 text-xs text-content-muted">
    <span>{label}</span>
    <NativeSelect
      inputSize="sm"
      value={value}
      onChange={event => onChange(event.target.value)}
      className="text-xs">
      {children}
    </NativeSelect>
  </label>
);

export default UsageLogPanel;
