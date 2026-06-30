import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
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
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import SettingsBackButton from '../components/SettingsBackButton';
import {
  SettingsEmptyState,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsTextArea,
  SettingsTextField,
} from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { normalizeMemoryDocuments } from './memoryDebugUtils';

const MemoryDebugPanel = () => {
  const { t } = useT();
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
        t('memory.deleteConfirm', 'Delete document "{documentId}" in namespace "{namespace}"?')
          .replace('{documentId}', doc.documentId)
          .replace('{namespace}', doc.namespace)
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
    [refreshAll, t]
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
      t(
        'memory.clearNamespaceConfirm',
        'This will permanently delete ALL documents in namespace "{namespace}". Continue?'
      ).replace('{namespace}', ns)
    );
    if (!confirmed) return;

    setClearLoading(true);
    setClearError(null);
    setClearSuccess(null);
    try {
      const result = await memoryClearNamespace(ns);
      if (result.cleared) {
        setClearSuccess(
          t('memory.clearNamespaceSuccess', 'Namespace "{namespace}" cleared.').replace(
            '{namespace}',
            result.namespace
          )
        );
      } else {
        setClearSuccess(
          t('memory.clearNamespaceEmpty', 'Nothing to clear in "{namespace}".').replace(
            '{namespace}',
            result.namespace
          )
        );
      }
      await refreshAll();
    } catch (error) {
      setClearError(error instanceof Error ? error.message : String(error));
    } finally {
      setClearLoading(false);
    }
  }, [clearNamespaceInput, refreshAll, t]);

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      testId="memory-debug-panel"
      description={t('devOptions.debugPanelsDesc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="p-4 space-y-5">
        {/* Documents */}
        <SettingsSection title={t('memory.documents')}>
          <div className="px-4 py-3 space-y-3">
            <div className="flex gap-2">
              <SettingsTextField
                className="flex-1"
                value={documentsNamespaceFilter}
                onChange={e => setDocumentsNamespaceFilter(e.target.value)}
                placeholder={t('memory.filterByNamespace')}
                aria-label={t('memory.filterByNamespace')}
                inputSize="sm"
              />
              <Button
                type="button"
                variant="secondary"
                size="xs"
                onClick={() => void loadDocuments()}
                disabled={documentsLoading}>
                {documentsLoading ? '...' : t('memory.refresh')}
              </Button>
            </div>
            <SettingsStatusLine saving={false} error={documentsError} savingLabel="" />
            {documents.length === 0 && !documentsLoading ? (
              <SettingsEmptyState label={t('memory.noDocumentsFound')} />
            ) : (
              <div className="space-y-1">
                {documents.map(doc => (
                  <div
                    key={`${doc.namespace}:${doc.documentId}`}
                    className="flex items-start justify-between gap-2 rounded-lg border border-line bg-surface-muted p-2">
                    <div className="min-w-0">
                      <div className="text-xs font-medium text-content break-all">
                        {doc.documentId}
                      </div>
                      <div className="text-[11px] text-content-muted break-all">
                        {doc.namespace}
                      </div>
                      {doc.title && (
                        <div className="text-[11px] text-content-muted">{doc.title}</div>
                      )}
                    </div>
                    <Button
                      type="button"
                      variant="tertiary"
                      size="xs"
                      disabled={Boolean(deleteLoadingId)}
                      onClick={() => void handleDelete(doc)}>
                      {deleteLoadingId === doc.documentId ? '...' : t('memory.delete')}
                    </Button>
                  </div>
                ))}
              </div>
            )}
            <details className="text-xs">
              <summary className="cursor-pointer text-content-muted">
                {t('memory.rawResponse')}
              </summary>
              <pre className="mt-1 max-h-32 overflow-auto rounded-lg border border-line bg-neutral-950 dark:bg-neutral-50 p-2 text-[11px] text-neutral-100 whitespace-pre-wrap break-words">
                {JSON.stringify(documentsRaw, null, 2)}
              </pre>
            </details>
          </div>
        </SettingsSection>

        {/* Namespaces */}
        <SettingsSection title={t('memory.namespaces')}>
          <div className="px-4 py-3 space-y-2">
            <div className="flex items-center justify-between">
              <Button
                type="button"
                variant="secondary"
                size="xs"
                onClick={() => void loadNamespaces()}
                disabled={namespacesLoading}>
                {namespacesLoading ? '...' : t('memory.refresh')}
              </Button>
            </div>
            <SettingsStatusLine saving={false} error={namespacesError} savingLabel="" />
            {namespaces.length > 0 ? (
              <div className="flex flex-wrap gap-1">
                {namespaces.map(ns => (
                  <span
                    key={ns}
                    className="rounded-full bg-surface-subtle px-2 py-0.5 text-[11px] text-content-muted">
                    {ns}
                  </span>
                ))}
              </div>
            ) : (
              <SettingsEmptyState label={t('memory.noNamespacesFound')} />
            )}
          </div>
        </SettingsSection>

        {/* Query & Recall */}
        <SettingsSection title={t('memory.queryRecall')}>
          <div className="px-4 py-3 space-y-2">
            <SettingsTextField
              value={namespaceInput}
              onChange={e => setNamespaceInput(e.target.value)}
              placeholder={t('memory.namespace')}
              aria-label={t('memory.namespace')}
              inputSize="sm"
            />
            <SettingsTextArea
              value={queryInput}
              onChange={e => setQueryInput(e.target.value)}
              rows={2}
              placeholder={t('memory.queryText')}
              aria-label={t('memory.queryText')}
            />
            <div className="flex items-center gap-2">
              <SettingsTextField
                className="w-16"
                value={maxChunksInput}
                onChange={e => setMaxChunksInput(e.target.value)}
                placeholder={t('memory.defaultMaxChunks')}
                aria-label={t('memory.maxChunks')}
                inputSize="sm"
              />
              <span className="text-[11px] text-content-muted">{t('memory.maxChunks')}</span>
              <div className="flex-1" />
              <Button
                type="button"
                variant="secondary"
                size="xs"
                onClick={() => void handleQuery()}
                disabled={queryLoading || !namespaceInput.trim() || !queryInput.trim()}>
                {queryLoading ? '...' : t('memory.query')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="xs"
                onClick={() => void handleRecall()}
                disabled={recallLoading || !namespaceInput.trim()}>
                {recallLoading ? '...' : t('memory.recall')}
              </Button>
            </div>
            <SettingsStatusLine
              saving={false}
              error={
                queryError
                  ? `${t('memory.queryLabel')}: ${queryError}`
                  : recallError
                    ? `${t('memory.recallLabel')}: ${recallError}`
                    : null
              }
              savingLabel=""
            />
            {(queryResult || recallResult) && (
              <div className="space-y-2">
                {queryResult && (
                  <div>
                    <div className="text-[11px] font-medium text-content-muted mb-1">
                      {t('memory.queryResult')}
                    </div>
                    <MemoryTextWithEntities
                      text={queryResult.text ?? ''}
                      entities={queryResult.entities}
                      className="rounded-lg border border-line bg-surface-muted p-2 text-[11px] leading-5 min-h-12 whitespace-pre-wrap"
                    />
                  </div>
                )}
                {recallResult && (
                  <div>
                    <div className="text-[11px] font-medium text-content-muted mb-1">
                      {t('memory.recallResult')}
                    </div>
                    <MemoryTextWithEntities
                      text={recallResult.text ?? ''}
                      entities={recallResult.entities}
                      className="rounded-lg border border-line bg-surface-muted p-2 text-[11px] leading-5 min-h-12 whitespace-pre-wrap"
                    />
                  </div>
                )}
              </div>
            )}
          </div>
        </SettingsSection>

        {/* Clear Namespace */}
        <SettingsSection
          title={t('memory.clearNamespace')}
          description={t('memory.clearNamespaceDescription')}>
          <div className="px-4 py-3 space-y-2">
            <div className="flex gap-2">
              {namespaces.length > 0 ? (
                <SettingsSelect
                  className="flex-1"
                  value={clearNamespaceInput}
                  onChange={e => setClearNamespaceInput(e.target.value)}
                  aria-label={t('memory.selectNamespace')}
                  inputSize="sm">
                  <option value="">{t('memory.selectNamespace')}</option>
                  {namespaces.map(ns => (
                    <option key={ns} value={ns}>
                      {ns}
                    </option>
                  ))}
                </SettingsSelect>
              ) : (
                <SettingsTextField
                  className="flex-1"
                  value={clearNamespaceInput}
                  onChange={e => setClearNamespaceInput(e.target.value)}
                  placeholder={t('memory.exampleNamespace')}
                  aria-label={t('memory.exampleNamespace')}
                  inputSize="sm"
                />
              )}
              <Button
                type="button"
                variant="secondary"
                tone="danger"
                size="xs"
                onClick={() => void handleClearNamespace()}
                disabled={clearLoading || !clearNamespaceInput.trim()}>
                {clearLoading ? '...' : t('memory.clear')}
              </Button>
            </div>
            <SettingsStatusLine
              saving={false}
              savedNote={clearSuccess}
              error={clearError}
              savingLabel=""
            />
          </div>
        </SettingsSection>
      </div>
    </PanelPage>
  );
};

export default MemoryDebugPanel;
