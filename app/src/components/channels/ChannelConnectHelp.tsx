/**
 * In-app "how to connect" guidance shown at the top of a channel's setup card.
 *
 * Grounds users in the real connect flow so they don't have to ask the agent
 * for a navigation path (the agent previously hallucinated non-existent menus
 * like "Settings → Automation & Channels"). Only channels with a documented
 * flow render a callout; everything else renders nothing.
 */
import { useT } from '../../lib/i18n/I18nContext';

/** Per-channel help copy. Add a key here to surface guidance for a channel. */
const CHANNEL_HELP_KEY: Record<string, string> = {
  discord: 'channels.connectHelp.discord',
  telegram: 'channels.connectHelp.telegram',
};

interface ChannelConnectHelpProps {
  channelId: string;
}

export default function ChannelConnectHelp({ channelId }: ChannelConnectHelpProps) {
  const { t } = useT();
  const bodyKey = CHANNEL_HELP_KEY[channelId];
  if (!bodyKey) return null;

  return (
    <div className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50/80 dark:bg-primary-500/10 px-4 py-3 text-sm text-content-secondary">
      <p className="font-medium text-content">{t('channels.connectHelp.title')}</p>
      <p className="mt-1 text-xs text-content-secondary">{t(bodyKey)}</p>
    </div>
  );
}
