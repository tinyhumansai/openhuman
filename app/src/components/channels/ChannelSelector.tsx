import { useMemo } from 'react';

import { resolvePreferredAuthModeForChannel } from '../../lib/channels/routing';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import type { ChannelConnectionStatus, ChannelDefinition, ChannelType } from '../../types/channels';
import { renderChannelIcon } from './channelIcon';
import ChannelStatusBadge from './ChannelStatusBadge';

interface ChannelSelectorProps {
  definitions: ChannelDefinition[];
  selectedChannel: ChannelType;
  onSelectChannel: (channel: ChannelType) => void;
}

/** Virtual (static) tabs that are not backed by a ChannelDefinition from the core. */
const VIRTUAL_TABS: { id: ChannelType; display_name: string }[] = [
  { id: 'mcp', display_name: 'MCP Servers' },
];

const CHANNEL_STATUS_PRIORITY: ChannelConnectionStatus[] = [
  'connected',
  'connecting',
  'error',
  'disconnected',
];

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
    <section className="rounded-xl border border-line bg-surface p-4 space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-content">{t('channels.title')}</h2>
        <p className="text-xs text-content-faint">
          {t('channels.activeRoute')}:{' '}
          <span className="text-primary-600 dark:text-primary-300">{activeRoute}</span>
        </p>
      </div>

      <div className="flex gap-2 flex-wrap">
        {definitions.map(def => {
          const channelId = def.id as ChannelType;
          const isSelected = selectedChannel === channelId;

          // Determine best connection status for this channel.
          const channelModes = channelConnections.connections[channelId];
          const modeStatuses = channelModes
            ? Object.values(channelModes)
                .map(connection => connection?.status)
                .filter((status): status is ChannelConnectionStatus => Boolean(status))
            : [];
          const bestStatus =
            CHANNEL_STATUS_PRIORITY.find(status => modeStatuses.includes(status)) ?? 'disconnected';

          return (
            <button
              key={channelId}
              type="button"
              data-testid={`channel-select-${channelId}`}
              onClick={() => onSelectChannel(channelId)}
              className={`flex-1 flex items-center justify-between gap-2 rounded-lg border px-4 py-3 text-sm transition-colors ${
                isSelected
                  ? 'border-primary-500/60 bg-primary-50 dark:bg-primary-500/15 text-primary-600 dark:text-primary-300'
                  : 'border-line bg-surface-muted text-content-secondary hover:border-line-strong dark:hover:border-line-strong'
              }`}>
              <span className="flex items-center gap-2">
                {renderChannelIcon(def.icon)}
                <span className="font-medium">{def.display_name}</span>
              </span>
              <ChannelStatusBadge status={bestStatus} />
            </button>
          );
        })}

        {/* Virtual tabs — not backed by a ChannelDefinition from the core */}
        {VIRTUAL_TABS.map(tab => {
          const isSelected = selectedChannel === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => onSelectChannel(tab.id)}
              className={`flex-1 flex items-center gap-2 rounded-lg border px-4 py-3 text-sm transition-colors ${
                isSelected
                  ? 'border-primary-500/60 bg-primary-50 dark:bg-primary-500/15 text-primary-600 dark:text-primary-300'
                  : 'border-line bg-surface-muted text-content-secondary hover:border-line-strong dark:hover:border-line-strong'
              }`}>
              {renderChannelIcon(tab.id)}
              <span className="font-medium">{tab.display_name}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
};

export default ChannelSelector;
