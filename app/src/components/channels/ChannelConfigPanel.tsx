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
      <section className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 space-y-3">
        <div>
          <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
            {definition.display_name}
          </h3>
          <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
            {definition.description}
          </p>
        </div>
        {selectedChannel === 'telegram' && <TelegramConfig definition={definition} />}
        {selectedChannel === 'discord' && <DiscordConfig definition={definition} />}
        {selectedChannel === 'web' && <WebChannelConfig definition={definition} />}
      </section>

      <ChannelCapabilities capabilities={definition.capabilities} />
    </div>
  );
};

export default ChannelConfigPanel;
