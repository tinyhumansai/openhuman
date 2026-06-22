import type { ChannelConnectionStatus, ChannelDefinition } from '../../types/channels';

/** Status badge styles for channel connection states. */
export const STATUS_STYLES: Record<ChannelConnectionStatus, { label: string; className: string }> =
  {
    connected: { label: 'Connected', className: 'bg-sage-500/10 text-sage-700 border-sage-500/30' },
    connecting: {
      label: 'Connecting',
      className: 'bg-amber-500/10 text-amber-700 border-amber-500/30',
    },
    disconnected: {
      label: 'Disconnected',
      className: 'bg-stone-100 text-stone-500 border-stone-200',
    },
    error: { label: 'Error', className: 'bg-coral-500/10 text-coral-700 border-coral-500/30' },
  };

/** Human-readable labels for auth modes. */
export const AUTH_MODE_LABELS: Record<string, string> = {
  managed_dm: 'Login with OpenHuman',
  oauth: 'OAuth Sign-in',
  bot_token: 'Use your own Bot Token',
  api_key: 'Use your own API Key',
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
        mode: 'managed_dm',
        description: 'Message the OpenHuman Telegram bot directly.',
        fields: [],
        auth_action: 'telegram_managed_dm',
      },
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
            key: 'chat_id',
            label: 'Chat ID',
            field_type: 'string',
            required: false,
            placeholder: 'Optional: default chat for outbound messages',
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
          {
            key: 'channel_id',
            label: 'Channel ID',
            field_type: 'string',
            required: false,
            placeholder: 'Optional: default channel for outbound messages',
          },
          {
            key: 'allowed_users',
            label: 'Allowed Users',
            field_type: 'string',
            required: false,
            placeholder: 'Comma-separated Discord user IDs, or * for everyone (blank = everyone)',
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
      {
        mode: 'managed_dm',
        description: 'Link your personal Discord account to the OpenHuman bot.',
        fields: [],
        auth_action: 'discord_managed_link',
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
  // Lark / Feishu — fields must stay aligned with `LarkConfig` in
  // `src/openhuman/config/schema/channels.rs` and `lark_definition()` in
  // `src/openhuman/channels/controllers/definitions.rs`. See #2048.
  {
    id: 'lark',
    display_name: 'Lark / Feishu',
    description: 'Send and receive via Lark (international) or Feishu (中国版).',
    icon: 'lark',
    auth_modes: [
      {
        mode: 'api_key',
        description: 'Provide your Lark/Feishu app credentials from the Open Platform.',
        fields: [
          {
            key: 'app_id',
            label: 'App ID',
            field_type: 'string',
            required: true,
            placeholder: 'cli_xxxxxxxxxxxx',
          },
          {
            key: 'app_secret',
            label: 'App Secret',
            field_type: 'secret',
            required: true,
            placeholder: 'Your Lark app secret',
          },
          {
            key: 'encrypt_key',
            label: 'Encrypt Key',
            field_type: 'secret',
            required: false,
            placeholder: 'Optional — required only if you enabled message encryption',
          },
          {
            key: 'verification_token',
            label: 'Verification Token',
            field_type: 'secret',
            required: false,
            placeholder: 'Optional — used for HTTP webhook verification',
          },
          {
            key: 'use_feishu',
            label: 'Use Feishu (中国版)',
            field_type: 'boolean',
            required: false,
            placeholder: 'On = open.feishu.cn (China); off = open.larksuite.com',
          },
          {
            key: 'receive_mode',
            label: 'Receive Mode',
            field_type: 'string',
            required: false,
            placeholder: 'websocket (default) or webhook',
          },
          {
            key: 'port',
            label: 'Webhook Port',
            // Numeric — field_type stays 'string' because the schema-driven
            // form renderer only accepts 'string' | 'secret' | 'boolean'.
            // LarkConfig parses it back to u16. Keep aligned with the Rust
            // lark_definition() entry.
            field_type: 'string',
            required: false,
            placeholder: 'Optional — local HTTP port when receive_mode = webhook (e.g. 8080)',
          },
          {
            key: 'allowed_users',
            label: 'Allowed Users',
            field_type: 'string',
            required: false,
            placeholder: 'Comma-separated open_id / union_id; leave empty to allow any',
          },
        ],
        auth_action: undefined,
      },
    ],
    capabilities: ['send_text', 'receive_text', 'threaded_replies'],
  },
  // DingTalk (钉钉) — fields must stay aligned with `DingTalkConfig` in
  // `src/openhuman/config/schema/channels.rs`. See #2048.
  {
    id: 'dingtalk',
    display_name: 'DingTalk (钉钉)',
    description: 'Send and receive via DingTalk Stream Mode (钉钉).',
    icon: 'dingtalk',
    auth_modes: [
      {
        mode: 'api_key',
        description: 'Provide your DingTalk app credentials from the developer console.',
        fields: [
          {
            key: 'client_id',
            label: 'Client ID (AppKey)',
            field_type: 'string',
            required: true,
            placeholder: 'ding_xxxxxxxxxxxx',
          },
          {
            key: 'client_secret',
            label: 'Client Secret (AppSecret)',
            field_type: 'secret',
            required: true,
            placeholder: 'Your DingTalk app secret',
          },
          {
            key: 'allowed_users',
            label: 'Allowed Users',
            field_type: 'string',
            required: false,
            placeholder: 'Comma-separated DingTalk userIds; leave empty to allow any',
          },
        ],
        auth_action: undefined,
      },
    ],
    capabilities: ['send_text', 'receive_text'],
  },
];
