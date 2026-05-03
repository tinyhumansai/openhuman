import { useCallback, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import ChannelSetupModal from '../components/channels/ChannelSetupModal';
import ComposioConnectModal from '../components/composio/ComposioConnectModal';
import {
  composioToolkitMeta,
  type ComposioToolkitMeta,
  KNOWN_COMPOSIO_TOOLKITS,
} from '../components/composio/toolkitMeta';
import { ToastContainer } from '../components/intelligence/Toast';
import AutocompleteSetupModal from '../components/skills/AutocompleteSetupModal';
import CreateSkillModal from '../components/skills/CreateSkillModal';
import InstallSkillDialog from '../components/skills/InstallSkillDialog';
import ScreenIntelligenceSetupModal from '../components/skills/ScreenIntelligenceSetupModal';
import UnifiedSkillCard from '../components/skills/SkillCard';
import { SKILL_CATEGORY_ORDER, type SkillCategory } from '../components/skills/skillCategories';
import SkillCategoryFilter from '../components/skills/SkillCategoryFilter';
import SkillDetailDrawer from '../components/skills/SkillDetailDrawer';
import {
  BUILT_IN_SKILL_ICONS,
  CHANNEL_ICONS,
  skillCategoryHeadingClassName,
  SkillCategoryIcon,
} from '../components/skills/skillIcons';
import SkillSearchBar from '../components/skills/SkillSearchBar';
import UninstallSkillConfirmDialog from '../components/skills/UninstallSkillConfirmDialog';
import VoiceSetupModal from '../components/skills/VoiceSetupModal';
import { useAutocompleteSkillStatus } from '../features/autocomplete/useAutocompleteSkillStatus';
import { useScreenIntelligenceSkillStatus } from '../features/screen-intelligence/useScreenIntelligenceSkillStatus';
import { useVoiceSkillStatus } from '../features/voice/useVoiceSkillStatus';
import { useChannelDefinitions } from '../hooks/useChannelDefinitions';
import { useComposioIntegrations } from '../lib/composio/hooks';
import { canonicalizeComposioToolkitSlug } from '../lib/composio/toolkitSlug';
import { type ComposioConnection, deriveComposioState } from '../lib/composio/types';
import { skillsApi, type SkillSummary } from '../services/api/skillsApi';
import { useAppSelector } from '../store/hooks';
import type { ChannelConnectionStatus, ChannelDefinition, ChannelType } from '../types/channels';
import type { ToastNotification } from '../types/intelligence';
import { IS_DEV } from '../utils/config';
import { subconsciousEscalationsDismiss } from '../utils/tauriCommands';

function channelStatusLabel(status: ChannelConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'Connected';
    case 'connecting':
      return 'Connecting';
    case 'error':
      return 'Error';
    default:
      return 'Not configured';
  }
}

function channelStatusColor(status: ChannelConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'text-sage-600';
    case 'connecting':
      return 'text-amber-600';
    case 'error':
      return 'text-coral-600';
    default:
      return 'text-stone-400';
  }
}

// ─── Composio visual mappers ─────────────────────────────────────────────
// Reuse the same dot/label/color vocabulary as the channel cards so the
// "Integrations" section sits visually flush with the rest of the grid.

function composioStatusLabel(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'Connected';
    case 'pending':
      return 'Connecting';
    case 'error':
      return 'Error';
    default:
      return '';
  }
}

function composioStatusColor(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'text-sage-600';
    case 'pending':
      return 'text-amber-600';
    case 'error':
      return 'text-coral-600';
    default:
      return 'text-stone-400';
  }
}

/** Sort order for the integrations grid: connected first, then pending, errors, disconnected. */
function composioSortRank(connection: ComposioConnection | undefined): number {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 0;
    case 'pending':
      return 1;
    case 'error':
      return 2;
    default:
      return 3;
  }
}

interface ComposioConnectorTileProps {
  meta: ComposioToolkitMeta;
  connection: ComposioConnection | undefined;
  hasComposioError: boolean;
  onOpen: () => void;
  onRetryGlobal: () => void;
}

function ComposioConnectorTile({
  meta,
  connection,
  hasComposioError,
  onOpen,
  onRetryGlobal,
}: ComposioConnectorTileProps) {
  const state = hasComposioError ? 'error' : deriveComposioState(connection);
  const statusLabel = hasComposioError ? 'Status unavailable' : composioStatusLabel(connection);
  const ctaLabel = hasComposioError
    ? 'Retry'
    : state === 'connected'
      ? 'Manage'
      : state === 'pending'
        ? 'Waiting'
        : state === 'error'
          ? 'Retry'
          : 'Connect';

  const isConnected = state === 'connected';
  const isPending = state === 'pending';
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
      onClick={handleClick}
      title={`${meta.name} — ${meta.description}`}
      aria-label={`${meta.name}, ${statusLabel}. ${ctaLabel}.`}
      className={`group flex flex-col justify-center items-center rounded-2xl border p-3 text-center transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40 ${
        isConnected
          ? 'border-sage-300 bg-sage-50/80 shadow-[0_0_0_1px_rgba(34,197,94,0.12)] hover:bg-sage-50'
          : isPending
            ? 'border-amber-200 bg-amber-50/40 hover:bg-amber-50/70'
            : isError
              ? 'border-coral-200 bg-coral-50/30 hover:bg-coral-50/50'
              : 'border-stone-200 bg-white hover:bg-stone-50'
      }`}>
      <div className="relative flex h-12 w-12 flex-shrink-0 items-center justify-center text-stone-700 [&_img]:max-h-10 [&_img]:max-w-10 [&_svg]:h-8 [&_svg]:w-8">
        {meta.icon}
      </div>
      <div className="flex w-full min-w-0 flex-col items-center justify-start gap-0.5">
        <span className="line-clamp-2 text-[11px] font-semibold leading-tight text-stone-900">
          {meta.name}
        </span>
        <span
          className={`line-clamp-1 text-[10px] font-medium ${
            hasComposioError ? 'text-amber-700' : composioStatusColor(connection)
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
  onOpen: () => void;
}

function ChannelTile({ def, status, icon, onOpen }: ChannelTileProps) {
  const isConnected = status === 'connected';
  const isPending = status === 'connecting';
  const isError = status === 'error';
  const statusLabel = channelStatusLabel(status);
  const ctaLabel = isConnected ? 'Manage' : 'Setup';

  return (
    <button
      type="button"
      onClick={onOpen}
      title={`${def.display_name} — ${def.description}`}
      aria-label={`${def.display_name}, ${statusLabel}. ${ctaLabel}.`}
      className={`group flex flex-col items-center gap-2 rounded-2xl border p-3 pb-3 text-center transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40 ${
        isConnected
          ? 'border-sage-300 bg-sage-50/80 shadow-[0_0_0_1px_rgba(34,197,94,0.12)] hover:bg-sage-50'
          : isPending
            ? 'border-amber-200 bg-amber-50/40 hover:bg-amber-50/70'
            : isError
              ? 'border-coral-200 bg-coral-50/30 hover:bg-coral-50/50'
              : 'border-stone-200 bg-white hover:bg-stone-50'
      }`}>
      <div className="relative flex h-12 w-12 flex-shrink-0 items-center justify-center text-stone-700 [&>span]:h-12 [&>span]:w-12 [&>span]:rounded-2xl [&_svg]:h-7 [&_svg]:w-7">
        {icon}
      </div>
      <div className="flex min-h-[2.5rem] w-full min-w-0 flex-col items-center justify-start gap-0.5">
        <span className="line-clamp-2 text-[11px] font-semibold leading-tight text-stone-900">
          {def.display_name}
        </span>
        <span className={`line-clamp-1 text-[10px] font-medium ${channelStatusColor(status)}`}>
          {statusLabel}
        </span>
      </div>
    </button>
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
  kind: 'builtin' | 'channel' | 'discovered';
  // For built-in
  route?: string;
  icon?: React.ReactNode;
  // For channel
  channelDef?: ChannelDefinition;
  channelStatus?: ChannelConnectionStatus;
  // For discovered SKILL.md skills
  discoveredSkill?: SkillSummary;
}

// ─── Main Skills Page ──────────────────────────────────────────────────────────

export default function Skills() {
  const location = useLocation();
  const navigate = useNavigate();
  const { definitions: channelDefs } = useChannelDefinitions();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const {
    toolkits: composioToolkits,
    connectionByToolkit: composioConnectionByToolkit,
    error: composioError,
    refresh: refreshComposio,
  } = useComposioIntegrations();

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

  const [searchQuery, setSearchQuery] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<SkillCategory>('All');
  const [discoveredSkills, setDiscoveredSkills] = useState<SkillSummary[]>([]);
  const [selectedSkill, setSelectedSkill] = useState<SkillSummary | null>(null);
  const [createModalOpen, setCreateModalOpen] = useState(false);
  const [installDialogOpen, setInstallDialogOpen] = useState(false);
  const [uninstallCandidate, setUninstallCandidate] = useState<SkillSummary | null>(null);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);
  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    const newToast: ToastNotification = { ...toast, id: `toast-${Date.now()}-${Math.random()}` };
    setToasts(prev => [...prev, newToast]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);
  const pendingEscalationId =
    location.state &&
    typeof location.state === 'object' &&
    'subconsciousEscalationId' in location.state &&
    typeof location.state.subconsciousEscalationId === 'string'
      ? location.state.subconsciousEscalationId
      : null;

  const clearPendingEscalationState = useCallback(() => {
    navigate(location.pathname, { replace: true, state: null });
  }, [location.pathname, navigate]);

  const dismissPendingEscalationIfResolved = useCallback(
    async (resolution: string) => {
      if (!pendingEscalationId) return;
      console.debug('[skills][subconscious] dismiss escalation:start', {
        escalationId: pendingEscalationId,
        resolution,
      });
      try {
        await subconsciousEscalationsDismiss(pendingEscalationId);
        console.debug('[skills][subconscious] dismiss escalation:success', {
          escalationId: pendingEscalationId,
          resolution,
        });
      } catch (error) {
        console.debug('[skills][subconscious] dismiss escalation:error', {
          escalationId: pendingEscalationId,
          resolution,
          error: error instanceof Error ? error.message : String(error),
        });
        return;
      }
      clearPendingEscalationState();
    },
    [clearPendingEscalationState, pendingEscalationId]
  );

  // Discover SKILL.md skills via the core RPC. Ignore failures — the rest of
  // the page still works when the sidecar is unreachable or no skills exist.
  // Extracted so create/install flows can trigger a refresh on success.
  const refreshDiscoveredSkills = useCallback(async (): Promise<SkillSummary[]> => {
    try {
      const skills = await skillsApi.listSkills();
      console.debug('[skills][discovered] listSkills ok', { count: skills.length });
      setDiscoveredSkills(skills);
      return skills;
    } catch (err) {
      console.debug('[skills][discovered] listSkills error', {
        error: err instanceof Error ? err.message : String(err),
      });
      return [];
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const skills = await refreshDiscoveredSkills();
      if (cancelled) {
        // If the effect was cancelled mid-fetch, the state update still
        // fired inside `refreshDiscoveredSkills`. That's fine — React
        // will bail on the unmounted update; no retry needed.
        return;
      }
      void skills;
    })();
    return () => {
      cancelled = true;
    };
  }, [refreshDiscoveredSkills]);

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
        icon: CHANNEL_ICONS[def.icon],
      });
    }

    // Composio toolkits are rendered in a dedicated icon grid (see below)
    // so ~100+ connectors stay scannable without a vertical list per category.

    // Discovered SKILL.md skills — surface each as a card whose CTA opens
    // the detail drawer. They live under the generic "Other" category so
    // they don't displace hand-curated built-ins or Channels.
    for (const skill of discoveredSkills) {
      items.push({
        id: `discovered-${skill.id}`,
        name: skill.name,
        description: skill.description,
        category: 'Other',
        kind: 'discovered',
        icon: BUILT_IN_SKILL_ICONS.screenIntelligence,
        discoveredSkill: skill,
      });
    }

    return items;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [configurableChannels, channelConnections, discoveredSkills]);

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
    const matchesCategory =
      selectedCategory === 'All'
        ? () => true
        : (meta: ComposioToolkitMeta) => meta.category === selectedCategory;
    const matchesSearch = (meta: ComposioToolkitMeta) =>
      !q || meta.name.toLowerCase().includes(q) || meta.description.toLowerCase().includes(q);

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
    return SKILL_CATEGORY_ORDER.filter(c => c !== 'Channels' && cats.has(c));
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
    () => groupedItems.filter(g => g.category !== 'Channels'),
    [groupedItems]
  );

  const renderGroup = ({ category, items }: { category: SkillCategory; items: SkillItem[] }) => (
    <div
      key={category}
      className="rounded-2xl border border-stone-200 bg-white p-3 shadow-soft animate-fade-up">
      <div className="px-1 pb-3 pt-1">
        <h2 className="flex items-center gap-2 text-sm font-semibold text-stone-900">
          <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-stone-100">
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
                ctaLabel="Settings"
                onCtaClick={() => navigate(item.route!)}
              />
            );
            /* v8 ignore stop */
          }
          if (item.kind === 'discovered') {
            const skill = item.discoveredSkill!;
            const scopeLabel = skill.legacy
              ? 'Legacy'
              : skill.scope === 'user'
                ? 'User'
                : skill.scope === 'project'
                  ? 'Project'
                  : 'Legacy';
            const scopeColor = skill.legacy
              ? 'text-stone-600'
              : skill.scope === 'user'
                ? 'text-sage-600'
                : skill.scope === 'project'
                  ? 'text-amber-600'
                  : 'text-stone-600';
            const canUninstall = skill.scope === 'user' && !skill.legacy;
            return (
              <UnifiedSkillCard
                key={item.id}
                icon={item.icon}
                title={item.name}
                description={item.description}
                statusLabel={scopeLabel}
                statusColor={scopeColor}
                ctaLabel="View"
                onCtaClick={() => {
                  console.debug('[skills][discovered] open drawer', { skillId: skill.id });
                  setSelectedSkill(skill);
                }}
                secondaryActions={
                  canUninstall
                    ? [
                        {
                          label: 'Uninstall',
                          testId: `uninstall-skill-${skill.id}`,
                          icon: (
                            <svg
                              className="h-3.5 w-3.5"
                              fill="none"
                              stroke="currentColor"
                              strokeWidth="2"
                              viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14z"
                              />
                            </svg>
                          ),
                          onClick: () => {
                            console.debug('[skills][discovered] open uninstall', {
                              skillId: skill.id,
                            });
                            setUninstallCandidate(skill);
                          },
                        },
                      ]
                    : undefined
                }
              />
            );
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
                <h1 className="text-base font-semibold text-stone-900">Skills</h1>
                <p className="text-xs text-stone-500">
                  Scaffold a new <code className="font-mono">SKILL.md</code> or install a published
                  package.
                </p>
              </div>
              <div className="flex flex-shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={() => setInstallDialogOpen(true)}
                  className="rounded-lg border border-stone-200 bg-white px-3 py-2 text-xs font-medium text-stone-700 shadow-soft transition-colors hover:bg-stone-50 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
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
                      Connections are showing stale status
                    </h2>
                    <p className="mt-1 text-xs leading-relaxed text-amber-800">{composioError}</p>
                  </div>
                  <button
                    type="button"
                    onClick={() => void refreshComposio()}
                    className="flex-shrink-0 rounded-lg border border-amber-300 bg-white px-3 py-1.5 text-[11px] font-medium text-amber-800 transition-colors hover:bg-amber-100">
                    Retry
                  </button>
                </div>
              </div>
            )}

            {
              <>
                {channelsGroup && (
                  <div className="rounded-2xl border border-stone-200 bg-white p-3 shadow-soft animate-fade-up">
                    <div className="px-1 pb-3 pt-1">
                      <h2 className="flex items-center gap-2 text-sm font-semibold text-stone-900">
                        <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-stone-100">
                          <SkillCategoryIcon
                            category="Channels"
                            className={skillCategoryHeadingClassName('Channels')}
                          />
                        </span>
                        Channels
                      </h2>
                      <p className="mt-0.5 text-[11px] leading-relaxed text-stone-500">
                        Connect messaging platforms so your assistant can chat where your community
                        already lives.
                      </p>
                    </div>
                    <div
                      className="grid gap-2 sm:gap-3"
                      style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(5.5rem, 1fr))' }}>
                      {channelsGroup.items.map(item => (
                        <ChannelTile
                          key={item.id}
                          def={item.channelDef!}
                          status={item.channelStatus!}
                          icon={item.icon}
                          onOpen={() => setChannelModalDef(item.channelDef!)}
                        />
                      ))}
                    </div>
                  </div>
                )}

                <div className="rounded-2xl border border-stone-200 bg-white p-3 shadow-soft animate-fade-up">
                  <div className="px-1 pb-3 pt-1">
                    <h2 className="text-sm font-semibold text-stone-900">Integrations</h2>
                    <p className="mt-0.5 text-[11px] leading-relaxed text-stone-500">
                      Connect external apps. Connected services are sorted first and highlighted in
                      green.
                    </p>
                  </div>
                  <div className="space-y-3 px-1 pb-3">
                    <SkillSearchBar value={searchQuery} onChange={setSearchQuery} />
                    <SkillCategoryFilter
                      categories={availableCategories}
                      selected={selectedCategory}
                      onChange={setSelectedCategory}
                    />
                  </div>
                  {composioSortedEntries.length > 0 ? (
                    <div
                      className="grid gap-2 sm:gap-3"
                      style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(5.5rem, 1fr))' }}>
                      {composioSortedEntries.map(({ meta, connection }) => (
                        <ComposioConnectorTile
                          key={meta.slug}
                          meta={meta}
                          connection={connection}
                          hasComposioError={Boolean(composioError)}
                          onOpen={() => setComposioModalToolkit(meta)}
                          onRetryGlobal={() => void refreshComposio()}
                        />
                      ))}
                    </div>
                  ) : (
                    <p className="px-1 py-4 text-center text-xs text-stone-400">
                      No integrations match your search.
                    </p>
                  )}
                </div>

                {otherGroups.map(group => renderGroup(group))}
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
          connection={composioConnectionByToolkit.get(composioModalToolkit.slug)}
          onChanged={() => {
            void refreshComposio();
            void dismissPendingEscalationIfResolved(`composio:${composioModalToolkit.slug}`);
          }}
          onClose={() => setComposioModalToolkit(null)}
        />
      )}

      {selectedSkill && (
        <SkillDetailDrawer skill={selectedSkill} onClose={() => setSelectedSkill(null)} />
      )}

      {createModalOpen && (
        <CreateSkillModal
          onClose={() => setCreateModalOpen(false)}
          onCreated={skill => {
            console.debug('[skills][create] created', { id: skill.id, scope: skill.scope });
            setCreateModalOpen(false);
            // Optimistically append; then reconcile against a fresh list so
            // version/author/warnings picked up by the Rust discoverer end
            // up in state too.
            setDiscoveredSkills(prev =>
              prev.some(s => s.id === skill.id) ? prev : [...prev, skill]
            );
            setSelectedSkill(skill);
            void refreshDiscoveredSkills();
          }}
        />
      )}

      {installDialogOpen && (
        <InstallSkillDialog
          onClose={() => setInstallDialogOpen(false)}
          onInstalled={result => {
            console.debug('[skills][install] complete', {
              url: result.url,
              newSkills: result.newSkills.length,
            });
            void (async () => {
              const skills = await refreshDiscoveredSkills();
              // Auto-select the first newly-installed skill, if any — matches
              // the create flow's UX of landing the user in the detail view.
              const firstNewId = result.newSkills[0];
              if (firstNewId) {
                const match = skills.find(s => s.id === firstNewId);
                if (match) {
                  setSelectedSkill(match);
                }
              }
            })();
          }}
        />
      )}

      {uninstallCandidate && (
        <UninstallSkillConfirmDialog
          skill={uninstallCandidate}
          onClose={() => setUninstallCandidate(null)}
          onUninstalled={result => {
            console.debug('[skills][uninstall] complete', {
              name: result.name,
              removedPath: result.removedPath,
            });
            addToast({
              type: 'success',
              title: 'Skill uninstalled',
              message: `"${result.name}" was removed successfully.`,
            });
            // If the detail drawer was showing the skill we just removed,
            // close it — the resource tree is now stale and any `read_resource`
            // RPC would fail with a clean "not installed" error.
            setSelectedSkill(prev => (prev && prev.id === result.name ? null : prev));
            // Drop it from local state so the card disappears without a
            // round-trip; refresh to pick up any side effects (e.g. a
            // previously-shadowed project-scope skill now surfaces).
            setDiscoveredSkills(prev => prev.filter(s => s.id !== result.name));
            void refreshDiscoveredSkills();
          }}
        />
      )}
      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
}
