/**
 * Reusable modal for configuring a channel integration (Telegram, Discord, etc.).
 * Built on the shared `ModalShell` primitive (Radix `Dialog` underneath). Can be
 * opened from the Skills page or Settings.
 */
import { useId } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ChannelDefinition, ChannelType } from '../../types/channels';
import Badge from '../ui/Badge';
import { ModalShell } from '../ui/ModalShell';
import ChannelConnectHelp from './ChannelConnectHelp';
import { renderChannelIcon } from './channelIcon';
import CredentialChannelConfig from './CredentialChannelConfig';
import DiscordConfig from './DiscordConfig';
import TelegramConfig from './TelegramConfig';
import YuanbaoConfig from './YuanbaoConfig';

interface ChannelSetupModalProps {
  definition: ChannelDefinition;
  onClose: () => void;
}

function renderChannelConfig(
  definition: ChannelDefinition,
  channelId: ChannelType,
  t: (key: string, fallback?: string) => string
) {
  switch (channelId) {
    case 'telegram':
      return <TelegramConfig definition={definition} />;
    case 'discord':
      return <DiscordConfig definition={definition} />;
    case 'yuanbao':
      return <YuanbaoConfig definition={definition} />;
    // Credential-form channels (Lark/DingTalk/Email) render the same generic
    // form here as on the Channels page — otherwise clicking their Skills-grid
    // tile fell through to "config not available" (#4280 review).
    case 'lark':
    case 'dingtalk':
    case 'email':
      return <CredentialChannelConfig definition={definition} />;
    default:
      return (
        <p className="py-4 text-sm text-content-faint">
          {t('channels.configNotAvailable')} {definition.display_name}
        </p>
      );
  }
}

function ChannelConfigContent({ definition }: { definition: ChannelDefinition }) {
  const { t } = useT();
  const channelId = definition.id as ChannelType;
  return (
    <div className="space-y-3">
      <ChannelConnectHelp channelId={channelId} />
      {renderChannelConfig(definition, channelId, t)}
    </div>
  );
}

export default function ChannelSetupModal({ definition, onClose }: ChannelSetupModalProps) {
  const { t } = useT();
  const titleId = useId();

  return (
    <ModalShell
      onClose={onClose}
      titleId={titleId}
      icon={renderChannelIcon(definition.icon)}
      maxWidthClassName="max-w-[500px]"
      contentClassName="max-h-[70vh] overflow-y-auto px-5 py-4"
      title={
        <span className="flex items-center gap-2">
          {definition.display_name}
          <Badge variant="primary">{t('channels.channel')}</Badge>
        </span>
      }
      subtitle={definition.description}>
      <ChannelConfigContent definition={definition} />
    </ModalShell>
  );
}
