import { useT } from '../../lib/i18n/I18nContext';
import type { ChannelDefinition, ChannelType } from '../../types/channels';
import ChannelCapabilities from './ChannelCapabilities';
import CredentialChannelConfig from './CredentialChannelConfig';
import DiscordConfig from './DiscordConfig';
import McpServersTab from './mcp/McpServersTab';
import TelegramConfig from './TelegramConfig';
import WebChannelConfig from './WebChannelConfig';

interface ChannelConfigPanelProps {
  selectedChannel: ChannelType;
  definitions: ChannelDefinition[];
}

const ChannelConfigPanel = ({ selectedChannel, definitions }: ChannelConfigPanelProps) => {
  const { t } = useT();

  // MCP is a virtual tab — not backed by a ChannelDefinition from the core.
  if (selectedChannel === 'mcp') {
    return (
      <div className="space-y-4">
        <section className="rounded-xl border border-line bg-surface p-4 space-y-3">
          <div>
            <h3 className="text-base font-semibold text-content">{t('channels.mcp.title')}</h3>
            <p className="text-xs text-content-muted mt-1">{t('channels.mcp.description')}</p>
          </div>
          <McpServersTab />
        </section>
      </div>
    );
  }

  const definition = definitions.find(d => d.id === selectedChannel);
  if (!definition) return null;

  return (
    <div className="space-y-4">
      <section className="rounded-xl border border-line bg-surface p-4 space-y-3">
        <div>
          <h3 className="text-base font-semibold text-content">
            {t(`channels.${definition.id}.displayName`, definition.display_name)}
          </h3>
          <p className="text-xs text-content-muted mt-1">
            {t(`channels.${definition.id}.description`, definition.description)}
          </p>
        </div>
        {selectedChannel === 'telegram' && <TelegramConfig definition={definition} />}
        {selectedChannel === 'discord' && <DiscordConfig definition={definition} />}
        {selectedChannel === 'web' && <WebChannelConfig definition={definition} />}
        {(selectedChannel === 'lark' || selectedChannel === 'dingtalk') && (
          <CredentialChannelConfig definition={definition} />
        )}
      </section>

      <ChannelCapabilities capabilities={definition.capabilities} />
    </div>
  );
};

export default ChannelConfigPanel;
