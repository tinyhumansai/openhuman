import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type CodingSessionSourceStatus,
  getCodingSessionStatus,
  ingestCodingSessions,
} from '../../services/memorySourcesService';
import type { ToastNotification } from '../../types/intelligence';
import Button from '../ui/Button';

interface CodingSessionsCardProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

const SOURCE_LABEL_KEYS: Record<CodingSessionSourceStatus['kind'], string> = {
  claude_code: 'memorySources.codingSessions.claude',
  codex: 'memorySources.codingSessions.codex',
};

export function CodingSessionsCard({ onToast }: CodingSessionsCardProps) {
  const { t } = useT();
  const [sources, setSources] = useState<CodingSessionSourceStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [ingesting, setIngesting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    console.debug('[coding-sessions] status: entry');
    setError(null);
    try {
      const next = await getCodingSessionStatus();
      setSources(next);
      console.debug('[coding-sessions] status: exit sources=%d', next.length);
    } catch (cause) {
      console.error('[coding-sessions] status failed', cause);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const totals = useMemo(
    () => ({
      files: sources.reduce((sum, source) => sum + source.session_files, 0),
      evidence: sources.reduce((sum, source) => sum + source.evidence_units, 0),
    }),
    [sources]
  );
  const hasImportableHistory =
    totals.files > 0 || sources.some(source => source.scan_truncated === true);

  const ingest = useCallback(async () => {
    console.debug('[coding-sessions] ingest: entry');
    setIngesting(true);
    setError(null);
    try {
      const result = await ingestCodingSessions(false);
      console.debug(
        '[coding-sessions] ingest: exit processed=%d failed=%d budget_hit=%s',
        result.sessions_processed,
        result.sessions_failed,
        result.budget_hit
      );
      const incomplete = result.sessions_failed > 0 || result.budget_hit;
      onToast?.({
        type: incomplete ? 'warning' : 'success',
        title: t('memorySources.codingSessions.complete'),
        message:
          result.sessions_failed > 0
            ? t('memorySources.codingSessions.partialFailure')
                .replace('{failed}', String(result.sessions_failed))
                .replace('{processed}', String(result.sessions_processed))
            : result.budget_hit
              ? t('memorySources.codingSessions.moreRemaining')
              : t('memorySources.codingSessions.completeMessage')
                  .replace('{processed}', String(result.sessions_processed))
                  .replace('{observations}', String(result.observations)),
      });
      await load();
    } catch (cause) {
      console.error('[coding-sessions] ingest failed', cause);
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
      onToast?.({ type: 'error', title: t('memorySources.codingSessions.failed'), message });
    } finally {
      setIngesting(false);
    }
  }, [load, onToast, t]);

  return (
    <section
      className="rounded-lg border border-line bg-surface p-4"
      data-testid="coding-sessions-card">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-content">
            {t('memorySources.codingSessions.title')}
          </h3>
          <p className="mt-1 text-xs text-content-secondary">
            {t('memorySources.codingSessions.description')}
          </p>
        </div>
        <Button
          analyticsId="brain-sources-coding-sessions-ingest"
          size="sm"
          onClick={() => void ingest()}
          disabled={loading || ingesting || !hasImportableHistory}
          data-testid="coding-sessions-ingest">
          {ingesting
            ? t('memorySources.codingSessions.ingesting')
            : t('memorySources.codingSessions.ingest')}
        </Button>
      </div>

      <div className="mt-3 grid gap-2 sm:grid-cols-2">
        {sources.map(source => (
          <div
            key={source.kind}
            className="rounded-md border border-line-subtle bg-surface-secondary px-3 py-2"
            data-testid={`coding-session-source-${source.kind}`}>
            <div className="text-xs font-medium text-content">
              {t(SOURCE_LABEL_KEYS[source.kind])}
            </div>
            <div className="mt-1 text-xs text-content-secondary">
              {source.available
                ? t('memorySources.codingSessions.counts')
                    .replace('{files}', String(source.session_files))
                    .replace('{evidence}', String(source.evidence_units))
                : t('memorySources.codingSessions.notFound')}
            </div>
            {source.available && source.scan_truncated && (
              <div className="mt-1 text-xs text-amber-600 dark:text-amber-400">
                {t('memorySources.codingSessions.truncated')}
              </div>
            )}
          </div>
        ))}
      </div>

      {loading && (
        <p className="mt-3 text-xs text-content-secondary">
          {t('memorySources.codingSessions.scanning')}
        </p>
      )}
      {error && (
        <p className="mt-3 text-xs text-coral-600 dark:text-coral-400" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
