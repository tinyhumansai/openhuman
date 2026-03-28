import { useCallback, useEffect, useMemo, useState } from 'react';

import { useIntelligenceStats } from '../../hooks/useIntelligenceStats';
import type { ToastNotification } from '../../types/intelligence';
import {
  aiListMemoryFiles,
  aiReadMemoryFile,
  aiWriteMemoryFile,
  isTauri,
  memoryDeleteDocument,
  memoryListDocuments,
  memoryListNamespaces,
  memoryQueryNamespace,
  memoryRecallNamespace,
} from '../../utils/tauriCommands';

type MemoryDoc = { documentId: string; namespace: string; title?: string; raw: unknown };

type MemoryNode = { label: string; value?: string };

interface MemoryWorkspaceProps {
  onToast: (toast: Omit<ToastNotification, 'id'>) => void;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

function pickString(record: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim()) {
      return value;
    }
  }
  return undefined;
}

function findDocumentRows(payload: unknown): unknown[] {
  if (Array.isArray(payload)) {
    return payload;
  }

  const root = asRecord(payload);
  if (!root) return [];

  const candidates = ['documents', 'items', 'results'];
  for (const key of candidates) {
    const value = root[key];
    if (Array.isArray(value)) return value;
  }

  const data = asRecord(root.data);
  if (!data) return [];

  for (const key of candidates) {
    const value = data[key];
    if (Array.isArray(value)) return value;
  }

  return [];
}

function normalizeMemoryDocuments(payload: unknown): MemoryDoc[] {
  const rows = findDocumentRows(payload);

  return rows
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

function parseTimestamp(raw: unknown): Date | null {
  const record = asRecord(raw);
  if (!record) return null;

  const maybeDateKeys = [
    'createdAt',
    'created_at',
    'updatedAt',
    'updated_at',
    'timestamp',
    'insertedAt',
    'inserted_at',
  ];

  for (const key of maybeDateKeys) {
    const value = record[key];

    if (typeof value === 'number' && Number.isFinite(value)) {
      const asMs = value > 9999999999 ? value : value * 1000;
      const date = new Date(asMs);
      if (!Number.isNaN(date.getTime())) return date;
    }

    if (typeof value === 'string') {
      const date = new Date(value);
      if (!Number.isNaN(date.getTime())) return date;
    }
  }

  return null;
}

function isSameLocalDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

export function MemoryWorkspace({ onToast }: MemoryWorkspaceProps) {
  const {
    sessions,
    memoryFiles,
    entities,
    isLoading: statsLoading,
    refetch: refetchStats,
  } = useIntelligenceStats();

  const [memoryDocs, setMemoryDocs] = useState<MemoryDoc[]>([]);
  const [memoryNamespaces, setMemoryNamespaces] = useState<string[]>([]);
  const [memoryFilesList, setMemoryFilesList] = useState<string[]>([]);
  const [memoryWorkspaceLoading, setMemoryWorkspaceLoading] = useState(false);
  const [memoryWorkspaceError, setMemoryWorkspaceError] = useState<string | null>(null);

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

  const today = new Date();
  const todayKey = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, '0')}-${String(
    today.getDate()
  ).padStart(2, '0')}`;
  const memoryFilesToday = memoryFilesList.filter(filePath => filePath.includes(todayKey)).length;
  const memoryDocsToday = memoryDocs.filter(doc => {
    const timestamp = parseTimestamp(doc.raw);
    return timestamp ? isSameLocalDay(timestamp, today) : false;
  }).length;

  const topNamespaces = useMemo(() => {
    const counter = new Map<string, number>();

    for (const doc of memoryDocs) {
      counter.set(doc.namespace, (counter.get(doc.namespace) || 0) + 1);
    }

    return Array.from(counter.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, 3)
      .map(([namespace, count]) => ({ namespace, count }));
  }, [memoryDocs]);

  const graphNodes = useMemo<MemoryNode[]>(() => {
    const namespaceNodes = topNamespaces.map(item => ({
      label: item.namespace,
      value: `${item.count} docs`,
    }));

    const entityNodes = Object.entries(entities || {})
      .sort((a, b) => b[1] - a[1])
      .slice(0, 3)
      .map(([type, count]) => ({ label: type, value: `${count} entities` }));

    return [...namespaceNodes, ...entityNodes].slice(0, 6);
  }, [entities, topNamespaces]);

  return (
    <section className="glass rounded-2xl p-5 border border-white/10 animate-fade-up">
      <div className="flex items-start justify-between gap-4 mb-4">
        <div>
          <h2 className="text-lg font-semibold text-white">Memory Workspace</h2>
          <p className="text-sm text-stone-400">
            Browse memory files, inspect graph relationships, and manage stored context.
          </p>
        </div>
        <button
          onClick={() => {
            void Promise.all([loadWorkspace(), refetchStats()]);
          }}
          disabled={memoryWorkspaceLoading || statsLoading}
          className="px-3 py-1.5 text-xs bg-white/5 hover:bg-white/10 border border-white/10 rounded-lg text-stone-300 disabled:opacity-40">
          Refresh Memory
        </button>
      </div>

      <div className="grid grid-cols-2 lg:grid-cols-4 gap-3 mb-5">
        <div className="rounded-xl border border-white/10 bg-black/20 p-3">
          <div className="text-[11px] uppercase tracking-wide text-stone-500">Files (All-Time)</div>
          <div className="text-xl text-white font-semibold mt-1">
            {memoryFiles !== null ? formatNumber(memoryFiles) : '--'}
          </div>
        </div>
        <div className="rounded-xl border border-white/10 bg-black/20 p-3">
          <div className="text-[11px] uppercase tracking-wide text-stone-500">Files (Today)</div>
          <div className="text-xl text-white font-semibold mt-1">
            {formatNumber(memoryFilesToday)}
          </div>
        </div>
        <div className="rounded-xl border border-white/10 bg-black/20 p-3">
          <div className="text-[11px] uppercase tracking-wide text-stone-500">Docs (All-Time)</div>
          <div className="text-xl text-white font-semibold mt-1">
            {formatNumber(memoryDocs.length)}
          </div>
        </div>
        <div className="rounded-xl border border-white/10 bg-black/20 p-3">
          <div className="text-[11px] uppercase tracking-wide text-stone-500">Docs (Today)</div>
          <div className="text-xl text-white font-semibold mt-1">
            {formatNumber(memoryDocsToday)}
          </div>
        </div>
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-3 gap-4">
        <div className="rounded-xl border border-white/10 bg-black/20 p-4">
          <h3 className="text-sm font-semibold text-white mb-3">Sample Memory Graph</h3>
          <div className="relative h-64 rounded-lg bg-stone-950/70 overflow-hidden border border-white/5">
            <svg
              className="absolute inset-0 w-full h-full"
              viewBox="0 0 100 100"
              preserveAspectRatio="none">
              {graphNodes.map((_, index) => {
                const points = [
                  [50, 12],
                  [80, 24],
                  [88, 52],
                  [76, 80],
                  [24, 80],
                  [12, 50],
                ] as const;
                const [x, y] = points[index] || [50, 90];

                return (
                  <line
                    key={`line-${index}`}
                    x1="50"
                    y1="50"
                    x2={x}
                    y2={y}
                    stroke="rgba(74, 131, 221, 0.45)"
                    strokeWidth="0.6"
                  />
                );
              })}
            </svg>

            <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 px-2.5 py-1.5 rounded-full bg-primary-500/20 border border-primary-400/40 text-[11px] text-primary-200 whitespace-nowrap">
              You (Core Memory)
            </div>

            {graphNodes.map((node, index) => {
              const positions = [
                'left-1/2 top-3 -translate-x-1/2',
                'right-3 top-9',
                'right-2 top-1/2 -translate-y-1/2',
                'right-8 bottom-3',
                'left-8 bottom-3',
                'left-2 top-1/2 -translate-y-1/2',
              ];

              return (
                <div
                  key={`node-${node.label}`}
                  className={`absolute ${positions[index] || positions[0]} max-w-28 px-2 py-1 rounded-md bg-white/5 border border-white/10`}>
                  <div className="text-[11px] text-white truncate">{node.label}</div>
                  {node.value && (
                    <div className="text-[10px] text-stone-400 truncate">{node.value}</div>
                  )}
                </div>
              );
            })}

            {graphNodes.length === 0 && (
              <div className="absolute inset-0 flex items-center justify-center text-xs text-stone-500">
                No graph nodes yet. Refresh memory to load.
              </div>
            )}
          </div>
          <div className="mt-3 text-xs text-stone-400">
            Nodes show top namespaces and entity buckets connected to your core profile.
          </div>
        </div>

        <div className="rounded-xl border border-white/10 bg-black/20 p-4 xl:col-span-2">
          <h3 className="text-sm font-semibold text-white mb-3">Memory Files</h3>
          <div className="grid grid-cols-1 lg:grid-cols-[220px_1fr] gap-3">
            <div className="rounded-lg border border-white/10 bg-stone-950/50 p-2 h-64 overflow-y-auto">
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

            <div className="rounded-lg border border-white/10 bg-stone-950/50 p-3 h-64 overflow-auto">
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
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-3 gap-4 mt-4">
        <div className="rounded-xl border border-white/10 bg-black/20 p-4">
          <h3 className="text-sm font-semibold text-white mb-3">Manage Memory</h3>
          <label className="block text-xs text-stone-400 mb-2">Namespace</label>
          <select
            value={selectedNamespace}
            onChange={event => setSelectedNamespace(event.target.value)}
            className="w-full px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white mb-3 focus:outline-none focus:border-primary-500/50">
            {memoryNamespaces.length === 0 ? (
              <option value="">No namespaces</option>
            ) : (
              memoryNamespaces.map(namespace => (
                <option key={namespace} value={namespace}>
                  {namespace}
                </option>
              ))
            )}
          </select>

          <label className="block text-xs text-stone-400 mb-2">Query</label>
          <textarea
            value={queryInput}
            onChange={event => setQueryInput(event.target.value)}
            rows={3}
            className="w-full px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white mb-3 focus:outline-none focus:border-primary-500/50"
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

          <label className="block text-xs text-stone-400 mb-2">Add Note to memory.md</label>
          <textarea
            value={memoryNote}
            onChange={event => setMemoryNote(event.target.value)}
            rows={3}
            className="w-full px-3 py-2 text-sm bg-white/5 border border-white/10 rounded-lg text-white mb-3 focus:outline-none focus:border-primary-500/50"
            placeholder="Store a durable user fact, preference, or decision"
          />
          <button
            onClick={() => void handleSaveMemoryNote()}
            disabled={!memoryNote.trim() || memoryNoteSaving}
            className="px-3 py-1.5 text-xs rounded-lg border border-primary-500/40 bg-primary-500/20 hover:bg-primary-500/30 text-primary-200 disabled:opacity-40">
            {memoryNoteSaving ? 'Saving...' : 'Save Note'}
          </button>
          {memoryActionError && (
            <div className="mt-3 text-xs text-coral-300 border border-coral-500/30 bg-coral-500/10 rounded p-2">
              {memoryActionError}
            </div>
          )}
        </div>

        <div className="rounded-xl border border-white/10 bg-black/20 p-4 xl:col-span-2">
          <h3 className="text-sm font-semibold text-white mb-3">Namespace Responses</h3>
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-3 mb-3">
            <div>
              <div className="text-xs text-stone-400 mb-1">Query response</div>
              <pre className="rounded-lg border border-white/10 bg-stone-950/50 p-2 h-36 overflow-auto text-[11px] leading-5 text-stone-200 whitespace-pre-wrap">
                {queryResult || 'No query result yet.'}
              </pre>
            </div>
            <div>
              <div className="text-xs text-stone-400 mb-1">Recall response</div>
              <pre className="rounded-lg border border-white/10 bg-stone-950/50 p-2 h-36 overflow-auto text-[11px] leading-5 text-stone-200 whitespace-pre-wrap">
                {recallResult || 'No recall result yet.'}
              </pre>
            </div>
          </div>

          <div className="text-xs text-stone-400 mb-2">Top namespaces by document volume</div>
          <div className="flex flex-wrap gap-2 mb-3">
            {topNamespaces.length === 0 ? (
              <div className="text-xs text-stone-500">No namespace data yet.</div>
            ) : (
              topNamespaces.map(item => (
                <div
                  key={item.namespace}
                  className="px-2 py-1 rounded-full border border-white/10 bg-white/5 text-xs text-stone-300">
                  {item.namespace}: {item.count}
                </div>
              ))
            )}
          </div>

          <div className="text-xs text-stone-400 mb-2">Recent documents</div>
          <div className="space-y-2 max-h-40 overflow-y-auto pr-1">
            {memoryDocs.slice(0, 8).map(doc => (
              <div
                key={`${doc.namespace}:${doc.documentId}`}
                className="flex items-center justify-between gap-3 rounded-lg border border-white/10 bg-stone-950/50 px-3 py-2">
                <div className="min-w-0">
                  <div className="text-xs text-white truncate">{doc.title || doc.documentId}</div>
                  <div className="text-[11px] text-stone-400 truncate">
                    {doc.namespace} · {doc.documentId}
                  </div>
                </div>
                <button
                  onClick={() => void handleDeleteMemoryDoc(doc)}
                  className="text-[11px] px-2 py-1 rounded border border-coral-500/30 text-coral-300 hover:bg-coral-500/10">
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

      {(memoryWorkspaceError || (!isTauri() && !memoryWorkspaceLoading)) && (
        <div className="mt-4 text-xs text-amber-300 border border-amber-500/30 bg-amber-500/10 rounded p-2">
          {memoryWorkspaceError ||
            'Memory workspace requires the desktop Tauri runtime to load real data.'}
        </div>
      )}

      <div className="mt-3 text-[11px] text-stone-500">
        Sessions: {sessions ? formatNumber(sessions.total) : '--'} · Token volume:{' '}
        {sessions ? formatNumber(sessions.totalTokens) : '--'}
        {entities && <> · Entity buckets: {formatNumber(Object.keys(entities).length)}</>}
      </div>
    </section>
  );
}
