/**
 * Graph Core tab (container). Owns load-on-mount and the namespace selector;
 * delegates all rendering to the pure <GraphCorePanel>. Read-only — the result
 * is recomputed from the live graph, never persisted.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { CoreResult } from '../../lib/memory/graphCore';
import { loadCore, loadNamespaces } from '../../services/api/graphCoreApi';
import GraphCorePanel from './GraphCorePanel';

const log = debug('graph-core:tab');

const GraphCoreTab = () => {
  const { t } = useT();
  const [result, setResult] = useState<CoreResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [namespaces, setNamespaces] = useState<string[]>([]);
  const [namespace, setNamespace] = useState('');
  // Monotonic token: ignore a response if a newer load has since started, so
  // an out-of-order resolution can never overwrite the latest result.
  const latestRequestId = useRef(0);

  const load = useCallback(async (ns: string) => {
    const requestId = (latestRequestId.current += 1);
    log('load:start request=%d namespace=%s', requestId, ns || '(all)');
    setLoading(true);
    setError(null);
    try {
      const next = await loadCore(ns || undefined);
      if (requestId !== latestRequestId.current) {
        log('load:stale request=%d (newer load in flight)', requestId);
        return;
      }
      log(
        'load:success request=%d nodes=%d degeneracy=%d',
        requestId,
        next.nodeCount,
        next.degeneracy
      );
      setResult(next);
    } catch (err) {
      if (requestId !== latestRequestId.current) {
        log('load:stale-error request=%d', requestId);
        return;
      }
      log('load:error request=%d %o', requestId, err);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (requestId === latestRequestId.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Namespaces are optional UI sugar; a failure to list them must not block
    // the core view, so swallow that error specifically.
    const loadNamespaceOptions = async (): Promise<void> => {
      try {
        const next = await loadNamespaces();
        log('namespaces:loaded count=%d', next.length);
        setNamespaces(next);
      } catch (err) {
        log('namespaces:error (non-blocking) %o', err);
        setNamespaces([]);
      }
    };
    void loadNamespaceOptions();
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
          {t('graphCore.namespaceLabel')}
          <select
            value={namespace}
            onChange={e => handleNamespace(e.target.value)}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-sm text-stone-800 dark:text-neutral-100">
            <option value="">{t('graphCore.namespaceAll')}</option>
            {namespaces.map(ns => (
              <option key={ns} value={ns}>
                {ns}
              </option>
            ))}
          </select>
        </label>
      )}

      <GraphCorePanel
        result={result}
        loading={loading}
        error={error}
        onRetry={() => void load(namespace)}
      />
    </div>
  );
};

export default GraphCoreTab;
