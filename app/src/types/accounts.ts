import { IS_DEV } from '../utils/config';

export type AccountProvider =
  | 'whatsapp'
  | 'wechat'
  | 'telegram'
  | 'linkedin'
  | 'slack'
  | 'discord'
  | 'gmail'
  | 'outlook'
  | 'instagram'
  | 'twitter'
  | 'google-meet'
  | 'zoom'
  | 'browserscan';

// Status lifecycle for an embedded webview account:
//   'pending'  — openWebviewAccount invoked, Rust-side add_child not yet confirmed
//   'loading'  — CEF child webview spawned off-screen, waiting for first page-loaded
//                signal; WebviewHost shows its spinner
//   'timeout'  — initial load watchdog elapsed; keep overlay visible and let user retry
//   'open'     — page loaded, webview_account_reveal completed, webview on-screen
//   'closed'   — webview destroyed
//   'error'    — open/reveal failed (lastError populated)
export type AccountStatus = 'pending' | 'loading' | 'timeout' | 'open' | 'error' | 'closed';

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
  /**
   * Issue #1233 — most-recently-active non-agent account id, persisted
   * across sessions. Drives the on-mount prewarm of `Accounts.tsx` so the
   * first user click hits the warm-reopen branch instead of paying a
   * cold load. Updated on rail click + new-account pick. `null` until the
   * user activates a real (non-agent) account at least once.
   */
  lastActiveAccountId: string | null;
  messages: Record<string, IngestedMessage[]>;
  unread: Record<string, number>;
  logs: Record<string, AccountLogEntry[]>;
  /**
   * True while a rail overlay (add-account modal or the right-click context
   * menu) is open. The app rail now lives in the persistent sidebar, while the
   * active provider webview is composited by the chat page — so the rail signals
   * overlay state here and the chat page hides/restores the native webview
   * accordingly (DOM z-index can't paint React overlays above a CEF webview).
   * Transient: not in the persist whitelist.
   */
  overlayOpen: boolean;
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
    id: 'wechat',
    label: 'WeChat Web',
    description: 'Open WeChat in-app for QR sign-in and desktop chat access.',
    serviceUrl: 'https://web.wechat.com/',
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
    id: 'gmail',
    label: 'Gmail',
    description: 'Your Gmail inbox, embedded and observed.',
    serviceUrl: 'https://mail.google.com/mail/u/0/',
  },
  {
    id: 'outlook',
    label: 'Outlook',
    description: 'Outlook / Microsoft 365 mail, embedded in-app.',
    serviceUrl: 'https://outlook.live.com/mail/',
  },
  {
    id: 'instagram',
    label: 'Instagram',
    description: 'Instagram direct messages — DMs and conversations.',
    serviceUrl: 'https://www.instagram.com/direct/inbox/',
  },
  {
    id: 'twitter',
    label: 'X (Twitter)',
    description: 'X / Twitter direct messages.',
    serviceUrl: 'https://x.com/messages/',
  },
  // Google Meet + Zoom are hidden from the picker for now — usage is low
  // and the integrations need more polish before re-surfacing them. Their
  // AccountProvider ids stay in the type union so existing accounts keep
  // rendering correctly.
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
