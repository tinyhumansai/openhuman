/**
 * Connection Path tab (container). Loads the graph once (entities for the
 * pickers + relations), then runs the pure synchronous path engine on the
 * selected endpoints via useMemo — so tracing is instant with no extra
 * round-trips. Read-only. Delegates rendering to <ConnectionPathPanel>.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { findConnectionPath } from '../../lib/memory/connectionPath';
import { type GraphData, loadGraph, loadNamespaces } from '../../services/api/connectionPathApi';
import ConnectionPathPanel from './ConnectionPathPanel';

const EMPTY_GRAPH: GraphData = { entities: [], relations: [] };
const ENTITY_LIST_ID = 'connection-path-entities';

const ConnectionPathTab = () => {
  const { t } = useT();
  const [graph, setGraph] = useState<GraphData>(EMPTY_GRAPH);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [namespaces, setNamespaces] = useState<string[]>([]);
  const [namespace, setNamespace] = useState('');
  const [source, setSource] = useState('');
  const [target, setTarget] = useState('');
  // Monotonic token: ignore a graph response if a newer load has since started.
  const latestRequestId = useRef(0);

  const load = useCallback(async (ns: string) => {
    const requestId = (latestRequestId.current += 1);
    setLoading(true);
    setError(null);
    try {
      const next = await loadGraph(ns || undefined);
      if (requestId !== latestRequestId.current) return;
      setGraph(next);
    } catch (err) {
      if (requestId !== latestRequestId.current) return;
      setGraph(EMPTY_GRAPH);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (requestId === latestRequestId.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadNamespaces()
      .then(setNamespaces)
      .catch(() => setNamespaces([]));
    // Intentional fetch-on-mount. `loading` initializes to true and `error` to
    // null, so load()'s synchronous setState is a no-op on the first render and
    // triggers no cascading re-render.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load('');
  }, [load]);

  const handleNamespace = (next: string): void => {
    setNamespace(next);
    setSource('');
    setTarget('');
    void load(next);
  };

  const trimmedSource = source.trim();
  const trimmedTarget = target.trim();
  const result = useMemo(() => {
    if (!trimmedSource || !trimmedTarget) return null;
    return findConnectionPath(graph.relations, trimmedSource, trimmedTarget);
  }, [graph.relations, trimmedSource, trimmedTarget]);

  return (
    <div className="space-y-4">
      {namespaces.length > 0 && (
        <label className="flex items-center gap-2 text-xs text-stone-600 dark:text-neutral-300">
          {t('connectionPath.namespaceLabel')}
          <select
            value={namespace}
            onChange={e => handleNamespace(e.target.value)}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-sm text-stone-800 dark:text-neutral-100">
            <option value="">{t('connectionPath.namespaceAll')}</option>
            {namespaces.map(ns => (
              <option key={ns} value={ns}>
                {ns}
              </option>
            ))}
          </select>
        </label>
      )}

      <div className="flex flex-wrap gap-3">
        <label className="flex flex-1 min-w-[10rem] flex-col gap-1 text-xs text-stone-600 dark:text-neutral-300">
          {t('connectionPath.sourceLabel')}
          <input
            type="text"
            list={ENTITY_LIST_ID}
            value={source}
            onChange={e => setSource(e.target.value)}
            placeholder={t('connectionPath.sourcePlaceholder')}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1.5 text-sm text-stone-800 dark:text-neutral-100"
          />
        </label>
        <label className="flex flex-1 min-w-[10rem] flex-col gap-1 text-xs text-stone-600 dark:text-neutral-300">
          {t('connectionPath.targetLabel')}
          <input
            type="text"
            list={ENTITY_LIST_ID}
            value={target}
            onChange={e => setTarget(e.target.value)}
            placeholder={t('connectionPath.targetPlaceholder')}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1.5 text-sm text-stone-800 dark:text-neutral-100"
          />
        </label>
        <datalist id={ENTITY_LIST_ID}>
          {graph.entities.map(entity => (
            <option key={entity} value={entity} />
          ))}
        </datalist>
      </div>

      <ConnectionPathPanel
        result={result}
        hasGraph={graph.entities.length > 0}
        loading={loading}
        error={error}
        onRetry={() => void load(namespace)}
      />
    </div>
  );
};

export default ConnectionPathTab;
