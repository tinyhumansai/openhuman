import { useMemo } from 'react';

import { resolvePreferredAuthModeForChannel } from '../../lib/channels/routing';
import { useAppSelector } from '../../store/hooks';
import type { ChannelDefinition, ChannelType } from '../../types/channels';
import ChannelStatusBadge from './ChannelStatusBadge';

interface ChannelSelectorProps {
  definitions: ChannelDefinition[];
  selectedChannel: ChannelType;
  onSelectChannel: (channel: ChannelType) => void;
}

const CHANNEL_ICONS: Record<string, string> = {
  telegram: '\u2708\uFE0F',
  discord: '\uD83C\uDFAE',
  web: '\uD83C\uDF10',
};

const ChannelSelector = ({
  definitions,
  selectedChannel,
  onSelectChannel,
}: ChannelSelectorProps) => {
  const channelConnections = useAppSelector(state => state.channelConnections);

  const activeRoute = useMemo(() => {
    const channel = channelConnections.defaultMessagingChannel;
    const authMode = resolvePreferredAuthModeForChannel(channelConnections, channel);
    return authMode ? `${channel} via ${authMode}` : 'No active route';
  }, [channelConnections]);

  return (
    <section className="rounded-xl border border-stone-800/60 bg-black/40 backdrop-blur-md p-4 space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-white">Channels</h2>
        <p className="text-xs text-stone-400">
          Active route: <span className="text-primary-300">{activeRoute}</span>
        </p>
      </div>

      <div className="flex gap-2">
        {definitions.map(def => {
          const channelId = def.id as ChannelType;
          const isSelected = selectedChannel === channelId;

          // Determine best connection status for this channel.
          const channelModes = channelConnections.connections[channelId];
          const bestStatus = channelModes
            ? (Object.values(channelModes).find(c => c?.status === 'connected')?.status ??
              Object.values(channelModes).find(c => c?.status === 'connecting')?.status ??
              'disconnected')
            : 'disconnected';

          return (
            <button
              key={channelId}
              type="button"
              onClick={() => onSelectChannel(channelId)}
              className={`flex-1 flex items-center justify-between gap-2 rounded-lg border px-4 py-3 text-sm transition-colors ${
                isSelected
                  ? 'border-primary-500/60 bg-primary-500/20 text-primary-200'
                  : 'border-stone-700 bg-stone-900/30 text-stone-300 hover:border-stone-500'
              }`}>
              <span className="flex items-center gap-2">
                <span className="text-base">{CHANNEL_ICONS[def.icon] ?? ''}</span>
                <span className="font-medium">{def.display_name}</span>
              </span>
              <ChannelStatusBadge status={bestStatus} />
            </button>
          );
        })}
      </div>
    </section>
  );
};

export default ChannelSelector;
