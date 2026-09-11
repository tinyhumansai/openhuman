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
  /**
   * Hide the entry unless the session is a resolved cloud session. Applied by
   * both rails through `useCloudNavGate()`; see that hook for why the gate is
   * three terms rather than a token check.
   */
  cloudOnly?: boolean;
}

/**
 * Ordered list of sidebar nav entries:
 *   chat → brain → flows → connections → rewards
 *
 * Human has no primary tab: `/human` is reached from the chat composer, whose
 * primary button becomes the mascot when there is nothing to send (see
 * `ComposerIdleAction` in `AssistantUiChat`), so a sidebar row would be a
 * second door to the same place.
 *
 * Settings has no primary tab — it's reached via the gear icon in the sidebar
 * header. Chat is the default landing and the merged Home surface: its empty
 * "new window" state shows the former Home greeting + banners.
 *
 * Human and Chat both surface the mascot, deliberately. `/human` is the
 * dedicated full-bleed mascot stage with a right-rail chat; `/chat` carries the
 * same mascot docked on the composer, expandable in place. They share one set of
 * mascot preferences (colour, voice, speak-replies) via `mascotSlice`, so the
 * two never disagree. Ids/paths/walkthroughAttrs travel with each tab so
 * analytics and the walkthrough tour stay attached to the right feature
 * regardless of position.
 */
export const NAV_TABS: NavTab[] = [
  { id: 'chat', labelKey: 'nav.chat', path: '/chat', walkthroughAttr: 'tab-chat' },
  { id: 'brain', labelKey: 'nav.brain', path: '/brain', walkthroughAttr: 'tab-brain' },
  { id: 'flows', labelKey: 'nav.flows', path: '/flows', walkthroughAttr: 'tab-flows' },
  {
    id: 'connections',
    labelKey: 'nav.connections',
    path: '/connections',
    walkthroughAttr: 'tab-connections',
  },
  // Rewards was a footer row beside Feedback; it is a primary destination now,
  // directly below Connections. The cloud gate travelled with it — a local
  // session still never sees it, because the page has nothing to show one.
  {
    id: 'rewards',
    labelKey: 'nav.rewards',
    path: '/rewards',
    walkthroughAttr: 'tab-rewards',
    cloudOnly: true,
  },
  // Settings is reached via the gear icon in the sidebar header, so it no
  // longer has its own primary nav tab. Feedback lives in a slim footer row
  // pinned just above the status bar (see AppSidebar).
];

// ── Avatar / account menu ─────────────────────────────────────────────────────

/**
 * Determines how the menu item is activated.
 * - `navigate` — internal `react-router-dom` navigation to `target`.
 * - `openUrl`  — opens `target` in the system browser via `openUrl()`.
 */
type AvatarMenuItemKind = 'navigate' | 'openUrl';

interface AvatarMenuItem {
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
 * Order: Account → Billing → Invites → Wallet.
 *
 * Rewards is not here: it is a primary `NAV_TABS` destination now, and one
 * door per surface is the point of moving it.
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
    target: '/settings/billing',
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
