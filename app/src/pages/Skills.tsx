import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ChannelSetupModal from '../components/channels/ChannelSetupModal';
import ComposioConnectModal from '../components/composio/ComposioConnectModal';
import {
  composioToolkitMeta,
  type ComposioToolkitMeta,
  KNOWN_COMPOSIO_TOOLKITS,
} from '../components/composio/toolkitMeta';
import AutocompleteSetupModal from '../components/skills/AutocompleteSetupModal';
import ScreenIntelligenceSetupModal from '../components/skills/ScreenIntelligenceSetupModal';
import UnifiedSkillCard from '../components/skills/SkillCard';
import SkillCategoryFilter, { type SkillCategory } from '../components/skills/SkillCategoryFilter';
import SkillSearchBar from '../components/skills/SkillSearchBar';
import VoiceSetupModal from '../components/skills/VoiceSetupModal';
import { useAutocompleteSkillStatus } from '../features/autocomplete/useAutocompleteSkillStatus';
import { useScreenIntelligenceSkillStatus } from '../features/screen-intelligence/useScreenIntelligenceSkillStatus';
import { useVoiceSkillStatus } from '../features/voice/useVoiceSkillStatus';
import { useChannelDefinitions } from '../hooks/useChannelDefinitions';
import { useComposioIntegrations } from '../lib/composio/hooks';
import { type ComposioConnection, deriveComposioState } from '../lib/composio/types';
import { useAppSelector } from '../store/hooks';
import type { ChannelConnectionStatus, ChannelDefinition, ChannelType } from '../types/channels';

const CHANNEL_ICONS: Record<string, string> = {
  telegram: '\u2708\uFE0F',
  discord: '\uD83C\uDFAE',
  web: '\uD83C\uDF10',
};

function channelStatusDot(status: ChannelConnectionStatus): string {
  switch (status) {
    case 'connected':
      return 'bg-sage-500';
    case 'connecting':
      return 'bg-amber-500 animate-pulse';
    case 'error':
      return 'bg-coral-500';
    default:
      return 'bg-stone-300';
  }
}

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

function composioStatusDot(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'bg-sage-500';
    case 'pending':
      return 'bg-amber-500 animate-pulse';
    case 'error':
      return 'bg-coral-500';
    default:
      return 'bg-stone-300';
  }
}

function composioStatusLabel(connection: ComposioConnection | undefined): string {
  switch (deriveComposioState(connection)) {
    case 'connected':
      return 'Connected';
    case 'pending':
      return 'Connecting';
    case 'error':
      return 'Error';
    default:
      return 'Not connected';
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

// ─── Built-in skill definitions ────────────────────────────────────────────────

const BUILT_IN_SKILLS = [
  {
    id: 'screen-intelligence',
    title: 'Screen Intelligence',
    description:
      'Capture windows, summarize what is on screen, and feed useful context into memory.',
    route: '/settings/screen-intelligence',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M3 5h18v12H3zM8 21h8m-4-4v4"
        />
      </svg>
    ),
  },
  {
    id: 'text-autocomplete',
    title: 'Text Auto-Complete',
    description:
      'Suggest inline completions while you type and control where autocomplete is active.',
    route: '/settings/autocomplete',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M4 7h16M4 12h10m-10 5h7m10 0l3 3m0 0l3-3m-3 3v-8"
        />
      </svg>
    ),
  },
  {
    id: 'voice-stt',
    title: 'Voice Intelligence',
    description: 'Use the microphone for dictation and voice-driven chat with your AI.',
    route: '/settings/voice',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={1.8}
          d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
        />
      </svg>
    ),
  },
];

// ─── Item type for unified list ────────────────────────────────────────────────

interface SkillItem {
  id: string;
  name: string;
  description: string;
  category: SkillCategory;
  kind: 'builtin' | 'channel' | 'composio';
  // For built-in
  route?: string;
  icon?: React.ReactNode;
  // For channel
  channelDef?: ChannelDefinition;
  channelStatus?: ChannelConnectionStatus;
  // For composio
  composioToolkit?: ComposioToolkitMeta;
  composioConnection?: ComposioConnection;
}

// ─── Main Skills Page ──────────────────────────────────────────────────────────

export default function Skills() {
  const navigate = useNavigate();
  const { definitions: channelDefs } = useChannelDefinitions();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const {
    toolkits: composioToolkits,
    connectionByToolkit: composioConnectionByToolkit,
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
        icon: <span className="text-lg">{CHANNEL_ICONS[def.icon] ?? ''}</span>,
      });
    }

    // Composio toolkits — rendered with the same UnifiedSkillCard used
    // for channels/skills so they sit flush in the grid. Each entry is
    // keyed by slug and routed through `ComposioConnectModal` for the
    // authorize/OAuth/poll flow.
    const sortedToolkits = Array.from(
      new Set([...KNOWN_COMPOSIO_TOOLKITS, ...composioToolkits.map(slug => slug.toLowerCase())])
    ).sort((a, b) => a.localeCompare(b));
    for (const slug of sortedToolkits) {
      const meta = composioToolkitMeta(slug);
      const connection = composioConnectionByToolkit.get(meta.slug);
        items.push({
          id: `composio-${meta.slug}`,
          name: meta.name,
          description: meta.description,
          category: meta.category,
          kind: 'composio',
          icon: <span className="text-lg">{meta.icon}</span>,
          composioToolkit: meta,
          composioConnection: connection,
        });
    }

    return items;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [configurableChannels, channelConnections, composioToolkits, composioConnectionByToolkit]);

  const availableCategories: SkillCategory[] = useMemo(() => {
    const cats = new Set<SkillCategory>(['All']);
    for (const item of allItems) {
      cats.add(item.category);
    }
    const order: SkillCategory[] = [
      'All',
      'Built-in',
      'Channels',
      'Chat',
      'Productivity',
      'Tools & Automation',
      'Social',
      'Platform',
      'Other',
    ];
    return order.filter(c => cats.has(c));
  }, [allItems]);

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

  return (
    <div className="min-h-full">
      <div className="min-h-full flex flex-col">
        <div className="flex-1 flex items-start justify-center p-4 pt-6">
          <div className="max-w-lg w-full space-y-4">
            <SkillSearchBar value={searchQuery} onChange={setSearchQuery} />

            <SkillCategoryFilter
              categories={availableCategories}
              selected={selectedCategory}
              onChange={setSelectedCategory}
            />

            {filteredItems.length === 0 ? (
              <div className="py-8 text-center">
                <p className="text-sm text-stone-400">No skills found</p>
              </div>
            ) : (
              groupedItems.map(({ category, items }) => (
                <div
                  key={category}
                  className="rounded-2xl border border-stone-200 bg-white p-3 shadow-soft animate-fade-up">
                  <div className="px-1 pb-3 pt-1">
                    <h2 className="text-sm font-semibold text-stone-900">{category}</h2>
                  </div>
                  <div className="space-y-2">
                    {items.map(item => {
                      if (item.kind === 'builtin') {
                        // Screen Intelligence gets a state-aware card
                        if (item.id === 'screen-intelligence') {
                          return (
                            <UnifiedSkillCard
                              key={item.id}
                              icon={item.icon}
                              title={item.name}
                              description={item.description}
                              statusDot={screenIntelligenceStatus.statusDot}
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
                        // Text Auto-Complete gets a state-aware card
                        if (item.id === 'text-autocomplete') {
                          return (
                            <UnifiedSkillCard
                              key={item.id}
                              icon={item.icon}
                              title={item.name}
                              description={item.description}
                              statusDot={autocompleteStatus.statusDot}
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
                        // Voice Intelligence gets a state-aware card
                        if (item.id === 'voice-stt') {
                          return (
                            <UnifiedSkillCard
                              key={item.id}
                              icon={item.icon}
                              title={item.name}
                              description={item.description}
                              statusDot={voiceStatus.statusDot}
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
                      }
                      if (item.kind === 'channel') {
                        const status = item.channelStatus!;
                        return (
                          <UnifiedSkillCard
                            key={item.id}
                            icon={item.icon}
                            title={item.name}
                            description={item.description}
                            statusDot={channelStatusDot(status)}
                            statusLabel={channelStatusLabel(status)}
                            statusColor={channelStatusColor(status)}
                            ctaLabel={status === 'connected' ? 'Manage' : 'Setup'}
                            onCtaClick={() => setChannelModalDef(item.channelDef!)}
                          />
                        );
                      }
                      if (item.kind === 'composio') {
                        const meta = item.composioToolkit!;
                        const connection = item.composioConnection;
                        const state = deriveComposioState(connection);
                        const ctaLabel =
                          state === 'connected'
                            ? 'Manage'
                            : state === 'pending'
                              ? 'Waiting'
                              : state === 'error'
                                ? 'Retry'
                                : 'Connect';
                        const ctaVariant: 'primary' | 'sage' | 'amber' =
                          state === 'connected' ? 'sage' : state === 'error' ? 'amber' : 'primary';
                        return (
                          <UnifiedSkillCard
                            key={item.id}
                            icon={item.icon}
                            title={item.name}
                            description={item.description}
                            statusDot={composioStatusDot(connection)}
                            statusLabel={composioStatusLabel(connection)}
                            statusColor={composioStatusColor(connection)}
                            ctaLabel={ctaLabel}
                            ctaVariant={ctaVariant}
                            onCtaClick={() => setComposioModalToolkit(meta)}
                          />
                        );
                      }
                    })}
                  </div>
                </div>
              ))
            )}
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
          onChanged={() => void refreshComposio()}
          onClose={() => setComposioModalToolkit(null)}
        />
      )}
    </div>
  );
}
