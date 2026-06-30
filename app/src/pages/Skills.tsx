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
import PanelPage from '../components/layout/PanelPage';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../components/layout/TwoPaneNav';
import { SettingsLayoutProvider } from '../components/settings/layout/SettingsLayoutContext';
import AIPanel from '../components/settings/panels/AIPanel';
import ComposioPanel from '../components/settings/panels/ComposioPanel';
import EmbeddingsPanel from '../components/settings/panels/EmbeddingsPanel';
import SearchPanel from '../components/settings/panels/SearchPanel';
import VoicePanel from '../components/settings/panels/VoicePanel';
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
import BetaBanner from '../components/ui/BetaBanner';
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

/** Small inline icon helper for the Connections sidebar nav. */
const navIcon = (d: string) => (
  <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={d} />
  </svg>
);

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
      return 'text-content-faint';
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
      return 'text-content-faint';
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
                : 'border-line bg-surface hover:bg-surface-hover'
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
      <div className="relative flex h-12 w-12 flex-shrink-0 items-center justify-center text-content-secondary [&_img]:max-h-10 [&_img]:max-w-10 [&_svg]:h-8 [&_svg]:w-8">
        {meta.icon}
      </div>
      <div className="flex w-full min-w-0 flex-col items-center justify-start gap-0.5">
        <span className="line-clamp-2 text-[11px] font-semibold leading-tight text-content">
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
  /** Whether this channel is the current default messaging channel. */
  isDefault: boolean;
  /** Set this channel as the default. */
  onSetDefault: () => void;
  /** Test id for the "set as default" control (kept stable for E2E). */
  setDefaultTestId?: string;
  /** Disable the default control while a write is in flight. */
  setDefaultBusy?: boolean;
}

function ChannelTile({
  def,
  status,
  icon,
  testId,
  onOpen,
  isDefault,
  onSetDefault,
  setDefaultTestId,
  setDefaultBusy,
}: ChannelTileProps) {
  const { t } = useT();
  const isConnected = status === 'connected';
  const isPending = status === 'connecting';
  const isError = status === 'error';
  const statusLabel = channelStatusLabel(status, t);
  const ctaLabel = isConnected ? t('skills.configure') : t('channels.setup');

  // Horizontal tile: icon on the left; name → status → default control stacked
  // on the right. The tile is a container (not one button) so "configure" (the
  // icon + name row) and "set as default" stay distinct, focusable controls —
  // collapsing the old two-selector layout (connect grid + separate picker)
  // into one place. "Set as default" only appears for channels you can actually
  // route through (connected); the default badge still shows on whichever
  // channel is persisted as default, connected or not.
  const showDefaultControl = isDefault || isConnected;

  return (
    <div
      className={`group flex flex-col gap-2 rounded-2xl border p-3 transition-colors ${
        isConnected
          ? 'border-sage-300 bg-sage-50/80 shadow-[0_0_0_1px_rgba(34,197,94,0.12)] dark:border-sage-500/30 dark:bg-sage-500/10'
          : isPending
            ? 'border-amber-200 bg-amber-50/40 dark:border-amber-500/30 dark:bg-amber-500/10'
            : isError
              ? 'border-coral-200 bg-coral-50/30 dark:border-coral-500/30 dark:bg-coral-500/10'
              : 'border-line bg-surface'
      } ${
        // The default channel keeps its connection-status colour but gains a
        // primary ring so "which one is the default" reads at a glance without
        // masking whether it is connected.
        isDefault
          ? 'ring-2 ring-primary-400 ring-offset-1 ring-offset-white dark:ring-offset-neutral-900'
          : ''
      }`}>
      <button
        type="button"
        data-testid={testId}
        onClick={onOpen}
        title={`${def.display_name} — ${def.description}`}
        aria-label={`${def.display_name}, ${statusLabel}. ${ctaLabel}.`}
        className="flex w-full items-center gap-3 rounded-xl text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40">
        <div className="relative flex h-10 w-10 flex-shrink-0 items-center justify-center text-content-secondary [&>span]:h-10 [&>span]:w-10 [&>span]:rounded-2xl [&_svg]:h-6 [&_svg]:w-6">
          {icon}
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="line-clamp-2 text-xs font-semibold leading-tight text-content">
            {def.display_name}
          </span>
          <span className={`line-clamp-1 text-[11px] font-medium ${channelStatusColor(status)}`}>
            {statusLabel}
          </span>
        </div>
      </button>
      {showDefaultControl && (
        // Aligns under the name/status text (icon 2.5rem + gap 0.75rem).
        <div className="pl-[3.25rem]">
          {isDefault ? (
            <span
              data-testid={setDefaultTestId}
              className="inline-flex items-center justify-center gap-1 rounded-lg border border-primary-400/60 bg-primary-100/70 px-2.5 py-1 text-[11px] font-semibold text-primary-700 dark:border-primary-500/40 dark:bg-primary-500/15 dark:text-primary-200">
              <svg className="h-3 w-3" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
                <path
                  fillRule="evenodd"
                  d="M16.704 5.29a1 1 0 010 1.42l-7.5 7.5a1 1 0 01-1.42 0l-3.5-3.5a1 1 0 111.42-1.42l2.79 2.79 6.79-6.79a1 1 0 011.42 0z"
                  clipRule="evenodd"
                />
              </svg>
              {t('channels.defaultBadge')}
            </span>
          ) : (
            <button
              type="button"
              data-testid={setDefaultTestId}
              onClick={onSetDefault}
              disabled={setDefaultBusy}
              className="inline-flex items-center justify-center rounded-lg border border-line bg-surface/70 px-2.5 py-1 text-[11px] font-medium text-content-muted transition-colors hover:border-primary-300 hover:text-primary-600 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40 disabled:cursor-not-allowed disabled:opacity-50 dark:bg-surface/60 dark:hover:border-primary-500/40 dark:hover:text-primary-300">
              {t('channels.setAsDefault')}
            </button>
          )}
        </div>
      )}
    </div>
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
 *   mcp       → mcp
 *   skills    → skills (kept secondary)
 *
 * Back-compat: the old ?tab= values (composio, channels, mcp, meetings) are
 * normalised to the new values so existing deep links continue to work.
 */
type ConnectionsTab =
  | 'composio'
  | 'channels'
  | 'mcp'
  | 'skills'
  | 'meetings'
  | 'llm'
  | 'voice'
  | 'embeddings'
  | 'search'
  | 'composio-key';

/** Tabs that render a relocated settings panel (the "API keys" group). */
const INTELLIGENCE_TABS: ReadonlySet<ConnectionsTab> = new Set<ConnectionsTab>([
  'llm',
  'voice',
  'embeddings',
  'search',
  'composio-key',
]);

export default function Skills() {
  const { t } = useT();
  const channelIcons = useMemo(() => getChannelIcons(t), [t]);
  const location = useLocation();
  const navigate = useNavigate();
  const isLocalSession = isLocalSessionToken(getCoreStateSnapshot().snapshot.sessionToken);
  // Honour `?tab=<apps|messaging|mcp|skills>` so deep links land on the
  // right sub-tab.  Also normalise legacy tab names from the old /skills route
  // so that e.g. `/skills?tab=composio` still works after the redirect.
  const activeTab = useMemo<ConnectionsTab>(() => {
    const params = new URLSearchParams(location.search);
    const raw = params.get('tab');
    // New canonical values
    if (
      raw === 'composio' ||
      raw === 'channels' ||
      raw === 'mcp' ||
      raw === 'skills' ||
      raw === 'meetings' ||
      raw === 'llm' ||
      raw === 'voice' ||
      raw === 'embeddings' ||
      raw === 'search' ||
      raw === 'composio-key'
    )
      return raw;
    // Legacy back-compat aliases
    if (raw === 'apps') return 'composio';
    if (raw === 'messaging') return 'channels';
    if (raw === 'tools') return 'mcp';
    if (raw === 'talents') return 'meetings';
    if (raw === 'explorer') return 'skills';
    return 'composio';
  }, [location.search]);

  const handleTabChange = useCallback(
    (tab: ConnectionsTab) => {
      const params = new URLSearchParams(location.search);
      params.set('tab', tab);
      navigate({ pathname: location.pathname, search: `?${params.toString()}` });
    },
    [location.pathname, location.search, navigate]
  );
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
    // Default to an empty map so the component is resilient when a test
    // mock (or an older hook build) omits the dynamic-catalog field.
    catalogByToolkit: composioCatalogByToolkit = new Map(),
    connectionByToolkit: composioConnectionByToolkit,
    connectionsByToolkit: composioConnectionsByToolkit,
    loading: composioLoading,
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
    // Base-list selection (see COMPOSIO_DYNAMIC_CATALOG_PLAN.md / #3933):
    //  1. Dynamic catalog present → drive the grid straight off the backend.
    //  2. Still fetching (no catalog yet) → render NOTHING from the hardcoded
    //     list. The grid shows a loading skeleton instead. This is the fix for
    //     the "flash of stale hardcoded toolkits" that appeared before the
    //     backend catalog landed.
    //  3. Fetch finished with no catalog (a genuine failure, or an older core
    //     that predates the dynamic catalog) → fall back to the hardcoded
    //     KNOWN_COMPOSIO_TOOLKITS so the grid is never empty.
    const dynamicSlugs = Array.from(composioCatalogByToolkit.keys());
    const hasDynamicCatalog = dynamicSlugs.length > 0;
    let baseSlugs: readonly string[];
    let source: 'dynamic-backend' | 'loading' | 'hardcoded-fallback';
    if (hasDynamicCatalog) {
      baseSlugs = dynamicSlugs;
      source = 'dynamic-backend';
    } else if (composioLoading) {
      baseSlugs = [];
      source = 'loading';
    } else {
      baseSlugs = KNOWN_COMPOSIO_TOOLKITS;
      source = 'hardcoded-fallback';
    }

    if (IS_DEV) {
      const missingKnownToolkits = KNOWN_COMPOSIO_TOOLKITS.filter(
        slug => !normalizedToolkits.includes(slug)
      );
      console.debug('[skills][composio] building catalog', {
        source,
        dynamicCount: dynamicSlugs.length,
        toolkitCount: composioToolkits.length,
        connectionCount: composioConnectionByToolkit.size,
        loading: composioLoading,
        hasError: Boolean(composioError),
        missingKnownToolkits: source === 'hardcoded-fallback' ? missingKnownToolkits : [],
      });
    }

    // Union base slugs with enabled slugs and any connected toolkit so a
    // connection always renders even if it's missing from the catalog.
    return Array.from(new Set([...baseSlugs, ...normalizedToolkits])).sort((a, b) =>
      a.localeCompare(b)
    );
  }, [
    composioToolkits,
    composioCatalogByToolkit,
    composioConnectionByToolkit,
    composioLoading,
    composioError,
  ]);

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
      const canonical = canonicalizeComposioToolkitSlug(slug);
      const entry = composioCatalogByToolkit.get(canonical);
      const meta = composioToolkitMeta(slug, entry);
      const connection = composioConnectionByToolkit.get(meta.slug);
      entries.push({ meta, connection });
    }
    return entries;
  }, [composioCatalogToolkits, composioCatalogByToolkit, composioConnectionByToolkit]);

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
      className="rounded-2xl border border-line bg-surface p-3 shadow-soft animate-fade-up">
      <div className="px-1 pb-3 pt-1">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-content">
          <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-surface-subtle">
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
    <div className="h-full">
      {/* The Connections navigation lives in the root app sidebar's dynamic region. */}
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <TwoPaneNav
            ariaLabel={t('nav.connections')}
            selected={activeTab}
            onSelect={value => handleTabChange(value as ConnectionsTab)}
            groups={[
              {
                label: t('connections.groups.integrations'),
                items: [
                  {
                    value: 'composio',
                    label: t('connections.tabs.oauth'),
                    icon: navIcon('M13 10V3L4 14h7v7l9-11h-7z'),
                  },
                  {
                    value: 'channels',
                    label: t('connections.tabs.channels'),
                    icon: navIcon(
                      'M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z'
                    ),
                  },
                  {
                    value: 'mcp',
                    label: t('connections.tabs.mcp'),
                    icon: navIcon(
                      'M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z'
                    ),
                  },
                  {
                    value: 'skills',
                    label: t('connections.tabs.skills'),
                    icon: navIcon(
                      'M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664zM21 12a9 9 0 11-18 0 9 9 0 0118 0z'
                    ),
                  },
                  {
                    value: 'meetings',
                    label: t('connections.tabs.meetings'),
                    icon: navIcon(
                      'M15 10l4.553-2.276A1 1 0 0121 8.618v6.764a1 1 0 01-1.447.894L15 14M5 18h8a2 2 0 002-2V8a2 2 0 00-2-2H5a2 2 0 00-2 2v8a2 2 0 002 2z'
                    ),
                  },
                ],
              },
              {
                label: t('connections.groups.apiKeys'),
                items: [
                  {
                    value: 'llm',
                    label: t('pages.settings.ai.llm'),
                    icon: navIcon(
                      'M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z'
                    ),
                  },
                  {
                    value: 'composio-key',
                    label: t('connections.tabs.composioKey'),
                    icon: navIcon(
                      'M15 7a2 2 0 012 2m4-2a6 6 0 01-7.743 5.743L11 14H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z'
                    ),
                  },
                  {
                    value: 'voice',
                    label: t('pages.settings.ai.voice'),
                    icon: navIcon(
                      'M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z'
                    ),
                  },
                  {
                    value: 'embeddings',
                    label: t('pages.settings.ai.embeddings'),
                    icon: navIcon(
                      'M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4'
                    ),
                  },
                  {
                    value: 'search',
                    label: t('settings.search.title'),
                    icon: navIcon('M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z'),
                  },
                ],
              },
            ]}
          />
        </div>
      </SidebarContent>
      <div className="mx-auto h-full w-full max-w-5xl">
        {/* Intelligence panels relocated from Settings are themselves PanelPage
            panels (description, no title; the back button hides because the
            Connections sidebar owns navigation), so they fill the content pane
            and own their scroll directly. */}
        {INTELLIGENCE_TABS.has(activeTab) ? (
          // API-keys / provider panels were orphaned flush on the shell — give
          // them a card surface (the integrations/skills grids below already
          // have their own card layouts, so they stay flush).
          <div className="h-full p-4">
            <div className="h-full overflow-hidden rounded-2xl border border-line bg-surface shadow-soft">
              <SettingsLayoutProvider value={{ inTwoPaneShell: true }}>
                {activeTab === 'llm' && <AIPanel />}
                {activeTab === 'voice' && <VoicePanel />}
                {activeTab === 'embeddings' && <EmbeddingsPanel />}
                {activeTab === 'search' && <SearchPanel />}
                {activeTab === 'composio-key' && <ComposioPanel />}
              </SettingsLayoutProvider>
            </div>
          </div>
        ) : (
          <PanelPage contentClassName="p-4">
            <div className="mx-auto w-full max-w-3xl space-y-4">
              {/* <div className="flex items-center justify-between gap-2">
              <div className="min-w-0">
                <h1 className="text-base font-semibold text-content">
                  Skills
                </h1>
                <p className="text-xs text-content-muted">
                  Scaffold a new <code className="font-mono">SKILL.md</code> or install a published
                  package.
                </p>
              </div>
              <div className="flex flex-shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={() => setInstallDialogOpen(true)}
                  className="rounded-lg border border-line bg-surface px-3 py-2 text-xs font-medium text-content-secondary shadow-soft transition-colors hover:bg-surface-hover focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
                  Install from URL
                </button>
                <button
                  type="button"
                  onClick={() => setCreateModalOpen(true)}
                  className="rounded-lg bg-primary-500 px-3 py-2 text-xs font-semibold text-content-inverted shadow-soft transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
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
                      className="flex-shrink-0 rounded-lg border border-amber-300 dark:border-amber-500/40 bg-surface px-3 py-1.5 text-[11px] font-medium text-amber-800 dark:text-amber-300 transition-colors hover:bg-amber-100 dark:hover:bg-amber-500/10">
                      {t('common.retry')}
                    </button>
                  </div>
                </div>
              )}

              {
                <>
                  {activeTab === 'channels' && channelsGroup && (
                    <div className="rounded-2xl border border-line bg-surface p-3 shadow-soft animate-fade-up">
                      <div className="px-1 pb-3 pt-1">
                        <h2
                          className="flex items-center gap-2 text-sm font-semibold text-content"
                          data-walkthrough="skills-channels">
                          <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-surface-subtle">
                            <SkillCategoryIcon
                              category="Channels"
                              className={skillCategoryHeadingClassName('Channels')}
                            />
                          </span>
                          {t('skills.channels')}
                        </h2>
                        <p className="mt-0.5 text-[11px] leading-relaxed text-content-muted">
                          {t('channels.defaultMessaging')}
                        </p>
                      </div>
                      {/* One unified surface: each tile shows connection status,
                          opens setup/configure on click, and owns the "default
                          messaging channel" selection via its footer control.
                          Connected channels and not-yet-connected channels are
                          rendered as two separate grids (no divider/label) so
                          each group occupies its own rows. */}
                      {(() => {
                        // The built-in web channel needs no connection — treat it
                        // as always available so it stays selectable as default.
                        const statusFor = (def: ChannelDefinition): ChannelConnectionStatus =>
                          def.id === 'web' ? 'connected' : bestChannelStatus(def.id as ChannelType);
                        const renderTile = (def: ChannelDefinition) => {
                          const channelId = def.id as ChannelType;
                          return (
                            <div key={channelId} data-testid={`skill-row-channel-${channelId}`}>
                              <ChannelTile
                                def={def}
                                status={statusFor(def)}
                                icon={channelIcons[def.icon]}
                                testId={`skill-install-channel-${channelId}`}
                                onOpen={() => setChannelModalDef(def)}
                                isDefault={channelConnections.defaultMessagingChannel === channelId}
                                onSetDefault={() => void handleSetDefaultChannel(channelId)}
                                setDefaultTestId={`channel-select-${channelId}`}
                                setDefaultBusy={defaultChannelBusy !== null}
                              />
                            </div>
                          );
                        };
                        const connected = channelDefs.filter(d => statusFor(d) === 'connected');
                        const notConnected = channelDefs.filter(d => statusFor(d) !== 'connected');
                        const gridStyle = {
                          gridTemplateColumns: 'repeat(auto-fill, minmax(13rem, 1fr))',
                        };
                        return (
                          <div className="space-y-2 sm:space-y-3">
                            {connected.length > 0 && (
                              <div className="grid gap-2 sm:gap-3" style={gridStyle}>
                                {connected.map(renderTile)}
                              </div>
                            )}
                            {notConnected.length > 0 && (
                              <div className="grid gap-2 sm:gap-3" style={gridStyle}>
                                {notConnected.map(renderTile)}
                              </div>
                            )}
                          </div>
                        );
                      })()}
                    </div>
                  )}

                  {activeTab === 'composio' && (
                    <div
                      className="rounded-2xl border border-line bg-surface p-3 shadow-soft animate-fade-up"
                      data-walkthrough="skills-grid"
                      data-testid="composio-integrations-card">
                      <p className="px-1 pb-3 text-xs leading-relaxed text-content-muted">
                        {t('skills.integrationsSubtitle')}
                      </p>
                      {showLocalComposioApiKeyBanner && (
                        <ComposioApiKeyEmptyState
                          onOpenSettings={() => handleTabChange('composio-key')}
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
                        // While the dynamic catalog is still being fetched and we
                        // have nothing real to show yet, render a loading skeleton
                        // instead of the hardcoded toolkit list. The hardcoded
                        // KNOWN_COMPOSIO_TOOLKITS list is only used as a post-fetch
                        // fallback (see composioCatalogToolkits), never during the
                        // in-flight loading window (#3933).
                        (composioLoading && composioSortedEntries.length === 0 ? (
                          <div
                            className="grid gap-2 sm:gap-3"
                            data-testid="composio-integrations-loading"
                            role="status"
                            aria-label={t('skills.loadingIntegrations')}
                            aria-busy="true"
                            style={{
                              gridTemplateColumns: 'repeat(auto-fill, minmax(5.5rem, 1fr))',
                              gridAutoRows: '6.5rem',
                            }}>
                            {Array.from({ length: 12 }).map((_, i) => (
                              <div
                                key={i}
                                data-testid="composio-skeleton-tile"
                                aria-hidden="true"
                                className="animate-pulse rounded-xl border border-line bg-surface-subtle"
                              />
                            ))}
                          </div>
                        ) : composioSortedEntries.length > 0 ? (
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
                          <p className="px-1 py-4 text-center text-xs text-content-faint">
                            {t('skills.noResults')}
                          </p>
                        ))}
                    </div>
                  )}

                  {activeTab === 'composio' && otherGroups.map(group => renderGroup(group))}

                  {activeTab === 'skills' && (
                    <div className="space-y-3 animate-fade-up">
                      <BetaBanner />
                      <SkillsExplorerTab onToast={addToast} />
                    </div>
                  )}

                  {activeTab === 'mcp' && (
                    <div className="space-y-3 animate-fade-up">
                      <BetaBanner />
                      <div className="rounded-2xl border border-line bg-surface p-4 shadow-soft">
                        <McpServersTab />
                      </div>
                    </div>
                  )}

                  {activeTab === 'meetings' && (
                    <div className="space-y-3 animate-fade-up">
                      <BetaBanner />
                      <MeetingBotsCard onToast={addToast} />
                    </div>
                  )}
                </>
              }
            </div>
          </PanelPage>
        )}
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
