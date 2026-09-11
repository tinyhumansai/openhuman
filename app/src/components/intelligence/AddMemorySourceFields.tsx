/**
 * Step-2 field renderers + the Composio connection picker for
 * `AddMemorySourceDialog`.
 *
 * Split out of the dialog file to keep it under the ~500-line budget —
 * everything here is pure presentation/validation for "fill in kind-specific
 * fields", independent of the dialog's open/close/submit orchestration.
 */
import debug from 'debug';
import {
  type KeyboardEvent as ReactKeyboardEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import type { ComposioConnection } from '../../lib/composio/types';
import { useT } from '../../lib/i18n/I18nContext';
import type { SourceKind } from '../../services/memorySourcesService';
import TextField from '../ui/TextField';
import { FolderField } from './FolderField';

const log = debug('intelligence:add-memory-source-dialog');

export function isKindFieldsValid(
  kind: SourceKind,
  fields: { path: string; url: string; query: string; connectionId: string }
): boolean {
  switch (kind) {
    case 'composio':
      return fields.connectionId.length > 0;
    case 'conversation':
      return true;
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

export function Field({ label, value, onChange, placeholder, type = 'text' }: FieldProps) {
  return (
    <label className="block">
      <span className="text-xs font-medium text-content-secondary">{label}</span>
      <TextField
        type={type}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={placeholder}
        className="mt-1"
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

export function KindFields(props: KindFieldsProps) {
  const { t } = useT();
  switch (props.kind) {
    case 'composio':
      return <ComposioPicker {...props} />;
    case 'conversation':
      return null;
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
    return <p className="text-xs text-content-muted">{t('memorySources.loadingConnections')}</p>;
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
      <span className="text-xs font-medium text-content-secondary">
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
          className="flex w-full items-center justify-between rounded-md border border-line-strong
                     bg-surface px-3 py-2 text-left text-sm text-content
                     focus:border-primary-400 focus:outline-hidden focus:ring-1 focus:ring-primary-400
                     dark:bg-surface-muted dark:text-content
                     dark:focus:border-primary-500">
          <span className={selected ? '' : 'text-content-faint'}>
            {selected ? selected.label : t('memorySources.selectConnection')}
          </span>
          <span aria-hidden className="ml-2 text-content-faint">
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
                       border-line bg-surface py-1 shadow-lg focus:outline-hidden
                       dark:border-line-strong dark:bg-surface-muted">
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
                      ? 'cursor-pointer text-content'
                      : 'cursor-not-allowed text-content-faint',
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
                      className="shrink-0 rounded-full bg-surface-subtle px-2 py-0.5 text-[10px]
                                 font-medium uppercase tracking-wide text-content-muted
                                 dark:bg-surface-strong dark:text-content-muted">
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
