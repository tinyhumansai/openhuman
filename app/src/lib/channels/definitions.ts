import type { ChannelConnectionStatus, ChannelDefinition } from '../../types/channels';

/** Status badge styles for channel connection states. */
export const STATUS_STYLES: Record<ChannelConnectionStatus, { label: string; className: string }> =
  {
    connected: { label: 'Connected', className: 'bg-sage-500/20 text-sage-300 border-sage-500/30' },
    connecting: {
      label: 'Connecting',
      className: 'bg-amber-500/20 text-amber-300 border-amber-500/30',
    },
    disconnected: {
      label: 'Disconnected',
      className: 'bg-stone-500/20 text-stone-300 border-stone-500/30',
    },
    error: { label: 'Error', className: 'bg-coral-500/20 text-coral-300 border-coral-500/30' },
  };

/** Human-readable labels for auth modes. */
export const AUTH_MODE_LABELS: Record<string, string> = {
  managed_dm: 'Managed DM',
  oauth: 'OAuth Sign-in',
  bot_token: 'Bot Token',
  api_key: 'API Key',
};

/** Fallback definitions used when the core sidecar is unreachable. */
export const FALLBACK_DEFINITIONS: ChannelDefinition[] = [
  {
    id: 'telegram',
    display_name: 'Telegram',
    description: 'Send and receive messages via Telegram.',
    icon: 'telegram',
    auth_modes: [
      {
        mode: 'bot_token',
        description: 'Provide your own Telegram Bot token from @BotFather.',
        fields: [
          {
            key: 'bot_token',
            label: 'Bot Token',
            field_type: 'secret',
            required: true,
            placeholder: '123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11',
          },
          {
            key: 'allowed_users',
            label: 'Allowed Users',
            field_type: 'string',
            required: false,
            placeholder: 'Comma-separated Telegram usernames',
          },
        ],
        auth_action: undefined,
      },
      {
        mode: 'managed_dm',
        description: 'Message the OpenHuman Telegram bot directly.',
        fields: [],
        auth_action: 'telegram_managed_dm',
      },
    ],
    capabilities: ['send_text', 'receive_text', 'typing', 'draft_updates'],
  },
  {
    id: 'discord',
    display_name: 'Discord',
    description: 'Send and receive messages via Discord.',
    icon: 'discord',
    auth_modes: [
      {
        mode: 'bot_token',
        description: 'Provide your own Discord bot token.',
        fields: [
          {
            key: 'bot_token',
            label: 'Bot Token',
            field_type: 'secret',
            required: true,
            placeholder: 'Your Discord bot token',
          },
          {
            key: 'guild_id',
            label: 'Server (Guild) ID',
            field_type: 'string',
            required: false,
            placeholder: 'Optional: restrict to a specific server',
          },
        ],
        auth_action: undefined,
      },
      {
        mode: 'oauth',
        description: 'Install the OpenHuman bot to your Discord server via OAuth.',
        fields: [],
        auth_action: 'discord_oauth',
      },
    ],
    capabilities: ['send_text', 'receive_text', 'typing', 'threaded_replies'],
  },
  {
    id: 'web',
    display_name: 'Web',
    description: 'Chat via the built-in web UI.',
    icon: 'web',
    auth_modes: [
      {
        mode: 'managed_dm',
        description: 'Use the embedded web chat — no setup required.',
        fields: [],
        auth_action: undefined,
      },
    ],
    capabilities: ['send_text', 'send_rich_text', 'receive_text'],
  },
];
