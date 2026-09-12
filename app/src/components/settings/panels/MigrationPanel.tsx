import debug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type MigrationReport,
  openhumanMigrateHermes,
  openhumanMigrateOpenclaw,
} from '../../../utils/tauriCommands/core';
import PanelPage from '../../layout/PanelPage';
import { Alert, AlertDescription } from '../../ui';
import Button from '../../ui/Button';
import { SettingsSection, SettingsSelect, SettingsTextField } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

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

  const body = (
    <>
      {/* The card fills the pane, but a paragraph does not want to: at this
          pane width an uncapped line runs past 180 characters. */}
      <p className="max-w-[68ch] text-sm text-content-secondary">{t('migration.description')}</p>

      <SettingsSection>
        <div className="p-4 space-y-5" data-testid="migration-form">
          {/* The card fills the pane; each field takes the width its own value
              needs. A two-option vendor picker a metre wide puts its label and
              its value at opposite ends of an empty control; a workspace path is
              long and genuinely uses the room. Sized per field, not uniformly.
              NOTE: the sibling Core connection panel runs its URL and token
              fields at full width, so the app has no settled rule here yet. */}
          <div className="max-w-sm space-y-1">
            <label className="block text-xs font-medium text-content-secondary">
              {t('migration.vendorLabel')}
            </label>
            <SettingsSelect
              aria-label={t('migration.vendorLabel')}
              data-testid="migration-vendor-select"
              value={vendor}
              onChange={e => setVendor(e.target.value as Vendor)}
              inputSize="sm"
              className="w-full">
              <option value="openclaw">{t('migration.vendor.openclaw')}</option>
              <option value="hermes">{t('migration.vendor.hermes')}</option>
            </SettingsSelect>
          </div>

          <div className="max-w-2xl space-y-1">
            <label className="block text-xs font-medium text-content-secondary">
              {t('migration.sourceLabel')}
            </label>
            <SettingsTextField
              data-testid="migration-source-input"
              value={sourcePath}
              onChange={e => setSourcePath(e.target.value)}
              placeholder={
                vendor === 'hermes'
                  ? t('migration.sourcePlaceholderHermes')
                  : t('migration.sourcePlaceholder')
              }
              aria-label={t('migration.sourceLabel')}
              inputSize="sm"
              className="w-full"
            />
            <p className="text-[11px] text-content-muted">{t('migration.sourceHint')}</p>
          </div>

          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="primary"
              size="sm"
              data-testid="migration-preview-button"
              onClick={() => void runPreview()}
              disabled={isPreviewing || isApplying}>
              {isPreviewing ? t('migration.previewRunning') : t('migration.previewAction')}
            </Button>
            {/* Was an amber wash over `variant="tertiary"`, which bypassed the
                danger tone and used the warning ramp. Disabled, that wash read
                as a broken button rather than a locked one; the gate is
                explained in the line below the row instead. */}
            <Button
              type="button"
              variant="primary"
              tone="danger"
              size="sm"
              data-testid="migration-apply-button"
              onClick={() => void runApply()}
              disabled={!canApply}>
              {isApplying ? t('migration.applyRunning') : t('migration.applyAction')}
            </Button>
          </div>

          <p className="max-w-[68ch] text-[11px] text-content-muted">
            {t('migration.applyDisclaimer')}
          </p>
        </div>
      </SettingsSection>

      {error != null && (
        <Alert variant="destructive" density="compact" data-testid="migration-error">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      {reportToRender != null && (
        <section
          data-testid={
            appliedReport != null ? 'migration-report-applied' : 'migration-report-preview'
          }
          className="bg-surface rounded-lg border border-line p-4 space-y-3">
          <h3 className="text-sm font-semibold text-content">
            {appliedReport != null
              ? t('migration.reportTitleApplied')
              : t('migration.reportTitlePreview')}
          </h3>
          <dl className="grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-1 text-xs">
            <dt className="text-content-muted">{t('migration.report.source')}</dt>
            <dd className="text-content break-all" data-testid="migration-report-source">
              {reportToRender.source_workspace}
            </dd>
            <dt className="text-content-muted">{t('migration.report.target')}</dt>
            <dd className="text-content break-all" data-testid="migration-report-target">
              {reportToRender.target_workspace}
            </dd>
            <dt className="text-content-muted">{t('migration.report.fromSqlite')}</dt>
            <dd className="text-content">{reportToRender.stats.from_sqlite}</dd>
            <dt className="text-content-muted">{t('migration.report.fromMarkdown')}</dt>
            <dd className="text-content">{reportToRender.stats.from_markdown}</dd>
            <dt className="text-content-muted">{t('migration.report.imported')}</dt>
            <dd className="text-content" data-testid="migration-report-imported">
              {reportToRender.stats.imported}
            </dd>
            <dt className="text-content-muted">{t('migration.report.skippedUnchanged')}</dt>
            <dd className="text-content">{reportToRender.stats.skipped_unchanged}</dd>
            <dt className="text-content-muted">{t('migration.report.renamedConflicts')}</dt>
            <dd className="text-content">{reportToRender.stats.renamed_conflicts}</dd>
          </dl>

          {reportToRender.warnings.length > 0 && (
            <div className="space-y-1">
              <p className="text-xs font-medium text-content-secondary">
                {t('migration.report.warnings')}
              </p>
              <ul
                data-testid="migration-report-warnings"
                className="text-xs text-content-secondary list-disc list-inside space-y-0.5">
                {reportToRender.warnings.map((w, i) => (
                  <li key={i}>{w}</li>
                ))}
              </ul>
            </div>
          )}

          <p className="max-w-[68ch] text-[11px] text-content-muted">
            {appliedReport != null
              ? t('migration.report.appliedHint')
              : t('migration.report.previewHint')}
          </p>
        </section>
      )}
    </>
  );

  if (embedded) {
    return (
      <PanelPage className="z-10" contentClassName="">
        <div className="space-y-5 p-6">{body}</div>
      </PanelPage>
    );
  }

  return (
    <SettingsPanel description={t('pages.settings.account.migrationDesc')}>{body}</SettingsPanel>
  );
};

export default MigrationPanel;
