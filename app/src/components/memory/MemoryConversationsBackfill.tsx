/**
 * Memory → Conversations → "Past conversations": chats from before automatic
 * saving was on are not in memory until they are synced here.
 *
 * Reads `memory_conversations_backfill_status` (what is still unsynced, and
 * the last run's progress). Nothing is uploaded without the consent dialog,
 * the only caller of `memory_conversations_backfill_start({consent: true})`;
 * a running sync is then polled until it finishes or fails. Syncing again
 * later sends only what is new.
 *
 * debug logging: DEBUG=openhuman:memory:backfill
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type BackfillView,
  memoryConversationsBackfillStart,
  memoryConversationsBackfillStatus,
  memoryErrorMessage,
  memoryStatusError,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, Button, Card, ConfirmDialog, Progress } from '../ui';
import MemoryErrorAlert from './MemoryErrorAlert';
import { fill } from './memoryFormat';

const log = debug('openhuman:memory:backfill');

/** How often a running sync is polled. */
export const BACKFILL_POLL_MS = 1_500;

export default function MemoryConversationsBackfill() {
  const { t } = useT();
  const [view, setView] = useState<BackfillView | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await memoryConversationsBackfillStatus();
      log(
        'status: phase=%s pending=%d/%d',
        next.state.phase,
        next.pending_threads,
        next.pending_turns
      );
      setView(next);
    } catch (err) {
      log('status failed: %o', err);
      setError(memoryErrorMessage(err, t));
    }
  }, [t]);

  useEffect(() => {
    let cancelled = false;
    memoryConversationsBackfillStatus()
      .then(next => {
        if (!cancelled) setView(next);
      })
      .catch(err => {
        if (cancelled) return;
        log('status failed: %o', err);
        setError(memoryErrorMessage(err, t));
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  const running = view?.state.phase === 'running';
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => void refresh(), BACKFILL_POLL_MS);
    return () => clearInterval(timer);
  }, [running, refresh]);

  const start = async () => {
    setStarting(true);
    setError(null);
    try {
      const next = await memoryConversationsBackfillStart();
      log('started: threads=%d turns=%d', next.pending_threads, next.pending_turns);
      setView(next);
    } catch (err) {
      log('start failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setStarting(false);
      setConsentOpen(false);
    }
  };

  const state = view?.state;
  const pendingText = view
    ? view.pending_turns > 0
      ? fill(t('memoryPage.backfill.pending'), {
          threads: view.pending_threads,
          turns: view.pending_turns,
        })
      : t('memoryPage.backfill.upToDate')
    : t('memoryPage.loading');

  return (
    <Card
      title={t('memoryPage.backfill.title')}
      description={t('memoryPage.backfill.description')}
      padded
      divided={false}
      data-testid="memory-backfill">
      <div className="space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm text-content" data-testid="memory-backfill-pending">
            {pendingText}
          </p>
          <Button
            type="button"
            variant="primary"
            size="sm"
            data-testid="memory-backfill-open"
            disabled={!view || running || view.pending_turns === 0}
            onClick={() => setConsentOpen(true)}>
            {state?.phase === 'error'
              ? t('memoryPage.backfill.resume')
              : t('memoryPage.backfill.action')}
          </Button>
        </div>

        {state?.phase === 'running' && (
          <div className="space-y-1.5" data-testid="memory-backfill-running">
            <Progress
              value={
                state.threads_total > 0
                  ? Math.round((state.threads_done / state.threads_total) * 100)
                  : 0
              }
              aria-label={t('memoryPage.backfill.running')}
            />
            <p className="text-xs text-content-muted">
              {fill(t('memoryPage.backfill.progress'), {
                done: state.threads_done,
                total: state.threads_total,
                turns: state.turns_stored,
              })}
            </p>
          </div>
        )}
        {state?.phase === 'done' && (
          <p className="text-xs text-content-muted" data-testid="memory-backfill-done">
            {fill(t('memoryPage.backfill.done'), {
              turns: state.turns_stored,
              threads: state.threads_done,
            })}
          </p>
        )}
        {state?.phase === 'error' && (
          <Alert variant="destructive" data-testid="memory-backfill-failed">
            <AlertDescription>
              {state.error ? memoryStatusError(state.error, t) : t('memoryPage.backfill.failed')}
            </AlertDescription>
          </Alert>
        )}
        {error !== null && <MemoryErrorAlert message={error} data-testid="memory-backfill-error" />}
      </div>

      {consentOpen && view && (
        <ConfirmDialog
          title={t('memoryPage.backfill.consentTitle')}
          testId="memory-backfill-consent"
          confirmTestId="memory-backfill-confirm"
          cancelTestId="memory-backfill-cancel"
          busy={starting}
          confirmLabel={t('memoryPage.backfill.consentConfirm')}
          body={
            <div className="space-y-2 text-sm text-content-secondary">
              <p>{t('memoryPage.backfill.consentBody')}</p>
              <p className="font-medium text-content">{pendingText}</p>
            </div>
          }
          onConfirm={() => void start()}
          onCancel={() => setConsentOpen(false)}
        />
      )}
    </Card>
  );
}
