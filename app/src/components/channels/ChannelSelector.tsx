import { useMemo } from 'react';

import { resolvePreferredAuthModeForChannel } from '../../lib/channels/routing';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import type { ChannelDefinition, ChannelType } from '../../types/channels';
import ChannelStatusBadge from './ChannelStatusBadge';

interface ChannelSelectorProps {
  definitions: ChannelDefinition[];
  selectedChannel: ChannelType;
  onSelectChannel: (channel: ChannelType) => void;
}

const CHANNEL_ICONS: Record<string, string> = { telegram: '✈️', discord: '🎮', web: '🌐' };

const ChannelSelector = ({
  definitions,
  selectedChannel,
  onSelectChannel,
}: ChannelSelectorProps) => {
  const { t } = useT();
  const channelConnections = useAppSelector(state => state.channelConnections);

  const activeRoute = useMemo(() => {
    const channel = channelConnections.defaultMessagingChannel;
    const authMode = resolvePreferredAuthModeForChannel(channelConnections, channel);
    return authMode
      ? t('channels.activeRouteValue').replace('{channel}', channel).replace('{authMode}', authMode)
      : t('channels.noActiveRoute');
  }, [channelConnections, t]);

  return (
    <section className="rounded-xl border border-stone-200 bg-white p-4 space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-stone-900">{t('channels.title')}</h2>
        <p className="text-xs text-stone-400">
          {t('channels.activeRoute')}: <span className="text-primary-600">{activeRoute}</span>
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
                  ? 'border-primary-500/60 bg-primary-50 text-primary-600'
                  : 'border-stone-200 bg-stone-50 text-stone-600 hover:border-stone-300'
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
