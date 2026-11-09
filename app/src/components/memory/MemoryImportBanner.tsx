/**
 * One memory banner, one flow: (1) import the old local memory, then
 * (2) organize CortexDB memory into the per-user tree.
 *
 * Step 1: when `memory_import_scan` finds old local memory, offer to upload
 * it. Nothing leaves the device without the consent dialog's confirmation,
 * the only caller of `memory_import_start({consent: true})`.
 * Step 2: the core starts the move itself once the import is done (and
 * refuses it while an import is unfinished); this banner shows its progress. With no import, it is offered
 * ("Migrate now") or runs in the background while free. A legacy tree other
 * accounts may share (self-hosted) is only taken after the takeover dialog.
 *
 * Both steps always show once their scans answer: a step with nothing to do
 * (nothing found, already imported, already moved, or waiting on the import)
 * renders disabled with why, so the Migration tab never looks empty.
 *
 * debug logging: DEBUG=openhuman:memory:import, DEBUG=openhuman:memory:migration
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type ImportScan,
  type ImportState,
  memoryErrorMessage,
  memoryImportRetryFailed,
  memoryImportScan,
  memoryImportStart,
  memoryImportStatus,
  memoryMigrationRetry,
  memoryMigrationScan,
  memoryMigrationStart,
  memoryMigrationStatus,
  memoryStatusError,
  type MigrationScan,
  type MigrationStatus,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, AlertTitle, Button, ConfirmDialog, Progress } from '../ui';
import MemoryErrorAlert from './MemoryErrorAlert';
import { fill } from './memoryFormat';

const log = debug('openhuman:memory:import');
const mlog = debug('openhuman:memory:migration');

/** How often a running import is polled. */
export const IMPORT_POLL_MS = 1_500;
/** How often a running move is polled. */
export const MIGRATION_POLL_MS = 2_000;
/** How often an offered move is polled, to notice the background job start it. */
export const MIGRATION_IDLE_POLL_MS = 15_000;

interface MemoryImportBannerProps {
  /** Label of the engine the data would be uploaded to. */
  engineLabel: string;
}

/** A step with nothing to do: its title, why, and its action, disabled. */
function DisabledStep({
  testId,
  title,
  body,
  action,
}: {
  testId: string;
  title: string;
  body: string;
  action: string;
}) {
  return (
    <Alert className="opacity-60" aria-disabled="true" data-testid={testId}>
      <div className="flex w-full flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <AlertTitle>{title}</AlertTitle>
          <AlertDescription>{body}</AlertDescription>
        </div>
        <Button type="button" size="sm" variant="secondary" disabled>
          {action}
        </Button>
      </div>
    </Alert>
  );
}

export default function MemoryImportBanner({ engineLabel }: MemoryImportBannerProps) {
  const { t } = useT();
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [state, setState] = useState<ImportState | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Step 2 waits until step 1's scan has answered, so it never flashes first.
  const [importChecked, setImportChecked] = useState(false);
  // The scan AND the status both answered. A failed read is unknown, not
  // "nothing to import", so the empty step stays hidden until both are known.
  const [importKnown, setImportKnown] = useState(false);

  useEffect(() => {
    let cancelled = false;
    Promise.all([memoryImportScan(), memoryImportStatus().catch(() => null)])
      .then(([found, status]) => {
        if (cancelled) return;
        log('scan: found=%s phase=%s', found.found, status?.state.phase ?? 'n/a');
        setScan(found);
        if (status && status.state.phase !== 'idle') setState(status.state);
        setImportKnown(status !== null);
      })
      .catch(err => {
        // A failed scan only hides the offer; it is not worth an error banner.
        log('scan failed: %o', err);
      })
      .finally(() => {
        if (!cancelled) setImportChecked(true);
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
      setError(memoryErrorMessage(err, t));
    }
  }, [t]);

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
      setError(memoryErrorMessage(err, t));
      setConsentOpen(false);
    } finally {
      setStarting(false);
    }
  };

  // Re-sends only the items the engine refused; no new local data leaves the
  // device beyond what the user already consented to import.
  const retryFailed = async () => {
    setStarting(true);
    setError(null);
    try {
      const res = await memoryImportRetryFailed();
      log('retrying failed items: phase=%s failed=%d', res.state.phase, res.state.failed ?? 0);
      setState(res.state);
    } catch (err) {
      log('retry of failed items failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setStarting(false);
    }
  };

  // ── Step 2: organize (the layout migration) ──
  const [mScan, setMScan] = useState<MigrationScan | null>(null);
  const [mStatus, setMStatus] = useState<MigrationStatus | null>(null);
  const [takeoverOpen, setTakeoverOpen] = useState(false);
  const [mBusy, setMBusy] = useState(false);

  // Every read of what is left to move goes through this one effect: at
  // mount, after a failed read (retried), when a run ends, and when the
  // import finishes. Bumping it re-runs the read and discards the answer of
  // the one it replaces.
  const [mScanAttempt, setMScanAttempt] = useState(0);
  // The latest read of what is left to move answered. It is cleared when a
  // new read starts and stays false after a failed one, so a stale or unknown
  // answer never shows as an offer or as "already moved".
  const [moveKnown, setMoveKnown] = useState(false);
  const rescanMove = useCallback(() => setMScanAttempt(n => n + 1), []);
  // Bumped by a start or retry: a read or poll asked before it answers for
  // an older state, and is dropped.
  const mStatusGen = useRef(0);
  useEffect(() => {
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const gen = mStatusGen.current;
    // Both answers or neither: an offer without a known status could start
    // a move that is already running.
    Promise.all([memoryMigrationScan(), memoryMigrationStatus()])
      .then(([found, current]) => {
        // A start or retry since this read began owns the status now; the
        // run's end reads again.
        if (cancelled || gen !== mStatusGen.current) return;
        mlog('scan: needed=%s shared=%s', found?.needed, found?.shared);
        setMScan(found ?? null);
        setMStatus(current ?? null);
        setMoveKnown(true);
      })
      .catch(err => {
        // A transient failure must not hide the move for good: try again.
        // Meanwhile what is left to move is unknown, so offer nothing (a run
        // that just ended may have moved it all).
        mlog('scan failed: %o', err);
        // A start or retry since this read began owns the state; its run's
        // end reads again.
        if (!cancelled && gen === mStatusGen.current) {
          setMScan(null);
          setMoveKnown(false);
          retry = setTimeout(rescanMove, MIGRATION_IDLE_POLL_MS);
        }
      });
    return () => {
      cancelled = true;
      if (retry) clearTimeout(retry);
    };
  }, [mScanAttempt, rescanMove]);

  const wasMoving = useRef(false);
  // One status request at a time: a slow answer must not land after, and
  // overwrite, a newer one.
  const mPolling = useRef(false);
  const mPoll = useCallback(async () => {
    if (mPolling.current) return;
    mPolling.current = true;
    const gen = mStatusGen.current;
    try {
      const next = await memoryMigrationStatus();
      if (gen !== mStatusGen.current) return;
      setMStatus(next);
      // A run just ended: whether anything is still left to move changed.
      if (wasMoving.current && !next.running) rescanMove();
      wasMoving.current = next.running;
    } catch (err) {
      mlog('status failed: %o', err);
      // A failure of a poll a start or retry made obsolete is dropped too.
      if (gen === mStatusGen.current) setError(memoryErrorMessage(err, t));
    } finally {
      mPolling.current = false;
    }
  }, [t, rescanMove]);

  const moving = mStatus?.running ?? false;
  const moveOffered = moveKnown && (mScan?.needed ?? false);
  useEffect(() => {
    wasMoving.current = moving;
  }, [moving]);
  useEffect(() => {
    if (!moving && !moveOffered) return;
    const timer = setInterval(
      () => void mPoll(),
      moving ? MIGRATION_POLL_MS : MIGRATION_IDLE_POLL_MS
    );
    return () => clearInterval(timer);
  }, [moving, moveOffered, mPoll]);

  const startMove = useCallback(
    async (takeover: boolean) => {
      setMBusy(true);
      setError(null);
      mStatusGen.current += 1;
      try {
        // Bumped again when the start answers: a poll asked while it was in
        // flight may have read the state from before the move began.
        const next = await memoryMigrationStart(takeover).finally(() => {
          mStatusGen.current += 1;
        });
        mlog('start: takeover=%s phase=%s running=%s', takeover, next.state.phase, next.running);
        setMStatus(next);
      } catch (err) {
        mlog('start failed: %o', err);
        setError(memoryErrorMessage(err, t));
      } finally {
        setMBusy(false);
        setTakeoverOpen(false);
      }
    },
    [t]
  );

  const retryMove = async () => {
    setMBusy(true);
    setError(null);
    mStatusGen.current += 1;
    try {
      await memoryMigrationRetry();
      setMStatus(
        await memoryMigrationStart(false).finally(() => {
          mStatusGen.current += 1;
        })
      );
    } catch (err) {
      mlog('retry failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setMBusy(false);
    }
  };

  const importDone = state?.phase === 'done';
  const importWasRunning = useRef(false);
  // The core starts the move when the import finishes: look again, since the
  // read at mount may have found nothing to move before the import landed.
  useEffect(() => {
    if (importDone) {
      // An import seen finishing: the move answer from before it no longer
      // applies, so hide it until the post-import read answers. (A state that
      // was already done at mount is covered by the read the mount started.)
      if (importWasRunning.current) setMoveKnown(false);
      rescanMove();
    }
    importWasRunning.current = running;
  }, [importDone, running, rescanMove]);

  const mState = mStatus?.state;
  const left = (mState?.failures?.length ?? 0) + (mState?.incomplete?.length ?? 0);
  const cleaned = mState?.phase === 'cleaned';
  const importBusy = state?.phase === 'running' || state?.phase === 'error';
  const importPending = scan?.found && (!state || state.phase === 'idle');
  // Step 2 shows only once step 1 is out of the way.
  const showMove =
    importChecked &&
    !importBusy &&
    !importPending &&
    (moving || moveOffered || (cleaned && left > 0));
  const paused = !moving && (mState?.phase === 'paused' || mStatus?.interrupted);

  const showOffer = importPending;
  // Until the import scan answers, nothing: a disabled step must not flash first.
  if (!importChecked) return null;
  // Step 1 renders live when offered, or when its run state is visible below.
  const importShown =
    showOffer ||
    (!!state && state.phase !== 'idle' && !(importDone && showMove && !(state.failed ?? 0)));

  const counts = scan?.counts ?? { documents: 0, conversations: 0, learnings: 0 };
  const countsText = fill(t('memoryPage.import.counts'), {
    documents: counts.documents,
    conversations: counts.conversations,
    learnings: counts.learnings,
  });

  return (
    <div className="space-y-3" data-testid="memory-import-banner">
      {!importShown && importKnown && (
        <DisabledStep
          testId="memory-import-idle"
          title={importDone ? t('memoryPage.import.done') : t('memoryPage.import.action')}
          body={importDone ? t('memoryPage.import.doneBody') : t('memoryPage.import.none')}
          action={t('memoryPage.import.short')}
        />
      )}

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

      {state &&
        state.phase !== 'idle' &&
        // A clean import gives way to step 2; one with refused items keeps
        // its count and Retry beside it.
        !(importDone && showMove && !(state.failed ?? 0)) && (
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
                  ? memoryStatusError(state.error, t)
                  : fill(t('memoryPage.import.progress'), {
                      imported: state.imported,
                      total: state.total,
                    })}
              </AlertDescription>
              {state.phase === 'done' && (state.failed ?? 0) > 0 && (
                <div className="flex flex-wrap items-center gap-3">
                  <span className="text-sm" data-testid="memory-import-failed-items">
                    {fill(t('memoryPage.import.failedItems'), { count: state.failed ?? 0 })}
                    {/* A retry the engine or account stopped says why; Retry again works. */}
                    {state.error ? ` ${memoryStatusError(state.error, t)}` : ''}
                  </span>
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={starting}
                    data-testid="memory-import-retry-failed"
                    onClick={() => void retryFailed()}>
                    {t('memoryPage.import.retryFailed')}
                  </Button>
                </div>
              )}
              {state.phase === 'error' && (
                // The core keeps the checkpoint, so starting again resumes where
                // the import stopped; it still goes through the consent dialog.
                <Button
                  type="button"
                  size="sm"
                  variant="primary"
                  data-testid="memory-import-resume"
                  onClick={() => setConsentOpen(true)}>
                  {t('memoryPage.import.resume')}
                </Button>
              )}
            </div>
          </Alert>
        )}

      {showMove && (
        <div data-testid="memory-migration-banner">
          {moving ? (
            <Alert variant="info" data-testid="memory-migration-running">
              <div className="w-full space-y-1">
                <AlertTitle>{t('memoryPage.migrate.running')}</AlertTitle>
                <AlertDescription>
                  {fill(
                    t(
                      mState?.copied === 1
                        ? 'memoryPage.migrate.progressOne'
                        : 'memoryPage.migrate.progress'
                    ),
                    { copied: mState?.copied ?? 0 }
                  )}
                </AlertDescription>
              </div>
            </Alert>
          ) : cleaned ? (
            <Alert variant="warning" data-testid="memory-migration-left">
              <div className="flex w-full flex-wrap items-center justify-between gap-3">
                <div className="min-w-0">
                  <AlertTitle>
                    {fill(
                      t(
                        left === 1
                          ? 'memoryPage.migrate.leftTitleOne'
                          : 'memoryPage.migrate.leftTitle'
                      ),
                      { count: left }
                    )}
                  </AlertTitle>
                  <AlertDescription>{t('memoryPage.migrate.leftBody')}</AlertDescription>
                </div>
                <Button
                  type="button"
                  size="sm"
                  variant="primary"
                  disabled={mBusy}
                  data-testid="memory-migration-retry"
                  onClick={() => void retryMove()}>
                  {t('memoryPage.migrate.retry')}
                </Button>
              </div>
            </Alert>
          ) : (
            <Alert
              variant="info"
              data-testid={paused ? 'memory-migration-paused' : 'memory-migration-offer'}>
              <div className="flex w-full flex-wrap items-center justify-between gap-3">
                <div className="min-w-0">
                  <AlertTitle>
                    {paused ? t('memoryPage.migrate.paused') : t('memoryPage.migrate.title')}
                  </AlertTitle>
                  <AlertDescription>
                    {paused && mState?.error
                      ? memoryStatusError(mState.error, t)
                      : t('memoryPage.migrate.body')}
                  </AlertDescription>
                </div>
                <Button
                  type="button"
                  size="sm"
                  variant="primary"
                  disabled={mBusy}
                  data-testid="memory-migration-start"
                  onClick={() => (mScan?.shared ? setTakeoverOpen(true) : void startMove(false))}>
                  {paused ? t('memoryPage.migrate.resume') : t('memoryPage.migrate.action')}
                </Button>
              </div>
            </Alert>
          )}
        </div>
      )}

      {!showMove && moveKnown && (
        <DisabledStep
          testId="memory-migration-idle"
          title={t('memoryPage.migrate.title')}
          body={
            importPending || importBusy
              ? t('memoryPage.migrate.afterImport')
              : t('memoryPage.migrate.doneBody')
          }
          action={t('memoryPage.migrate.action')}
        />
      )}

      {error !== null && (
        <MemoryErrorAlert message={error} className="mt-3" data-testid="memory-import-error" />
      )}

      {takeoverOpen && (
        <ConfirmDialog
          title={t('memoryPage.migrate.takeoverTitle')}
          testId="memory-migration-takeover"
          confirmTestId="memory-migration-takeover-confirm"
          cancelTestId="memory-migration-takeover-cancel"
          busy={mBusy}
          confirmLabel={t('memoryPage.migrate.takeoverConfirm')}
          body={
            <p className="text-sm text-content-secondary">{t('memoryPage.migrate.takeoverBody')}</p>
          }
          onConfirm={() => void startMove(true)}
          onCancel={() => setTakeoverOpen(false)}
        />
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
