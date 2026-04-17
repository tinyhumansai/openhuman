/** Sentinel id for the always-present agent entry in the Accounts page. */
export const AGENT_ACCOUNT_ID = '__agent__';

/**
 * True when the route + selection means the app should render the
 * embedded webview edge-to-edge (no bottom tab bar, no reserved padding).
 * The Agent entry keeps the regular chrome visible so the user still has
 * access to the tab bar while chatting.
 */
export function isAccountsFullscreen(
  pathname: string,
  activeAccountId: string | null | undefined
): boolean {
  if (!pathname.startsWith('/chat')) return false;
  // Agent selected (or nothing selected → defaults to Agent) keeps chrome.
  if (!activeAccountId || activeAccountId === AGENT_ACCOUNT_ID) return false;
  return true;
}
