import debug from 'debug';
import {
  ChevronRight,
  Download,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  Sparkles,
  Trash2,
} from 'lucide-react';
import {
  type KeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { LuLibrary, LuSparkles } from 'react-icons/lu';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type CatalogDetail,
  type CatalogEntry,
  type CatalogPage,
  isInstallable,
  type ParsedRegistryError,
  parseRegistryError,
  type ScanBlocked,
  skillRegistryApi,
} from '../../services/api/skillRegistryApi';
import {
  type InstallWorkflowFromUrlResult,
  skillsApi,
  type WorkflowSummary,
} from '../../services/api/skillsApi';
import McpIconButton from '../channels/mcp/McpIconButton';
import RowIcon from '../channels/mcp/RowIcon';
import EmptyStateCard from '../EmptyStateCard';
import { Alert, AlertDescription, Badge, type BadgeVariant, ModalShell } from '../ui';
import Button from '../ui/Button';
import DataTable, { type DataTableColumn } from '../ui/DataTable';
import { TableCell, TableRow } from '../ui/Table';
import CreateSkillModal from './CreateSkillModal';
import InstallSkillDialog from './InstallSkillDialog';
import { RegistryErrorNotice, RegistryStatusNotice } from './RegistryStatusNotice';
import ScanBlockedDialog from './ScanBlockedDialog';
import UninstallSkillConfirmDialog from './UninstallSkillConfirmDialog';

const log = debug('skills:explorer-tab');
const PAGE_SIZE = 25;
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

/**
 * Source tone table: where a skill comes from (shipped with the app, or
 * fetched from a remote catalogue) maps to a `Badge` variant instead of a
 * bespoke tint. See `docs/gitbooks/en/developing/theming.md`.
 */
const SOURCE_VARIANT: Record<string, BadgeVariant> = {
  'built-in': 'success',
  optional: 'primary',
};

function SourceBadge({ source }: { source: string }) {
  return (
    <Badge variant={SOURCE_VARIANT[source] ?? 'neutral'} dot={false}>
      {source}
    </Badge>
  );
}

/**
 * Format tone table. Three distinct variants for five formats, with no
 * collision: the Hermes family on `primary`, the ClawHub family on
 * `success`, and `legacy` on `warning` because it is the one row that means
 * "deprecated".
 */
const FORMAT_VARIANT: Record<string, BadgeVariant> = {
  hermes: 'primary',
  agentskills: 'primary',
  openclaw: 'success',
  clawhub: 'success',
  legacy: 'warning',
};

function SkillFormatBadge({ format }: { format: string }) {
  const lower = format.toLowerCase();
  const FORMAT_LABELS: Record<string, string> = {
    hermes: 'Hermes',
    agentskills: 'AgentSkills',
    openclaw: 'OpenClaw',
    clawhub: 'ClawHub',
    legacy: 'Legacy',
  };
  const label = FORMAT_LABELS[lower] ?? (format || 'Skill');
  return (
    <Badge variant={FORMAT_VARIANT[lower] ?? 'neutral'} dot={false}>
      {label}
    </Badge>
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
  return <Badge dot={false}>{label}</Badge>;
}

interface SkillTileProps {
  skill: WorkflowSummary;
  onUninstall: () => void;
  onClick: () => void;
  onRun: () => void;
  onEdit: () => void;
}

/** Enter / Space on a focused row opens it, like a click. */
const activateOnKey = (onActivate: () => void) => (event: KeyboardEvent<HTMLElement>) => {
  if (event.target !== event.currentTarget) return;
  if (event.key === 'Enter' || event.key === ' ' || event.key === 'Space') {
    event.preventDefault();
    onActivate();
  }
};

/** The name as the row's handle on its detail dialog (chevron = "opens"). */
function RowName({
  name,
  testId,
  onOpen,
}: {
  name: string;
  testId: string;
  onOpen: () => void;
}) {
  const { t } = useT();
  return (
    <button
      type="button"
      data-testid={testId}
      aria-label={t('skills.rows.open').replace('{name}', name)}
      onClick={event => {
        event.stopPropagation();
        onOpen();
      }}
      className="inline-flex max-w-full cursor-pointer items-center gap-0.5 rounded-sm text-sm font-medium text-content transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500">
      <span className="truncate">{name}</span>
      <ChevronRight className="size-3.5 shrink-0 text-content-muted" aria-hidden="true" />
    </button>
  );
}

/**
 * One installed skill as a table row: identity (name, description, tags),
 * format, scope, version, and run / edit / remove as icons — the same row
 * grammar as the MCP servers table. The row (and its name) opens the detail.
 */
function InstalledSkillRow({ skill, onUninstall, onClick, onRun, onEdit }: SkillTileProps) {
  const { t } = useT();
  const editable = skill.scope === 'user';
  return (
    <TableRow
      data-testid={`skill-explorer-tile-${skill.id}`}
      tabIndex={0}
      onClick={onClick}
      onKeyDown={activateOnKey(onClick)}
      className="cursor-pointer focus-visible:bg-surface-hover focus-visible:outline-hidden">
      <TableCell className="w-full max-w-0">
        <div className="flex min-w-0 items-center gap-3">
          <RowIcon>
            <Sparkles className="size-4 text-content-muted" aria-hidden="true" />
          </RowIcon>
          <div className="min-w-0 space-y-0.5">
            <RowName name={skill.name} testId={`skill-open-${skill.id}`} onOpen={onClick} />
            <p className="truncate text-xs text-content-muted" title={skill.description}>
              {skill.description || t('skills.explorer.noDescription')}
            </p>
            {(skill.tags.length > 0 || skill.warnings.length > 0) && (
              <div className="flex flex-wrap items-center gap-1 pt-0.5">
                {skill.tags.map(tag => (
                  <Badge key={tag} variant="neutral">
                    {tag}
                  </Badge>
                ))}
                {skill.warnings.map(warning => (
                  <Badge key={warning} variant="warning">
                    {warning}
                  </Badge>
                ))}
              </div>
            )}
          </div>
        </div>
      </TableCell>
      <TableCell className="w-px whitespace-nowrap">
        <SkillFormatBadge format={skill.sourceFormat} />
      </TableCell>
      <TableCell className="w-px whitespace-nowrap">
        <SkillScopeBadge scope={skill.scope} />
      </TableCell>
      <TableCell className="w-px whitespace-nowrap font-mono text-xs text-content-muted">
        {skill.version ? `v${skill.version}` : '—'}
      </TableCell>
      <TableCell className="w-px whitespace-nowrap text-right">
        <span className="inline-flex items-center gap-0.5">
          <McpIconButton
            label={t('skills.rows.run').replace('{name}', skill.name)}
            icon={Play}
            tone="primary"
            testId={`skill-run-${skill.id}`}
            onClick={onRun}
          />
          {editable && (
            <McpIconButton
              label={t('skills.rows.edit').replace('{name}', skill.name)}
              icon={Pencil}
              testId={`skill-edit-${skill.id}`}
              onClick={onEdit}
            />
          )}
          {editable ? (
            <McpIconButton
              label={t('skills.rows.remove').replace('{name}', skill.name)}
              icon={Trash2}
              tone="destructive"
              testId={`skill-uninstall-${skill.id}`}
              onClick={onUninstall}
            />
          ) : (
            <Badge variant="neutral" dot={false}>
              {t('skills.explorer.installed')}
            </Badge>
          )}
        </span>
      </TableCell>
    </TableRow>
  );
}

interface CatalogTileProps {
  entry: CatalogEntry;
  installed: boolean;
  installing: boolean;
  onInstall: () => void;
  onClick: () => void;
}

interface SkillDetailDialogProps {
  entry: CatalogEntry | null;
  detail?: CatalogDetail | null;
  skill: WorkflowSummary | null;
  installed: boolean;
  onClose: () => void;
  onInstall?: () => void;
  installing?: boolean;
}

function SourceLink({ url, testId }: { url: string; testId: string }) {
  const { t } = useT();
  return (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      data-testid={testId}
      onClick={event => event.stopPropagation()}
      className="text-xs font-medium text-primary-600 hover:underline dark:text-primary-300">
      {t('skills.registry.viewSource')}
    </a>
  );
}

function CatalogRow({ entry, installed, installing, onInstall, onClick }: CatalogTileProps) {
  const { t } = useT();
  return (
    <TableRow
      data-testid={`registry-tile-${entry.id}`}
      tabIndex={0}
      onClick={onClick}
      onKeyDown={activateOnKey(onClick)}
      className="cursor-pointer focus-visible:bg-surface-hover focus-visible:outline-hidden">
      <TableCell className="w-full max-w-0">
        <div className="flex min-w-0 items-center gap-3">
          <RowIcon>
            <span className="text-xs font-semibold text-content-muted">
              {entry.name.charAt(0).toUpperCase()}
            </span>
          </RowIcon>
          <div className="min-w-0 space-y-0.5">
            <RowName name={entry.name} testId={`registry-open-${entry.id}`} onOpen={onClick} />
            <p className="truncate text-xs text-content-muted" title={entry.description}>
              {entry.description}
            </p>
          </div>
        </div>
      </TableCell>
      <TableCell className="w-px whitespace-nowrap">
        <SourceBadge source={entry.source} />
      </TableCell>
      <TableCell className="w-px whitespace-nowrap text-right">
        {installed ? (
          <Badge variant="success">{t('skills.explorer.installed')}</Badge>
        ) : !isInstallable(entry) ? (
          <span className="inline-flex items-center gap-2">
            <Badge
              variant="neutral"
              title={t('skills.explorer.notInstallableHint')}
              data-testid={`registry-not-installable-${entry.id}`}>
              {t('skills.explorer.notInstallable')}
            </Badge>
            {entry.source_url && (
              <SourceLink url={entry.source_url} testId={`registry-source-link-${entry.id}`} />
            )}
          </span>
        ) : (
          <Button
            variant="secondary"
            size="sm"
            data-testid={`registry-install-${entry.id}`}
            disabled={installing}
            leadingIcon={<Download className="size-3.5" aria-hidden="true" />}
            onClick={event => {
              event.stopPropagation();
              onInstall();
            }}>
            {installing ? t('skills.explorer.installing') : t('skills.explorer.install')}
          </Button>
        )}
      </TableCell>
    </TableRow>
  );
}

function SkillDetailDialog({
  entry,
  detail,
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
  const downloadUrl = detail?.download_url || entry?.download_url || '';
  const license = detail?.license ?? entry?.license ?? '';
  const overview = detail?.overview?.trim() ?? '';
  const sourceUrl = detail?.source_url ?? entry?.source_url ?? '';
  const installable = entry ? isInstallable(entry) : false;

  return (
    <ModalShell
      onClose={onClose}
      titleId="skill-detail-title"
      maxWidthClassName="max-w-lg"
      contentClassName="p-5 space-y-4"
      title={
        <span className="flex items-center gap-2">
          <span className="truncate">{name}</span>
          {installed && (
            <Badge variant="success" className="shrink-0" dot={false}>
              {t('skills.explorer.installed')}
            </Badge>
          )}
        </span>
      }
      subtitle={
        <span className="mt-1.5 flex items-center gap-1.5">
          {source && <SourceBadge source={source} />}
          {category && <Badge dot={false}>{category}</Badge>}
        </span>
      }
      footer={
        !installed && onInstall ? (
          <div className="flex justify-end">
            {installable ? (
              <Button variant="secondary" size="sm" disabled={installing} onClick={onInstall}>
                {installing ? t('skills.explorer.installing') : t('skills.explorer.install')}
              </Button>
            ) : (
              <p className="flex flex-wrap items-center gap-2 text-xs text-content-muted">
                <span>{t('skills.explorer.notInstallableHint')}</span>
                {sourceUrl && <SourceLink url={sourceUrl} testId="skill-detail-source-link" />}
              </p>
            )}
          </div>
        ) : undefined
      }>
      <>
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

        {overview && overview !== description && (
          <div data-testid="skill-detail-overview">
            <h3 className="text-[11px] font-semibold uppercase tracking-wider text-content-faint mb-1">
              {t('skills.detail.overview')}
            </h3>
            <p className="text-sm text-content-secondary leading-relaxed whitespace-pre-wrap">
              {overview}
            </p>
          </div>
        )}

        <div className="flex flex-wrap gap-x-6 gap-y-2">
          {version && (
            <div>
              <span className="text-[10px] font-semibold uppercase tracking-wider text-content-faint">
                {t('skills.detail.version')}
              </span>
              <p className="text-xs font-mono text-content-secondary">v{version}</p>
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
                <Badge key={tag} dot={false}>
                  {tag}
                </Badge>
              ))}
            </div>
          </div>
        )}

        {downloadUrl && (
          <div>
            <h3 className="text-[11px] font-semibold uppercase tracking-wider text-content-faint mb-1">
              {t('skills.detail.source')}
            </h3>
            <p className="text-[11px] font-mono text-content-faint break-all">{downloadUrl}</p>
          </div>
        )}
      </>
    </ModalShell>
  );
}

export type ExplorerView = 'installed' | 'registry';

interface SkillsExplorerTabProps {
  onToast?: (toast: { type: 'success' | 'error'; title: string; message?: string }) => void;
  /** Which notation the page header picked. */
  view: ExplorerView;
}

export default function SkillsExplorerTab({ onToast, view }: SkillsExplorerTabProps) {
  const { t } = useT();
  const navigate = useNavigate();

  const [skills, setSkills] = useState<WorkflowSummary[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(true);
  const [skillsError, setSkillsError] = useState<string | null>(null);

  const [catalogPage, setCatalogPage] = useState<CatalogPage | null>(null);
  const [catalogPageNum, setCatalogPageNum] = useState(1);
  const [catalogPageSize, setCatalogPageSize] = useState(PAGE_SIZE);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState<ParsedRegistryError | null>(null);
  const [catalogInitialized, setCatalogInitialized] = useState(false);
  const catalogRequestRef = useRef(0);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [scanBlocked, setScanBlocked] = useState<{
    entry: CatalogEntry;
    scan: ScanBlocked;
  } | null>(null);
  const [scanOverrideError, setScanOverrideError] = useState<string | null>(null);
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
  // Scope facet on the Installed table (user / project / legacy).
  const [activeScopes, setActiveScopes] = useState<ReadonlySet<string>>(new Set());
  const [debouncedQuery, setDebouncedQuery] = useState('');
  const [installDialogOpen, setInstallDialogOpen] = useState(false);
  // `null` closed, `undefined` creating, a skill editing.
  const [createOpen, setCreateOpen] = useState<WorkflowSummary | null | undefined>(null);
  const [uninstallTarget, setUninstallTarget] = useState<WorkflowSummary | null>(null);
  const [detailEntry, setDetailEntry] = useState<CatalogEntry | null>(null);
  const [entryDetail, setEntryDetail] = useState<CatalogDetail | null>(null);
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
      const result = await skillsApi.listWorkflows({ includeSkills: true });
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

  const activeSourceFilter = useMemo(() => {
    if (activeSources.size === 0 || activeSources.size >= sources.length) return undefined;
    return [...activeSources].sort();
  }, [activeSources, sources.length]);
  const activeSourceKey = activeSourceFilter?.join('\u0000') ?? '';

  const fetchCatalog = useCallback(
    async (
      query: string,
      sourceFilter: string[] | undefined,
      page: number,
      pageSize: number,
      forceRefresh: boolean
    ) => {
      const request = ++catalogRequestRef.current;
      log(
        'fetchCatalog: query=%s sources=%d page=%d size=%d forceRefresh=%s',
        query,
        sourceFilter?.length ?? 0,
        page,
        pageSize,
        forceRefresh
      );
      setCatalogLoading(true);
      setCatalogError(null);
      try {
        const result = await skillRegistryApi.browsePage({
          query,
          sources: sourceFilter,
          page,
          pageSize,
          forceRefresh,
        });
        if (request !== catalogRequestRef.current) return;
        log(
          'fetchCatalog: total=%d page=%d freshness=%s',
          result.total,
          result.page,
          result.freshness
        );
        setCatalogPage(result);
        setCatalogInitialized(true);
      } catch (err) {
        if (request !== catalogRequestRef.current) return;
        const parsed = parseRegistryError(err);
        log('fetchCatalog: error kind=%s msg=%s', parsed.kind, parsed.message);
        setCatalogError(parsed);
      } finally {
        if (request === catalogRequestRef.current) setCatalogLoading(false);
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

  useEffect(() => {
    setCatalogPageNum(1);
  }, [debouncedQuery, activeSourceKey]);

  useEffect(() => {
    if (view === 'registry') {
      void fetchCatalog(debouncedQuery, activeSourceFilter, catalogPageNum, catalogPageSize, false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view, debouncedQuery, activeSourceKey, catalogPageNum, catalogPageSize, fetchCatalog]);

  const refreshCatalog = useCallback(
    () =>
      void fetchCatalog(debouncedQuery, activeSourceFilter, catalogPageNum, catalogPageSize, true),
    [fetchCatalog, debouncedQuery, activeSourceFilter, catalogPageNum, catalogPageSize]
  );

  useEffect(() => {
    if (!detailEntry) {
      setEntryDetail(null);
      return;
    }
    let cancelled = false;
    skillRegistryApi
      .detail(detailEntry.id)
      .then(detail => {
        if (!cancelled) setEntryDetail(detail);
      })
      .catch(err => {
        log('detail: error=%s', parseRegistryError(err).message);
      });
    return () => {
      cancelled = true;
    };
  }, [detailEntry]);

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
    const inScope = skills.filter(s => activeScopes.size === 0 || activeScopes.has(s.scope));
    if (!q) return inScope;
    return inScope.filter(
      s =>
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some(tag => tag.toLowerCase().includes(q)) ||
        s.sourceFormat.toLowerCase().includes(q)
    );
  }, [skills, searchQuery, activeScopes]);

  const sortedSkills = useMemo(() => {
    return [...filteredSkills].sort((a, b) => {
      if (a.sourceFormat === 'hermes' && b.sourceFormat !== 'hermes') return -1;
      if (a.sourceFormat !== 'hermes' && b.sourceFormat === 'hermes') return 1;
      return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
    });
  }, [filteredSkills]);

  const catalogEntries = catalogPage?.entries ?? [];

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

  const registryInstallErrorMessage = useCallback(
    (entry: CatalogEntry, err: unknown): string => {
      const parsed = parseRegistryError(err);
      log('registryInstall: error kind=%s msg=%s', parsed.kind, parsed.message);
      if (parsed.kind === 'no_direct_download') {
        return entry.source_url
          ? `${t('skills.registry.noDirectDownload')} ${entry.source_url}`
          : t('skills.registry.noDirectDownload');
      }
      if (parsed.kind === 'upstream_ambiguous') return t('skills.registry.upstreamAmbiguous');
      if (parsed.kind === 'rate_limited') {
        return parsed.retryAfterSecs != null
          ? t('skills.registry.rateLimited').replace('{seconds}', String(parsed.retryAfterSecs))
          : t('skills.registry.rateLimitedShortly');
      }
      return parsed.message;
    },
    [t]
  );

  const runRegistryInstall = useCallback(
    async (entry: CatalogEntry, acknowledgedDigest?: string): Promise<boolean> => {
      log(
        'runRegistryInstall: id=%s source=%s acknowledged=%s',
        entry.id,
        entry.source,
        Boolean(acknowledgedDigest)
      );
      setInstallingId(entry.id);
      try {
        const result = await skillRegistryApi.install(entry.id, { acknowledgedDigest });
        if (result.status === 'scan_blocked') {
          log('runRegistryInstall: scan_blocked findings=%d', result.scan.findings.length);
          setScanOverrideError(null);
          setScanBlocked({ entry, scan: result.scan });
          return false;
        }
        setInstalledEntryIds(prev => {
          const next = new Set(prev);
          next.add(entry.id);
          return next;
        });
        await fetchSkills();
        onToast?.({
          type: 'success',
          title: t('skills.install.installComplete'),
          message: `Installed ${entry.name}${result.newSkills.length > 0 ? ` (${result.newSkills.join(', ')})` : ''}`,
        });
        return true;
      } finally {
        setInstallingId(null);
      }
    },
    [fetchSkills, onToast, t]
  );

  const handleRegistryInstall = useCallback(
    async (entry: CatalogEntry) => {
      try {
        await runRegistryInstall(entry);
      } catch (err) {
        onToast?.({
          type: 'error',
          title: t('skills.install.errors.genericTitle'),
          message: registryInstallErrorMessage(entry, err),
        });
      }
    },
    [onToast, registryInstallErrorMessage, runRegistryInstall, t]
  );

  const handleScanBlock = useCallback(() => {
    log('scanBlocked: user kept %s uninstalled', scanBlocked?.entry.id);
    setScanBlocked(null);
    setScanOverrideError(null);
    onToast?.({
      type: 'error',
      title: t('skills.scan.declinedTitle'),
      message: t('skills.scan.declinedHint'),
    });
  }, [onToast, scanBlocked, t]);

  const handleScanInstallAnyway = useCallback(async () => {
    if (!scanBlocked) return;
    const { entry, scan } = scanBlocked;
    try {
      const installed = await runRegistryInstall(entry, scan.digest);
      if (installed) setScanBlocked(null);
    } catch (err) {
      setScanOverrideError(registryInstallErrorMessage(entry, err));
    }
  }, [registryInstallErrorMessage, runRegistryInstall, scanBlocked]);

  const loading = view === 'installed' ? skillsLoading : catalogLoading;

  const runSkill = (skill: WorkflowSummary) =>
    navigate(`/workflows/run?workflow=${encodeURIComponent(skill.id)}&lock=1`);

  const installedErrorNode =
    !skillsLoading && skillsError ? (
      <Alert variant="destructive" density="compact">
        <AlertDescription className="flex flex-wrap items-center justify-between gap-2">
          <span>{skillsError}</span>
          <Button variant="secondary" tone="danger" size="xs" onClick={() => void fetchSkills()}>
            {t('common.retry')}
          </Button>
        </AlertDescription>
      </Alert>
    ) : undefined;

  const registryErrorNode =
    !catalogLoading && catalogError ? (
      <RegistryErrorNotice error={catalogError} onRetry={refreshCatalog} />
    ) : (
      <RegistryStatusNotice
        page={catalogPage}
        firstLoad={catalogLoading && !catalogInitialized}
        onRetry={refreshCatalog}
      />
    );

  const search = {
    value: searchQuery,
    onChange: setSearchQuery,
    placeholder: t('skills.explorer.searchPlaceholder'),
    ariaLabel: t('skills.explorer.title'),
    testId: 'skill-search-input',
  };

  const installedEmpty =
    // A search that matched nothing is not the same as having no skills — the
    // second offers an install CTA, the first would be nonsense.
    skills.length > 0 ? (
      <p className="text-center text-sm text-content-muted">{t('skills.noResults')}</p>
    ) : (
      <EmptyStateCard
        className="py-6"
        icon={<LuSparkles className="h-7 w-7 text-primary-500" strokeWidth={1.5} />}
        title={t('skills.explorer.emptyTitle')}
        description={t('skills.explorer.emptyDescription')}
        actionLabel={t('skills.explorer.emptyCta')}
        onAction={() => setInstallDialogOpen(true)}
      />
    );

  const registryEmpty = catalogInitialized ? (
    <EmptyStateCard
      className="py-6"
      icon={<LuLibrary className="h-7 w-7 text-primary-500" strokeWidth={1.5} />}
      title={debouncedQuery ? t('skills.noResults') : t('skills.explorer.registryEmptyTitle')}
      description={debouncedQuery ? '' : t('skills.explorer.registryEmptyDescription')}
      actionLabel={debouncedQuery ? undefined : t('skills.explorer.refreshRegistry')}
      onAction={debouncedQuery ? undefined : refreshCatalog}
    />
  ) : null;

  const dialogs = (
    <>
      {installDialogOpen && (
        <InstallSkillDialog
          onClose={() => setInstallDialogOpen(false)}
          onInstalled={handleInstalled}
        />
      )}

      {createOpen !== null && (
        <CreateSkillModal
          editing={createOpen ?? undefined}
          onClose={() => setCreateOpen(null)}
          onCreated={() => {
            setCreateOpen(null);
            void fetchSkills();
          }}
        />
      )}

      {scanBlocked && (
        <ScanBlockedDialog
          skillName={scanBlocked.entry.name}
          scan={scanBlocked.scan}
          installing={installingId === scanBlocked.entry.id}
          error={scanOverrideError}
          onBlock={handleScanBlock}
          onInstallAnyway={() => void handleScanInstallAnyway()}
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
          detail={entryDetail}
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
    </>
  );

  const installedColumns: DataTableColumn<WorkflowSummary>[] = [
    { id: 'name', header: t('common.name'), className: 'w-full max-w-0' },
    { id: 'format', header: t('dataTable.column.format'), className: 'w-px whitespace-nowrap' },
    { id: 'scope', header: t('dataTable.column.scope'), className: 'w-px whitespace-nowrap' },
    { id: 'version', header: t('dataTable.column.version'), className: 'w-px whitespace-nowrap' },
    {
      id: 'actions',
      header: <span className="sr-only">{t('dataTable.column.actions')}</span>,
      align: 'right',
      className: 'w-px whitespace-nowrap',
    },
  ];

  const catalogColumns: DataTableColumn<CatalogEntry>[] = [
    { id: 'name', header: t('common.name'), className: 'w-full max-w-0' },
    { id: 'source', header: t('dataTable.column.source'), className: 'w-px whitespace-nowrap' },
    {
      id: 'action',
      header: <span className="sr-only">{t('dataTable.column.actions')}</span>,
      align: 'right',
      className: 'w-px whitespace-nowrap',
    },
  ];

  if (view === 'installed') {
    return (
      <section
        className="flex h-full min-h-0 animate-fade-up flex-col"
        data-testid="skills-installed-section">
        <DataTable<WorkflowSummary>
          title={t('skills.rows.installedTitle')}
          description={t('skills.rows.installedIntro')}
          actions={
            <>
              <Button
                variant="secondary"
                size="sm"
                data-testid="skill-install-from-url-btn"
                onClick={() => setInstallDialogOpen(true)}
                leadingIcon={<Download className="size-4" aria-hidden="true" />}>
                {t('skills.explorer.installFromUrl')}
              </Button>
              <Button
                variant="primary"
                size="sm"
                data-testid="skill-new-btn"
                onClick={() => setCreateOpen(undefined)}
                leadingIcon={<Plus className="size-4" aria-hidden="true" />}>
                {t('skills.explorer.newSkill')}
              </Button>
            </>
          }
          columns={installedColumns}
          rows={sortedSkills}
          rowKey={skill => skill.id}
          renderRow={skill => (
            <InstalledSkillRow
              key={skill.id}
              skill={skill}
              onClick={() => setDetailSkill(skill)}
              onRun={() => runSkill(skill)}
              onEdit={() => setCreateOpen(skill)}
              onUninstall={() => setUninstallTarget(skill)}
            />
          )}
          search={search}
          filters={[
            {
              id: 'scope',
              label: t('dataTable.column.scope'),
              options: [
                { value: 'user', label: t('skills.explorer.scopeUser') },
                { value: 'project', label: t('skills.explorer.scopeProject') },
                { value: 'legacy', label: t('skills.explorer.scopeLegacy') },
              ],
              selected: activeScopes,
              onChange: setActiveScopes,
              testId: 'skill-scope-filter',
            },
          ]}
          pagination={{ pageSize: PAGE_SIZE }}
          loading={loading}
          error={installedErrorNode}
          empty={installedEmpty}
          ariaLabel={t('skills.rows.installedTitle')}
        />
        {dialogs}
      </section>
    );
  }

  return (
    <section
      className="flex h-full min-h-0 animate-fade-up flex-col"
      data-testid="skills-registry-section">
      <DataTable<CatalogEntry>
        title={t('skills.rows.registryTitle')}
        description={t('skills.rows.registryIntro')}
        actions={
          <Button
            variant="secondary"
            size="sm"
            onClick={refreshCatalog}
            disabled={catalogLoading}
            aria-label={t('skills.explorer.refreshRegistry')}
            leadingIcon={
              <RefreshCw
                className={`size-3.5 ${catalogLoading ? 'animate-spin' : ''}`}
                aria-hidden="true"
              />
            }>
            {t('skills.explorer.refreshRegistry')}
          </Button>
        }
        columns={catalogColumns}
        rows={catalogEntries}
        rowKey={entry => `${entry.source}-${entry.id}`}
        renderRow={entry => (
          <CatalogRow
            key={`${entry.source}-${entry.id}`}
            entry={entry}
            installed={entryInstalled(entry)}
            installing={installingId === entry.id}
            onClick={() => setDetailEntry(entry)}
            onInstall={() => void handleRegistryInstall(entry)}
          />
        )}
        search={search}
        filters={
          sources.length > 0
            ? [
                {
                  id: 'source',
                  label: t('dataTable.column.source'),
                  ariaLabel: t('skills.explorer.sourceFilterAria'),
                  testId: 'skill-source-filter',
                  options: sources.map(source => ({ value: source })),
                  selected: activeSources,
                  onChange: setActiveSources,
                },
              ]
            : undefined
        }
        pagination={{
          page: catalogPageNum,
          pageSize: catalogPageSize,
          total: catalogPage?.total,
          onPageChange: setCatalogPageNum,
          onPageSizeChange: setCatalogPageSize,
          testId: 'registry-pagination',
        }}
        loading={loading && catalogEntries.length === 0}
        error={registryErrorNode}
        empty={registryEmpty ?? undefined}
        ariaLabel={t('skills.rows.registryTitle')}
      />
      {dialogs}
    </section>
  );
}
