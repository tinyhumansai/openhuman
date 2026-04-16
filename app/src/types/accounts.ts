import { IS_DEV } from '../utils/config';

export type AccountProvider =
  | 'whatsapp'
  | 'telegram'
  | 'linkedin'
  | 'gmail'
  | 'slack'
  | 'discord'
  | 'google-meet'
  | 'browserscan';

export type AccountStatus = 'pending' | 'open' | 'error' | 'closed';

export interface Account {
  id: string;
  provider: AccountProvider;
  label: string;
  createdAt: string;
  status: AccountStatus;
  lastError?: string;
}

export interface IngestedMessage {
  id: string;
  from?: string | null;
  body?: string | null;
  unread?: number;
  ts?: number;
}

export interface AccountsState {
  accounts: Record<string, Account>;
  order: string[];
  activeAccountId: string | null;
  messages: Record<string, IngestedMessage[]>;
  unread: Record<string, number>;
  logs: Record<string, AccountLogEntry[]>;
}

export interface AccountLogEntry {
  ts: number;
  level: 'info' | 'warn' | 'error' | 'debug';
  msg: string;
}

export interface ProviderDescriptor {
  id: AccountProvider;
  label: string;
  description: string;
  serviceUrl: string;
}

const BASE_PROVIDERS: ProviderDescriptor[] = [
  {
    id: 'whatsapp',
    label: 'WhatsApp Web',
    description: 'Open web.whatsapp.com inside the app and stream chat updates.',
    serviceUrl: 'https://web.whatsapp.com/',
  },
  {
    id: 'telegram',
    label: 'Telegram Web',
    description: 'Your Telegram chats, embedded and observed.',
    serviceUrl: 'https://web.telegram.org/k/',
  },
  {
    id: 'linkedin',
    label: 'LinkedIn',
    description: 'LinkedIn messaging — DMs and conversations.',
    serviceUrl: 'https://www.linkedin.com/messaging/',
  },
  {
    id: 'gmail',
    label: 'Gmail',
    description: 'Your Gmail inbox. Google may require sign-in a couple of times.',
    serviceUrl: 'https://mail.google.com/mail/u/0/',
  },
  {
    id: 'slack',
    label: 'Slack',
    description: 'Slack workspaces and channels.',
    serviceUrl: 'https://app.slack.com/client/',
  },
  {
    id: 'discord',
    label: 'Discord',
    description: 'Discord servers and DMs — channel list and unread counts.',
    serviceUrl: 'https://discord.com/channels/@me',
  },
  {
    id: 'google-meet',
    label: 'Google Meet',
    description: 'Join Google Meet calls and capture live captions.',
    serviceUrl: 'https://meet.google.com/',
  },
];

const DEV_PROVIDERS: ProviderDescriptor[] = [
  {
    id: 'browserscan',
    label: 'BrowserScan (dev)',
    description: 'Bot-detection sandbox for sanity-checking our webview fingerprint.',
    serviceUrl: 'https://www.browserscan.net/bot-detection',
  },
];

export const PROVIDERS: ProviderDescriptor[] = IS_DEV
  ? [...BASE_PROVIDERS, ...DEV_PROVIDERS]
  : BASE_PROVIDERS;
