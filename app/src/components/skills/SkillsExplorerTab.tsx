import debug from 'debug';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type CatalogEntry, skillRegistryApi } from '../../services/api/skillRegistryApi';
import {
  type InstallWorkflowFromUrlResult,
  workflowsApi,
  type WorkflowSummary,
} from '../../services/api/workflowsApi';
import EmptyStateCard from '../EmptyStateCard';
import ChipTabs from '../layout/ChipTabs';
import Button from '../ui/Button';
import InstallSkillDialog from './InstallSkillDialog';
import UninstallSkillConfirmDialog from './UninstallSkillConfirmDialog';

const log = debug('skills:explorer-tab');
const CATALOG_PAGE_SIZE = 60;
const SEARCH_DEBOUNCE_MS = 300;

function slugifyInstallKey(value: string | null | undefined): string | null {
  const raw = value?.trim();
  if (!raw) return null;

  let out = '';
  let lastDash = false;
  for (const ch of raw) {
    if (/[a-z0-9]/i.test(ch)) {
      out += ch.toLowerCase();
      lastDash = false;
    } else if (!lastDash && out.length > 0) {
      out += '-';
      lastDash = true;
    }
  }
  return out.replace(/-+$/, '') || null;
}

function lastPathSegment(value: string | null | undefined): string | null {
  const raw = value?.trim();
  if (!raw) return null;
  const parts = raw.split(/[/:#?]+/).filter(Boolean);
  return parts.at(-1) ?? null;
}

function parentPathSegment(value: string | null | undefined): string | null {
  const raw = value?.trim();
  if (!raw) return null;
  const parts = raw.split(/[\\/]+/).filter(Boolean);
  return parts.length >= 2 ? (parts.at(-2) ?? null) : null;
}

function catalogInstallKeys(entry: CatalogEntry): string[] {
  return [
    slugifyInstallKey(entry.id),
    slugifyInstallKey(lastPathSegment(entry.id)),
    slugifyInstallKey(parentPathSegment(entry.docs_path)),
    slugifyInstallKey(parentPathSegment(entry.download_url)),
  ].filter((key): key is string => Boolean(key));
}

function workflowInstallKeys(skill: WorkflowSummary): string[] {
  return [slugifyInstallKey(skill.id), slugifyInstallKey(parentPathSegment(skill.location))].filter(
    (key): key is string => Boolean(key)
  );
}

function isCatalogEntryInstalled(entry: CatalogEntry, installedKeys: Set<string>): boolean {
  return catalogInstallKeys(entry).some(key => installedKeys.has(key));
}

function SourceBadge({ source }: { source: string }) {
  const SOURCE_COLORS: Record<string, string> = {
    'built-in':
      'bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-300 dark:border-emerald-500/30',
    optional:
      'bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-500/10 dark:text-blue-300 dark:border-blue-500/30',
    ClawHub:
      'bg-teal-50 text-teal-700 border-teal-200 dark:bg-teal-500/10 dark:text-teal-300 dark:border-teal-500/30',
    'skills.sh':
      'bg-violet-50 text-violet-700 border-violet-200 dark:bg-violet-500/10 dark:text-violet-300 dark:border-violet-500/30',
    LobeHub:
      'bg-pink-50 text-pink-700 border-pink-200 dark:bg-pink-500/10 dark:text-pink-300 dark:border-pink-500/30',
    'browse.sh':
      'bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-500/10 dark:text-amber-300 dark:border-amber-500/30',
  };
  const colors =
    SOURCE_COLORS[source] ??
    'bg-surface-muted text-content-secondary border-line';
  return (
    <span
      className={`inline-flex items-center rounded-full border px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider ${colors}`}>
      {source}
    </span>
  );
}

function SkillFormatBadge({ format }: { format: string }) {
  const lower = format.toLowerCase();
  const FORMAT_MAP: Record<string, { label: string; colors: string }> = {
    hermes: {
      label: 'Hermes',
      colors:
        'bg-violet-50 text-violet-700 border-violet-200 dark:bg-violet-500/10 dark:text-violet-300 dark:border-violet-500/30',
    },
    agentskills: {
      label: 'AgentSkills',
      colors:
        'bg-violet-50 text-violet-700 border-violet-200 dark:bg-violet-500/10 dark:text-violet-300 dark:border-violet-500/30',
    },
    openclaw: {
      label: 'OpenClaw',
      colors:
        'bg-teal-50 text-teal-700 border-teal-200 dark:bg-teal-500/10 dark:text-teal-300 dark:border-teal-500/30',
    },
    clawhub: {
      label: 'ClawHub',
      colors:
        'bg-teal-50 text-teal-700 border-teal-200 dark:bg-teal-500/10 dark:text-teal-300 dark:border-teal-500/30',
    },
    legacy: {
      label: 'Legacy',
      colors:
        'bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-500/10 dark:text-amber-300 dark:border-amber-500/30',
    },
  };
  const entry = FORMAT_MAP[lower] ?? {
    label: format || 'Skill',
    colors:
      'bg-surface-muted text-content-secondary border-line',
  };
  return (
    <span
      className={`inline-flex items-center rounded-full border px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider ${entry.colors}`}>
      {entry.label}
    </span>
  );
}

function SkillScopeBadge({ scope }: { scope: string }) {
  const { t } = useT();
  const label =
    scope === 'user'
      ? t('skills.explorer.scopeUser')
      : scope === 'project'
        ? t('skills.explorer.scopeProject')
        : t('skills.explorer.scopeLegacy');
  return (
    <span className="inline-flex items-center rounded-full border border-line bg-surface-muted px-1.5 py-0.5 text-[9px] font-medium text-content-muted">
      {label}
    </span>
  );
}

interface SkillTileProps {
  skill: WorkflowSummary;
  onUninstall: () => void;
  onClick: () => void;
}

function SkillTile({ skill, onUninstall, onClick }: SkillTileProps) {
  const { t } = useT();
  const canUninstall = skill.scope === 'user';

  return (
    <div
      data-testid={`skill-explorer-tile-${skill.id}`}
      role="button"
      tabIndex={0}
      onClick={onClick}
      onKeyDown={e => {
        if (e.key === 'Enter') onClick();
        if (e.key === ' ' || e.key === 'Space') {
          e.preventDefault();
          onClick();
        }
      }}
      className="group flex flex-col justify-between rounded-2xl border border-line bg-surface p-3 transition-colors cursor-pointer hover:bg-surface-hover">
      <div className="min-w-0">
        <div className="flex items-start justify-between gap-2">
          <div className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl bg-surface-subtle">
            <svg
              className="h-5 w-5 text-content-muted"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M9.813 15.904 9 18.75l-.813-2.846a4.5 4.5 0 0 0-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 0 0 3.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 0 0 3.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 0 0-3.09 3.09ZM18.259 8.715 18 9.75l-.259-1.035a3.375 3.375 0 0 0-2.455-2.456L14.25 6l1.036-.259a3.375 3.375 0 0 0 2.455-2.456L18 2.25l.259 1.035a3.375 3.375 0 0 0 2.455 2.456L21.75 6l-1.036.259a3.375 3.375 0 0 0-2.455 2.456ZM16.894 20.567 16.5 21.75l-.394-1.183a2.25 2.25 0 0 0-1.423-1.423L13.5 18.75l1.183-.394a2.25 2.25 0 0 0 1.423-1.423l.394-1.183.394 1.183a2.25 2.25 0 0 0 1.423 1.423l1.183.394-1.183.394a2.25 2.25 0 0 0-1.423 1.423Z"
              />
            </svg>
          </div>
          <div className="flex items-center gap-1">
            <SkillFormatBadge format={skill.sourceFormat} />
            <SkillScopeBadge scope={skill.scope} />
          </div>
        </div>

        <h3 className="mt-2 line-clamp-1 text-sm font-semibold text-content">
          {skill.name}
        </h3>
        <p className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed text-content-muted">
          {skill.description || t('skills.explorer.noDescription')}
        </p>

        {skill.tags.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1">
            {skill.tags.slice(0, 3).map(tag => (
              <span
                key={tag}
                className="rounded-full bg-surface-subtle px-1.5 py-0.5 text-[9px] font-medium text-content-muted">
                {tag}
              </span>
            ))}
            {skill.tags.length > 3 && (
              <span className="rounded-full bg-surface-subtle px-1.5 py-0.5 text-[9px] font-medium text-content-faint">
                +{skill.tags.length - 3}
              </span>
            )}
          </div>
        )}
      </div>

      <div className="mt-3 flex items-center justify-between gap-2">
        {skill.version && (
          <span className="text-[10px] font-mono text-content-faint">
            v{skill.version}
          </span>
        )}
        {!skill.version && <span />}
        {canUninstall && (
          <Button
            variant="secondary"
            tone="danger"
            size="xs"
            data-testid={`skill-uninstall-${skill.id}`}
            onClick={e => {
              e.stopPropagation();
              onUninstall();
            }}
            className="opacity-0 group-hover:opacity-100">
            {t('skills.disconnect')}
          </Button>
        )}
      </div>

      {skill.warnings.length > 0 && (
        <div className="mt-2 rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-2 py-1.5">
          <p className="text-[10px] font-medium text-amber-700 dark:text-amber-300">
            {skill.warnings[0]}
          </p>
        </div>
      )}
    </div>
  );
}

interface CatalogTileProps {
  entry: CatalogEntry;
  installed: boolean;
  installing: boolean;
  onInstall: () => void;
  onClick: () => void;
}

function CatalogTile({ entry, installed, installing, onInstall, onClick }: CatalogTileProps) {
  const { t } = useT();
  return (
    <div
      data-testid={`registry-tile-${entry.id}`}
      role="button"
      tabIndex={0}
      onClick={onClick}
      onKeyDown={e => {
        if (e.key === 'Enter') onClick();
        if (e.key === ' ' || e.key === 'Space') {
          e.preventDefault();
          onClick();
        }
      }}
      className={`group flex flex-col justify-between rounded-2xl border p-3 transition-colors cursor-pointer ${
        installed
          ? 'border-sage-300 bg-sage-50/60 dark:border-sage-500/30 dark:bg-sage-500/10'
          : 'border-line bg-surface hover:bg-surface-hover'
      }`}>
      <div className="min-w-0">
        <div className="flex items-start justify-between gap-2">
          <div className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-xl bg-surface-subtle">
            <svg
              className="h-5 w-5 text-primary-500"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M12 21v-8.25M15.75 21v-8.25M8.25 21v-8.25M3 9l9-6 9 6m-1.5 12V10.332A48.36 48.36 0 0 0 12 9.75c-2.551 0-5.056.2-7.5.582V21M3 21h18M12 6.75h.008v.008H12V6.75Z"
              />
            </svg>
          </div>
          <div className="flex items-center gap-1">
            <SourceBadge source={entry.source} />
          </div>
        </div>

        <h3 className="mt-2 line-clamp-1 text-sm font-semibold text-content">
          {entry.name}
        </h3>
        <p className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed text-content-muted">
          {entry.description}
        </p>

        {entry.tags.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1">
            {entry.tags.slice(0, 3).map(tag => (
              <span
                key={tag}
                className="rounded-full bg-surface-subtle px-1.5 py-0.5 text-[9px] font-medium text-content-muted">
                {tag}
              </span>
            ))}
          </div>
        )}
      </div>

      <div className="mt-3 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          {entry.version && (
            <span className="text-[10px] font-mono text-content-faint">
              v{entry.version}
            </span>
          )}
          {entry.author && (
            <span className="text-[10px] text-content-faint">{entry.author}</span>
          )}
        </div>
        {installed ? (
          <span className="rounded-lg border border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 px-2 py-1 text-[10px] font-medium text-sage-700 dark:text-sage-300">
            {t('skills.explorer.installed')}
          </span>
        ) : (
          <Button
            variant="secondary"
            size="xs"
            data-testid={`registry-install-${entry.id}`}
            disabled={installing}
            onClick={e => {
              e.stopPropagation();
              onInstall();
            }}>
            {installing ? t('skills.explorer.installing') : t('skills.explorer.install')}
          </Button>
        )}
      </div>
    </div>
  );
}

interface SkillDetailDialogProps {
  entry: CatalogEntry | null;
  skill: WorkflowSummary | null;
  installed: boolean;
  onClose: () => void;
  onInstall?: () => void;
  installing?: boolean;
}

function SkillDetailDialog({
  entry,
  skill,
  installed,
  onClose,
  onInstall,
  installing,
}: SkillDetailDialogProps) {
  const { t } = useT();
  const name = entry?.name ?? skill?.name ?? '';
  const description = entry?.description ?? skill?.description ?? '';
  const tags = entry?.tags ?? skill?.tags ?? [];
  const version = entry?.version ?? skill?.version ?? '';
  const author = entry?.author ?? '';
  const source = entry?.source ?? '';
  const category = entry?.category ?? '';
  const downloadUrl = entry?.download_url ?? '';
  const license = entry?.license ?? '';

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm"
      onClick={onClose}>
      <div
        className="mx-4 w-full max-w-lg rounded-2xl border border-line bg-surface shadow-xl"
        onClick={e => e.stopPropagation()}>
        <div className="flex items-start justify-between gap-3 border-b border-line-subtle p-5">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2 className="text-base font-semibold text-content truncate">
                {name}
              </h2>
              {installed && (
                <span className="flex-shrink-0 rounded-full border border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 px-2 py-0.5 text-[10px] font-medium text-sage-700 dark:text-sage-300">
                  {t('skills.explorer.installed')}
                </span>
              )}
            </div>
            <div className="mt-1.5 flex items-center gap-1.5">
              {source && <SourceBadge source={source} />}
              {category && (
                <span className="inline-flex items-center rounded-full border border-line bg-surface-muted px-1.5 py-0.5 text-[9px] font-medium text-content-muted">
                  {category}
                </span>
              )}
            </div>
          </div>
          <Button
            iconOnly
            variant="tertiary"
            size="sm"
            aria-label={t('common.close')}
            onClick={onClose}
            className="flex-shrink-0 text-content-faint">
            <svg
              className="h-5 w-5"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18 18 6M6 6l12 12" />
            </svg>
          </Button>
        </div>

        <div className="p-5 space-y-4">
          {description && (
            <div>
              <h3 className="text-[11px] font-semibold uppercase tracking-wider text-content-faint mb-1">
                {t('skills.detail.description')}
              </h3>
              <p className="text-sm text-content-secondary leading-relaxed whitespace-pre-wrap">
                {description}
              </p>
            </div>
          )}

          <div className="flex flex-wrap gap-x-6 gap-y-2">
            {version && (
              <div>
                <span className="text-[10px] font-semibold uppercase tracking-wider text-content-faint">
                  {t('skills.detail.version')}
                </span>
                <p className="text-xs font-mono text-content-secondary">{version}</p>
              </div>
            )}
            {author && (
              <div>
                <span className="text-[10px] font-semibold uppercase tracking-wider text-content-faint">
                  {t('skills.detail.author')}
                </span>
                <p className="text-xs text-content-secondary">{author}</p>
              </div>
            )}
            {license && (
              <div>
                <span className="text-[10px] font-semibold uppercase tracking-wider text-content-faint">
                  {t('skills.detail.license')}
                </span>
                <p className="text-xs text-content-secondary">{license}</p>
              </div>
            )}
          </div>

          {tags.length > 0 && (
            <div>
              <h3 className="text-[11px] font-semibold uppercase tracking-wider text-content-faint mb-1.5">
                {t('skills.detail.tags')}
              </h3>
              <div className="flex flex-wrap gap-1.5">
                {tags.map(tag => (
                  <span
                    key={tag}
                    className="rounded-full bg-surface-subtle px-2 py-0.5 text-[10px] font-medium text-content-secondary">
                    {tag}
                  </span>
                ))}
              </div>
            </div>
          )}

          {downloadUrl && (
            <div>
              <h3 className="text-[11px] font-semibold uppercase tracking-wider text-content-faint mb-1">
                {t('skills.detail.source')}
              </h3>
              <p className="text-[11px] font-mono text-content-faint break-all">
                {downloadUrl}
              </p>
            </div>
          )}
        </div>

        {!installed && onInstall && (
          <div className="border-t border-line-subtle p-4 flex justify-end">
            <Button variant="secondary" size="sm" disabled={installing} onClick={onInstall}>
              {installing ? t('skills.explorer.installing') : t('skills.explorer.install')}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

type ExplorerView = 'installed' | 'registry';

interface SkillsExplorerTabProps {
  onToast?: (toast: { type: 'success' | 'error'; title: string; message?: string }) => void;
}

export default function SkillsExplorerTab({ onToast }: SkillsExplorerTabProps) {
  const { t } = useT();
  const [view, setView] = useState<ExplorerView>('registry');

  const [skills, setSkills] = useState<WorkflowSummary[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(true);
  const [skillsError, setSkillsError] = useState<string | null>(null);

  const [catalogEntries, setCatalogEntries] = useState<CatalogEntry[]>([]);
  const [catalogTotal, setCatalogTotal] = useState(0);
  // How many catalog entries are currently revealed. We fetch the whole list
  // up front, then page through it client-side via the "Show more" control.
  const [visibleCount, setVisibleCount] = useState(CATALOG_PAGE_SIZE);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogInitialized, setCatalogInitialized] = useState(false);
  const [installingId, setInstallingId] = useState<string | null>(null);
  // Catalog entry ids we just installed this session. The "installed" badge is
  // otherwise derived purely from `isCatalogEntryInstalled`, a heuristic that
  // maps a refetched installed skill (whose post-install id/location can differ
  // from the catalog entry) back to the catalog card. When that mapping misses,
  // a successful install fell back to "Install" — the only signal was a fleeting
  // toast, so the card looked unchanged (#4150). Recording the installed entry
  // id here makes the card flip to "Installed" deterministically on success.
  const [installedEntryIds, setInstalledEntryIds] = useState<Set<string>>(new Set());

  const [sources, setSources] = useState<string[]>([]);
  const [activeSources, setActiveSources] = useState<Set<string>>(new Set());
  const [searchQuery, setSearchQuery] = useState('');
  const [debouncedQuery, setDebouncedQuery] = useState('');
  const [installDialogOpen, setInstallDialogOpen] = useState(false);
  const [uninstallTarget, setUninstallTarget] = useState<WorkflowSummary | null>(null);
  const [detailEntry, setDetailEntry] = useState<CatalogEntry | null>(null);
  const [detailSkill, setDetailSkill] = useState<WorkflowSummary | null>(null);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Debounce search input
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      setDebouncedQuery(searchQuery);
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [searchQuery]);

  const fetchSkills = useCallback(async () => {
    log('fetchSkills: start');
    setSkillsLoading(true);
    setSkillsError(null);
    try {
      // Include `skills/`-root installs (registry installs land there) so they
      // appear in the Installed tab and flip the catalog Install button.
      const result = await workflowsApi.listWorkflows({ includeSkills: true });
      log('fetchSkills: count=%d', result.length);
      setSkills(result);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchSkills: error=%s', msg);
      setSkillsError(msg);
    } finally {
      setSkillsLoading(false);
    }
  }, []);

  // Compute the active source filter for RPC calls.
  // Only apply when the user has deselected at least one source.
  const activeSourceFilter = useMemo(() => {
    if (activeSources.size === 0 || activeSources.size >= sources.length) return undefined;
    // If exactly one source is active, pass it as the filter
    if (activeSources.size === 1) return [...activeSources][0];
    return undefined;
  }, [activeSources, sources.length]);

  // Fetch catalog via RPC search (handles both browse and search).
  // When query is empty and no source filter, uses browse; otherwise uses search.
  const fetchCatalog = useCallback(
    async (query: string, sourceFilter: string | undefined, forceRefresh: boolean) => {
      log('fetchCatalog: query=%s source=%s forceRefresh=%s', query, sourceFilter, forceRefresh);
      setCatalogLoading(true);
      setCatalogError(null);
      try {
        let entries: CatalogEntry[];
        if (!query && !sourceFilter && !forceRefresh) {
          entries = await skillRegistryApi.browse(false);
        } else if (!query && !sourceFilter && forceRefresh) {
          entries = await skillRegistryApi.browse(true);
        } else {
          entries = await skillRegistryApi.search(query || '', sourceFilter);
        }
        log('fetchCatalog: total=%d', entries.length);
        setCatalogTotal(entries.length);
        // Keep the full list so "Show more" can page through it without another
        // RPC; only a window of it is rendered (see displayedCatalog).
        setCatalogEntries(entries);
        setVisibleCount(CATALOG_PAGE_SIZE);
        setCatalogInitialized(true);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('fetchCatalog: error=%s', msg);
        setCatalogError(msg);
      } finally {
        setCatalogLoading(false);
      }
    },
    []
  );

  useEffect(() => {
    void fetchSkills();
    skillRegistryApi
      .sources()
      .then(s => {
        setSources(s);
        setActiveSources(new Set(s));
      })
      .catch(() => {});
  }, [fetchSkills]);

  // Trigger catalog search when debounced query or source filter changes
  useEffect(() => {
    if (view === 'registry') {
      void fetchCatalog(debouncedQuery, activeSourceFilter, false);
    }
  }, [view, debouncedQuery, activeSourceFilter, fetchCatalog]);

  const installedKeys = useMemo(
    () => new Set(skills.flatMap(skill => workflowInstallKeys(skill))),
    [skills]
  );

  // A catalog entry counts as installed if the refetched installed list maps
  // back to it (`isCatalogEntryInstalled`) OR we installed it this session. The
  // latter guarantees the card reflects a successful install even when the
  // heuristic key-match misses (#4150).
  const entryInstalled = useCallback(
    (entry: CatalogEntry): boolean =>
      installedEntryIds.has(entry.id) || isCatalogEntryInstalled(entry, installedKeys),
    [installedEntryIds, installedKeys]
  );

  const filteredSkills = useMemo(() => {
    const q = searchQuery.toLowerCase().trim();
    if (!q) return skills;
    return skills.filter(
      s =>
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some(tag => tag.toLowerCase().includes(q)) ||
        s.sourceFormat.toLowerCase().includes(q)
    );
  }, [skills, searchQuery]);

  const sortedSkills = useMemo(() => {
    return [...filteredSkills].sort((a, b) => {
      if (a.sourceFormat === 'hermes' && b.sourceFormat !== 'hermes') return -1;
      if (a.sourceFormat !== 'hermes' && b.sourceFormat === 'hermes') return 1;
      return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
    });
  }, [filteredSkills]);

  // When multiple sources are active (but not all), do client-side filtering
  // on the already-fetched results since the RPC only supports single source filter.
  const filteredCatalog = useMemo(() => {
    if (
      activeSources.size === 0 ||
      activeSources.size >= sources.length ||
      activeSources.size === 1
    ) {
      return catalogEntries;
    }
    return catalogEntries.filter(e => activeSources.has(e.source));
  }, [catalogEntries, activeSources, sources.length]);

  // Client-side pagination window: we already hold the full fetched list, so
  // "Show more" reveals the next page instantly with no extra RPC.
  const displayedCatalog = useMemo(
    () => filteredCatalog.slice(0, visibleCount),
    [filteredCatalog, visibleCount]
  );

  const handleInstalled = useCallback(
    (result: InstallWorkflowFromUrlResult) => {
      log('handleInstalled: newSkills=%d', result.newWorkflows.length);
      void fetchSkills();
      if (result.newWorkflows.length > 0) {
        onToast?.({
          type: 'success',
          title: t('skills.install.installComplete'),
          message: t('skills.install.successDiscovered').replace(
            '{count}',
            String(result.newWorkflows.length)
          ),
        });
      }
    },
    [fetchSkills, onToast, t]
  );

  const handleUninstalled = useCallback(() => {
    log('handleUninstalled');
    void fetchSkills();
    onToast?.({ type: 'success', title: t('skills.explorer.uninstallSuccess') });
  }, [fetchSkills, onToast, t]);

  const handleRegistryInstall = useCallback(
    async (entry: CatalogEntry) => {
      log('handleRegistryInstall: id=%s source=%s', entry.id, entry.source);
      setInstallingId(entry.id);
      try {
        const result = await skillRegistryApi.install(entry.id);
        // Authoritatively mark this entry installed so the card flips to
        // "Installed" on success regardless of whether the refetched list maps
        // back to it via the install-key heuristic (#4150).
        setInstalledEntryIds(prev => {
          const next = new Set(prev);
          next.add(entry.id);
          return next;
        });
        // Await the refetch so `installedKeys` is fresh before the button
        // re-renders — otherwise it briefly flips back to "Install" between
        // clearing the installing state and the list updating. `fetchSkills`
        // swallows its own errors, so this never throws into the catch below.
        await fetchSkills();
        onToast?.({
          type: 'success',
          title: t('skills.install.installComplete'),
          message: `Installed ${entry.name}${result.newSkills.length > 0 ? ` (${result.newSkills.join(', ')})` : ''}`,
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('handleRegistryInstall: error=%s', msg);
        onToast?.({ type: 'error', title: t('skills.install.errors.genericTitle'), message: msg });
      } finally {
        setInstallingId(null);
      }
    },
    [fetchSkills, onToast, t]
  );

  const loading = view === 'installed' ? skillsLoading : catalogLoading;
  const error = view === 'installed' ? skillsError : catalogError;

  return (
    <div className="rounded-2xl border border-line bg-surface p-3 shadow-soft animate-fade-up">
      <div className="px-1 pb-3 pt-1">
        <div className="flex items-center justify-between gap-2">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold text-content">
              {t('skills.explorer.title')}
            </h2>
            <p className="mt-0.5 text-[11px] leading-relaxed text-content-muted">
              {t('skills.explorer.subtitle')}
            </p>
          </div>
          <Button
            variant="secondary"
            size="sm"
            data-testid="skill-install-from-url-btn"
            onClick={() => setInstallDialogOpen(true)}
            className="flex-shrink-0">
            {t('skills.explorer.installFromUrl')}
          </Button>
        </div>
      </div>

      {/* View toggle */}
      <ChipTabs<ExplorerView>
        className="flex gap-2 px-1 pb-3"
        ariaLabel={t('skills.explorer.title')}
        value={view}
        onChange={setView}
        items={[
          {
            id: 'registry',
            label: (
              <>
                {t('skills.explorer.registryTab')}
                {catalogTotal > 0 && (
                  <span className="ml-1.5 text-[10px] opacity-70">
                    {catalogTotal.toLocaleString()}
                  </span>
                )}
              </>
            ),
          },
          {
            id: 'installed',
            label: (
              <>
                {t('skills.explorer.installedTab')}
                {skills.length > 0 && (
                  <span className="ml-1.5 text-[10px] opacity-70">{skills.length}</span>
                )}
              </>
            ),
          },
        ]}
      />

      {/* Source toggles */}
      {view === 'registry' && sources.length > 0 && (
        <div className="flex flex-wrap gap-1.5 px-1 pb-3">
          {sources.map(src => {
            const active = activeSources.has(src);
            return (
              <button
                key={src}
                type="button"
                onClick={() => {
                  setActiveSources(prev => {
                    const next = new Set(prev);
                    if (next.has(src)) next.delete(src);
                    else next.add(src);
                    return next;
                  });
                }}
                className={`rounded-full border px-2.5 py-1 text-[10px] font-medium transition-colors ${
                  active
                    ? 'border-primary-300 dark:border-primary-500/50 bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300'
                    : 'border-line bg-surface-muted text-content-faint hover:text-content-secondary'
                }`}>
                {src}
              </button>
            );
          })}
        </div>
      )}

      {/* Search */}
      <div className="flex gap-2 px-1 pb-3">
        <div className="relative flex-1">
          <svg
            className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-content-faint"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}>
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="m21 21-5.197-5.197m0 0A7.5 7.5 0 1 0 5.196 5.196a7.5 7.5 0 0 0 10.607 10.607Z"
            />
          </svg>
          <input
            type="text"
            data-testid="skill-search-input"
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            placeholder={t('skills.explorer.searchPlaceholder')}
            className="w-full rounded-lg border border-line bg-surface py-2 pl-9 pr-3 text-xs text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
          />
        </div>
        {view === 'registry' && (
          <Button
            iconOnly
            variant="secondary"
            size="md"
            onClick={() => void fetchCatalog(debouncedQuery, activeSourceFilter, true)}
            disabled={catalogLoading}
            title={t('skills.explorer.refreshRegistry')}
            aria-label={t('skills.explorer.refreshRegistry')}
            className="flex-shrink-0 text-content-muted shadow-sm">
            <svg
              className={`h-4 w-4 ${catalogLoading ? 'animate-spin' : ''}`}
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182"
              />
            </svg>
          </Button>
        )}
      </div>

      {/* Loading */}
      {loading && (
        <div className="flex items-center justify-center py-12">
          <span className="h-5 w-5 animate-spin rounded-full border-2 border-line border-t-primary-500" />
        </div>
      )}

      {/* Error */}
      {!loading && error && (
        <div className="mx-1 mb-3 rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 p-3">
          <p className="text-xs font-medium text-coral-700 dark:text-coral-300">{error}</p>
          <Button
            variant="secondary"
            tone="danger"
            size="xs"
            onClick={() =>
              void (view === 'installed'
                ? fetchSkills()
                : fetchCatalog(debouncedQuery, activeSourceFilter, true))
            }
            className="mt-2">
            {t('common.retry')}
          </Button>
        </div>
      )}

      {/* ── Installed view ── */}
      {view === 'installed' && !loading && !error && (
        <>
          {skills.length === 0 && (
            <EmptyStateCard
              className="mx-1 mb-3 py-10"
              icon={
                <svg
                  className="h-7 w-7 text-primary-500"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={1.5}>
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M9.813 15.904 9 18.75l-.813-2.846a4.5 4.5 0 0 0-3.09-3.09L2.25 12l2.846-.813a4.5 4.5 0 0 0 3.09-3.09L9 5.25l.813 2.846a4.5 4.5 0 0 0 3.09 3.09L15.75 12l-2.846.813a4.5 4.5 0 0 0-3.09 3.09Z"
                  />
                </svg>
              }
              title={t('skills.explorer.emptyTitle')}
              description={t('skills.explorer.emptyDescription')}
              actionLabel={t('skills.explorer.emptyCta')}
              onAction={() => setInstallDialogOpen(true)}
            />
          )}

          {skills.length > 0 && sortedSkills.length === 0 && (
            <p className="px-1 py-8 text-center text-xs text-content-faint">
              {t('skills.noResults')}
            </p>
          )}

          {sortedSkills.length > 0 && (
            <div
              className="grid gap-2 sm:gap-3"
              style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(14rem, 1fr))' }}>
              {sortedSkills.map(skill => (
                <SkillTile
                  key={skill.id}
                  skill={skill}
                  onClick={() => setDetailSkill(skill)}
                  onUninstall={() => setUninstallTarget(skill)}
                />
              ))}
            </div>
          )}
        </>
      )}

      {/* ── Registry view ── */}
      {view === 'registry' && !loading && !error && (
        <>
          {catalogInitialized && filteredCatalog.length === 0 && (
            <EmptyStateCard
              className="mx-1 mb-3 py-10"
              icon={
                <svg
                  className="h-7 w-7 text-primary-500"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={1.5}>
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M12 21v-8.25M15.75 21v-8.25M8.25 21v-8.25M3 9l9-6 9 6m-1.5 12V10.332A48.36 48.36 0 0 0 12 9.75c-2.551 0-5.056.2-7.5.582V21M3 21h18M12 6.75h.008v.008H12V6.75Z"
                  />
                </svg>
              }
              title={
                debouncedQuery ? t('skills.noResults') : t('skills.explorer.registryEmptyTitle')
              }
              description={debouncedQuery ? '' : t('skills.explorer.registryEmptyDescription')}
              actionLabel={debouncedQuery ? undefined : t('skills.explorer.refreshRegistry')}
              onAction={debouncedQuery ? undefined : () => void fetchCatalog('', undefined, true)}
            />
          )}

          {displayedCatalog.length > 0 && (
            <>
              <div
                className="grid gap-2 sm:gap-3"
                style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(14rem, 1fr))' }}>
                {displayedCatalog.map(entry => (
                  <CatalogTile
                    key={`${entry.source}-${entry.id}`}
                    entry={entry}
                    installed={entryInstalled(entry)}
                    installing={installingId === entry.id}
                    onClick={() => setDetailEntry(entry)}
                    onInstall={() => void handleRegistryInstall(entry)}
                  />
                ))}
              </div>
              {filteredCatalog.length > displayedCatalog.length && (
                <div className="mt-3 flex flex-col items-center gap-1">
                  <button
                    type="button"
                    data-testid="registry-show-more"
                    onClick={() => setVisibleCount(c => c + CATALOG_PAGE_SIZE)}
                    className="rounded-lg border border-line bg-surface px-4 py-2 text-xs font-medium text-content-secondary shadow-soft transition-colors hover:bg-surface-hover focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
                    {t('common.showMore')}
                  </button>
                  <p className="text-[11px] text-content-faint">
                    {displayedCatalog.length.toLocaleString()} /{' '}
                    {filteredCatalog.length.toLocaleString()}
                  </p>
                </div>
              )}
            </>
          )}
        </>
      )}

      {installDialogOpen && (
        <InstallSkillDialog
          onClose={() => setInstallDialogOpen(false)}
          onInstalled={handleInstalled}
        />
      )}

      {uninstallTarget && (
        <UninstallSkillConfirmDialog
          skill={uninstallTarget}
          onClose={() => setUninstallTarget(null)}
          onUninstalled={handleUninstalled}
        />
      )}

      {(detailEntry || detailSkill) && (
        <SkillDetailDialog
          entry={detailEntry}
          skill={detailSkill}
          installed={detailEntry ? entryInstalled(detailEntry) : true}
          onClose={() => {
            setDetailEntry(null);
            setDetailSkill(null);
          }}
          onInstall={
            detailEntry && !entryInstalled(detailEntry)
              ? () => {
                  void handleRegistryInstall(detailEntry);
                  setDetailEntry(null);
                }
              : undefined
          }
          installing={detailEntry ? installingId === detailEntry.id : false}
        />
      )}
    </div>
  );
}
