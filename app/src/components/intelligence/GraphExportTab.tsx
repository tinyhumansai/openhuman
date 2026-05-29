/**
 * Graph Export tab (container). Loads the graph on mount, holds the chosen
 * format, and performs the file download (Blob + anchor) in the click handler —
 * never during render. Serialization is delegated to the pure engine.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { type ExportFormat, exportMimeType, serializeGraph } from '../../lib/memory/graphExport';
import { loadGraphRelations } from '../../services/api/graphExportApi';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphExportPanel from './GraphExportPanel';

const nowSeconds = (): number => Math.floor(Date.now() / 1000);

const GraphExportTab = () => {
  const [relations, setRelations] = useState<GraphRelation[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [format, setFormat] = useState<ExportFormat>('json');
  // Monotonic token: ignore a response if a newer load has since started.
  const latestRequestId = useRef(0);

  const load = useCallback(async () => {
    const requestId = (latestRequestId.current += 1);
    setLoading(true);
    setError(null);
    try {
      const next = await loadGraphRelations();
      if (requestId !== latestRequestId.current) return;
      setRelations(next);
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

  const handleDownload = useCallback(() => {
    if (!relations) return;
    const content = serializeGraph(relations, format);
    const blob = new Blob([content], { type: exportMimeType(format) });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `memory-graph-${nowSeconds()}.${format}`;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);
  }, [relations, format]);

  return (
    <GraphExportPanel
      count={relations === null ? null : relations.length}
      format={format}
      onFormatChange={setFormat}
      onDownload={handleDownload}
      loading={loading}
      error={error}
      onRetry={() => void load()}
    />
  );
};

export default GraphExportTab;
