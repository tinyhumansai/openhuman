/**
 * "Import previous memory": when `memory_import_scan` finds data from the old
 * local memory, offer to upload it to the selected engine. Nothing leaves the
 * device without the consent dialog's explicit confirmation, which is the only
 * caller of `memory_import_start({consent: true})`. Progress is then polled
 * from `memory_import_status` until the import finishes or fails.
 *
 * debug logging: DEBUG=openhuman:memory:import
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type ImportScan,
  type ImportState,
  memoryErrorMessage,
  memoryImportScan,
  memoryImportStart,
  memoryImportStatus,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, AlertTitle, Button, ConfirmDialog, Progress } from '../ui';
import { fill } from './memoryFormat';

const log = debug('openhuman:memory:import');

/** How often a running import is polled. */
export const IMPORT_POLL_MS = 1_500;

interface MemoryImportBannerProps {
  /** Label of the engine the data would be uploaded to. */
  engineLabel: string;
}

export default function MemoryImportBanner({ engineLabel }: MemoryImportBannerProps) {
  const { t } = useT();
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [state, setState] = useState<ImportState | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([memoryImportScan(), memoryImportStatus().catch(() => null)])
      .then(([found, status]) => {
        if (cancelled) return;
        log('scan: found=%s phase=%s', found.found, status?.state.phase ?? 'n/a');
        setScan(found);
        if (status && status.state.phase !== 'idle') setState(status.state);
      })
      .catch(err => {
        // A failed scan only hides the offer; it is not worth an error banner.
        log('scan failed: %o', err);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const poll = useCallback(async () => {
    try {
      const res = await memoryImportStatus();
      setState(res.state);
    } catch (err) {
      log('status failed: %o', err);
      setError(memoryErrorMessage(err));
    }
  }, []);

  const running = state?.phase === 'running';
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => void poll(), IMPORT_POLL_MS);
    return () => clearInterval(timer);
  }, [running, poll]);

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      const res = await memoryImportStart();
      log('import started: phase=%s total=%d', res.state.phase, res.state.total);
      setState(res.state);
      setConsentOpen(false);
    } catch (err) {
      log('import start failed: %o', err);
      setError(memoryErrorMessage(err));
      setConsentOpen(false);
    } finally {
      setStarting(false);
    }
  };

  const showOffer = scan?.found && (!state || state.phase === 'idle');
  if (!showOffer && !state) return null;

  const counts = scan?.counts ?? { documents: 0, conversations: 0, learnings: 0 };
  const countsText = fill(t('memoryPage.import.counts'), {
    documents: counts.documents,
    conversations: counts.conversations,
    learnings: counts.learnings,
  });

  return (
    <div data-testid="memory-import-banner">
      {showOffer && (
        <Alert variant="info">
          <div className="flex w-full flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <AlertTitle>{t('memoryPage.import.title')}</AlertTitle>
              <AlertDescription>
                <span data-testid="memory-import-counts">{countsText}</span>
              </AlertDescription>
            </div>
            <Button
              type="button"
              size="sm"
              variant="primary"
              data-testid="memory-import-open"
              onClick={() => setConsentOpen(true)}>
              {t('memoryPage.import.action')}
            </Button>
          </div>
        </Alert>
      )}

      {state && state.phase !== 'idle' && (
        <Alert
          variant={
            state.phase === 'error' ? 'destructive' : state.phase === 'done' ? 'success' : 'info'
          }
          data-testid={`memory-import-${state.phase}`}>
          <div className="w-full space-y-2">
            <AlertTitle>
              {state.phase === 'running'
                ? t('memoryPage.import.running')
                : state.phase === 'done'
                  ? t('memoryPage.import.done')
                  : t('memoryPage.import.failed')}
            </AlertTitle>
            {state.phase === 'running' && (
              <Progress
                value={state.total > 0 ? Math.round((state.imported / state.total) * 100) : 0}
                aria-label={t('memoryPage.import.running')}
              />
            )}
            <AlertDescription>
              {state.phase === 'error' && state.error
                ? state.error
                : fill(t('memoryPage.import.progress'), {
                    imported: state.imported,
                    total: state.total,
                  })}
            </AlertDescription>
          </div>
        </Alert>
      )}

      {error !== null && (
        <Alert variant="destructive" className="mt-3" data-testid="memory-import-error">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      {consentOpen && (
        <ConfirmDialog
          title={t('memoryPage.import.consentTitle')}
          testId="memory-import-consent"
          confirmTestId="memory-import-confirm"
          cancelTestId="memory-import-cancel"
          busy={starting}
          confirmLabel={t('memoryPage.import.consentConfirm')}
          body={
            <div className="space-y-2 text-sm text-content-secondary">
              <p>{fill(t('memoryPage.import.consentBody'), { engine: engineLabel })}</p>
              <p className="font-medium text-content">{countsText}</p>
            </div>
          }
          onConfirm={() => void start()}
          onCancel={() => setConsentOpen(false)}
        />
      )}
    </div>
  );
}
