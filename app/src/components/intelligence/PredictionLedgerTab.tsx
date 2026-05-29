/**
 * Prediction Calibration Ledger tab (container). Owns load-on-mount,
 * persistence, and id/timestamp minting (in event handlers, never during
 * render). Delegates all rendering to the pure <PredictionLedger>.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  addRecord,
  deleteRecord,
  normalizeConfidence,
  type PredictionOutcome,
  type PredictionRecord,
  resolveRecord,
} from '../../lib/memory/predictionLedger';
import { loadLedger, saveLedger } from '../../services/api/predictionLedgerApi';
import type {
  ConfirmationModal as ConfirmationModalType,
  ToastNotification,
} from '../../types/intelligence';
import { ConfirmationModal } from './ConfirmationModal';
import PredictionLedger, { type AddPredictionFormValues } from './PredictionLedger';

interface PredictionLedgerTabProps {
  onToast: (toast: Omit<ToastNotification, 'id'>) => void;
}

const CLOSED_MODAL: ConfirmationModalType = {
  isOpen: false,
  title: '',
  message: '',
  onConfirm: () => {},
  onCancel: () => {},
};

const nowSeconds = (): number => Math.floor(Date.now() / 1000);

const PredictionLedgerTab = ({ onToast }: PredictionLedgerTabProps) => {
  const { t } = useT();
  const [records, setRecords] = useState<PredictionRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [modal, setModal] = useState<ConfirmationModalType>(CLOSED_MODAL);
  // Monotonic token: only the most recent persist may roll back on failure.
  const persistSeq = useRef(0);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setRecords(await loadLedger());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Defer the initial fetch through a microtask so `setState` calls inside
    // `refresh` don't run during effect-commit (which trips the
    // `react-hooks/set-state-in-effect` lint). Matches SubconsciousReflectionCards.
    let cancelled = false;
    Promise.resolve().then(() => {
      if (cancelled) return;
      void refresh();
    });
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  // Optimistically update the in-memory ledger, then persist. On a write
  // failure, roll back to the pre-write state and toast — but ONLY if this is
  // still the latest persist. A stale failure (a newer save already succeeded)
  // must not clobber the newer state, so it is ignored via the seq token.
  const persist = useCallback(
    (next: PredictionRecord[], successTitle?: string) => {
      const seq = (persistSeq.current += 1);
      const previous = records;
      setRecords(next);
      void saveLedger(next)
        .then(() => {
          if (successTitle) onToast({ type: 'success', title: successTitle });
        })
        .catch(err => {
          if (seq !== persistSeq.current) return;
          setRecords(previous);
          onToast({
            type: 'error',
            title: t('predictionLedger.saveFailed'),
            message: err instanceof Error ? err.message : String(err),
          });
        });
    },
    [records, onToast, t]
  );

  const handleAdd = useCallback(
    (values: AddPredictionFormValues) => {
      const record: PredictionRecord = {
        id: crypto.randomUUID(),
        statement: values.statement,
        confidence: normalizeConfidence(values.confidencePercent / 100),
        createdAt: nowSeconds(),
        resolveBy: values.resolveBy,
        outcome: null,
        resolvedAt: null,
        category: values.category,
        notes: null,
      };
      persist(addRecord(records, record), t('predictionLedger.saved'));
    },
    [records, persist, t]
  );

  const handleResolve = useCallback(
    (id: string, outcome: PredictionOutcome) => {
      persist(resolveRecord(records, id, outcome, nowSeconds()));
    },
    [records, persist]
  );

  const handleDelete = useCallback(
    (id: string) => {
      setModal({
        isOpen: true,
        title: t('predictionLedger.deleteModalTitle'),
        message: t('predictionLedger.deleteModalMessage'),
        destructive: true,
        onCancel: () => setModal(CLOSED_MODAL),
        onConfirm: () => {
          setModal(CLOSED_MODAL);
          persist(deleteRecord(records, id));
        },
      });
    },
    [records, persist, t]
  );

  return (
    <div className="space-y-4">
      {error && (
        <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
          {t('predictionLedger.errorPrefix')} {error}
        </p>
      )}

      <PredictionLedger
        records={records}
        loading={loading}
        onAdd={handleAdd}
        onResolve={handleResolve}
        onDelete={handleDelete}
      />

      <ConfirmationModal modal={modal} onClose={() => setModal(CLOSED_MODAL)} />
    </div>
  );
};

export default PredictionLedgerTab;
