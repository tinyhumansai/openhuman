import { useCallback, useMemo, useState } from 'react';

import { useChannelDefinitions } from '../../../hooks/useChannelDefinitions';
import { resolvePreferredAuthModeForChannel } from '../../../lib/channels/routing';
import { channelConnectionsApi } from '../../../services/api/channelConnectionsApi';
import { setDefaultMessagingChannel } from '../../../store/channelConnectionsSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import type {
  ChannelConnectionStatus,
  ChannelDefinition,
  ChannelType,
} from '../../../types/channels';
import ChannelSetupModal from '../../channels/ChannelSetupModal';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const CHANNEL_ICONS: Record<string, string> = {
  telegram: '\u2708\uFE0F',
  discord: '\uD83C\uDFAE',
  web: '\uD83C\uDF10',
};

function statusDot(status: ChannelConnectionStatus): string {
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

function statusLabel(status: ChannelConnectionStatus): string {
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

function statusColor(status: ChannelConnectionStatus): string {
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

const MessagingPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const channelConnections = useAppSelector(state => state.channelConnections);
  const { definitions, loading, error: loadError } = useChannelDefinitions();

  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [channelModalDef, setChannelModalDef] = useState<ChannelDefinition | null>(null);

  const configurableChannels = useMemo(
    () => definitions.filter(d => d.id !== 'web'),
    [definitions]
  );

  const recommendedRoute = useMemo(() => {
    const channel = channelConnections.defaultMessagingChannel;
    const authMode = resolvePreferredAuthModeForChannel(channelConnections, channel);
    return authMode ? `${channel} via ${authMode}` : 'No active route';
  }, [channelConnections]);

  const bestStatus = useCallback(
    (channelId: ChannelType): ChannelConnectionStatus => {
      const conns = channelConnections.connections[channelId];
      if (!conns) return 'disconnected';
      const statuses = Object.values(conns).map(c => c?.status ?? 'disconnected');
      if (statuses.includes('connected')) return 'connected';
      if (statuses.includes('connecting')) return 'connecting';
      if (statuses.includes('error')) return 'error';
      return 'disconnected';
    },
    [channelConnections]
  );

  const handleSetDefaultChannel = useCallback(
    (channel: ChannelType) => {
      const key = `default:${channel}`;
      setBusy(prev => ({ ...prev, [key]: true }));
      dispatch(setDefaultMessagingChannel(channel));
      void channelConnectionsApi.updatePreferences(channel).finally(() => {
        setBusy(prev => ({ ...prev, [key]: false }));
      });
    },
    [dispatch]
  );

  return (
    <div>
      <SettingsHeader
        title="Messaging"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* Default channel selector */}
        <section className="rounded-xl border border-stone-200 bg-white p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Default Messaging Channel</h3>
          <div className="grid grid-cols-2 gap-2">
            {definitions.map(def => {
              const channelId = def.id as ChannelType;
              const selected = channelConnections.defaultMessagingChannel === channelId;
              const busyKey = `default:${channelId}`;
              return (
                <button
                  key={channelId}
                  type="button"
                  onClick={() => handleSetDefaultChannel(channelId)}
                  disabled={busy[busyKey]}
                  className={`rounded-lg border px-3 py-2 text-sm transition-colors ${
                    selected
                      ? 'border-primary-500/60 bg-primary-50 text-primary-600'
                      : 'border-stone-200 bg-stone-50 text-stone-600 hover:border-stone-300'
                  }`}>
                  {def.display_name}
                </button>
              );
            })}
          </div>
          <p className="text-xs text-stone-400">
            Active route: <span className="text-primary-600">{recommendedRoute}</span>
          </p>
        </section>

        {loadError && (
          <div className="rounded-lg border border-coral-500/40 bg-coral-500/10 px-4 py-3 text-sm text-coral-100">
            {loadError}
          </div>
        )}

        {loading && (
          <div className="rounded-xl border border-stone-200 bg-white p-4 text-sm text-stone-400">
            Loading channel definitions...
          </div>
        )}

        {/* Channel cards — click to open the shared ChannelSetupModal */}
        {!loading && (
          <section className="rounded-xl border border-stone-200 bg-white p-4 space-y-3">
            <h3 className="text-sm font-semibold text-stone-900">Channel Integrations</h3>
            <p className="text-xs text-stone-400">
              Configure auth modes for each messaging channel.
            </p>
            <div className="space-y-2">
              {configurableChannels.map(def => {
                const channelId = def.id as ChannelType;
                const status = bestStatus(channelId);
                const icon = CHANNEL_ICONS[def.icon] ?? '';

                return (
                  <button
                    key={channelId}
                    type="button"
                    onClick={() => setChannelModalDef(def)}
                    className="w-full rounded-lg border border-stone-200 bg-stone-50 p-3 text-left transition-colors hover:bg-white hover:border-stone-300">
                    <div className="flex items-center gap-3">
                      <span className="text-lg flex-shrink-0">{icon}</span>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-sm font-medium text-stone-900">
                            {def.display_name}
                          </span>
                          <div
                            className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${statusDot(status)}`}
                          />
                          <span className={`text-xs ${statusColor(status)}`}>
                            {statusLabel(status)}
                          </span>
                        </div>
                        <p className="text-xs text-stone-500 mt-0.5">{def.description}</p>
                      </div>
                      <svg
                        className="w-4 h-4 text-stone-400 flex-shrink-0"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M9 5l7 7-7 7"
                        />
                      </svg>
                    </div>
                  </button>
                );
              })}
            </div>
          </section>
        )}
      </div>

      {/* Shared channel config modal */}
      {channelModalDef && (
        <ChannelSetupModal definition={channelModalDef} onClose={() => setChannelModalDef(null)} />
      )}
    </div>
  );
};

export default MessagingPanel;
