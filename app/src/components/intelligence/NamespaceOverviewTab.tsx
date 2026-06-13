/**
 * Namespace Overview tab (container). Loads the whole graph on mount and
 * delegates rendering to the pure <NamespaceOverviewPanel>. Read-only. No
 * namespace selector — this view's axis IS the namespace, so it shows them all.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { NamespaceOverviewReport } from '../../lib/memory/namespaceOverview';
import { loadNamespaceOverview } from '../../services/api/namespaceOverviewApi';
import NamespaceOverviewPanel from './NamespaceOverviewPanel';

const NamespaceOverviewTab = () => {
  const [report, setReport] = useState<NamespaceOverviewReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Monotonic token: ignore a response if a newer load has since started.
  const latestRequestId = useRef(0);

  const load = useCallback(async () => {
    const requestId = (latestRequestId.current += 1);
    setLoading(true);
    setError(null);
    try {
      const next = await loadNamespaceOverview();
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
    void load();
  }, [load]);

  return (
    <NamespaceOverviewPanel
      report={report}
      loading={loading}
      error={error}
      onRetry={() => void load()}
    />
  );
};

export default NamespaceOverviewTab;
