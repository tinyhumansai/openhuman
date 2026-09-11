/**
 * Dialog for adding a new memory source.
 *
 * Step 1: pick a source kind (Composio / Folder / GitHub / RSS / Web / Twitter).
 * Step 2: fill in kind-specific fields and submit.
 *
 * For Composio, the dialog fetches the user's active connections and
 * presents them as a dropdown — the user picks an existing OAuth
 * connection rather than typing toolkit + connection_id.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { listConnections } from '../../lib/composio/composioApi';
import type { ComposioConnection } from '../../lib/composio/types';
import { useT } from '../../lib/i18n/I18nContext';
import {
  addMemorySource,
  getSupportedToolkits,
  type MemorySourceEntry,
  SOURCE_KIND_ICONS,
  SOURCE_KIND_LABEL_KEYS,
  type SourceKind,
} from '../../services/memorySourcesService';
import Button from '../ui/Button';
import { DialogContent, DialogRoot, DialogTitle } from '../ui/Dialog';
import { Field, isKindFieldsValid, KindFields } from './AddMemorySourceFields';

const log = debug('intelligence:add-memory-source-dialog');

/** Safe, PII-free string for an unknown error — message/name only, no stack. */
function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

interface AddMemorySourceDialogProps {
  open: boolean;
  onClose: () => void;
  onAdded: (source: MemorySourceEntry) => void;
}

const ALL_KINDS: SourceKind[] = [
  'composio',
  'conversation',
  'folder',
  'github_repo',
  'rss_feed',
  'web_page',
  'twitter_query',
];

export function AddMemorySourceDialog({ open, onClose, onAdded }: AddMemorySourceDialogProps) {
  const { t } = useT();
  const [kind, setKind] = useState<SourceKind | null>(null);
  const [label, setLabel] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Kind-specific fields
  const [path, setPath] = useState('');
  const [glob, setGlob] = useState('**/*.md');
  const [url, setUrl] = useState('');
  const [branch, setBranch] = useState('main');
  const [query, setQuery] = useState('');
  const [selector, setSelector] = useState('');
  const [connectionId, setConnectionId] = useState('');
  const [toolkit, setToolkit] = useState('');

  // Composio connection picker state
  const [connections, setConnections] = useState<ComposioConnection[]>([]);
  const [loadingConnections, setLoadingConnections] = useState(false);
  // Toolkit slugs that can actually sync (backend registry). `null` means the
  // set hasn't loaded (or the RPC failed) — in that case we treat every
  // connection as supported rather than locking the user out of all of them.
  const [supportedToolkits, setSupportedToolkits] = useState<string[] | null>(null);

  // Fetch composio connections + the supported-toolkit set when the user picks
  // the composio kind. setState calls live inside the spawned async closure
  // (not the synchronous effect body) to satisfy `react-hooks/set-state-in-effect`.
  useEffect(() => {
    if (kind !== 'composio') return undefined;
    let cancelled = false;
    void (async () => {
      if (cancelled) return;
      setLoadingConnections(true);
      try {
        const [resp, toolkits] = await Promise.all([
          listConnections(),
          getSupportedToolkits().catch((err: unknown) => {
            // Non-fatal: fall back to "everything supported" so the picker
            // still works if the supported-toolkit RPC is unavailable.
            log('[composio-picker] getSupportedToolkits failed: %s', errMessage(err));
            return null;
          }),
        ]);
        if (cancelled) return;
        setConnections(resp.connections);
        setSupportedToolkits(toolkits);
      } catch (err) {
        if (cancelled) return;
        log('[add-memory-source] listConnections failed: %s', errMessage(err));
        setError(t('memorySources.composioListFailed'));
      } finally {
        if (!cancelled) setLoadingConnections(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [kind, t]);

  const reset = useCallback(() => {
    setKind(null);
    setLabel('');
    setPath('');
    setGlob('**/*.md');
    setUrl('');
    setBranch('main');
    setQuery('');
    setSelector('');
    setConnectionId('');
    setToolkit('');
    setError(null);
  }, []);

  const handleClose = useCallback(() => {
    reset();
    onClose();
  }, [onClose, reset]);

  const handleSubmit = useCallback(async () => {
    if (!kind || !label.trim()) return;
    setSubmitting(true);
    setError(null);

    try {
      const params: Record<string, unknown> = { kind, label: label.trim(), enabled: true };

      switch (kind) {
        case 'composio':
          params.toolkit = toolkit;
          params.connection_id = connectionId;
          break;
        case 'conversation':
          break;
        case 'folder':
          params.path = path.trim();
          params.glob = glob.trim() || '**/*.md';
          break;
        case 'github_repo':
          params.url = url.trim();
          params.branch = branch.trim() || 'main';
          break;
        case 'rss_feed':
          params.url = url.trim();
          break;
        case 'web_page':
          params.url = url.trim();
          if (selector.trim()) params.selector = selector.trim();
          break;
        case 'twitter_query':
          params.query = query.trim();
          break;
      }

      const source = await addMemorySource(params as Omit<MemorySourceEntry, 'id'>);
      onAdded(source);
      handleClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }, [
    kind,
    label,
    path,
    glob,
    url,
    branch,
    query,
    selector,
    connectionId,
    toolkit,
    onAdded,
    handleClose,
  ]);

  const isValid =
    kind && label.trim() && isKindFieldsValid(kind, { path, url, query, connectionId });

  return (
    <DialogRoot
      open={open}
      onOpenChange={next => {
        if (!next) handleClose();
      }}>
      <DialogContent className="max-w-lg p-6">
        <DialogTitle className="text-lg">{t('memorySources.addSource')}</DialogTitle>

        {!kind ? (
          <>
            <p className="mt-2 text-sm text-content-muted">{t('memorySources.pickKind')}</p>
            <div className="mt-4 grid grid-cols-2 gap-3">
              {ALL_KINDS.map(k => (
                <Button
                  key={k}
                  variant="secondary"
                  onClick={() => setKind(k)}
                  className="h-auto items-center justify-start gap-3 p-3 text-left font-normal
                             hover:border-primary-400 hover:bg-primary-50
                             dark:hover:border-primary-500 dark:hover:bg-primary-500/10">
                  <span className="text-xl">{SOURCE_KIND_ICONS[k]}</span>
                  <span className="text-sm font-medium text-content">
                    {t(SOURCE_KIND_LABEL_KEYS[k])}
                  </span>
                </Button>
              ))}
            </div>
            <div className="mt-4 flex justify-end">
              <Button variant="tertiary" size="md" onClick={handleClose}>
                {t('common.cancel')}
              </Button>
            </div>
          </>
        ) : (
          <>
            <p className="mt-1 text-sm text-content-muted">
              {SOURCE_KIND_ICONS[kind]} {t(SOURCE_KIND_LABEL_KEYS[kind])}
            </p>

            <div className="mt-4 space-y-3">
              <Field
                label={t('memorySources.label')}
                value={label}
                onChange={setLabel}
                placeholder={t('memorySources.labelPlaceholder')}
              />
              <KindFields
                kind={kind}
                path={path}
                setPath={setPath}
                glob={glob}
                setGlob={setGlob}
                url={url}
                setUrl={setUrl}
                branch={branch}
                setBranch={setBranch}
                query={query}
                setQuery={setQuery}
                selector={selector}
                setSelector={setSelector}
                connections={connections}
                loadingConnections={loadingConnections}
                supportedToolkits={supportedToolkits}
                connectionId={connectionId}
                setConnection={(connId, tk, identityLabel) => {
                  setConnectionId(connId);
                  setToolkit(tk);
                  if (!label) setLabel(identityLabel);
                }}
              />
            </div>

            {error && (
              <p className="mt-3 rounded-md bg-coral-50 p-2 text-xs text-coral-800 dark:bg-coral-500/10 dark:text-coral-300">
                {error}
              </p>
            )}

            <div className="mt-5 flex items-center justify-between">
              <Button
                variant="tertiary"
                size="sm"
                onClick={() => {
                  setKind(null);
                  setError(null);
                }}>
                ← {t('memorySources.backToKinds')}
              </Button>
              <div className="flex gap-2">
                <Button variant="tertiary" size="md" onClick={handleClose}>
                  {t('common.cancel')}
                </Button>
                <Button
                  variant="primary"
                  size="md"
                  onClick={handleSubmit}
                  disabled={!isValid || submitting}>
                  {submitting ? t('memorySources.adding') : t('memorySources.add')}
                </Button>
              </div>
            </div>
          </>
        )}
      </DialogContent>
    </DialogRoot>
  );
}

export { deduplicateConnections } from './AddMemorySourceFields';
