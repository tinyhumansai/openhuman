import debug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type MigrationReport,
  openhumanMigrateHermes,
  openhumanMigrateOpenclaw,
} from '../../../utils/tauriCommands/core';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('migration-panel');

type Vendor = 'openclaw' | 'hermes';

interface MigrationPanelProps {
  /** When true, render without the SettingsHeader chrome (used when embedded
   *  inside the onboarding custom wizard). Mirrors the embed contract used
   *  by VoicePanel / MemoryDataPanel. */
  embedded?: boolean;
}

const MigrationPanel = ({ embedded = false }: MigrationPanelProps = {}) => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [vendor, setVendor] = useState<Vendor>('openclaw');
  const [sourcePath, setSourcePath] = useState<string>('');
  const [previewReport, setPreviewReport] = useState<MigrationReport | null>(null);
  // Snapshot of `{ vendor, source }` that produced `previewReport`. Apply
  // must match these exactly — otherwise the user could preview path A,
  // edit the field to path B, and apply against B without ever seeing
  // the diff. CodeRabbit flagged this on PR #2087.
  const [previewInput, setPreviewInput] = useState<{
    vendor: Vendor;
    source: string | undefined;
  } | null>(null);
  const [appliedReport, setAppliedReport] = useState<MigrationReport | null>(null);
  const [isPreviewing, setIsPreviewing] = useState(false);
  const [isApplying, setIsApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const normalizedSource = sourcePath.trim() || undefined;

  const runMigrationRpc = useCallback(
    (dryRun: boolean) => {
      const source = normalizedSource;
      if (vendor === 'hermes') {
        return openhumanMigrateHermes(source, dryRun);
      }
      return openhumanMigrateOpenclaw(source, dryRun);
    },
    [vendor, normalizedSource]
  );

  // Apply is only enabled after a successful Preview *of the same input*.
  // Without that gate the user can mutate their workspace without ever
  // seeing what would change for the currently-typed path — exactly the
  // surprise issue #1440 calls out about the existing RPC's `dry_run=true`
  // default, and the regression CodeRabbit flagged on PR #2087.
  const canApply =
    previewReport != null &&
    previewInput != null &&
    previewInput.vendor === vendor &&
    previewInput.source === normalizedSource &&
    !isApplying &&
    !isPreviewing;

  const runPreview = useCallback(async () => {
    setError(null);
    setIsPreviewing(true);
    setAppliedReport(null);
    try {
      log('[migration] preview start vendor=%s source=%s', vendor, normalizedSource ?? '<default>');
      const response = await runMigrationRpc(true);
      // `runMigrationRpc` returns `CommandResponse<MigrationReport>`
      // — `.result` is the actual report.
      setPreviewReport(response.result);
      setPreviewInput({ vendor, source: normalizedSource });
      log(
        '[migration] preview ok stats=%o warnings=%d',
        response.result.stats,
        response.result.warnings.length
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      log('[migration] preview failed: %s', message);
      setError(message);
      setPreviewReport(null);
      setPreviewInput(null);
    } finally {
      setIsPreviewing(false);
    }
  }, [runMigrationRpc]);

  const runApply = useCallback(async () => {
    if (!canApply || previewReport == null) return;
    const summary = previewReport.stats;
    const totalPlanned = summary.from_sqlite + summary.from_markdown - summary.skipped_unchanged;
    const template = t(
      totalPlanned === 1 ? 'migration.confirmImport.singular' : 'migration.confirmImport.plural'
    );
    const ok = window.confirm(
      template
        .replace('{count}', String(totalPlanned))
        .replace('{source}', previewReport.source_workspace)
        .replace('{target}', previewReport.target_workspace)
    );
    if (!ok) return;

    setError(null);
    setIsApplying(true);
    try {
      log('[migration] apply start vendor=%s source=%s', vendor, normalizedSource ?? '<default>');
      const response = await runMigrationRpc(false);
      setAppliedReport(response.result);
      // Clear preview so the operator can't accidentally re-apply the same
      // dry-run a second time without re-previewing the new on-disk state.
      setPreviewReport(null);
      setPreviewInput(null);
      log('[migration] apply ok stats=%o', response.result.stats);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      log('[migration] apply failed: %s', message);
      setError(message);
    } finally {
      setIsApplying(false);
    }
  }, [runMigrationRpc, previewReport, canApply, t]);

  const reportToRender = appliedReport ?? previewReport;

  return (
    <div className="z-10 relative">
      {!embedded && (
        <SettingsHeader
          title={t('migration.title')}
          showBackButton={true}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
      )}

      <div className="max-w-3xl space-y-6 p-6">
        <p className="text-sm text-stone-600 dark:text-neutral-300">{t('migration.description')}</p>

        <section
          className="bg-stone-50 dark:bg-neutral-900/40 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-4"
          data-testid="migration-form">
          <label className="block space-y-1">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              {t('migration.vendorLabel')}
            </span>
            <select
              aria-label={t('migration.vendorLabel')}
              data-testid="migration-vendor-select"
              value={vendor}
              onChange={e => setVendor(e.target.value as Vendor)}
              className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:outline-none focus:ring-1 focus:ring-primary-400">
              <option value="openclaw">{t('migration.vendor.openclaw')}</option>
              <option value="hermes">{t('migration.vendor.hermes')}</option>
            </select>
          </label>

          <label className="block space-y-1">
            <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              {t('migration.sourceLabel')}
            </span>
            <input
              type="text"
              data-testid="migration-source-input"
              value={sourcePath}
              onChange={e => setSourcePath(e.target.value)}
              placeholder={
                vendor === 'hermes'
                  ? t('migration.sourcePlaceholderHermes')
                  : t('migration.sourcePlaceholder')
              }
              className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-1 focus:ring-primary-400"
            />
            <p className="text-[11px] text-stone-500 dark:text-neutral-400">
              {t('migration.sourceHint')}
            </p>
          </label>

          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              data-testid="migration-preview-button"
              onClick={runPreview}
              disabled={isPreviewing || isApplying}
              className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
              {isPreviewing ? t('migration.previewRunning') : t('migration.previewAction')}
            </button>
            <button
              type="button"
              data-testid="migration-apply-button"
              onClick={runApply}
              disabled={!canApply}
              className="px-3 py-1.5 text-xs rounded-md bg-amber-600 hover:bg-amber-700 disabled:opacity-60 text-white">
              {isApplying ? t('migration.applyRunning') : t('migration.applyAction')}
            </button>
          </div>

          <p className="text-[11px] text-stone-500 dark:text-neutral-400">
            {t('migration.applyDisclaimer')}
          </p>
        </section>

        {error != null && (
          <div
            data-testid="migration-error"
            className="rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
            {error}
          </div>
        )}

        {reportToRender != null && (
          <section
            data-testid={
              appliedReport != null ? 'migration-report-applied' : 'migration-report-preview'
            }
            className="bg-white dark:bg-neutral-900/40 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
            <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
              {appliedReport != null
                ? t('migration.reportTitleApplied')
                : t('migration.reportTitlePreview')}
            </h3>
            <dl className="grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-1 text-xs">
              <dt className="text-stone-500 dark:text-neutral-400">
                {t('migration.report.source')}
              </dt>
              <dd
                className="text-stone-900 dark:text-neutral-100 break-all"
                data-testid="migration-report-source">
                {reportToRender.source_workspace}
              </dd>
              <dt className="text-stone-500 dark:text-neutral-400">
                {t('migration.report.target')}
              </dt>
              <dd
                className="text-stone-900 dark:text-neutral-100 break-all"
                data-testid="migration-report-target">
                {reportToRender.target_workspace}
              </dd>
              <dt className="text-stone-500 dark:text-neutral-400">
                {t('migration.report.fromSqlite')}
              </dt>
              <dd className="text-stone-900 dark:text-neutral-100">
                {reportToRender.stats.from_sqlite}
              </dd>
              <dt className="text-stone-500 dark:text-neutral-400">
                {t('migration.report.fromMarkdown')}
              </dt>
              <dd className="text-stone-900 dark:text-neutral-100">
                {reportToRender.stats.from_markdown}
              </dd>
              <dt className="text-stone-500 dark:text-neutral-400">
                {t('migration.report.imported')}
              </dt>
              <dd
                className="text-stone-900 dark:text-neutral-100"
                data-testid="migration-report-imported">
                {reportToRender.stats.imported}
              </dd>
              <dt className="text-stone-500 dark:text-neutral-400">
                {t('migration.report.skippedUnchanged')}
              </dt>
              <dd className="text-stone-900 dark:text-neutral-100">
                {reportToRender.stats.skipped_unchanged}
              </dd>
              <dt className="text-stone-500 dark:text-neutral-400">
                {t('migration.report.renamedConflicts')}
              </dt>
              <dd className="text-stone-900 dark:text-neutral-100">
                {reportToRender.stats.renamed_conflicts}
              </dd>
            </dl>

            {reportToRender.warnings.length > 0 && (
              <div className="space-y-1">
                <p className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                  {t('migration.report.warnings')}
                </p>
                <ul
                  data-testid="migration-report-warnings"
                  className="text-xs text-stone-700 dark:text-neutral-300 list-disc list-inside space-y-0.5">
                  {reportToRender.warnings.map((w, i) => (
                    <li key={i}>{w}</li>
                  ))}
                </ul>
              </div>
            )}

            <p className="text-[11px] text-stone-500 dark:text-neutral-400">
              {appliedReport != null
                ? t('migration.report.appliedHint')
                : t('migration.report.previewHint')}
            </p>
          </section>
        )}
      </div>
    </div>
  );
};

export default MigrationPanel;
