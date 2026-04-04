import type { ChannelDefinition, ChannelType } from '../../types/channels';
import ChannelCapabilities from './ChannelCapabilities';
import DiscordConfig from './DiscordConfig';
import TelegramConfig from './TelegramConfig';
import WebChannelConfig from './WebChannelConfig';

interface ChannelConfigPanelProps {
  selectedChannel: ChannelType;
  definitions: ChannelDefinition[];
}

const ChannelConfigPanel = ({ selectedChannel, definitions }: ChannelConfigPanelProps) => {
  const definition = definitions.find(d => d.id === selectedChannel);
  if (!definition) return null;

  return (
    <div className="space-y-4">
      {selectedChannel === 'telegram' && <TelegramConfig definition={definition} />}
      {selectedChannel === 'discord' && <DiscordConfig definition={definition} />}
      {selectedChannel === 'web' && <WebChannelConfig definition={definition} />}

      <ChannelCapabilities capabilities={definition.capabilities} />
    </div>
  );
};

export default ChannelConfigPanel;
