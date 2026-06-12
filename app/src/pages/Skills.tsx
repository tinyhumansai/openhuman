import { useCallback, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import ChannelSetupModal from '../components/channels/ChannelSetupModal';
import McpServersTab from '../components/channels/mcp/McpServersTab';
import ComposioConnectModal from '../components/composio/ComposioConnectModal';
import {
  composioToolkitMeta,
  type ComposioToolkitMeta,
  KNOWN_COMPOSIO_TOOLKITS,
} from '../components/composio/toolkitMeta';
import EmptyStateCard from '../components/EmptyStateCard';
import { ToastContainer } from '../components/intelligence/Toast';
import PillTabBar from '../components/PillTabBar';
import AutocompleteSetupModal from '../components/skills/AutocompleteSetupModal';
import MeetingBotsCard from '../components/skills/MeetingBotsCard';
import ScreenIntelligenceSetupModal from '../components/skills/ScreenIntelligenceSetupModal';
import UnifiedSkillCard from '../components/skills/SkillCard';
import { SKILL_CATEGORY_ORDER, type SkillCategory } from '../components/skills/skillCategories';
import SkillCategoryFilter from '../components/skills/SkillCategoryFilter';
import {
  getChannelIcons,
  skillCategoryHeadingClassName,
  SkillCategoryIcon,
} from '../components/skills/skillIcons';
import SkillSearchBar from '../components/skills/SkillSearchBar';
import SkillsExplorerTab from '../components/skills/SkillsExplorerTab';
import VoiceSetupModal from '../components/skills/VoiceSetupModal';
import { useAutocompleteSkillStatus } from '../features/autocomplete/useAutocompleteSkillStatus';
import { useScreenIntelligenceSkillStatus } from '../features/screen-intelligence/useScreenIntelligenceSkillStatus';
import { useVoiceSkillStatus } from '../features/voice/useVoiceSkillStatus';
import { useChannelDefinitions } from '../hooks/useChannelDefinitions';
import { useAgentReadyComposioToolkits, useComposioIntegrations } from '../lib/composio/hooks';
import { canonicalizeComposioToolkitSlug } from '../lib/composio/toolkitSlug';
import { type ComposioConnection, deriveComposioState } from '../lib/composio/types';
import { getCoreStateSnapshot } from '../lib/coreState/store';
import { useT } from '../lib/i18n/I18nContext';
import { channelConnectionsApi } from '../services/api/channelConnectionsApi';
import { setDefaultMessagingChannel } from '../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import type { ChannelConnectionStatus, ChannelDefinition, ChannelType } from '../types/channels';
import type { ToastNotification } from '../types/intelligence';
import { IS_DEV } from '../utils/config';
import { isLocalSessionToken } from '../utils/localSession';
import { openhumanComposioGetMode } from '../utils/tauriCommands';

function channelStatusLabel(status: ChannelConnectionStatus, t: (key: string) => string): string {
  switch (status) {
    case 'connected':
      return t('skills.connected');
    case 'connecting':
      return t('channels.status.connecting');
    case 'error':
      return t('common.error');
    default:
      return t('channels.status.notConfigured');
  }
}

function channelStatusColor(status: ChannelConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'text-sage-600 dark:text-sage-300';
    case 'connecting':
      return 'text-amber-600 dark:text-amber-300';
    case 'error':
      return 'text-coral-600 dark:text-coral-300';
    default:
      return 'text-stone-400 dark:text-neutral-500';
  }
}

// ─── Composio visual mappers ─────────────────────────────────────────────
// Reuse the same dot/label/color vocabulary as the channel cards so the
// "Integrations" section sits visually flush with the rest of the grid.

function composioStatusLabel(
  connection: ComposioConnection | undefined,
  t: (key: string) => string
): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return t('skills.connected');
    case 'pending':
      return t('channels.status.connecting');
    case 'expired':
      return t('composio.authExpired');
    case 'error':
      return t('common.error');
    default:
      return '';
  }
}

function composioStatusColor(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'text-sage-600 dark:text-sage-300';
    case 'pending':
      return 'text-amber-600 dark:text-amber-300';
    case 'expired':
      return 'text-coral-600 dark:text-coral-300';
    case 'error':
      return 'text-coral-600 dark:text-coral-300';
    default:
      return 'text-stone-400 dark:text-neutral-500';
  }
}

/** Sort order for the integrations grid: connected first, then pending, errors, disconnected. */
function composioSortRank(connection: ComposioConnection | undefined): number {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 0;
    case 'pending':
      return 1;
    case 'expired':
      return 2;
    case 'error':
      return 3;
    default:
      return 4;
  }
}

interface ComposioConnectorTileProps {
  meta: ComposioToolkitMeta;
  connection: ComposioConnection | undefined;
  /** Number of active connections for this toolkit (for multi-account badge). */
  activeConnectionCount?: number;
  hasComposioError: boolean;
  agentUnsupported: boolean;
  testId?: string;
  onOpen: () => void;
  onRetryGlobal: () => void;
}

function ComposioConnectorTile({
  meta,
  connection,
  activeConnectionCount = 0,
  hasComposioError,
  agentUnsupported,
  testId,
  onOpen,
  onRetryGlobal,
}: ComposioConnectorTileProps) {
  const { t } = useT();
  const rawState = deriveComposioState(connection);
  const state = hasComposioError ? 'error' : rawState;
  const isPreview = !hasComposioError && agentUnsupported && rawState === 'connected';
  const statusLabel = hasComposioError
    ? t('composio.statusUnavailable')
    : isPreview
      ? t('composio.previewBadge')
      : composioStatusLabel(connection, t);
  const ctaLabel = hasComposioError
    ? t('common.retry')
    : state === 'connected'
      ? t('skills.configure')
      : state === 'pending'
        ? t('skills.connect')
        : state === 'expired'
          ? t('composio.reconnect')
          : state === 'error'
            ? t('common.retry')
            : t('skills.connect');

  const isConnected = state === 'connected' && !isPreview;
  const isPending = state === 'pending';
  const isExpired = state === 'expired';
  const isError = state === 'error' || hasComposioError;

  const handleClick = () => {
    if (hasComposioError) {
      void onRetryGlobal();
      return;
    }
    onOpen();
  };

  return (
    <button
      type="button"
      data-testid={testId}
      onClick={handleClick}
      title={`${meta.name} — ${isPreview ? t('composio.previewTooltip') : meta.description}`}
      aria-label={`${meta.name}, ${statusLabel}. ${ctaLabel}.`}
      className={`group relative flex h-full w-full flex-col justify-center items-center rounded-2xl border p-3 text-center transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40 ${
        isConnected
          ? 'border-sage-300 bg-sage-50/80 shadow-[0_0_0_1px_rgba(34,197,94,0.12)] hover:bg-sage-50 dark:border-sage-500/30 dark:bg-sage-500/10 dark:hover:bg-sage-500/15'
          : isPreview
            ? 'border-amber-200 bg-amber-50/60 shadow-[0_0_0_1px_rgba(245,158,11,0.12)] hover:bg-amber-50/80 dark:border-amber-500/30 dark:bg-amber-500/10 dark:hover:bg-amber-500/15'
            : isPending
              ? 'border-amber-200 bg-amber-50/40 hover:bg-amber-50/70 dark:border-amber-500/30 dark:bg-amber-500/10 dark:hover:bg-amber-500/15'
              : isExpired || isError
                ? 'border-coral-200 bg-coral-50/30 hover:bg-coral-50/50 dark:border-coral-500/30 dark:bg-coral-500/10 dark:hover:bg-coral-500/15'
                : 'border-stone-200 bg-white hover:bg-stone-50 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:bg-neutral-800/60'
      }`}>
      {isPreview && (
        <span
          data-testid={`composio-preview-badge-${meta.slug}`}
          className="absolute right-1.5 top-1.5 max-w-[4.5rem] truncate rounded-full border border-amber-200 bg-amber-100 px-1.5 py-0.5 text-[9px] font-semibold uppercase leading-none text-amber-800 dark:border-amber-500/40 dark:bg-amber-500/15 dark:text-amber-200"
          title={t('composio.previewTooltip')}>
          {t('composio.previewBadge')}
        </span>
      )}
      {!isPreview && activeConnectionCount > 1 && (
        <span
          className="absolute right-1.5 top-1.5 rounded-full border border-sage-200 bg-sage-100 px-1.5 py-0.5 text-[9px] font-semibold leading-none text-sage-800 dark:border-sage-500/40 dark:bg-sage-500/15 dark:text-sage-200"
          title={t('composio.connect.connectedAccounts')}>
          {activeConnectionCount}
        </span>
      )}
      <div className="relative flex h-12 w-12 flex-shrink-0 items-center justify-center text-stone-700 dark:text-neutral-200 [&_img]:max-h-10 [&_img]:max-w-10 [&_svg]:h-8 [&_svg]:w-8">
        {meta.icon}
      </div>
      <div className="flex w-full min-w-0 flex-col items-center justify-start gap-0.5">
        <span className="line-clamp-2 text-[11px] font-semibold leading-tight text-stone-900 dark:text-neutral-100">
          {meta.name}
        </span>
        <span
          className={`line-clamp-1 text-[10px] font-medium ${
            hasComposioError
              ? 'text-amber-700 dark:text-amber-300'
              : isPreview
                ? 'text-amber-700 dark:text-amber-300'
                : composioStatusColor(connection)
          }`}>
          {statusLabel}
        </span>
      </div>
    </button>
  );
}

interface ChannelTileProps {
  def: ChannelDefinition;
  status: ChannelConnectionStatus;
  icon: React.ReactNode;
  testId?: string;
  onOpen: () => void;
}

function ChannelTile({ def, status, icon, testId, onOpen }: ChannelTileProps) {
  const { t } = useT();
  const isConnected = status === 'connected';
  const isPending = status === 'connecting';
  const isError = status === 'error';
  const statusLabel = channelStatusLabel(status, t);
  const ctaLabel = isConnected ? t('skills.configure') : t('channels.setup');

  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onOpen}
      title={`${def.display_name} — ${def.description}`}
      aria-label={`${def.display_name}, ${statusLabel}. ${ctaLabel}.`}
      className={`group flex flex-col items-center gap-2 rounded-2xl border p-3 pb-3 text-center transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40 ${
        isConnected
          ? 'border-sage-300 bg-sage-50/80 shadow-[0_0_0_1px_rgba(34,197,94,0.12)] hover:bg-sage-50 dark:border-sage-500/30 dark:bg-sage-500/10 dark:hover:bg-sage-500/15'
          : isPending
            ? 'border-amber-200 bg-amber-50/40 hover:bg-amber-50/70 dark:border-amber-500/30 dark:bg-amber-500/10 dark:hover:bg-amber-500/15'
            : isError
              ? 'border-coral-200 bg-coral-50/30 hover:bg-coral-50/50 dark:border-coral-500/30 dark:bg-coral-500/10 dark:hover:bg-coral-500/15'
              : 'border-stone-200 bg-white hover:bg-stone-50 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:bg-neutral-800/60'
      }`}>
      <div className="relative flex h-12 w-12 flex-shrink-0 items-center justify-center text-stone-700 dark:text-neutral-200 [&>span]:h-12 [&>span]:w-12 [&>span]:rounded-2xl [&_svg]:h-7 [&_svg]:w-7">
        {icon}
      </div>
      <div className="flex min-h-[2.5rem] w-full min-w-0 flex-col items-center justify-start gap-0.5">
        <span className="line-clamp-2 text-[11px] font-semibold leading-tight text-stone-900 dark:text-neutral-100">
          {def.display_name}
        </span>
        <span className={`line-clamp-1 text-[10px] font-medium ${channelStatusColor(status)}`}>
          {statusLabel}
        </span>
      </div>
    </button>
  );
}

function ComposioApiKeyEmptyState({ onOpenSettings }: { onOpenSettings: () => void }) {
  const { t } = useT();
  return (
    <EmptyStateCard
      className="mx-1 mb-3 py-10"
      icon={
        <svg
          className="h-7 w-7 text-primary-500"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={1.5}>
          <path strokeLinecap="round" strokeLinejoin="round" d="M13 10V3L4 14h7v7l9-11h-7Z" />
        </svg>
      }
      title={t('skills.composio.noApiKeyTitle')}
      description={t('skills.composio.noApiKeyDescription')}
      actionLabel={t('skills.composio.noApiKeyCta')}
      onAction={onOpenSettings}
    />
  );
}

// ─── Built-in skill definitions ────────────────────────────────────────────────

const BUILT_IN_SKILLS: Array<{
  id: string;
  title: string;
  description: string;
  route: string;
  icon: React.ReactNode;
}> = [
  // Hidden — not active yet. Uncomment to re-enable.
  // {
  //   id: 'screen-intelligence',
  //   title: 'Screen Intelligence',
  //   description:
  //     'Capture windows, summarize what is on screen, and feed useful context into memory.',
  //   route: '/settings/screen-intelligence',
  //   icon: BUILT_IN_SKILL_ICONS.screenIntelligence,
  // },
  // text-autocomplete + voice-stt hidden per #717 (modals/status hooks retained for re-enable).
];

// ─── Item type for unified list ────────────────────────────────────────────────

interface SkillItem {
  id: string;
  name: string;
  description: string;
  category: SkillCategory;
  kind: 'builtin' | 'channel';
  // For built-in
  route?: string;
  icon?: React.ReactNode;
  // For channel
  channelDef?: ChannelDefinition;
  channelStatus?: ChannelConnectionStatus;
}

// ─── Main Skills Page ──────────────────────────────────────────────────────────

/**
 * Primary tab values for the Connections page.
 *
 * Phase 2 rename mapping (old → new):
 *   composio  → apps
 *   channels  → messaging
 *   mcp       → tools   (meetings content folded in under Tools)
 *   skills    → explorer (kept secondary)
 *
 * Back-compat: the old ?tab= values (composio, channels, mcp, meetings) are
 * normalised to the new values so existing deep links continue to work.
 */
type ConnectionsTab = 'apps' | 'messaging' | 'tools' | 'explorer' | 'talents';

export default function Skills() {
  const { t } = useT();
  const channelIcons = useMemo(() => getChannelIcons(t), [t]);
  const location = useLocation();
  const navigate = useNavigate();
  const isLocalSession = isLocalSessionToken(getCoreStateSnapshot().snapshot.sessionToken);
  // Honour `?tab=<apps|messaging|tools|explorer>` so deep links land on the
  // right sub-tab.  Also normalise legacy tab names from the old /skills route
  // so that e.g. `/skills?tab=composio` still works after the redirect.
  const initialTab: ConnectionsTab = (() => {
    const params = new URLSearchParams(location.search);
    const raw = params.get('tab');
    // New canonical values
    if (
      raw === 'apps' ||
      raw === 'messaging' ||
      raw === 'tools' ||
      raw === 'explorer' ||
      raw === 'talents'
    )
      return raw;
    // Legacy back-compat aliases
    if (raw === 'composio') return 'apps';
    if (raw === 'channels') return 'messaging';
    if (raw === 'mcp') return 'tools';
    if (raw === 'meetings') return 'talents';
    if (raw === 'skills') return 'explorer';
    return 'apps';
  })();
  const [activeTab, setActiveTab] = useState<ConnectionsTab>(initialTab);
  const dispatch = useAppDispatch();
  const [defaultChannelBusy, setDefaultChannelBusy] = useState<ChannelType | null>(null);
  const handleSetDefaultChannel = useCallback(
    async (channel: ChannelType) => {
      // Single-flight: ignore re-entries while a write is in progress so two
      // back-to-back clicks can't interleave (would leave UI + persisted
      // preference disagreeing on which channel won).
      if (defaultChannelBusy !== null) return;
      setDefaultChannelBusy(channel);
      try {
        // Persist first, then dispatch — on failure the UI keeps the previous
        // selection and the user sees no false-positive flicker.
        await channelConnectionsApi.updatePreferences(channel);
        dispatch(setDefaultMessagingChannel(channel));
      } catch (err) {
        console.warn('[skills] default channel persist failed:', err);
      } finally {
        setDefaultChannelBusy(null);
      }
    },
    [dispatch, defaultChannelBusy]
  );
  const { definitions: channelDefs } = useChannelDefinitions();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const {
    toolkits: composioToolkits,
    connectionByToolkit: composioConnectionByToolkit,
    connectionsByToolkit: composioConnectionsByToolkit,
    error: composioError,
    refresh: refreshComposio,
  } = useComposioIntegrations();
  const {
    agentReady: agentReadyComposioToolkits,
    loading: agentReadyComposioLoading,
    error: agentReadyComposioError,
  } = useAgentReadyComposioToolkits();
  const agentReadinessKnown = !agentReadyComposioLoading && agentReadyComposioError === null;

  const [channelModalDef, setChannelModalDef] = useState<ChannelDefinition | null>(null);
  const [composioModalToolkit, setComposioModalToolkit] = useState<ComposioToolkitMeta | null>(
    null
  );
  const [screenIntelligenceModalOpen, setScreenIntelligenceModalOpen] = useState(false);
  const [autocompleteModalOpen, setAutocompleteModalOpen] = useState(false);
  const [voiceModalOpen, setVoiceModalOpen] = useState(false);
  const screenIntelligenceStatus = useScreenIntelligenceSkillStatus();
  const autocompleteStatus = useAutocompleteSkillStatus();
  const voiceStatus = useVoiceSkillStatus();

  const [toasts, setToasts] = useState<ToastNotification[]>([]);
  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    setToasts(prev => [...prev, { ...toast, id: `toast-${Date.now()}-${Math.random()}` }]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(t => t.id !== id));
  }, []);

  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<SkillCategory>('All');
  const [hasComposioApiKey, setHasComposioApiKey] = useState<boolean | null>(null);
  const showLocalComposioApiKeyBanner = isLocalSession && hasComposioApiKey === false;

  useEffect(() => {
    if (!isLocalSession) {
      setHasComposioApiKey(null);
      return;
    }
    let cancelled = false;
    void openhumanComposioGetMode()
      .then(res => {
        if (!cancelled) {
          setHasComposioApiKey(Boolean(res.result?.api_key_set));
        }
      })
      .catch(err => {
        if (!cancelled) {
          console.warn('[skills][composio] failed to load composio mode status:', err);
          setHasComposioApiKey(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [isLocalSession]);

  const bestChannelStatus = (channelId: ChannelType): ChannelConnectionStatus => {
    const conns = channelConnections.connections[channelId];
    if (!conns) return 'disconnected';
    const statuses = Object.values(conns).map(c => c?.status ?? 'disconnected');
    if (statuses.includes('connected')) return 'connected';
    if (statuses.includes('connecting')) return 'connecting';
    if (statuses.includes('error')) return 'error';
    return 'disconnected';
  };

  const configurableChannels = useMemo(
    () => channelDefs.filter(d => d.id !== 'web'),
    [channelDefs]
  );

  const composioCatalogToolkits = useMemo(() => {
    const normalizedToolkits = composioToolkits.map(slug => canonicalizeComposioToolkitSlug(slug));
    const missingKnownToolkits = KNOWN_COMPOSIO_TOOLKITS.filter(
      slug => !normalizedToolkits.includes(slug)
    );
    if (IS_DEV && missingKnownToolkits.length > 0) {
      console.debug('[skills][composio] filling gaps from KNOWN_COMPOSIO_TOOLKITS', {
        toolkitCount: composioToolkits.length,
        connectionCount: composioConnectionByToolkit.size,
        hasError: Boolean(composioError),
        missingKnownToolkits,
      });
    }
    return Array.from(new Set([...KNOWN_COMPOSIO_TOOLKITS, ...normalizedToolkits])).sort((a, b) =>
      a.localeCompare(b)
    );
  }, [composioToolkits, composioConnectionByToolkit, composioError]);

  // Unified item list
  const allItems: SkillItem[] = useMemo(() => {
    const items: SkillItem[] = [];

    for (const s of BUILT_IN_SKILLS) {
      items.push({
        id: s.id,
        name: s.title,
        description: s.description,
        category: 'Built-in',
        kind: 'builtin',
        route: s.route,
        icon: s.icon,
      });
    }

    for (const def of configurableChannels) {
      items.push({
        id: `channel-${def.id}`,
        name: def.display_name,
        description: def.description,
        category: 'Channels',
        kind: 'channel',
        channelDef: def,
        channelStatus: bestChannelStatus(def.id as ChannelType),
        icon: channelIcons[def.icon],
      });
    }

    // Composio toolkits are rendered in a dedicated icon grid (see below)
    // so ~100+ connectors stay scannable without a vertical list per category.
    //
    // NOTE: discovered SKILL.md workflows used to be surfaced here as cards.
    // Workflows now live exclusively on the Intelligence → Workflows tab, so
    // Connections is integrations-only (Composio / channels / MCP).

    return items;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channelIcons, configurableChannels, channelConnections]);

  const composioGridEntries = useMemo(() => {
    const entries: Array<{
      meta: ComposioToolkitMeta;
      connection: ComposioConnection | undefined;
    }> = [];
    for (const slug of composioCatalogToolkits) {
      const meta = composioToolkitMeta(slug);
      const connection = composioConnectionByToolkit.get(meta.slug);
      entries.push({ meta, connection });
    }
    return entries;
  }, [composioCatalogToolkits, composioConnectionByToolkit]);

  const composioFilteredEntries = useMemo(() => {
    const q = searchQuery.toLowerCase();
    const matchesSearch = (meta: ComposioToolkitMeta) =>
      !q || meta.name.toLowerCase().includes(q) || meta.description.toLowerCase().includes(q);

    const matchesCategory =
      selectedCategory === 'All'
        ? () => true
        : (meta: ComposioToolkitMeta) => meta.category === selectedCategory;

    return composioGridEntries.filter(({ meta }) => matchesCategory(meta) && matchesSearch(meta));
  }, [composioGridEntries, searchQuery, selectedCategory]);

  const composioSortedEntries = useMemo(() => {
    return [...composioFilteredEntries].sort((a, b) => {
      const rankA = composioSortRank(a.connection);
      const rankB = composioSortRank(b.connection);
      if (rankA !== rankB) return rankA - rankB;
      return a.meta.name.localeCompare(b.meta.name, undefined, { sensitivity: 'base' });
    });
  }, [composioFilteredEntries]);

  useEffect(() => {
    if (!IS_DEV) return;
    console.debug('[skills][composio] hook result', {
      toolkitCount: composioToolkits.length,
      connectionCount: composioConnectionByToolkit.size,
      hasError: Boolean(composioError),
      error: composioError,
      gridVisibleCount: composioSortedEntries.length,
    });
  }, [composioToolkits, composioConnectionByToolkit, composioError, composioSortedEntries.length]);

  const availableCategories: SkillCategory[] = useMemo(() => {
    const cats = new Set<SkillCategory>(['All']);
    for (const item of allItems) {
      if (item.category === 'Channels') continue;
      cats.add(item.category);
    }
    for (const { meta } of composioGridEntries) {
      cats.add(meta.category);
    }
    return SKILL_CATEGORY_ORDER.filter(
      c => c !== 'Channels' && cats.has(c) && (IS_DEV || c !== 'Other')
    );
  }, [allItems, composioGridEntries]);

  const filteredItems = useMemo(() => {
    const q = searchQuery.toLowerCase();
    return allItems.filter(item => {
      const matchesCategory = selectedCategory === 'All' || item.category === selectedCategory;
      const matchesSearch =
        !q || item.name.toLowerCase().includes(q) || item.description.toLowerCase().includes(q);
      return matchesCategory && matchesSearch;
    });
  }, [allItems, searchQuery, selectedCategory]);

  const groupedItems = useMemo(() => {
    const groups = new Map<SkillCategory, SkillItem[]>();
    for (const item of filteredItems) {
      const existing = groups.get(item.category);
      if (existing) {
        existing.push(item);
      } else {
        groups.set(item.category, [item]);
      }
    }
    return Array.from(groups.entries()).map(([category, items]) => ({ category, items }));
  }, [filteredItems]);

  const channelsGroup = useMemo(() => {
    const items = allItems.filter(item => item.category === 'Channels');
    return items.length > 0 ? { category: 'Channels' as SkillCategory, items } : undefined;
  }, [allItems]);
  const otherGroups = useMemo(
    () => groupedItems.filter(g => g.category !== 'Channels' && (IS_DEV || g.category !== 'Other')),
    [groupedItems]
  );

  const renderGroup = ({ category, items }: { category: SkillCategory; items: SkillItem[] }) => (
    <div
      key={category}
      className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3 shadow-soft animate-fade-up">
      <div className="px-1 pb-3 pt-1">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-stone-900 dark:text-neutral-100">
          <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-stone-100 dark:bg-neutral-800">
            <SkillCategoryIcon
              category={category}
              className={skillCategoryHeadingClassName(category)}
            />
          </span>
          {category}
        </h2>
      </div>
      <div className="space-y-2">
        {items.map(item => {
          if (item.kind === 'builtin') {
            /* v8 ignore start -- BUILT_IN_SKILLS list is empty today; the per-id
               branches below are kept for re-enabling screen-intelligence /
               text-autocomplete / voice-stt and shouldn't drag the diff-coverage
               gate down while they're unreachable. */
            if (item.id === 'screen-intelligence') {
              return (
                <UnifiedSkillCard
                  key={item.id}
                  icon={item.icon}
                  title={item.name}
                  description={item.description}
                  statusLabel={screenIntelligenceStatus.statusLabel}
                  statusColor={screenIntelligenceStatus.statusColor}
                  ctaLabel={screenIntelligenceStatus.ctaLabel}
                  ctaVariant={screenIntelligenceStatus.ctaVariant}
                  testId={`skill-row-${item.id}`}
                  ctaTestId={`skill-install-${item.id}`}
                  onCtaClick={() => {
                    if (screenIntelligenceStatus.platformUnsupported) {
                      navigate(item.route!);
                      return;
                    }
                    if (
                      screenIntelligenceStatus.connectionStatus === 'connected' ||
                      screenIntelligenceStatus.connectionStatus === 'disconnected'
                    ) {
                      navigate(item.route!);
                      return;
                    }
                    setScreenIntelligenceModalOpen(true);
                  }}
                />
              );
            }
            if (item.id === 'text-autocomplete') {
              return (
                <UnifiedSkillCard
                  key={item.id}
                  icon={item.icon}
                  title={item.name}
                  description={item.description}
                  statusLabel={autocompleteStatus.statusLabel}
                  statusColor={autocompleteStatus.statusColor}
                  ctaLabel={autocompleteStatus.ctaLabel}
                  ctaVariant={autocompleteStatus.ctaVariant}
                  testId={`skill-row-${item.id}`}
                  ctaTestId={`skill-install-${item.id}`}
                  onCtaClick={() => {
                    if (
                      autocompleteStatus.platformUnsupported ||
                      autocompleteStatus.connectionStatus === 'connected' ||
                      autocompleteStatus.connectionStatus === 'disconnected'
                    ) {
                      navigate(item.route!);
                      return;
                    }
                    setAutocompleteModalOpen(true);
                  }}
                />
              );
            }
            if (item.id === 'voice-stt') {
              return (
                <UnifiedSkillCard
                  key={item.id}
                  icon={item.icon}
                  title={item.name}
                  description={item.description}
                  statusLabel={voiceStatus.statusLabel}
                  statusColor={voiceStatus.statusColor}
                  ctaLabel={voiceStatus.ctaLabel}
                  ctaVariant={voiceStatus.ctaVariant}
                  testId={`skill-row-${item.id}`}
                  ctaTestId={`skill-install-${item.id}`}
                  onCtaClick={() => {
                    if (
                      voiceStatus.connectionStatus === 'connected' ||
                      voiceStatus.connectionStatus === 'connecting' ||
                      voiceStatus.connectionStatus === 'disconnected'
                    ) {
                      navigate(item.route!);
                      return;
                    }
                    setVoiceModalOpen(true);
                  }}
                />
              );
            }
            return (
              <UnifiedSkillCard
                key={item.id}
                icon={item.icon}
                title={item.name}
                description={item.description}
                ctaLabel={t('nav.settings')}
                testId={`skill-row-${item.id}`}
                ctaTestId={`skill-install-${item.id}`}
                onCtaClick={() => navigate(item.route!)}
              />
            );
            /* v8 ignore stop */
          }
        })}
      </div>
    </div>
  );

  return (
    <div className="min-h-full">
      <div className="min-h-full flex flex-col">
        <div className="flex-1 flex items-start justify-center p-4 pt-6">
          <div className="w-full max-w-3xl space-y-4">
            {/* <div className="flex items-center justify-between gap-2">
              <div className="min-w-0">
                <h1 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
                  Skills
                </h1>
                <p className="text-xs text-stone-500 dark:text-neutral-400">
                  Scaffold a new <code className="font-mono">SKILL.md</code> or install a published
                  package.
                </p>
              </div>
              <div className="flex flex-shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={() => setInstallDialogOpen(true)}
                  className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-xs font-medium text-stone-700 dark:text-neutral-200 shadow-soft transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
                  Install from URL
                </button>
                <button
                  type="button"
                  onClick={() => setCreateModalOpen(true)}
                  className="rounded-lg bg-primary-500 px-3 py-2 text-xs font-semibold text-white shadow-soft transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
                  New skill
                </button>
              </div>
            </div> */}

            {composioError && (
              <div className="rounded-2xl border border-amber-200 bg-amber-50 p-3 shadow-soft">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h2 className="text-sm font-semibold text-amber-900">
                      {t('skills.composio.staleStatusTitle')}
                    </h2>
                    <p className="mt-1 text-xs leading-relaxed text-amber-800">{composioError}</p>
                  </div>
                  <button
                    type="button"
                    onClick={() => void refreshComposio()}
                    className="flex-shrink-0 rounded-lg border border-amber-300 dark:border-amber-500/40 bg-white dark:bg-neutral-900 px-3 py-1.5 text-[11px] font-medium text-amber-800 dark:text-amber-300 transition-colors hover:bg-amber-100 dark:hover:bg-amber-500/10">
                    {t('common.retry')}
                  </button>
                </div>
              </div>
            )}

            <PillTabBar<ConnectionsTab>
              selected={activeTab}
              onChange={setActiveTab}
              items={[
                { value: 'apps', label: t('connections.tabs.apps') },
                { value: 'messaging', label: t('connections.tabs.messaging') },
                { value: 'tools', label: t('connections.tabs.tools') },
                { value: 'explorer', label: t('connections.tabs.explorer') },
                { value: 'talents', label: t('connections.tabs.talents') },
              ]}
            />
            {
              <>
                {activeTab === 'messaging' && channelsGroup && (
                  <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3 shadow-soft animate-fade-up">
                    <div className="px-1 pb-3 pt-1">
                      <h2
                        className="flex items-center gap-2 text-sm font-semibold text-stone-900 dark:text-neutral-100"
                        data-walkthrough="skills-channels">
                        <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-stone-100 dark:bg-neutral-800">
                          <SkillCategoryIcon
                            category="Channels"
                            className={skillCategoryHeadingClassName('Channels')}
                          />
                        </span>
                        {t('skills.channels')}
                      </h2>
                      <p className="mt-0.5 text-[11px] leading-relaxed text-stone-500 dark:text-neutral-400">
                        {t('channels.defaultMessaging')}
                      </p>
                    </div>
                    <div
                      className="grid gap-2 sm:gap-3"
                      style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(5.5rem, 1fr))' }}>
                      {channelsGroup.items.map(item => (
                        <div key={item.id} data-testid={`skill-row-${item.id}`}>
                          <ChannelTile
                            def={item.channelDef!}
                            status={item.channelStatus!}
                            icon={item.icon}
                            testId={`skill-install-${item.id}`}
                            onOpen={() => setChannelModalDef(item.channelDef!)}
                          />
                        </div>
                      ))}
                    </div>

                    <div className="mt-4 pt-3 border-t border-stone-100 dark:border-neutral-800">
                      <div className="text-[10px] font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400 mb-2">
                        {t('channels.defaultMessaging')}
                      </div>
                      <div className="grid grid-cols-2 gap-2">
                        {channelDefs.map(def => {
                          const channelId = def.id as ChannelType;
                          const selected = channelConnections.defaultMessagingChannel === channelId;
                          return (
                            <button
                              key={channelId}
                              type="button"
                              onClick={() => void handleSetDefaultChannel(channelId)}
                              disabled={defaultChannelBusy !== null}
                              className={`rounded-lg border px-3 py-2 text-xs font-medium transition-colors ${
                                selected
                                  ? 'border-primary-500/60 bg-primary-50 dark:bg-primary-500/10 text-primary-600 dark:text-primary-300'
                                  : 'border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-700'
                              }`}>
                              {def.display_name}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  </div>
                )}

                {activeTab === 'apps' && (
                  <div className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3 shadow-soft animate-fade-up">
                    <div className="px-1 pb-3 pt-1">
                      <div className="flex items-center gap-2">
                        <h2
                          className="text-sm font-semibold text-stone-900 dark:text-neutral-100"
                          data-walkthrough="skills-grid">
                          {t('skills.integrations')}
                        </h2>
                        <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[9px] font-semibold uppercase tracking-wider bg-primary-50 text-primary-700 dark:bg-primary-900/30 dark:text-primary-300 border border-primary-100 dark:border-primary-800/50">
                          {t('skills.composio.poweredBy')}
                        </span>
                      </div>
                      <p className="mt-0.5 text-[11px] leading-relaxed text-stone-500 dark:text-neutral-400">
                        {t('skills.integrationsSubtitle')}
                      </p>
                    </div>
                    {showLocalComposioApiKeyBanner && (
                      <ComposioApiKeyEmptyState
                        onOpenSettings={() => navigate('/settings/composio-routing')}
                      />
                    )}
                    {!showLocalComposioApiKeyBanner && (
                      <div className="space-y-3 px-1 pb-3">
                        <SkillSearchBar value={searchQuery} onChange={setSearchQuery} />
                        <SkillCategoryFilter
                          categories={availableCategories}
                          selected={selectedCategory}
                          onChange={setSelectedCategory}
                        />
                      </div>
                    )}
                    {!showLocalComposioApiKeyBanner &&
                      (composioSortedEntries.length > 0 ? (
                        <div
                          className="grid gap-2 sm:gap-3"
                          style={{
                            gridTemplateColumns: 'repeat(auto-fill, minmax(5.5rem, 1fr))',
                            gridAutoRows: '6.5rem',
                          }}>
                          {composioSortedEntries.map(({ meta, connection }) => {
                            const allConns = composioConnectionsByToolkit?.get(meta.slug);
                            const activeCount =
                              allConns?.filter(c => deriveComposioState(c) === 'connected')
                                .length ?? 0;
                            return (
                              <div
                                key={meta.slug}
                                data-testid={`skill-row-composio-${meta.slug}`}
                                className="overflow-hidden">
                                <ComposioConnectorTile
                                  meta={meta}
                                  connection={connection}
                                  activeConnectionCount={activeCount}
                                  hasComposioError={Boolean(composioError)}
                                  agentUnsupported={
                                    agentReadinessKnown &&
                                    deriveComposioState(connection) === 'connected' &&
                                    !agentReadyComposioToolkits.has(meta.slug)
                                  }
                                  testId={`skill-install-composio-${meta.slug}`}
                                  onOpen={() => setComposioModalToolkit(meta)}
                                  onRetryGlobal={() => void refreshComposio()}
                                />
                              </div>
                            );
                          })}
                        </div>
                      ) : (
                        <p className="px-1 py-4 text-center text-xs text-stone-400 dark:text-neutral-500">
                          {t('skills.noResults')}
                        </p>
                      ))}
                  </div>
                )}

                {activeTab === 'apps' && otherGroups.map(group => renderGroup(group))}

                {activeTab === 'explorer' && <SkillsExplorerTab onToast={addToast} />}

                {activeTab === 'tools' && (
                  <div className="space-y-4 animate-fade-up">
                    {/* MCP Servers */}
                    <McpServersTab />
                  </div>
                )}

                {activeTab === 'talents' && (
                  <div className="space-y-4 animate-fade-up">
                    {/* Meeting bots live under Talents (moved out of Tools) */}
                    <MeetingBotsCard onToast={addToast} />
                  </div>
                )}
              </>
            }
          </div>
        </div>
      </div>

      {channelModalDef && (
        <ChannelSetupModal definition={channelModalDef} onClose={() => setChannelModalDef(null)} />
      )}

      {screenIntelligenceModalOpen && (
        <ScreenIntelligenceSetupModal
          onClose={() => setScreenIntelligenceModalOpen(false)}
          initialStep={screenIntelligenceStatus.allPermissionsGranted ? 'enable' : 'permissions'}
        />
      )}

      {autocompleteModalOpen && (
        <AutocompleteSetupModal onClose={() => setAutocompleteModalOpen(false)} />
      )}

      {voiceModalOpen && (
        <VoiceSetupModal onClose={() => setVoiceModalOpen(false)} skillStatus={voiceStatus} />
      )}

      {composioModalToolkit && (
        <ComposioConnectModal
          toolkit={composioModalToolkit}
          connections={composioConnectionsByToolkit?.get(composioModalToolkit.slug)}
          agentUnsupported={
            agentReadinessKnown && !agentReadyComposioToolkits.has(composioModalToolkit.slug)
          }
          onChanged={() => {
            void refreshComposio();
          }}
          onClose={() => setComposioModalToolkit(null)}
        />
      )}

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
}
