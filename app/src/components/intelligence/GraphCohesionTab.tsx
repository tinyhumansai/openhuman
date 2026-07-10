/**
 * Graph Cohesion tab (container). Owns load-on-mount and the namespace
 * selector; delegates all rendering to the pure <GraphCohesionPanel>.
 * Read-only — the result is recomputed from the live graph, never persisted.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { CohesionResult } from '../../lib/memory/graphCohesion';
import { loadCohesion, loadNamespaces } from '../../services/api/graphCohesionApi';
import GraphCohesionPanel from './GraphCohesionPanel';

const GraphCohesionTab = () => {
  const { t } = useT();
  const [result, setResult] = useState<CohesionResult | null>(null);
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
      const next = await loadCohesion(ns || undefined);
      if (requestId !== latestRequestId.current) return;
      setResult(next);
    } catch (err) {
      if (requestId !== latestRequestId.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (requestId === latestRequestId.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Namespaces are optional UI sugar; a failure to list them must not block
    // the cohesion view, so swallow that error specifically.
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
        <label className="flex items-center gap-2 text-xs text-content-secondary">
          {t('graphCohesion.namespaceLabel')}
          <select
            value={namespace}
            onChange={e => handleNamespace(e.target.value)}
            className="rounded-lg border border-line bg-surface px-2 py-1 text-sm text-content">
            <option value="">{t('graphCohesion.namespaceAll')}</option>
            {namespaces.map(ns => (
              <option key={ns} value={ns}>
                {ns}
              </option>
            ))}
          </select>
        </label>
      )}

      <GraphCohesionPanel
        result={result}
        loading={loading}
        error={error}
        onRetry={() => void load(namespace)}
      />
    </div>
  );
};

export default GraphCohesionTab;
