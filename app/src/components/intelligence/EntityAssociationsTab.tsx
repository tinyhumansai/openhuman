/**
 * Entity Associations tab (container). Owns load-on-mount and the namespace
 * selector; delegates all rendering to the pure <EntityAssociationsPanel>.
 * Read-only — the result is recomputed from the live graph, never persisted.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { AssociationReport } from '../../lib/memory/entityAssociations';
import { loadAssociations, loadNamespaces } from '../../services/api/entityAssociationsApi';
import EntityAssociationsPanel from './EntityAssociationsPanel';

const EntityAssociationsTab = () => {
  const { t } = useT();
  const [report, setReport] = useState<AssociationReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [namespaces, setNamespaces] = useState<string[]>([]);
  const [namespace, setNamespace] = useState('');
  // Monotonic token: ignore a response if a newer load has since started, so
  // an out-of-order resolution can never overwrite the latest result.
  const latestRequestId = useRef(0);

  const load = useCallback(async (ns: string) => {
    const requestId = (latestRequestId.current += 1);
    setLoading(true);
    setError(null);
    try {
      const next = await loadAssociations(ns || undefined);
      if (requestId !== latestRequestId.current) return;
      setReport(next);
    } catch (err) {
      if (requestId !== latestRequestId.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (requestId === latestRequestId.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Namespaces are optional UI sugar; a failure to list them must not block
    // the associations view, so swallow that error specifically.
    loadNamespaces()
      .then(setNamespaces)
      .catch(() => setNamespaces([]));
    void load('');
  }, [load]);

  const handleNamespace = (next: string): void => {
    setNamespace(next);
    void load(next);
  };

  return (
    <div className="space-y-4">
      {namespaces.length > 0 && (
        <label className="flex items-center gap-2 text-xs text-stone-600 dark:text-neutral-300">
          {t('entityAssociations.namespaceLabel')}
          <select
            value={namespace}
            onChange={e => handleNamespace(e.target.value)}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-sm text-stone-800 dark:text-neutral-100">
            <option value="">{t('entityAssociations.namespaceAll')}</option>
            {namespaces.map(ns => (
              <option key={ns} value={ns}>
                {ns}
              </option>
            ))}
          </select>
        </label>
      )}

      <EntityAssociationsPanel
        report={report}
        loading={loading}
        error={error}
        onRetry={() => void load(namespace)}
      />
    </div>
  );
};

export default EntityAssociationsTab;
