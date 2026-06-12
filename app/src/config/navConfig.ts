/**
 * Single source of truth for bottom-tab-bar navigation entries and the
 * avatar-menu items that appear in the agent-profile popover.
 *
 * This module is pure data — no JSX, no React imports.  Icons are owned by
 * BottomTabBar.tsx and mapped from tab.id.
 */

// ── Tab bar ──────────────────────────────────────────────────────────────────

export interface NavTab {
  /** Stable identifier used for analytics, icon-maps, and walkthrough attrs. */
  id: string;
  /** i18n key resolved by `useT()` in the consuming component. */
  labelKey: string;
  /** Hash-router path this tab navigates to. */
  path: string;
  /** Value of `data-walkthrough` attribute on the rendered button, if any. */
  walkthroughAttr?: string;
}

/**
 * Ordered list of bottom-bar tabs.  Six entries:
 *   home → human → brain → connections → activity → settings
 *
 * The Human tab is a first-class destination again (it was briefly merged into
 * Assistant in IA Phase 6, then restored): `/human` renders the Human page on
 * desktop. The Assistant is NOT in this row — it's the raised center FAB (see
 * `CENTER_TAB`); the Brain sits in the regular row where the Assistant briefly
 * lived. Ids/paths/walkthroughAttrs travel with each tab, so analytics and the
 * walkthrough tour stay attached to the right feature regardless of position.
 */
export const NAV_TABS: NavTab[] = [
  { id: 'home', labelKey: 'nav.home', path: '/home', walkthroughAttr: 'tab-home' },
  { id: 'human', labelKey: 'nav.human', path: '/human', walkthroughAttr: 'tab-human' },
  { id: 'brain', labelKey: 'nav.brain', path: '/brain', walkthroughAttr: 'tab-brain' },
  {
    id: 'connections',
    labelKey: 'nav.connections',
    path: '/connections',
    walkthroughAttr: 'tab-connections',
  },
  { id: 'activity', labelKey: 'nav.activity', path: '/activity', walkthroughAttr: 'tab-activity' },
  { id: 'settings', labelKey: 'nav.settings', path: '/settings', walkthroughAttr: 'tab-settings' },
];

/**
 * The "Assistant" — the app's centerpiece chat surface. Rendered specially by
 * BottomTabBar.tsx as a raised circular button in the dead center of the bar
 * (a notch the button rises out of), NOT as part of the regular `NAV_TABS`
 * row. Kept separate so its special, elevated nature is explicit and the
 * 6-tab invariants for the regular row stay intact. The tab id stays `chat`
 * and walkthroughAttr stays `tab-chat` for back-compat with analytics and the
 * walkthrough tour.
 */
export const CENTER_TAB: NavTab = {
  id: 'chat',
  labelKey: 'nav.assistant',
  path: '/chat',
  walkthroughAttr: 'tab-chat',
};

// ── Avatar / account menu ─────────────────────────────────────────────────────

/**
 * Determines how the menu item is activated.
 * - `navigate` — internal `react-router-dom` navigation to `target`.
 * - `openUrl`  — opens `target` in the system browser via `openUrl()`.
 */
export type AvatarMenuItemKind = 'navigate' | 'openUrl';

export interface AvatarMenuItem {
  /** Stable identifier. */
  id: string;
  /** i18n key resolved by the consuming component. */
  labelKey: string;
  /** Navigation destination or external URL, depending on `kind`. */
  target: string;
  /** How the item is activated. */
  kind: AvatarMenuItemKind;
  /**
   * When `true`, the item should only be shown for non-local (cloud) sessions.
   * Cloud-gated items: billing, rewards, invites.
   */
  cloudOnly?: boolean;
}

/**
 * Avatar dropdown menu items shown beneath the agent-profile list.
 * Order: Account → Billing → Rewards → Invites → Wallet.
 */
export const AVATAR_MENU_ITEMS: AvatarMenuItem[] = [
  {
    id: 'account',
    labelKey: 'nav.avatarMenu.account',
    target: '/settings/account',
    kind: 'navigate',
  },
  {
    id: 'billing',
    labelKey: 'nav.avatarMenu.billing',
    // Resolved at runtime via BILLING_DASHBOARD_URL; placeholder keeps typing clean.
    target: 'https://tinyhumans.ai/dashboard',
    kind: 'openUrl',
    cloudOnly: true,
  },
  {
    id: 'rewards',
    labelKey: 'nav.avatarMenu.rewards',
    target: '/rewards',
    kind: 'navigate',
    cloudOnly: true,
  },
  {
    id: 'invites',
    labelKey: 'nav.avatarMenu.invites',
    target: '/invites',
    kind: 'navigate',
    cloudOnly: true,
  },
  {
    id: 'wallet',
    labelKey: 'nav.avatarMenu.wallet',
    target: '/settings/wallet-balances',
    kind: 'navigate',
  },
];
