import { useCallback, useEffect, useMemo, useState } from 'react';

import { useIntelligenceStats } from '../../hooks/useIntelligenceStats';
import type { ToastNotification } from '../../types/intelligence';
import {
  aiListMemoryFiles,
  aiReadMemoryFile,
  aiWriteMemoryFile,
  type GraphRelation,
  isTauri,
  memoryDeleteDocument,
  memoryGraphQuery,
  memoryListDocuments,
  memoryListNamespaces,
  memoryQueryNamespace,
  memoryRecallNamespace,
} from '../../utils/tauriCommands';
import { MemoryGraphMap } from './MemoryGraphMap';
import { MemoryHeatmap } from './MemoryHeatmap';
import { MemoryInsights } from './MemoryInsights';
import { MemoryStatsBar } from './MemoryStatsBar';

type MemoryDoc = { documentId: string; namespace: string; title?: string; raw: unknown };

interface MemoryWorkspaceProps {
  onToast: (toast: Omit<ToastNotification, 'id'>) => void;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function pickString(record: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim()) return value;
  }
  return undefined;
}

function findDocumentRows(payload: unknown): unknown[] {
  if (Array.isArray(payload)) return payload;
  const root = asRecord(payload);
  if (!root) return [];
  for (const key of ['documents', 'items', 'results']) {
    const value = root[key];
    if (Array.isArray(value)) return value;
  }
  const data = asRecord(root.data);
  if (!data) return [];
  for (const key of ['documents', 'items', 'results']) {
    const value = data[key];
    if (Array.isArray(value)) return value;
  }
  return [];
}

function normalizeMemoryDocuments(payload: unknown): MemoryDoc[] {
  return findDocumentRows(payload)
    .map(row => {
      const record = asRecord(row);
      if (!record) return null;
      const documentId = pickString(record, ['documentId', 'document_id', 'id']);
      const namespace = pickString(record, ['namespace']);
      const title = pickString(record, ['title', 'name']);
      if (!documentId || !namespace) return null;
      return { documentId, namespace, title, raw: row } as MemoryDoc;
    })
    .filter((doc): doc is MemoryDoc => Boolean(doc));
}

function extractTimestamp(raw: unknown): number | null {
  const record = asRecord(raw);
  if (!record) return null;
  for (const key of [
    'createdAt',
    'created_at',
    'updatedAt',
    'updated_at',
    'timestamp',
    'insertedAt',
    'inserted_at',
  ]) {
    const value = record[key];
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value > 9999999999 ? value / 1000 : value;
    }
    if (typeof value === 'string') {
      const date = new Date(value);
      if (!Number.isNaN(date.getTime())) return date.getTime() / 1000;
    }
  }
  return null;
}

function estimateContentSize(raw: unknown): number {
  const record = asRecord(raw);
  if (!record) return 0;
  const content = record.content;
  if (typeof content === 'string') return new TextEncoder().encode(content).length;
  return 0;
}

function isSameLocalDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function MemoryWorkspace({ onToast }: MemoryWorkspaceProps) {
  const { sessions, isLoading: statsLoading, refetch: refetchStats } = useIntelligenceStats();

  const [memoryDocs, setMemoryDocs] = useState<MemoryDoc[]>([]);
  const [memoryNamespaces, setMemoryNamespaces] = useState<string[]>([]);
  const [memoryFilesList, setMemoryFilesList] = useState<string[]>([]);
  const [memoryWorkspaceLoading, setMemoryWorkspaceLoading] = useState(false);
  const [memoryWorkspaceError, setMemoryWorkspaceError] = useState<string | null>(null);

  const [graphRelations, setGraphRelations] = useState<GraphRelation[]>([]);
  const [graphRelationsLoading, setGraphRelationsLoading] = useState(false);

  // Manage memory section (collapsible)
  const [manageOpen, setManageOpen] = useState(false);
  const [selectedNamespace, setSelectedNamespace] = useState('');
  const [selectedFile, setSelectedFile] = useState('memory.md');
  const [selectedFileContent, setSelectedFileContent] = useState('');
  const [selectedFileLoading, setSelectedFileLoading] = useState(false);
  const [selectedFileError, setSelectedFileError] = useState<string | null>(null);

  const [queryInput, setQueryInput] = useState('important user preferences and active goals');
  const [queryResult, setQueryResult] = useState('');
  const [queryLoading, setQueryLoading] = useState(false);
  const [recallResult, setRecallResult] = useState('');
  const [recallLoading, setRecallLoading] = useState(false);
  const [memoryActionError, setMemoryActionError] = useState<string | null>(null);

  const [memoryNote, setMemoryNote] = useState('');
  const [memoryNoteSaving, setMemoryNoteSaving] = useState(false);

  // ---------------------------------------------------------------------------
  // Data loading
  // ---------------------------------------------------------------------------

  const loadWorkspace = useCallback(async () => {
    if (!isTauri()) return;
    setMemoryWorkspaceLoading(true);
    setMemoryWorkspaceError(null);

    try {
      const [documentsPayload, namespacesPayload, memoryDirFiles] = await Promise.all([
        memoryListDocuments(),
        memoryListNamespaces(),
        aiListMemoryFiles('memory'),
      ]);

      setGraphRelationsLoading(true);
      try {
        const relations = await memoryGraphQuery();
        setGraphRelations(relations);
      } catch {
        setGraphRelations([]);
      } finally {
        setGraphRelationsLoading(false);
      }

      const docs = normalizeMemoryDocuments(documentsPayload);
      const combinedFiles = ['memory.md', ...memoryDirFiles.map(file => `memory/${file}`)];

      setMemoryDocs(docs);
      setMemoryNamespaces(namespacesPayload);
      setMemoryFilesList(combinedFiles);

      if (!selectedNamespace && namespacesPayload.length > 0) {
        setSelectedNamespace(namespacesPayload[0]);
      }
      if (!combinedFiles.includes(selectedFile)) {
        setSelectedFile(combinedFiles[0] || 'memory.md');
      }
    } catch (error) {
      setMemoryWorkspaceError(error instanceof Error ? error.message : 'Failed to load memory');
      setMemoryDocs([]);
      setMemoryNamespaces([]);
      setMemoryFilesList([]);
    } finally {
      setMemoryWorkspaceLoading(false);
    }
  }, [selectedFile, selectedNamespace]);

  const loadSelectedFile = useCallback(async () => {
    if (!isTauri() || !selectedFile) return;
    setSelectedFileLoading(true);
    setSelectedFileError(null);
    try {
      const content = await aiReadMemoryFile(selectedFile);
      setSelectedFileContent(content);
    } catch (error) {
      setSelectedFileError(error instanceof Error ? error.message : 'Failed to load file');
      setSelectedFileContent('');
    } finally {
      setSelectedFileLoading(false);
    }
  }, [selectedFile]);

  useEffect(() => {
    loadWorkspace();
  }, [loadWorkspace]);
  useEffect(() => {
    loadSelectedFile();
  }, [loadSelectedFile]);

  // ---------------------------------------------------------------------------
  // Management handlers
  // ---------------------------------------------------------------------------

  const handleDeleteMemoryDoc = useCallback(
    async (doc: MemoryDoc) => {
      const confirmed = window.confirm(
        `Delete document "${doc.documentId}" from namespace "${doc.namespace}"?`
      );
      if (!confirmed) return;
      try {
        await memoryDeleteDocument(doc.documentId, doc.namespace);
        await loadWorkspace();
        await refetchStats();
        onToast({
          type: 'success',
          title: 'Document Deleted',
          message: `${doc.documentId} removed from ${doc.namespace}`,
        });
      } catch (error) {
        setMemoryActionError(error instanceof Error ? error.message : 'Delete failed');
      }
    },
    [loadWorkspace, onToast, refetchStats]
  );

  const handleQueryNamespace = useCallback(async () => {
    if (!selectedNamespace || !queryInput.trim()) return;
    setQueryLoading(true);
    setMemoryActionError(null);
    try {
      const response = await memoryQueryNamespace(selectedNamespace, queryInput.trim(), 10);
      setQueryResult(response);
    } catch (error) {
      setMemoryActionError(error instanceof Error ? error.message : 'Query failed');
      setQueryResult('');
    } finally {
      setQueryLoading(false);
    }
  }, [queryInput, selectedNamespace]);

  const handleRecallNamespace = useCallback(async () => {
    if (!selectedNamespace) return;
    setRecallLoading(true);
    setMemoryActionError(null);
    try {
      const response = await memoryRecallNamespace(selectedNamespace, 10);
      setRecallResult(response ?? '');
    } catch (error) {
      setMemoryActionError(error instanceof Error ? error.message : 'Recall failed');
      setRecallResult('');
    } finally {
      setRecallLoading(false);
    }
  }, [selectedNamespace]);

  const handleSaveMemoryNote = useCallback(async () => {
    if (!memoryNote.trim()) return;
    setMemoryNoteSaving(true);
    setMemoryActionError(null);
    try {
      let existing = '';
      try {
        existing = await aiReadMemoryFile('memory.md');
      } catch {
        existing = '';
      }
      const timestamp = new Date().toLocaleString();
      const noteBlock = `\n\n## Manual note (${timestamp})\n${memoryNote.trim()}\n`;
      const nextContent = existing ? `${existing}${noteBlock}` : `# Memory\n${noteBlock}`;
      await aiWriteMemoryFile('memory.md', nextContent);
      setMemoryNote('');
      await loadWorkspace();
      await loadSelectedFile();
      await refetchStats();
      onToast({
        type: 'success',
        title: 'Memory Updated',
        message: 'Your note was saved to memory.md',
      });
    } catch (error) {
      setMemoryActionError(error instanceof Error ? error.message : 'Failed to save note');
    } finally {
      setMemoryNoteSaving(false);
    }
  }, [loadSelectedFile, loadWorkspace, memoryNote, onToast, refetchStats]);

  // ---------------------------------------------------------------------------
  // Derived data
  // ---------------------------------------------------------------------------

  const today = new Date();
  const docsToday = memoryDocs.filter(doc => {
    const ts = extractTimestamp(doc.raw);
    return ts ? isSameLocalDay(new Date(ts * 1000), today) : false;
  }).length;

  const estimatedStorageBytes = useMemo(
    () => memoryDocs.reduce((sum, doc) => sum + estimateContentSize(doc.raw), 0),
    [memoryDocs]
  );

  const docTimestamps = useMemo(
    () => memoryDocs.map(doc => extractTimestamp(doc.raw)).filter((t): t is number => t !== null),
    [memoryDocs]
  );

  const oldestTimestamp = docTimestamps.length > 0 ? Math.min(...docTimestamps) : null;
  const newestTimestamp = docTimestamps.length > 0 ? Math.max(...docTimestamps) : null;

  // Combine doc timestamps + graph relation updated_at for heatmap
  const heatmapTimestamps = useMemo(() => {
    const timestamps = [...docTimestamps];
    for (const rel of graphRelations) {
      if (rel.updatedAt) timestamps.push(rel.updatedAt);
    }
    return timestamps;
  }, [docTimestamps, graphRelations]);

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <section className="space-y-4 animate-fade-up">
      {/* Header */}
      <div className="glass rounded-2xl p-5 border border-white/10">
        <div className="flex items-start justify-between gap-4 mb-5">
          <div>
            <h2 className="text-lg font-semibold text-white">Memory</h2>
            <p className="text-sm text-stone-400">
              Your AI's knowledge graph, extracted insights, and ingestion activity.
            </p>
          </div>
          <button
            onClick={() => {
              void Promise.all([loadWorkspace(), refetchStats()]);
            }}
            disabled={memoryWorkspaceLoading || statsLoading}
            className="px-3 py-1.5 text-xs bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg text-stone-300 disabled:opacity-40 transition-colors">
            {memoryWorkspaceLoading ? 'Loading...' : 'Refresh'}
          </button>
        </div>

        {/* Stats bar */}
        <MemoryStatsBar
          totalDocs={memoryDocs.length}
          totalFiles={memoryFilesList.length}
          totalNamespaces={memoryNamespaces.length}
          totalRelations={graphRelations.length}
          totalSessions={sessions?.total ?? null}
          totalTokens={sessions?.totalTokens ?? null}
          estimatedStorageBytes={estimatedStorageBytes}
          oldestDocTimestamp={oldestTimestamp}
          newestDocTimestamp={newestTimestamp}
          docsToday={docsToday}
          loading={memoryWorkspaceLoading || statsLoading}
        />
      </div>

      {/* Knowledge Graph */}
      <MemoryGraphMap relations={graphRelations} loading={graphRelationsLoading} />

      {/* Intelligent Insights */}
      <MemoryInsights relations={graphRelations} loading={graphRelationsLoading} />

      {/* Activity Heatmap */}
      <MemoryHeatmap timestamps={heatmapTimestamps} loading={memoryWorkspaceLoading} />

      {/* Collapsible: Files & Management */}
      <div className="rounded-xl border border-white/10 bg-black/20">
        <button
          onClick={() => setManageOpen(!manageOpen)}
          className="w-full flex items-center justify-between p-4 text-left hover:bg-white/5 transition-colors rounded-xl">
          <div className="flex items-center gap-2">
            <svg
              className={`w-4 h-4 text-stone-400 transition-transform ${manageOpen ? 'rotate-90' : ''}`}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
            <h3 className="text-sm font-semibold text-white">Files & Management</h3>
            <span className="text-xs text-stone-500">
              {memoryFilesList.length} files · {memoryNamespaces.length} namespaces ·{' '}
              {memoryDocs.length} docs
            </span>
          </div>
        </button>

        {manageOpen && (
          <div className="px-4 pb-4 space-y-4 animate-fade-up">
            {/* File browser */}
            <div>
              <h4 className="text-xs font-medium text-stone-400 mb-2">Memory Files</h4>
              <div className="grid grid-cols-1 lg:grid-cols-[220px_1fr] gap-3">
                <div className="rounded-lg border border-white/10 bg-stone-950/50 p-2 h-52 overflow-y-auto">
                  {memoryFilesList.length === 0 ? (
                    <div className="text-xs text-stone-500 p-2">No files found.</div>
                  ) : (
                    memoryFilesList.map(filePath => (
                      <button
                        key={filePath}
                        onClick={() => setSelectedFile(filePath)}
                        className={`w-full text-left px-2 py-1.5 rounded text-xs mb-1 border transition-colors ${
                          selectedFile === filePath
                            ? 'border-primary-400/40 bg-primary-500/20 text-primary-200'
                            : 'border-transparent hover:border-white/10 hover:bg-white/5 text-stone-300'
                        }`}>
                        {filePath}
                      </button>
                    ))
                  )}
                </div>
                <div className="rounded-lg border border-white/10 bg-stone-950/50 p-3 h-52 overflow-auto">
                  {selectedFileLoading ? (
                    <div className="text-xs text-stone-400">Loading file...</div>
                  ) : selectedFileError ? (
                    <div className="text-xs text-coral-300">{selectedFileError}</div>
                  ) : (
                    <pre className="text-[11px] leading-5 text-stone-200 whitespace-pre-wrap">
                      {selectedFileContent || 'Empty file'}
                    </pre>
                  )}
                </div>
              </div>
            </div>

            {/* Namespace query & management */}
            <div className="grid grid-cols-1 xl:grid-cols-3 gap-4">
              <div>
                <h4 className="text-xs font-medium text-stone-400 mb-2">Manage Namespace</h4>
                <select
                  value={selectedNamespace}
                  onChange={e => setSelectedNamespace(e.target.value)}
                  className="w-full px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white mb-3 focus:outline-none focus:border-primary-500/50">
                  {memoryNamespaces.length === 0 ? (
                    <option value="">No namespaces</option>
                  ) : (
                    memoryNamespaces.map(ns => (
                      <option key={ns} value={ns}>
                        {ns}
                      </option>
                    ))
                  )}
                </select>

                <label className="block text-xs text-stone-400 mb-1">Query</label>
                <textarea
                  value={queryInput}
                  onChange={e => setQueryInput(e.target.value)}
                  rows={2}
                  className="w-full px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white mb-2 focus:outline-none focus:border-primary-500/50"
                  placeholder="Search this namespace..."
                />
                <div className="flex gap-2 mb-3">
                  <button
                    onClick={() => void handleQueryNamespace()}
                    disabled={!selectedNamespace || !queryInput.trim() || queryLoading}
                    className="px-3 py-1.5 text-xs rounded-lg border border-white/10 bg-white/5 hover:bg-white/10 text-stone-200 disabled:opacity-40">
                    {queryLoading ? 'Querying...' : 'Run Query'}
                  </button>
                  <button
                    onClick={() => void handleRecallNamespace()}
                    disabled={!selectedNamespace || recallLoading}
                    className="px-3 py-1.5 text-xs rounded-lg border border-white/10 bg-white/5 hover:bg-white/10 text-stone-200 disabled:opacity-40">
                    {recallLoading ? 'Recalling...' : 'Run Recall'}
                  </button>
                </div>

                <label className="block text-xs text-stone-400 mb-1">Add Note</label>
                <textarea
                  value={memoryNote}
                  onChange={e => setMemoryNote(e.target.value)}
                  rows={2}
                  className="w-full px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white mb-2 focus:outline-none focus:border-primary-500/50"
                  placeholder="Store a durable user fact, preference, or decision"
                />
                <button
                  onClick={() => void handleSaveMemoryNote()}
                  disabled={!memoryNote.trim() || memoryNoteSaving}
                  className="px-3 py-1.5 text-xs rounded-lg border border-primary-500/40 bg-primary-500/20 hover:bg-primary-500/30 text-primary-200 disabled:opacity-40">
                  {memoryNoteSaving ? 'Saving...' : 'Save Note'}
                </button>
                {memoryActionError && (
                  <div className="mt-2 text-xs text-coral-300 border border-coral-500/30 bg-coral-500/10 rounded p-2">
                    {memoryActionError}
                  </div>
                )}
              </div>

              <div className="xl:col-span-2">
                <h4 className="text-xs font-medium text-stone-400 mb-2">Namespace Responses</h4>
                <div className="grid grid-cols-1 lg:grid-cols-2 gap-3 mb-3">
                  <div>
                    <div className="text-[11px] text-stone-500 mb-1">Query response</div>
                    <pre className="rounded-lg border border-white/10 bg-stone-950/50 p-2 h-28 overflow-auto text-[11px] leading-5 text-stone-200 whitespace-pre-wrap">
                      {queryResult || 'No query result yet.'}
                    </pre>
                  </div>
                  <div>
                    <div className="text-[11px] text-stone-500 mb-1">Recall response</div>
                    <pre className="rounded-lg border border-white/10 bg-stone-950/50 p-2 h-28 overflow-auto text-[11px] leading-5 text-stone-200 whitespace-pre-wrap">
                      {recallResult || 'No recall result yet.'}
                    </pre>
                  </div>
                </div>

                <div className="text-[11px] text-stone-500 mb-2">Recent documents</div>
                <div className="space-y-1.5 max-h-40 overflow-y-auto pr-1">
                  {memoryDocs.slice(0, 8).map(doc => (
                    <div
                      key={`${doc.namespace}:${doc.documentId}`}
                      className="flex items-center justify-between gap-3 rounded-lg border border-white/10 bg-stone-950/50 px-3 py-2">
                      <div className="min-w-0">
                        <div className="text-xs text-white truncate">
                          {doc.title || doc.documentId}
                        </div>
                        <div className="text-[11px] text-stone-400 truncate">
                          {doc.namespace} · {doc.documentId}
                        </div>
                      </div>
                      <button
                        onClick={() => void handleDeleteMemoryDoc(doc)}
                        className="text-[11px] px-2 py-1 rounded border border-coral-500/30 text-coral-300 hover:bg-coral-500/10 shrink-0">
                        Delete
                      </button>
                    </div>
                  ))}
                  {memoryDocs.length === 0 && (
                    <div className="text-xs text-stone-500">No documents available.</div>
                  )}
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Warnings */}
      {(memoryWorkspaceError || (!isTauri() && !memoryWorkspaceLoading)) && (
        <div className="text-xs text-amber-300 border border-amber-500/30 bg-amber-500/10 rounded-lg p-3">
          {memoryWorkspaceError ||
            'Memory workspace requires the desktop Tauri runtime to load real data.'}
        </div>
      )}
    </section>
  );
}
