import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  memoryClearNamespace,
  type MemoryDebugDocument,
  memoryDeleteDocument,
  memoryListDocuments,
  memoryListNamespaces,
  memoryQueryNamespace,
  type MemoryQueryResult,
  memoryRecallNamespace,
} from '../../../utils/tauriCommands';
import { MemoryTextWithEntities } from '../../intelligence/MemoryTextWithEntities';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { normalizeMemoryDocuments } from './memoryDebugUtils';

const MemoryDebugPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const [documents, setDocuments] = useState<MemoryDebugDocument[]>([]);
  const [documentsRaw, setDocumentsRaw] = useState<unknown>(null);
  const [documentsNamespaceFilter, setDocumentsNamespaceFilter] = useState('');
  const [namespaces, setNamespaces] = useState<string[]>([]);
  const [documentsLoading, setDocumentsLoading] = useState(false);
  const [namespacesLoading, setNamespacesLoading] = useState(false);
  const [deleteLoadingId, setDeleteLoadingId] = useState<string | null>(null);
  const [documentsError, setDocumentsError] = useState<string | null>(null);
  const [namespacesError, setNamespacesError] = useState<string | null>(null);

  const [namespaceInput, setNamespaceInput] = useState('');
  const [queryInput, setQueryInput] = useState('');
  const [maxChunksInput, setMaxChunksInput] = useState('10');
  const [queryResult, setQueryResult] = useState<MemoryQueryResult | null>(null);
  const [recallResult, setRecallResult] = useState<MemoryQueryResult | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);
  const [recallError, setRecallError] = useState<string | null>(null);
  const [queryLoading, setQueryLoading] = useState(false);
  const [recallLoading, setRecallLoading] = useState(false);

  const [clearNamespaceInput, setClearNamespaceInput] = useState('');
  const [clearLoading, setClearLoading] = useState(false);
  const [clearSuccess, setClearSuccess] = useState<string | null>(null);
  const [clearError, setClearError] = useState<string | null>(null);

  const maxChunks = useMemo(() => {
    const parsed = Number(maxChunksInput);
    if (!Number.isFinite(parsed) || parsed <= 0) return 10;
    return Math.floor(parsed);
  }, [maxChunksInput]);

  const loadDocuments = useCallback(async () => {
    setDocumentsLoading(true);
    setDocumentsError(null);
    try {
      const namespace = documentsNamespaceFilter.trim();
      const payload = await memoryListDocuments(namespace || undefined);
      setDocumentsRaw(payload);
      setDocuments(normalizeMemoryDocuments(payload));
    } catch (error) {
      setDocumentsError(error instanceof Error ? error.message : String(error));
      setDocuments([]);
      setDocumentsRaw(null);
    } finally {
      setDocumentsLoading(false);
    }
  }, [documentsNamespaceFilter]);

  const loadNamespaces = useCallback(async () => {
    setNamespacesLoading(true);
    setNamespacesError(null);
    try {
      const result = await memoryListNamespaces();
      setNamespaces(result);
      if (!namespaceInput && result.length > 0) {
        setNamespaceInput(result[0]);
      }
    } catch (error) {
      setNamespacesError(error instanceof Error ? error.message : String(error));
      setNamespaces([]);
    } finally {
      setNamespacesLoading(false);
    }
  }, [namespaceInput]);

  const refreshAll = useCallback(async () => {
    await Promise.all([loadDocuments(), loadNamespaces()]);
  }, [loadDocuments, loadNamespaces]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  const handleDelete = useCallback(
    async (doc: MemoryDebugDocument) => {
      const confirmed = window.confirm(
        `Delete document "${doc.documentId}" in namespace "${doc.namespace}"?`
      );
      if (!confirmed) return;

      setDeleteLoadingId(doc.documentId);
      try {
        await memoryDeleteDocument(doc.documentId, doc.namespace);
        await refreshAll();
      } catch (error) {
        setDocumentsError(error instanceof Error ? error.message : String(error));
      } finally {
        setDeleteLoadingId(null);
      }
    },
    [refreshAll]
  );

  const handleQuery = useCallback(async () => {
    setQueryLoading(true);
    setQueryError(null);
    setQueryResult(null);
    try {
      const result = await memoryQueryNamespace(
        namespaceInput.trim(),
        queryInput.trim(),
        maxChunks
      );
      setQueryResult(result);
    } catch (error) {
      setQueryError(error instanceof Error ? error.message : String(error));
    } finally {
      setQueryLoading(false);
    }
  }, [maxChunks, namespaceInput, queryInput]);

  const handleRecall = useCallback(async () => {
    setRecallLoading(true);
    setRecallError(null);
    setRecallResult(null);
    try {
      const result = await memoryRecallNamespace(namespaceInput.trim(), maxChunks);
      setRecallResult(result);
    } catch (error) {
      setRecallError(error instanceof Error ? error.message : String(error));
    } finally {
      setRecallLoading(false);
    }
  }, [maxChunks, namespaceInput]);

  const handleClearNamespace = useCallback(async () => {
    const ns = clearNamespaceInput.trim();
    if (!ns) return;

    const confirmed = window.confirm(
      `This will permanently delete ALL documents in namespace "${ns}". Continue?`
    );
    if (!confirmed) return;

    setClearLoading(true);
    setClearError(null);
    setClearSuccess(null);
    try {
      const result = await memoryClearNamespace(ns);
      if (result.cleared) {
        setClearSuccess(`Namespace "${result.namespace}" cleared.`);
      } else {
        setClearSuccess(`Nothing to clear in "${result.namespace}".`);
      }
      await refreshAll();
    } catch (error) {
      setClearError(error instanceof Error ? error.message : String(error));
    } finally {
      setClearLoading(false);
    }
  }, [clearNamespaceInput, refreshAll]);

  return (
    <div>
      <SettingsHeader title="Memory Debug" showBackButton={true} onBack={navigateBack} />

      <div className="p-4 space-y-5">
        {/* Documents */}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-stone-900">Documents</h3>
          <div className="flex gap-2">
            <input
              value={documentsNamespaceFilter}
              onChange={e => setDocumentsNamespaceFilter(e.target.value)}
              className="flex-1 rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-700 placeholder:text-stone-400"
              placeholder="Filter by namespace..."
            />
            <button
              type="button"
              onClick={() => void loadDocuments()}
              disabled={documentsLoading}
              className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs font-medium text-stone-700 hover:bg-stone-100 disabled:opacity-50">
              {documentsLoading ? '...' : 'Refresh'}
            </button>
          </div>
          {documentsError && (
            <div className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
              {documentsError}
            </div>
          )}
          {documents.length === 0 && !documentsLoading ? (
            <p className="text-xs text-stone-400">No documents found.</p>
          ) : (
            <div className="space-y-1">
              {documents.map(doc => (
                <div
                  key={`${doc.namespace}:${doc.documentId}`}
                  className="flex items-start justify-between gap-2 rounded-lg border border-stone-200 bg-stone-50 p-2">
                  <div className="min-w-0">
                    <div className="text-xs font-medium text-stone-900 break-all">{doc.documentId}</div>
                    <div className="text-[11px] text-stone-500 break-all">{doc.namespace}</div>
                    {doc.title && <div className="text-[11px] text-stone-400">{doc.title}</div>}
                  </div>
                  <button
                    type="button"
                    disabled={Boolean(deleteLoadingId)}
                    onClick={() => void handleDelete(doc)}
                    className="shrink-0 rounded border border-stone-200 px-2 py-1 text-[10px] text-stone-500 hover:bg-stone-100 disabled:opacity-50">
                    {deleteLoadingId === doc.documentId ? '...' : 'Delete'}
                  </button>
                </div>
              ))}
            </div>
          )}
          <details className="text-xs">
            <summary className="cursor-pointer text-stone-400">Raw response</summary>
            <pre className="mt-1 max-h-32 overflow-auto rounded-lg border border-stone-200 bg-stone-950 p-2 text-[11px] text-stone-100 whitespace-pre-wrap break-words">
              {JSON.stringify(documentsRaw, null, 2)}
            </pre>
          </details>
        </section>

        {/* Namespaces */}
        <section className="space-y-2">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">Namespaces</h3>
            <button
              type="button"
              onClick={() => void loadNamespaces()}
              disabled={namespacesLoading}
              className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1 text-[11px] font-medium text-stone-600 hover:bg-stone-100 disabled:opacity-50">
              {namespacesLoading ? '...' : 'Refresh'}
            </button>
          </div>
          {namespacesError && (
            <div className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
              {namespacesError}
            </div>
          )}
          {namespaces.length > 0 ? (
            <div className="flex flex-wrap gap-1">
              {namespaces.map(ns => (
                <span key={ns} className="rounded-full bg-stone-100 px-2 py-0.5 text-[11px] text-stone-600">
                  {ns}
                </span>
              ))}
            </div>
          ) : (
            <p className="text-xs text-stone-400">No namespaces found.</p>
          )}
        </section>

        {/* Query & Recall */}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-stone-900">Query & Recall</h3>
          <input
            value={namespaceInput}
            onChange={e => setNamespaceInput(e.target.value)}
            className="w-full rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-700 placeholder:text-stone-400"
            placeholder="Namespace"
          />
          <textarea
            value={queryInput}
            onChange={e => setQueryInput(e.target.value)}
            className="w-full rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-700 placeholder:text-stone-400"
            rows={2}
            placeholder="Query text..."
          />
          <div className="flex items-center gap-2">
            <input
              value={maxChunksInput}
              onChange={e => setMaxChunksInput(e.target.value)}
              className="w-16 rounded-lg border border-stone-200 bg-stone-50 px-2 py-1.5 text-xs text-stone-700"
              placeholder="10"
            />
            <span className="text-[11px] text-stone-400">max chunks</span>
            <div className="flex-1" />
            <button
              type="button"
              onClick={() => void handleQuery()}
              disabled={queryLoading || !namespaceInput.trim() || !queryInput.trim()}
              className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs font-medium text-stone-700 hover:bg-stone-100 disabled:opacity-50">
              {queryLoading ? '...' : 'Query'}
            </button>
            <button
              type="button"
              onClick={() => void handleRecall()}
              disabled={recallLoading || !namespaceInput.trim()}
              className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs font-medium text-stone-700 hover:bg-stone-100 disabled:opacity-50">
              {recallLoading ? '...' : 'Recall'}
            </button>
          </div>
          {queryError && (
            <div className="text-xs text-coral-600">Query: {queryError}</div>
          )}
          {recallError && (
            <div className="text-xs text-coral-600">Recall: {recallError}</div>
          )}
          {(queryResult || recallResult) && (
            <div className="space-y-2">
              {queryResult && (
                <div>
                  <div className="text-[11px] font-medium text-stone-500 mb-1">Query result</div>
                  <MemoryTextWithEntities
                    text={queryResult.text ?? ''}
                    entities={queryResult.entities}
                    className="rounded-lg border border-stone-200 bg-stone-50 p-2 text-[11px] leading-5 min-h-12 whitespace-pre-wrap"
                  />
                </div>
              )}
              {recallResult && (
                <div>
                  <div className="text-[11px] font-medium text-stone-500 mb-1">Recall result</div>
                  <MemoryTextWithEntities
                    text={recallResult.text ?? ''}
                    entities={recallResult.entities}
                    className="rounded-lg border border-stone-200 bg-stone-50 p-2 text-[11px] leading-5 min-h-12 whitespace-pre-wrap"
                  />
                </div>
              )}
            </div>
          )}
        </section>

        {/* Clear Namespace */}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold text-stone-900">Clear Namespace</h3>
          <p className="text-xs text-stone-400">Permanently delete all documents within a namespace.</p>
          <div className="flex gap-2">
            {namespaces.length > 0 ? (
              <select
                value={clearNamespaceInput}
                onChange={e => setClearNamespaceInput(e.target.value)}
                className="flex-1 rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-700">
                <option value="">Select namespace...</option>
                {namespaces.map(ns => (
                  <option key={ns} value={ns}>{ns}</option>
                ))}
              </select>
            ) : (
              <input
                value={clearNamespaceInput}
                onChange={e => setClearNamespaceInput(e.target.value)}
                className="flex-1 rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-700 placeholder:text-stone-400"
                placeholder="e.g. skill:gmail:user@example.com"
              />
            )}
            <button
              type="button"
              onClick={() => void handleClearNamespace()}
              disabled={clearLoading || !clearNamespaceInput.trim()}
              className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-1.5 text-xs font-medium text-coral-600 hover:bg-coral-100 disabled:opacity-50">
              {clearLoading ? '...' : 'Clear'}
            </button>
          </div>
          {clearSuccess && <div className="text-xs text-sage-600">{clearSuccess}</div>}
          {clearError && <div className="text-xs text-coral-600">{clearError}</div>}
        </section>
      </div>
    </div>
  );
};

export default MemoryDebugPanel;
