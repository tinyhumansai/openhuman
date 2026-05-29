/**
 * Graph Export — presentational view. Pure: renders the fact count, a format
 * toggle, and a download button; the actual file download is performed by the
 * container's onDownload handler. No data fetching, no clock, no RNG.
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { ExportFormat } from '../../lib/memory/graphExport';

interface GraphExportPanelProps {
  count: number | null; // number of exportable facts; null until loaded
  format: ExportFormat;
  onFormatChange: (format: ExportFormat) => void;
  onDownload: () => void;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
}

const FORMATS: ExportFormat[] = ['json', 'csv'];

const GraphExportPanel = ({
  count,
  format,
  onFormatChange,
  onDownload,
  loading,
  error,
  onRetry,
}: GraphExportPanelProps) => {
  const { t } = useT();

  const intro = (
    <div
      role="note"
      className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
      <p className="font-medium mb-1">{t('graphExport.title')}</p>
      <p>{t('graphExport.intro')}</p>
    </div>
  );

  if (loading) {
    return (
      <div className="space-y-4">
        {intro}
        <div
          className="animate-pulse rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-24"
          role="status"
          aria-label={t('graphExport.loading')}
          data-testid="graph-export-loading"
        />
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 p-4 text-center">
          <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
            {t('graphExport.errorPrefix')} {error}
          </p>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="mt-2 rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
              {t('graphExport.retry')}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (count === null || count === 0) {
    return (
      <div className="space-y-4">
        {intro}
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('graphExport.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('graphExport.emptyHint')}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {intro}

      <div className="rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
        <p className="text-sm text-stone-700 dark:text-neutral-200 tabular-nums">
          {t('graphExport.countLabel').replace('{count}', String(count))}
        </p>

        <fieldset className="flex items-center gap-3">
          <legend className="sr-only">{t('graphExport.formatLabel')}</legend>
          <span className="text-xs text-stone-500 dark:text-neutral-400">
            {t('graphExport.formatLabel')}
          </span>
          {FORMATS.map(f => (
            <label
              key={f}
              className="flex items-center gap-1 text-sm text-stone-700 dark:text-neutral-200">
              <input
                type="radio"
                name="graph-export-format"
                value={f}
                checked={format === f}
                onChange={() => onFormatChange(f)}
              />
              {f === 'csv' ? t('graphExport.formatCsv') : t('graphExport.formatJson')}
            </label>
          ))}
        </fieldset>

        <button
          type="button"
          onClick={onDownload}
          className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600">
          {t('graphExport.downloadButton').replace('{format}', format.toUpperCase())}
        </button>
      </div>
    </div>
  );
};

export default GraphExportPanel;
