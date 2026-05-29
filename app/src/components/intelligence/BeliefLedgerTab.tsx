/**
 * Belief Ledger tab (container). Owns data fetching + the explain/correct
 * side-effects; delegates all rendering to the pure <BeliefLedger>. Mirrors
 * the structure of the other Intelligence tabs (load on mount, onToast for
 * feedback, ConfirmationModal for the destructive-ish correction step).
 */
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { BeliefHistory } from '../../lib/memory/beliefLedger';
import { explainConflict, loadRelations, saveCorrection } from '../../services/api/beliefLedgerApi';
import type {
  ConfirmationModal as ConfirmationModalType,
  ToastNotification,
} from '../../types/intelligence';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import BeliefLedger from './BeliefLedger';
import { ConfirmationModal } from './ConfirmationModal';

interface BeliefLedgerTabProps {
  onToast: (toast: Omit<ToastNotification, 'id'>) => void;
}

const CLOSED_MODAL: ConfirmationModalType = {
  isOpen: false,
  title: '',
  message: '',
  onConfirm: () => {},
  onCancel: () => {},
};

const BeliefLedgerTab = ({ onToast }: BeliefLedgerTabProps) => {
  const { t } = useT();
  const [relations, setRelations] = useState<GraphRelation[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [explanationByKey, setExplanationByKey] = useState<Record<string, string>>({});
  const [explainingKey, setExplainingKey] = useState<string | null>(null);
  const [modal, setModal] = useState<ConfirmationModalType>(CLOSED_MODAL);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const rels = await loadRelations();
      setRelations(rels);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleExplain = useCallback(
    async (history: BeliefHistory) => {
      if (explainingKey) return;
      setExplainingKey(history.claimKey);
      try {
        const text = await explainConflict(history);
        setExplanationByKey(prev => ({ ...prev, [history.claimKey]: text }));
      } catch (err) {
        onToast({
          type: 'error',
          title: t('beliefLedger.errorPrefix'),
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setExplainingKey(null);
      }
    },
    [explainingKey, onToast, t]
  );

  const runCorrection = useCallback(
    async (history: BeliefHistory) => {
      try {
        await saveCorrection({
          namespace: history.current.namespace ?? 'default',
          subject: history.subject,
          predicate: history.predicate,
          correctObject: history.current.object,
        });
      } catch (err) {
        onToast({
          type: 'error',
          title: t('beliefLedger.correctionFailed'),
          message: err instanceof Error ? err.message : String(err),
        });
        return;
      }
      // The save itself succeeded — report success regardless of the reload.
      onToast({ type: 'success', title: t('beliefLedger.correctionSaved') });
      // refresh() owns its own error handling (sets the error banner), so a
      // reload failure never masquerades as a failed correction.
      void refresh();
    },
    [onToast, refresh, t]
  );

  const handleCorrect = useCallback(
    (history: BeliefHistory) => {
      setModal({
        isOpen: true,
        title: t('beliefLedger.correctionModalTitle'),
        message: t('beliefLedger.correctionModalMessage'),
        onCancel: () => setModal(CLOSED_MODAL),
        onConfirm: () => {
          setModal(CLOSED_MODAL);
          void runCorrection(history);
        },
      });
    },
    [runCorrection, t]
  );

  return (
    <div className="space-y-4">
      <div
        role="note"
        className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
        <p className="font-medium mb-1">{t('beliefLedger.title')}</p>
        <p>{t('beliefLedger.intro')}</p>
      </div>

      {error && (
        <p role="alert" className="text-xs text-coral-700 dark:text-coral-300">
          {t('beliefLedger.errorPrefix')} {error}
        </p>
      )}

      <BeliefLedger
        relations={relations}
        loading={loading}
        explanationByKey={explanationByKey}
        explainingKey={explainingKey}
        onExplain={key => void handleExplain(key)}
        onCorrect={handleCorrect}
      />

      <ConfirmationModal modal={modal} onClose={() => setModal(CLOSED_MODAL)} />
    </div>
  );
};

export default BeliefLedgerTab;
