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
import PageSectionHeader from '../components/layout/PageSectionHeader';
import PageWelcome from '../components/layout/PageWelcome';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../components/layout/TwoPaneNav';
import { SettingsLayoutProvider } from '../components/settings/layout/SettingsLayoutContext';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import ComposioPanel from '../components/settings/panels/ComposioPanel';
import EmbeddingsPanel from '../components/settings/panels/EmbeddingsPanel';
import LlmConnectionsPanel from '../components/settings/panels/LlmConnectionsPanel';
import SearchPanel from '../components/settings/panels/SearchPanel';
import UsagePanel from '../components/settings/panels/UsagePanel';
import VoiceConnectionsPanel from '../components/settings/panels/VoiceConnectionsPanel';
import WalletPanel from '../components/settings/panels/WalletPanel';
import UnifiedSkillCard from '../components/skills/SkillCard';
import {
  SKILL_CATEGORY_LABEL_KEYS,
  SKILL_CATEGORY_ORDER,
  type SkillCategory,
} from '../components/skills/skillCategories';
import SkillCategoryFilter from '../components/skills/SkillCategoryFilter';
import {
  getChannelIcons,
  skillCategoryHeadingClassName,
  SkillCategoryIcon,
} from '../components/skills/skillIcons';
import SkillSearchBar from '../components/skills/SkillSearchBar';
import SkillsExplorerTab from '../components/skills/SkillsExplorerTab';
import VoiceSetupModal from '../components/skills/VoiceSetupModal';
import Badge from '../components/ui/Badge';
import BetaIndicator from '../components/ui/BetaIndicator';
import Button from '../components/ui/Button';
import Card from '../components/ui/Card';
import { useVoiceSkillStatus } from '../features/voice/useVoiceSkillStatus';
import { useChannelDefinitions } from '../hooks/useChannelDefinitions';
import { useAgentReadyComposioToolkits, useComposioIntegrations } from '../lib/composio/hooks';
import { canonicalizeComposioToolkitSlug } from '../lib/composio/toolkitSlug';
import { type ComposioConnection, deriveComposioState } from '../lib/composio/types';
import { getCoreStateSnapshot } from '../lib/coreState/store';
import { useT } from '../lib/i18n/I18nContext';
import { classifyIntegrationError } from '../lib/userErrors/classify';
import { channelConnectionsApi } from '../services/api/channelConnectionsApi';
import { setDefaultMessagingChannel } from '../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { dismissUserError, reportUserError } from '../store/userErrorsSlice';
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
    <Button
      type="button"
      variant="tertiary"
      size="md"
      data-testid={testId}
      onClick={handleClick}
      title={`${meta.name} — ${isPreview ? t('composio.previewTooltip') : meta.description}`}
      aria-label={`${meta.name}, ${statusLabel}. ${ctaLabel}.`}
      className={`group relative h-full w-full flex-col justify-center rounded-xl border p-3 text-center ${
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
          className="absolute right-1.5 top-1.5 max-w-18 truncate rounded-full border border-amber-200 bg-amber-100 px-1.5 py-0.5 text-[9px] font-semibold uppercase leading-none text-amber-800 dark:border-amber-500/40 dark:bg-amber-500/15 dark:text-amber-200"
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
      <div className="relative flex h-12 w-12 shrink-0 items-center justify-center text-content-secondary [&_img]:max-h-10 [&_img]:max-w-10 [&_svg]:h-8 [&_svg]:w-8">
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
    </Button>
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
      <Button
        type="button"
        variant="tertiary"
        size="md"
        data-testid={testId}
        onClick={onOpen}
        title={`${def.display_name} — ${def.description}`}
        aria-label={`${def.display_name}, ${statusLabel}. ${ctaLabel}.`}
        className="h-auto w-full justify-start gap-3 rounded-xl p-0 text-left">
        <div className="relative flex h-10 w-10 shrink-0 items-center justify-center text-content-secondary [&>span]:h-10 [&>span]:w-10 [&>span]:rounded-2xl [&_svg]:h-6 [&_svg]:w-6">
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
      </Button>
      {showDefaultControl && (
        // Aligns under the name/status text (icon 2.5rem + gap 0.75rem).
        <div className="pl-13">
          {isDefault ? (
            <Badge
              data-testid={setDefaultTestId}
              variant="primary"
              className="gap-1 border-primary-400/60 bg-primary-100/70 text-primary-700 dark:border-primary-500/40 dark:bg-primary-500/15 dark:text-primary-200">
              <svg className="h-3 w-3" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
                <path
                  fillRule="evenodd"
                  d="M16.704 5.29a1 1 0 010 1.42l-7.5 7.5a1 1 0 01-1.42 0l-3.5-3.5a1 1 0 111.42-1.42l2.79 2.79 6.79-6.79a1 1 0 011.42 0z"
                  clipRule="evenodd"
                />
              </svg>
              {t('channels.defaultBadge')}
            </Badge>
          ) : (
            <Button
              type="button"
              variant="secondary"
              size="xs"
              data-testid={setDefaultTestId}
              onClick={onSetDefault}
              disabled={setDefaultBusy}
              className="text-content-muted hover:border-primary-300 hover:text-primary-600 dark:hover:border-primary-500/40 dark:hover:text-primary-300">
              {t('channels.setAsDefault')}
            </Button>
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
}> = [];

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
 * Back-compat: the old ?tab= values (composio, channels, mcp) are
 * normalised to the new values so existing deep links continue to work.
 */
type ConnectionsTab =
  | 'welcome'
  | 'composio'
  | 'channels'
  | 'mcp'
  | 'skills'
  | 'llm'
  | 'voice'
  | 'embeddings'
  | 'search'
  | 'usage'
  | 'composio-key'
  | 'wallet';

/**
 * Tabs that render a relocated settings panel inside the shared card surface.
 * Covers the "API keys" group plus the Desktop capability panels and the wallet
 * balances panel relocated from Settings.
 */
/** Canonical text header (title + one-line description) per functional tab. */
const CONNECTIONS_HEADERS: Partial<Record<ConnectionsTab, { titleKey: string; descKey: string }>> =
  {
    composio: { titleKey: 'connections.tabs.oauth', descKey: 'connections.header.composio' },
    channels: { titleKey: 'connections.tabs.channels', descKey: 'connections.header.channels' },
    mcp: { titleKey: 'connections.tabs.mcp', descKey: 'connections.header.mcp' },
    skills: { titleKey: 'connections.tabs.skills', descKey: 'skills.explorer.subtitle' },
  };

/** Canonical header (title + description) for each relocated settings panel. */
const INTELLIGENCE_HEADERS: Partial<Record<ConnectionsTab, { titleKey: string; descKey: string }>> =
  {
    'composio-key': {
      titleKey: 'connections.tabs.composioKey',
      descKey: 'connections.header.composioKey',
    },
    embeddings: {
      titleKey: 'pages.settings.ai.embeddings',
      descKey: 'connections.header.embeddings',
    },
    search: { titleKey: 'settings.search.title', descKey: 'connections.header.search' },
    usage: { titleKey: 'settings.usage.title', descKey: 'settings.usage.menuDesc' },
  };

/** Intelligence tabs whose panel renders its own header card (with chip tabs in
 *  it), so the Connections pane skips the shared header + card wrapper. */
const SELF_HEADER_TABS: ReadonlySet<ConnectionsTab> = new Set<ConnectionsTab>([
  'llm',
  'voice',
  'embeddings',
  'search',
  'composio-key',
  'usage',
  'wallet',
]);

const INTELLIGENCE_TABS: ReadonlySet<ConnectionsTab> = new Set<ConnectionsTab>([
  'llm',
  'voice',
  'embeddings',
  'search',
  'usage',
  'composio-key',
  'wallet',
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
      raw === 'welcome' ||
      raw === 'composio' ||
      raw === 'channels' ||
      raw === 'mcp' ||
      raw === 'skills' ||
      raw === 'llm' ||
      raw === 'voice' ||
      raw === 'embeddings' ||
      raw === 'search' ||
      raw === 'usage' ||
      raw === 'composio-key' ||
      raw === 'wallet'
    )
      return raw;
    // Legacy back-compat aliases
    if (raw === 'apps') return 'composio';
    if (raw === 'messaging') return 'channels';
    if (raw === 'tools') return 'mcp';
    if (raw === 'explorer') return 'skills';
    // Default landing is the Welcome overview for the Connections page.
    return 'welcome';
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
  const [voiceModalOpen, setVoiceModalOpen] = useState(false);
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

  // Raise a Composio outage as an app-wide notice rather than an alert on this
  // page. The state is about the user's account, not about this screen: while
  // it is set, every tool-calling surface in the app is degraded, yet only a
  // visitor to Connections used to be told. `NoticeCenter` keeps it visible on
  // whatever route they are on, and the entry survives navigating away.
  //
  // The per-card `hasComposioError` degradation below stays — that IS local,
  // and it is what marks *which* cards are unreliable.
  useEffect(() => {
    const descriptor = classifyIntegrationError(composioError, 'composio');
    if (!descriptor) return;
    dispatch(reportUserError({ descriptor, at: Date.now() }));
  }, [composioError, dispatch]);

  // Clear it the moment the poll succeeds, so a recovered integration does not
  // leave a stale warning the user has to dismiss by hand.
  useEffect(() => {
    if (composioError) return;
    dispatch(dismissUserError({ id: 'integration_degraded:integration:composio' }));
  }, [composioError, dispatch]);

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
    <Card key={category} className="animate-fade-up">
      <div className="p-3">
        <div className="px-1 pb-3 pt-1">
          <h2 className="flex items-center gap-2 text-sm font-semibold text-content">
            <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-surface-subtle">
              <SkillCategoryIcon
                category={category}
                className={skillCategoryHeadingClassName(category)}
              />
            </span>
            {t(SKILL_CATEGORY_LABEL_KEYS[category])}
          </h2>
        </div>
        <div className="space-y-2">
          {items.map(item => {
            if (item.kind === 'builtin') {
              /* v8 ignore start -- BUILT_IN_SKILLS list is empty today; the per-id
               branches below are retained for the hidden voice skill without
               dragging the diff-coverage gate down while unreachable. */
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
    </Card>
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
                    // Wallet balances relocated from Settings → Data.
                    value: 'wallet',
                    label: t('pages.settings.account.walletBalances'),
                    icon: navIcon(
                      'M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z'
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
                  {
                    // Usage & limits (cost dashboard, token savings, background
                    // loops) relocated from Settings.
                    value: 'usage',
                    label: t('settings.usage.title'),
                    icon: navIcon(
                      'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z'
                    ),
                  },
                ],
              },
            ]}
          />
        </div>
      </SidebarContent>
      {/* The content pane is no longer capped at a centered 3xl column.
          Views that read best as a narrow column still center themselves
          (PageWelcome by design, the skills/integrations branch below via
          its own wrapper); the panel views are free to use the width. */}
      <div className="h-full w-full">
        {/* Welcome landing — the first destination; pitches the page and offers
            a few immediate actions into the functional views. */}
        {activeTab === 'welcome' ? (
          <PageWelcome
            testId="connections-welcome"
            accent="ocean"
            icon="🔗"
            eyebrow={t('connections.welcome.eyebrow')}
            title={t('connections.welcome.title')}
            description={t('connections.welcome.body')}
            ctas={[
              {
                label: t('connections.welcome.ctaChannel'),
                icon: '🔌',
                onClick: () => handleTabChange('channels'),
                testId: 'connections-welcome-cta-channel',
              },
              {
                label: t('connections.welcome.ctaApps'),
                icon: '🧩',
                onClick: () => handleTabChange('composio'),
              },
              {
                label: t('connections.welcome.ctaSkills'),
                icon: '✨',
                onClick: () => handleTabChange('skills'),
              },
            ]}
            featuresHeading={t('connections.welcome.featsLabel')}
            features={[
              {
                icon: '📥',
                title: t('connections.welcome.feat1Title'),
                description: t('connections.welcome.feat1Body'),
              },
              {
                icon: '🤖',
                title: t('connections.welcome.feat2Title'),
                description: t('connections.welcome.feat2Body'),
              },
              {
                icon: '🔒',
                title: t('connections.welcome.feat3Title'),
                description: t('connections.welcome.feat3Body'),
              },
            ]}
          />
        ) : /* Intelligence panels relocated from Settings are themselves PanelPage
            panels (description, no title; the back button hides because the
            Connections sidebar owns navigation), so they fill the content pane
            and own their scroll directly. */
        INTELLIGENCE_TABS.has(activeTab) ? (
          // API-keys / provider panels were orphaned flush on the shell — give
          // them a card surface (the integrations/skills grids below already
          // have their own card layouts, so they stay flush).
          <div className="flex h-full flex-col gap-4 p-4">
            {/* Panels render headerless (the header card owns the title +
                description) to avoid a doubled header in the Connections pane.
                LLM and Wallet render their OWN header card (with chip tabs in
                it), so they skip the Connections-level header + card wrapper. */}
            {SELF_HEADER_TABS.has(activeTab) ? (
              <SettingsLayoutProvider value={{ inTwoPaneShell: true, headerless: true }}>
                {activeTab === 'llm' && <LlmConnectionsPanel />}
                {activeTab === 'voice' && <VoiceConnectionsPanel />}
                {activeTab === 'embeddings' && (
                  <SettingsTabbedPage
                    title={t('pages.settings.ai.embeddings')}
                    description={t('connections.header.embeddings')}>
                    <EmbeddingsPanel embedded />
                  </SettingsTabbedPage>
                )}
                {activeTab === 'search' && (
                  <SettingsTabbedPage
                    title={t('settings.search.title')}
                    description={t('connections.header.search')}>
                    <SearchPanel embedded />
                  </SettingsTabbedPage>
                )}
                {activeTab === 'composio-key' && (
                  <SettingsTabbedPage
                    title={t('connections.tabs.composioKey')}
                    description={t('connections.header.composioKey')}>
                    <ComposioPanel embedded />
                  </SettingsTabbedPage>
                )}
                {activeTab === 'usage' && (
                  <SettingsTabbedPage
                    title={t('settings.usage.title')}
                    description={t('settings.usage.menuDesc')}>
                    <UsagePanel />
                  </SettingsTabbedPage>
                )}
                {activeTab === 'wallet' && <WalletPanel />}
              </SettingsLayoutProvider>
            ) : (
              <>
                {INTELLIGENCE_HEADERS[activeTab] && (
                  <PageSectionHeader
                    title={t(INTELLIGENCE_HEADERS[activeTab]!.titleKey)}
                    description={t(INTELLIGENCE_HEADERS[activeTab]!.descKey)}
                  />
                )}
                <div className="min-h-0 flex-1 overflow-hidden rounded-2xl border border-line bg-surface shadow-soft">
                  <SettingsLayoutProvider
                    value={{ inTwoPaneShell: true, headerless: true }}></SettingsLayoutProvider>
                </div>
              </>
            )}
          </div>
        ) : (
          <div className="h-full p-4">
            <SettingsTabbedPage
              title={t(CONNECTIONS_HEADERS[activeTab]!.titleKey)}
              description={t(CONNECTIONS_HEADERS[activeTab]!.descKey)}
              headerAction={
                activeTab === 'skills' || activeTab === 'mcp' ? <BetaIndicator /> : undefined
              }
              scrollable={activeTab !== 'skills'}>
              <div
                className={activeTab === 'skills' ? 'h-full min-h-0 w-full' : 'w-full space-y-4'}>
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
              <div className="flex shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={() => setInstallDialogOpen(true)}
                  className="rounded-lg border border-line bg-surface px-3 py-2 text-xs font-medium text-content-secondary shadow-soft transition-colors hover:bg-surface-hover focus:outline-hidden focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
                  Install from URL
                </button>
                <button
                  type="button"
                  onClick={() => setCreateModalOpen(true)}
                  className="rounded-lg bg-primary-500 px-3 py-2 text-xs font-semibold text-content-inverted shadow-soft transition-colors hover:bg-primary-600 focus:outline-hidden focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
                  New skill
                </button>
              </div>
            </div> */}

                {
                  <>
                    {activeTab === 'channels' && channelsGroup && (
                      <Card className="animate-fade-up">
                        <div className="p-3">
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
                            <p className="mt-1 text-[11px] leading-relaxed text-content-faint">
                              {t('channels.connectHelp.slackNote')}
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
                              def.id === 'web'
                                ? 'connected'
                                : bestChannelStatus(def.id as ChannelType);
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
                                    isDefault={
                                      channelConnections.defaultMessagingChannel === channelId
                                    }
                                    onSetDefault={() => void handleSetDefaultChannel(channelId)}
                                    setDefaultTestId={`channel-select-${channelId}`}
                                    setDefaultBusy={defaultChannelBusy !== null}
                                  />
                                </div>
                              );
                            };
                            const connected = channelDefs.filter(d => statusFor(d) === 'connected');
                            const notConnected = channelDefs.filter(
                              d => statusFor(d) !== 'connected'
                            );
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
                      </Card>
                    )}

                    {activeTab === 'composio' && (
                      <Card className="animate-fade-up" data-testid="composio-integrations-card">
                        <div className="p-3" data-walkthrough="skills-grid">
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
                      </Card>
                    )}

                    {activeTab === 'composio' && otherGroups.map(group => renderGroup(group))}

                    {activeTab === 'skills' && (
                      <div className="flex h-full min-h-0 flex-col gap-3 animate-fade-up">
                        <div className="min-h-0 flex-1">
                          <SkillsExplorerTab onToast={addToast} />
                        </div>
                      </div>
                    )}

                    {activeTab === 'mcp' && (
                      <div className="space-y-3 animate-fade-up">
                        <Card>
                          <div className="p-4">
                            <McpServersTab />
                          </div>
                        </Card>
                      </div>
                    )}
                  </>
                }
              </div>
            </SettingsTabbedPage>
          </div>
        )}
      </div>

      {channelModalDef && (
        <ChannelSetupModal definition={channelModalDef} onClose={() => setChannelModalDef(null)} />
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
