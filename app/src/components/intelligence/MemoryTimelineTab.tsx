/**
 * Memory Timeline tab (container). Load-on-mount, namespace selector, and mints
 * `nowSeconds` (in handlers, never during render) for the recency window.
 * Delegates rendering to the pure <MemoryTimelinePanel>. Read-only.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { TimelineReport } from '../../lib/memory/memoryTimeline';
import { loadNamespaces, loadTimeline } from '../../services/api/memoryTimelineApi';
import MemoryTimelinePanel from './MemoryTimelinePanel';

const nowSeconds = (): number => Math.floor(Date.now() / 1000);

const MemoryTimelineTab = () => {
  const { t } = useT();
  const [report, setReport] = useState<TimelineReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [namespaces, setNamespaces] = useState<string[]>([]);
  const [namespace, setNamespace] = useState('');
  // Monotonic token: ignore a response if a newer load has since started.
  const latestRequestId = useRef(0);

  const load = useCallback(async (ns: string) => {
    const requestId = (latestRequestId.current += 1);
    setLoading(true);
    setError(null);
    try {
      const next = await loadTimeline(nowSeconds(), ns || undefined);
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
          {t('memoryTimeline.namespaceLabel')}
          <select
            value={namespace}
            onChange={e => handleNamespace(e.target.value)}
            className="rounded-lg border border-line bg-surface px-2 py-1 text-sm text-content">
            <option value="">{t('memoryTimeline.namespaceAll')}</option>
            {namespaces.map(ns => (
              <option key={ns} value={ns}>
                {ns}
              </option>
            ))}
          </select>
        </label>
      )}

      <MemoryTimelinePanel
        report={report}
        loading={loading}
        error={error}
        onRetry={() => void load(namespace)}
      />
    </div>
  );
};

export default MemoryTimelineTab;
