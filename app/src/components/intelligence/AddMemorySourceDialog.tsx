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
import {
  type KeyboardEvent as ReactKeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

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

  if (!open) return null;

  const isValid =
    kind && label.trim() && isKindFieldsValid(kind, { path, url, query, connectionId });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm">
      <div className="w-full max-w-lg rounded-xl border border-stone-200 bg-white p-6 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
        <h2 className="text-lg font-semibold text-stone-900 dark:text-neutral-100">
          {t('memorySources.addSource')}
        </h2>

        {!kind ? (
          <>
            <p className="mt-2 text-sm text-stone-500 dark:text-neutral-400">
              {t('memorySources.pickKind')}
            </p>
            <div className="mt-4 grid grid-cols-2 gap-3">
              {ALL_KINDS.map(k => (
                <button
                  key={k}
                  type="button"
                  onClick={() => setKind(k)}
                  className="flex items-center gap-3 rounded-lg border border-stone-200 p-3
                             text-left transition-colors hover:border-primary-400 hover:bg-primary-50
                             dark:border-neutral-700 dark:hover:border-primary-500 dark:hover:bg-primary-500/10">
                  <span className="text-xl">{SOURCE_KIND_ICONS[k]}</span>
                  <span className="text-sm font-medium text-stone-800 dark:text-neutral-200">
                    {t(SOURCE_KIND_LABEL_KEYS[k])}
                  </span>
                </button>
              ))}
            </div>
            <div className="mt-4 flex justify-end">
              <button
                type="button"
                onClick={handleClose}
                className="rounded-md px-4 py-2 text-sm text-stone-600 hover:text-stone-900
                           dark:text-neutral-400 dark:hover:text-neutral-100">
                {t('common.cancel')}
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="mt-1 text-sm text-stone-500 dark:text-neutral-400">
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
              <button
                type="button"
                onClick={() => {
                  setKind(null);
                  setError(null);
                }}
                className="text-sm text-stone-500 hover:text-stone-800 dark:text-neutral-400 dark:hover:text-neutral-200">
                ← {t('memorySources.backToKinds')}
              </button>
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={handleClose}
                  className="rounded-md px-4 py-2 text-sm text-stone-600 hover:text-stone-900
                             dark:text-neutral-400 dark:hover:text-neutral-100">
                  {t('common.cancel')}
                </button>
                <button
                  type="button"
                  onClick={handleSubmit}
                  disabled={!isValid || submitting}
                  className="rounded-md bg-primary-500 px-4 py-2 text-sm font-semibold text-white
                             shadow-sm transition-colors hover:bg-primary-600
                             disabled:cursor-not-allowed disabled:opacity-50">
                  {submitting ? t('memorySources.adding') : t('memorySources.add')}
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function isKindFieldsValid(
  kind: SourceKind,
  fields: { path: string; url: string; query: string; connectionId: string }
): boolean {
  switch (kind) {
    case 'composio':
      return fields.connectionId.length > 0;
    case 'folder':
      return fields.path.trim().length > 0;
    case 'github_repo':
    case 'rss_feed':
    case 'web_page':
      return fields.url.trim().length > 0;
    case 'twitter_query':
      return fields.query.trim().length > 0;
    default:
      return true;
  }
}

interface FieldProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
}

interface FolderFieldProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
}

function FolderField({ label, value, onChange }: FolderFieldProps) {
  const { t } = useT();
  return (
    <label className="block">
      <span className="text-xs font-medium text-stone-600 dark:text-neutral-400">{label}</span>
      <div className="mt-1 flex gap-2">
        <input
          type="text"
          value={value}
          onChange={e => onChange(e.target.value)}
          placeholder={t('memorySources.folderPathPlaceholder')}
          className="block w-full rounded-md border border-stone-300 bg-white px-3 py-2
                     text-sm text-stone-900 placeholder-stone-400
                     focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-400
                     dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100
                     dark:placeholder-neutral-500 dark:focus:border-primary-500"
        />
        <label
          className="shrink-0 cursor-pointer rounded-md border border-stone-300 bg-white px-3 py-2
                     text-xs font-medium text-stone-700 transition-colors
                     hover:border-primary-400 hover:text-primary-600
                     dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-300
                     dark:hover:border-primary-500 dark:hover:text-primary-400">
          {t('memorySources.browse')}
          <input
            type="file"
            // @ts-expect-error — non-standard but supported in CEF/Chromium
            webkitdirectory=""
            multiple
            className="hidden"
            onChange={e => {
              const files = e.target.files;
              if (!files || files.length === 0) return;
              // Chromium exposes the chosen directory path on the first file's `path`
              // attribute when the renderer has filesystem-aware integration (CEF).
              // Fall back to webkitRelativePath split if `path` isn't available.
              const first = files[0] as File & { path?: string };
              if (first.path) {
                // first.path is the absolute path to the file. Derive the directory
                // by trimming the relative portion (everything after the chosen root).
                const rel = first.webkitRelativePath || first.name;
                const abs = first.path;
                const idx = abs.lastIndexOf(rel);
                onChange(idx > 0 ? abs.slice(0, idx).replace(/\/$/, '') : abs);
              } else if (first.webkitRelativePath) {
                onChange(first.webkitRelativePath.split('/')[0]);
              }
            }}
          />
        </label>
      </div>
    </label>
  );
}

function Field({ label, value, onChange, placeholder, type = 'text' }: FieldProps) {
  return (
    <label className="block">
      <span className="text-xs font-medium text-stone-600 dark:text-neutral-400">{label}</span>
      <input
        type={type}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={placeholder}
        className="mt-1 block w-full rounded-md border border-stone-300 bg-white px-3 py-2
                   text-sm text-stone-900 placeholder-stone-400
                   focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-400
                   dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100
                   dark:placeholder-neutral-500 dark:focus:border-primary-500"
      />
    </label>
  );
}

interface KindFieldsProps {
  kind: SourceKind;
  path: string;
  setPath: (v: string) => void;
  glob: string;
  setGlob: (v: string) => void;
  url: string;
  setUrl: (v: string) => void;
  branch: string;
  setBranch: (v: string) => void;
  query: string;
  setQuery: (v: string) => void;
  selector: string;
  setSelector: (v: string) => void;
  connections: ComposioConnection[];
  loadingConnections: boolean;
  /** Syncable toolkit slugs; `null` while unknown (treat all as supported). */
  supportedToolkits: string[] | null;
  connectionId: string;
  setConnection: (connectionId: string, toolkit: string, identityLabel: string) => void;
}

function KindFields(props: KindFieldsProps) {
  const { t } = useT();
  switch (props.kind) {
    case 'composio':
      return <ComposioPicker {...props} />;
    case 'folder':
      return (
        <>
          <FolderField
            label={t('memorySources.folderPath')}
            value={props.path}
            onChange={props.setPath}
          />
          <Field
            label={t('memorySources.globPattern')}
            value={props.glob}
            onChange={props.setGlob}
            placeholder={t('memorySources.globPatternPlaceholder')}
          />
        </>
      );
    case 'github_repo':
      return (
        <>
          <Field
            label={t('memorySources.repoUrl')}
            value={props.url}
            onChange={props.setUrl}
            placeholder={t('memorySources.repoUrlPlaceholder')}
          />
          <Field
            label={t('memorySources.branch')}
            value={props.branch}
            onChange={props.setBranch}
            placeholder={t('memorySources.branchPlaceholder')}
          />
        </>
      );
    case 'rss_feed':
      return (
        <Field
          label={t('memorySources.feedUrl')}
          value={props.url}
          onChange={props.setUrl}
          placeholder={t('memorySources.feedUrlPlaceholder')}
        />
      );
    case 'web_page':
      return (
        <>
          <Field
            label={t('memorySources.pageUrl')}
            value={props.url}
            onChange={props.setUrl}
            placeholder={t('memorySources.pageUrlPlaceholder')}
          />
          <Field
            label={t('memorySources.cssSelector')}
            value={props.selector}
            onChange={props.setSelector}
            placeholder={t('memorySources.cssSelectorPlaceholder')}
          />
        </>
      );
    case 'twitter_query':
      return (
        <Field
          label={t('memorySources.searchQuery')}
          value={props.query}
          onChange={props.setQuery}
          placeholder={t('memorySources.searchQueryPlaceholder')}
        />
      );
    default:
      return null;
  }
}

/** Active-first status rank — lower is better. */
const STATUS_RANK: Record<string, number> = {
  ACTIVE: 0,
  CONNECTED: 0,
  PENDING: 1,
  INITIATED: 1,
  INITIALIZING: 1,
  EXPIRED: 2,
  FAILED: 3,
  ERROR: 3,
};

function statusRank(conn: ComposioConnection): number {
  return STATUS_RANK[conn.status.toUpperCase()] ?? 2;
}

/**
 * Deduplicates and labels connections for display in the picker.
 *
 * - Sorts by status rank first (ACTIVE/CONNECTED before EXPIRED/FAILED) so
 *   that when two connections share the same toolkit + identity, the healthier
 *   one wins rather than the first-returned one.
 * - Connections sharing the same toolkit + identity (accountEmail / workspace /
 *   username) OR the same raw connection id are collapsed to the first
 *   occurrence, preventing both labeled and identity-less duplicates.
 * - Connections with no identity field fall back to showing the raw connection ID
 *   so users can unambiguously distinguish accounts.
 */
export function deduplicateConnections(
  connections: ComposioConnection[]
): Array<{ conn: ComposioConnection; label: string }> {
  const sorted = [...connections].sort((a, b) => statusRank(a) - statusRank(b));
  const seen = new Set<string>();
  const result: Array<{ conn: ComposioConnection; label: string }> = [];

  for (const conn of sorted) {
    // Always dedup by raw connection id to guard against identity-less dupes.
    if (seen.has(conn.id)) {
      log('[composio-picker] dropping duplicate connection toolkit=%s', conn.toolkit);
      continue;
    }
    seen.add(conn.id);

    const identity = conn.accountEmail ?? conn.workspace ?? conn.username;
    if (identity) {
      const key = `${conn.toolkit}:${identity}`;
      if (seen.has(key)) {
        log('[composio-picker] dropping duplicate connection toolkit=%s', conn.toolkit);
        continue;
      }
      seen.add(key);
      result.push({ conn, label: `${conn.toolkit} · ${identity}` });
    } else {
      // Fall back to the raw connection ID so the user can unambiguously
      // distinguish accounts when no identity data is available.
      result.push({ conn, label: `${conn.toolkit} · ${conn.id}` });
    }
  }
  return result;
}

/** A connection is syncable when its toolkit ships a provider. A `null`
 *  supported-set means "unknown" — treat everything as supported so a failed
 *  lookup never disables the whole picker. */
function isToolkitSupported(toolkit: string, supportedToolkits: string[] | null): boolean {
  if (supportedToolkits === null) return true;
  return supportedToolkits.includes(toolkit.trim().toLowerCase());
}

interface PickerEntry {
  conn: ComposioConnection;
  label: string;
  supported: boolean;
}

function ComposioPicker({
  connections,
  loadingConnections,
  supportedToolkits,
  connectionId,
  setConnection,
}: KindFieldsProps) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  // Index (into `entries`) of the keyboard-highlighted option; -1 when none.
  const [activeIndex, setActiveIndex] = useState(-1);
  const containerRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listboxRef = useRef<HTMLUListElement>(null);

  // useMemo must be declared before any early returns (Rules of Hooks).
  const entries = useMemo<PickerEntry[]>(() => {
    const deduped = deduplicateConnections(connections).map(({ conn, label }) => ({
      conn,
      label,
      supported: isToolkitSupported(conn.toolkit, supportedToolkits),
    }));
    // Supported connections first so the actionable ones surface at the top;
    // stable within each partition (dedup already ranked by health/status).
    return [...deduped.filter(e => e.supported), ...deduped.filter(e => !e.supported)];
  }, [connections, supportedToolkits]);

  // Indexes of keyboard-selectable (supported) options — unsupported rows are
  // skipped during arrow navigation, mirroring a native <select>'s disabled opts.
  const selectableIndexes = useMemo(
    () => entries.map((e, i) => (e.supported ? i : -1)).filter(i => i >= 0),
    [entries]
  );

  const selected = entries.find(e => e.conn.id === connectionId) ?? null;

  // Close the popover on outside click or Escape.
  useEffect(() => {
    if (!open) return undefined;
    const onPointerDown = (event: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  // Move keyboard focus into the listbox when it opens so arrow keys work
  // immediately. This is a DOM side-effect only — the highlighted index is set
  // in the open/close handlers, not here, to avoid setState-in-effect churn.
  useEffect(() => {
    if (open) listboxRef.current?.focus();
  }, [open]);

  if (loadingConnections) {
    return (
      <p className="text-xs text-stone-500 dark:text-neutral-400">
        {t('memorySources.loadingConnections')}
      </p>
    );
  }

  if (connections.length === 0) {
    return (
      <p className="rounded-md bg-amber-50 p-3 text-xs text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
        {t('memorySources.noConnections')}
      </p>
    );
  }

  // Highlight the current selection (or first selectable option) and open.
  const openListbox = () => {
    const selIdx = entries.findIndex(e => e.conn.id === connectionId && e.supported);
    setActiveIndex(selIdx >= 0 ? selIdx : (selectableIndexes[0] ?? -1));
    setOpen(true);
  };

  const close = (returnFocus = true) => {
    setActiveIndex(-1);
    setOpen(false);
    if (returnFocus) buttonRef.current?.focus();
  };

  const select = (entry: PickerEntry) => {
    if (!entry.supported) {
      log('[composio-picker] ignoring selection of unsupported toolkit=%s', entry.conn.toolkit);
      return;
    }
    setConnection(entry.conn.id, entry.conn.toolkit, entry.label);
    close();
  };

  // Move the highlight to the next/previous selectable option, wrapping around.
  const moveActive = (dir: 1 | -1) => {
    if (selectableIndexes.length === 0) return;
    const pos = selectableIndexes.indexOf(activeIndex);
    const nextPos =
      pos === -1
        ? dir === 1
          ? 0
          : selectableIndexes.length - 1
        : (pos + dir + selectableIndexes.length) % selectableIndexes.length;
    setActiveIndex(selectableIndexes[nextPos]);
  };

  const onButtonKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    // Open with the arrow keys; Enter/Space already toggle via onClick.
    if (!open && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      event.preventDefault();
      openListbox();
    }
  };

  const onListKeyDown = (event: ReactKeyboardEvent<HTMLUListElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        moveActive(1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        moveActive(-1);
        break;
      case 'Home':
        event.preventDefault();
        if (selectableIndexes.length) setActiveIndex(selectableIndexes[0]);
        break;
      case 'End':
        event.preventDefault();
        if (selectableIndexes.length)
          setActiveIndex(selectableIndexes[selectableIndexes.length - 1]);
        break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        if (activeIndex >= 0 && entries[activeIndex]) select(entries[activeIndex]);
        break;
      case 'Escape':
        event.preventDefault();
        close();
        break;
      case 'Tab':
        // Let focus leave naturally, but collapse the popover.
        setOpen(false);
        break;
      default:
        break;
    }
  };

  const LISTBOX_ID = 'composio-connection-listbox';
  const optionId = (entry: PickerEntry) => `composio-opt-${entry.conn.id}`;
  const activeOptionId =
    activeIndex >= 0 && entries[activeIndex] ? optionId(entries[activeIndex]) : undefined;

  return (
    <div className="block" ref={containerRef}>
      <span className="text-xs font-medium text-stone-600 dark:text-neutral-400">
        {t('memorySources.pickConnection')}
      </span>
      <div className="relative mt-1">
        <button
          ref={buttonRef}
          type="button"
          data-testid="composio-connection-picker"
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={open ? LISTBOX_ID : undefined}
          onClick={() => (open ? close(false) : openListbox())}
          onKeyDown={onButtonKeyDown}
          className="flex w-full items-center justify-between rounded-md border border-stone-300
                     bg-white px-3 py-2 text-left text-sm text-stone-900
                     focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-400
                     dark:border-neutral-600 dark:bg-neutral-800 dark:text-neutral-100
                     dark:focus:border-primary-500">
          <span className={selected ? '' : 'text-stone-400 dark:text-neutral-500'}>
            {selected ? selected.label : t('memorySources.selectConnection')}
          </span>
          <span aria-hidden className="ml-2 text-stone-400 dark:text-neutral-500">
            ▾
          </span>
        </button>

        {open && (
          <ul
            ref={listboxRef}
            id={LISTBOX_ID}
            role="listbox"
            tabIndex={-1}
            aria-label={t('memorySources.pickConnection')}
            aria-activedescendant={activeOptionId}
            onKeyDown={onListKeyDown}
            data-testid="composio-connection-listbox"
            className="absolute z-10 mt-1 max-h-60 w-full overflow-auto rounded-md border
                       border-stone-200 bg-white py-1 shadow-lg focus:outline-none
                       dark:border-neutral-700 dark:bg-neutral-800">
            {entries.map((entry, index) => {
              const isSelected = entry.conn.id === connectionId;
              const isActive = index === activeIndex;
              return (
                <li
                  key={entry.conn.id}
                  id={optionId(entry)}
                  role="option"
                  aria-selected={isSelected}
                  aria-disabled={!entry.supported}
                  data-testid={`composio-option-${entry.conn.id}`}
                  data-supported={entry.supported}
                  data-active={isActive}
                  onClick={() => select(entry)}
                  onMouseEnter={() => entry.supported && setActiveIndex(index)}
                  className={[
                    'flex items-center justify-between gap-2 px-3 py-2 text-sm',
                    entry.supported
                      ? 'cursor-pointer text-stone-800 dark:text-neutral-200'
                      : 'cursor-not-allowed text-stone-400 dark:text-neutral-500',
                    isActive && entry.supported ? 'bg-primary-50 dark:bg-primary-500/10' : '',
                  ].join(' ')}>
                  <span className="flex items-center gap-2 truncate">
                    {isSelected && entry.supported && (
                      <span aria-hidden className="text-primary-500">
                        ✓
                      </span>
                    )}
                    <span className="truncate">{entry.label}</span>
                  </span>
                  {!entry.supported && (
                    <span
                      data-testid={`composio-option-coming-soon-${entry.conn.id}`}
                      className="shrink-0 rounded-full bg-stone-100 px-2 py-0.5 text-[10px]
                                 font-medium uppercase tracking-wide text-stone-500
                                 dark:bg-neutral-700 dark:text-neutral-400">
                      {t('memorySources.comingSoon')}
                    </span>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
