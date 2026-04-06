export type ChannelType = 'telegram' | 'discord' | 'web';

export type ChannelAuthMode = 'managed_dm' | 'oauth' | 'bot_token' | 'api_key';

export type ChannelConnectionStatus = 'connected' | 'connecting' | 'disconnected' | 'error';

export interface ChannelConnection {
  channel: ChannelType;
  authMode: ChannelAuthMode;
  status: ChannelConnectionStatus;
  selectedDefault: boolean;
  lastError?: string;
  capabilities: string[];
  updatedAt: string;
}

export interface ChannelConnectionsByMode {
  managed_dm?: ChannelConnection;
  oauth?: ChannelConnection;
  bot_token?: ChannelConnection;
  api_key?: ChannelConnection;
}

export interface ChannelConnectionsState {
  schemaVersion: number;
  migrationCompleted: boolean;
  defaultMessagingChannel: ChannelType;
  connections: Record<ChannelType, ChannelConnectionsByMode>;
}

export interface OutboundRoute {
  channel: ChannelType;
  authMode: ChannelAuthMode;
}

// --- Backend-driven definitions (from openhuman.channels_list) ---

export interface FieldRequirement {
  key: string;
  label: string;
  field_type: string; // "string" | "secret" | "boolean"
  required: boolean;
  placeholder: string;
}

export interface AuthModeSpec {
  mode: ChannelAuthMode;
  description: string;
  fields: FieldRequirement[];
  auth_action?: string; // e.g. "telegram_managed_dm", "discord_oauth"
}

export type ChannelCapability =
  | 'send_text'
  | 'send_rich_text'
  | 'receive_text'
  | 'typing'
  | 'draft_updates'
  | 'threaded_replies'
  | 'file_attachments'
  | 'reactions';

export interface ChannelDefinition {
  id: string;
  display_name: string;
  description: string;
  icon: string;
  auth_modes: AuthModeSpec[];
  capabilities: ChannelCapability[];
}

export interface ChannelStatusEntry {
  channel_id: string;
  auth_mode: ChannelAuthMode;
  connected: boolean;
  has_credentials: boolean;
}

export interface ChannelConnectionResult {
  status: string; // "connected" | "pending_auth"
  restart_required: boolean;
  auth_action?: string;
  message?: string;
}

// --- Discord guild/channel discovery types ---

export interface DiscordGuild {
  id: string;
  name: string;
  icon: string | null;
}

export interface DiscordTextChannel {
  id: string;
  name: string;
  type: number;
  position: number;
  parent_id: string | null;
}

export interface BotPermissionCheck {
  can_view_channel: boolean;
  can_send_messages: boolean;
  can_read_message_history: boolean;
  missing_permissions: string[];
}
