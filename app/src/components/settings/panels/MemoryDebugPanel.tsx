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
import { PrimaryButton } from './components/ActionPanel';
import SectionCard from './components/SectionCard';
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
      `This will permanently delete ALL documents in namespace "${ns}". This action cannot be undone.\n\nContinue?`
    );
    if (!confirmed) return;

    setClearLoading(true);
    setClearError(null);
    setClearSuccess(null);
    try {
      const result = await memoryClearNamespace(ns);
      if (result.cleared) {
        setClearSuccess(`Namespace "${result.namespace}" cleared successfully.`);
      } else {
        setClearSuccess(`Clear request completed for "${result.namespace}" (nothing to clear).`);
      }
      await refreshAll();
    } catch (error) {
      setClearError(error instanceof Error ? error.message : String(error));
    } finally {
      setClearLoading(false);
    }
  }, [clearNamespaceInput, refreshAll]);

  return (
    <div className="overflow-hidden h-full flex flex-col">
      <SettingsHeader title="Memory Debug" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        <SectionCard
          title="Documents"
          priority="tools"
          icon={
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 12h6m2 8H7a2 2 0 01-2-2V6a2 2 0 012-2h6l6 6v8a2 2 0 01-2 2z"
              />
            </svg>
          }>
          <div className="space-y-3">
            <PrimaryButton onClick={() => void loadDocuments()} loading={documentsLoading}>
              Refresh Documents
            </PrimaryButton>
            <label className="block text-xs text-stone-300">
              Namespace Filter (optional)
              <input
                value={documentsNamespaceFilter}
                onChange={e => setDocumentsNamespaceFilter(e.target.value)}
                className="mt-1 w-full rounded border border-stone-600 bg-black/30 px-3 py-2 text-sm text-white"
                placeholder="e.g. conversations"
              />
            </label>
            {documentsError && (
              <div className="text-xs text-coral-300 border border-coral-500/30 bg-coral-500/10 rounded p-2">
                {documentsError}
              </div>
            )}
            {documents.length === 0 && !documentsLoading ? (
              <div className="text-xs text-stone-400">
                No structured documents found. Raw response is shown below.
              </div>
            ) : (
              <div className="space-y-2">
                {documents.map(doc => (
                  <div
                    key={`${doc.namespace}:${doc.documentId}`}
                    className="rounded border border-stone-700 bg-black/20 p-2">
                    <div className="text-xs text-white break-all">ID: {doc.documentId}</div>
                    <div className="text-xs text-stone-300 break-all">
                      Namespace: {doc.namespace}
                    </div>
                    {doc.title ? (
                      <div className="text-xs text-stone-400">Title: {doc.title}</div>
                    ) : null}
                    <div className="pt-2">
                      <PrimaryButton
                        variant="outline"
                        loading={deleteLoadingId === doc.documentId}
                        disabled={Boolean(deleteLoadingId)}
                        onClick={() => void handleDelete(doc)}
                        className="px-3 py-1.5 text-xs">
                        Delete
                      </PrimaryButton>
                    </div>
                  </div>
                ))}
              </div>
            )}
            <details className="text-xs">
              <summary className="cursor-pointer text-stone-300">Raw documents response</summary>
              <pre className="mt-2 rounded border border-stone-700 bg-black/20 p-2 overflow-auto text-[11px] leading-5">
                {JSON.stringify(documentsRaw, null, 2)}
              </pre>
            </details>
          </div>
        </SectionCard>

        <SectionCard
          title="Namespaces"
          priority="tools"
          icon={
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M4 7h16M4 12h16M4 17h16"
              />
            </svg>
          }>
          <div className="space-y-3">
            <PrimaryButton onClick={() => void loadNamespaces()} loading={namespacesLoading}>
              Refresh Namespaces
            </PrimaryButton>
            {namespacesError && (
              <div className="text-xs text-coral-300 border border-coral-500/30 bg-coral-500/10 rounded p-2">
                {namespacesError}
              </div>
            )}
            <div className="rounded border border-stone-700 bg-black/20 p-2 text-xs">
              {namespaces.length > 0 ? namespaces.join('\n') : 'No namespaces found.'}
            </div>
          </div>
        </SectionCard>

        <SectionCard
          title="Clear Namespace"
          priority="tools"
          icon={
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
              />
            </svg>
          }>
          <div className="space-y-3">
            <p className="text-xs text-stone-400">
              Delete all documents within a namespace. This is a destructive operation and cannot be
              undone.
            </p>

            <label className="block text-xs text-stone-300">
              Namespace
              {namespaces.length > 0 ? (
                <select
                  value={clearNamespaceInput}
                  onChange={e => setClearNamespaceInput(e.target.value)}
                  className="mt-1 w-full rounded border border-stone-600 bg-black/30 px-3 py-2 text-sm text-white">
                  <option value="">-- select a namespace --</option>
                  {namespaces.map(ns => (
                    <option key={ns} value={ns}>
                      {ns}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  value={clearNamespaceInput}
                  onChange={e => setClearNamespaceInput(e.target.value)}
                  className="mt-1 w-full rounded border border-stone-600 bg-black/30 px-3 py-2 text-sm text-white"
                  placeholder="e.g. skill:gmail:user@example.com"
                />
              )}
            </label>

            <PrimaryButton
              variant="outline"
              onClick={() => void handleClearNamespace()}
              loading={clearLoading}
              disabled={!clearNamespaceInput.trim()}
              className="border-coral-500/50 text-coral-300 hover:bg-coral-500/10">
              Clear Namespace
            </PrimaryButton>

            {clearSuccess && (
              <div className="text-xs text-sage-300 border border-sage-500/30 bg-sage-500/10 rounded p-2">
                {clearSuccess}
              </div>
            )}
            {clearError && (
              <div className="text-xs text-coral-300 border border-coral-500/30 bg-coral-500/10 rounded p-2">
                {clearError}
              </div>
            )}
          </div>
        </SectionCard>

        <SectionCard
          title="Query & Recall"
          priority="tools"
          icon={
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M8 10h.01M12 10h.01M16 10h.01M9 16h6M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
              />
            </svg>
          }>
          <div className="space-y-3">
            <label className="block text-xs text-stone-300">
              Namespace
              <input
                value={namespaceInput}
                onChange={e => setNamespaceInput(e.target.value)}
                className="mt-1 w-full rounded border border-stone-600 bg-black/30 px-3 py-2 text-sm text-white"
                placeholder="e.g. conversations"
              />
            </label>

            <label className="block text-xs text-stone-300">
              Query
              <textarea
                value={queryInput}
                onChange={e => setQueryInput(e.target.value)}
                className="mt-1 w-full rounded border border-stone-600 bg-black/30 px-3 py-2 text-sm text-white"
                rows={3}
                placeholder="What do I remember about..."
              />
            </label>

            <label className="block text-xs text-stone-300">
              Max Chunks
              <input
                value={maxChunksInput}
                onChange={e => setMaxChunksInput(e.target.value)}
                className="mt-1 w-full rounded border border-stone-600 bg-black/30 px-3 py-2 text-sm text-white"
              />
            </label>

            <div className="flex flex-wrap gap-2">
              <PrimaryButton
                onClick={() => void handleQuery()}
                loading={queryLoading}
                disabled={!namespaceInput.trim() || !queryInput.trim()}>
                Run Query
              </PrimaryButton>
              <PrimaryButton
                variant="secondary"
                onClick={() => void handleRecall()}
                loading={recallLoading}
                disabled={!namespaceInput.trim()}>
                Run Recall
              </PrimaryButton>
            </div>

            {queryError && (
              <div className="text-xs text-coral-300 border border-coral-500/30 bg-coral-500/10 rounded p-2">
                Query error: {queryError}
              </div>
            )}
            {recallError && (
              <div className="text-xs text-coral-300 border border-coral-500/30 bg-coral-500/10 rounded p-2">
                Recall error: {recallError}
              </div>
            )}

            <div className="space-y-2">
              <div className="text-xs text-stone-400">Query response</div>
              <MemoryTextWithEntities
                text={queryResult?.text ?? ''}
                entities={queryResult?.entities}
                className="rounded border border-stone-700 bg-black/20 p-2 overflow-auto text-[11px] leading-5 min-h-16 whitespace-pre-wrap"
              />
              <div className="text-xs text-stone-400">Recall response</div>
              <MemoryTextWithEntities
                text={recallResult?.text ?? ''}
                entities={recallResult?.entities}
                className="rounded border border-stone-700 bg-black/20 p-2 overflow-auto text-[11px] leading-5 min-h-16 whitespace-pre-wrap"
              />
            </div>
          </div>
        </SectionCard>
      </div>
    </div>
  );
};

export default MemoryDebugPanel;
