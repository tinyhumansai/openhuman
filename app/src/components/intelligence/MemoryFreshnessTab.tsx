/**
 * Knowledge Freshness tab (container). Owns load-on-mount, the namespace
 * selector, and minting `nowSeconds` (in handlers, never during render).
 * Delegates all rendering to the pure <MemoryFreshnessPanel>. Read-only.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { FreshnessReport } from '../../lib/memory/memoryFreshness';
import { loadFreshness, loadNamespaces } from '../../services/api/memoryFreshnessApi';
import MemoryFreshnessPanel from './MemoryFreshnessPanel';

const nowSeconds = (): number => Math.floor(Date.now() / 1000);

const MemoryFreshnessTab = () => {
  const { t } = useT();
  const [report, setReport] = useState<FreshnessReport | null>(null);
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
      const next = await loadFreshness(nowSeconds(), ns || undefined);
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
    // the freshness view, so swallow that error specifically.
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
          {t('memoryFreshness.namespaceLabel')}
          <select
            value={namespace}
            onChange={e => handleNamespace(e.target.value)}
            className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-sm text-stone-800 dark:text-neutral-100">
            <option value="">{t('memoryFreshness.namespaceAll')}</option>
            {namespaces.map(ns => (
              <option key={ns} value={ns}>
                {ns}
              </option>
            ))}
          </select>
        </label>
      )}

      <MemoryFreshnessPanel
        report={report}
        loading={loading}
        error={error}
        onRetry={() => void load(namespace)}
      />
    </div>
  );
};

export default MemoryFreshnessTab;
