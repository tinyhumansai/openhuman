import debugFactory from 'debug';
import { useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { trackEvent } from '../../../services/analytics';
import { purgeWebviewAccount } from '../../../services/webviewAccountService';
import {
  addAccount,
  removeAccount,
  setAccountsOverlayOpen,
  setActiveAccount,
  setLastActiveAccount,
} from '../../../store/accountsSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import type { Account, AccountProvider, ProviderDescriptor } from '../../../types/accounts';
import { AGENT_ACCOUNT_ID as AGENT_ID } from '../../../utils/accountsFullscreen';
import AddAccountModal from '../../accounts/AddAccountModal';
import { AgentIcon, ProviderIcon } from '../../accounts/providerIcons';

const debug = debugFactory('layout:sidebar-app-rail');

function makeAccountId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === 'function') return c.randomUUID();
  if (c && typeof c.getRandomValues === 'function') {
    const bytes = new Uint8Array(4);
    c.getRandomValues(bytes);
    const suffix = Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
    return `acct-${Date.now().toString(36)}-${suffix}`;
  }
  return `acct-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

interface RailButtonProps {
  active: boolean;
  onClick: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  tooltip: string;
  analyticsId: string;
  badge?: number;
  children: React.ReactNode;
}

const RailButton = ({
  active,
  onClick,
  onContextMenu,
  tooltip,
  analyticsId,
  badge,
  children,
}: RailButtonProps) => (
  <button
    type="button"
    onClick={onClick}
    onContextMenu={onContextMenu}
    title={tooltip}
    data-analytics-id={analyticsId}
    // Issue #1284 — `hover:z-50` lifts the entire button (and its tooltip
    // child) above sibling rail buttons during hover so the native `title`
    // tooltip isn't trapped under a later sibling's stacking context.
    className={`group relative flex h-9 w-9 flex-none items-center justify-center rounded-lg transition-all hover:z-50 ${
      active
        ? 'bg-primary-500/15 ring-2 ring-primary-500'
        : 'hover:bg-surface-hover hover:scale-105'
    }`}
    aria-label={tooltip}>
    {children}
    {badge && badge > 0 ? (
      <span className="absolute -right-0.5 -top-0.5 flex min-w-[16px] items-center justify-center rounded-full bg-coral-500 px-1 text-[9px] font-semibold text-content-inverted">
        {badge > 99 ? '99+' : badge}
      </span>
    ) : null}
  </button>
);

interface ContextMenuState {
  accountId: string;
  x: number;
  y: number;
}

/**
 * The persistent app rail (agent + connected provider apps + add button),
 * rendered directly in {@link AppSidebar} so it sticks regardless of the active
 * route. Selecting an app sets the active account in Redux and navigates to the
 * chat surface, where the provider webview is composited; the rail itself never
 * unmounts as the user moves between pages.
 */
export default function SidebarAppRail() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const location = useLocation();
  const accountsById = useAppSelector(state => state.accounts.accounts);
  const order = useAppSelector(state => state.accounts.order);
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  const unreadByAccount = useAppSelector(state => state.accounts.unread);
  const [addOpen, setAddOpen] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState | null>(null);

  const accounts: Account[] = useMemo(
    () => order.map(id => accountsById[id]).filter((a): a is Account => Boolean(a)),
    [order, accountsById]
  );

  const connectedProviders = useMemo(
    () => new Set<AccountProvider>(accounts.map(a => a.provider)),
    [accounts]
  );

  const selectedId = activeAccountId ?? AGENT_ID;
  const isAgentSelected = selectedId === AGENT_ID;

  // New-user affordance: when no provider apps are connected yet, the lone
  // dashed "+" tile reads as ambiguous. Expand it into a labelled "Add apps"
  // pill so first-run users understand they can connect more apps; once at
  // least one app is connected, collapse back to the compact icon to reclaim
  // horizontal space in the rail.
  const showAddLabel = accounts.length === 0;

  // The chat page hides its active provider webview while a rail overlay is
  // open (the native CEF view composites above HTML, so React overlays would be
  // painted over). Mirror local overlay state into Redux for it to read.
  const overlayOpen = addOpen || ctxMenu !== null;
  useEffect(() => {
    dispatch(setAccountsOverlayOpen(overlayOpen));
  }, [overlayOpen, dispatch]);

  // Bring the chat surface forward when the user explicitly picks the agent.
  // Provider CEF selection is a high-level rail overlay and intentionally does
  // not mutate the current route.
  const goToChat = () => {
    const onChat = location.pathname === '/chat' || location.pathname.startsWith('/chat/');
    if (!onChat) {
      navigate('/chat');
    }
  };

  const handlePickProvider = (p: ProviderDescriptor) => {
    setAddOpen(false);
    trackEvent('account_connect_start', { provider: p.id });
    const id = makeAccountId();
    const acct: Account = {
      id,
      provider: p.id,
      label: p.label,
      createdAt: new Date().toISOString(),
      status: 'pending',
    };
    dispatch(addAccount(acct));
    dispatch(setActiveAccount(id));
    // Issue #1233 — record this real-account selection in the persisted MRU
    // pointer so the next session can prewarm it.
    dispatch(setLastActiveAccount(id));
  };

  const selectAgent = () => {
    trackEvent('tauri_browser_click', {
      surface: 'sidebar_app_rail',
      action: 'select_agent',
      provider: 'agent',
    });
    dispatch(setActiveAccount(AGENT_ID));
    goToChat();
  };

  const selectAccount = (id: string) => {
    const account = accountsById[id];
    if (account) {
      trackEvent('tauri_browser_click', {
        surface: 'sidebar_app_rail',
        action: 'select_account',
        provider: account.provider,
        account_status: account.status ?? 'unknown',
      });
    }
    dispatch(setActiveAccount(id));
    dispatch(setLastActiveAccount(id));
  };

  const openContextMenu = (accountId: string, e: React.MouseEvent) => {
    e.preventDefault();
    setCtxMenu({ accountId, x: e.clientX, y: e.clientY });
  };

  const handleLogout = async (accountId: string) => {
    setCtxMenu(null);
    const account = accountsById[accountId];
    if (account) {
      trackEvent('tauri_browser_click', {
        surface: 'sidebar_app_rail',
        action: 'disconnect_account',
        provider: account.provider,
        account_status: account.status ?? 'unknown',
      });
    }
    try {
      await purgeWebviewAccount(accountId);
    } catch {
      // Purge failures are already logged by the service; still drop the
      // account from the UI so the user isn't stuck with a zombie icon.
      debug('purge failed for %s — dropping from UI anyway', accountId);
    }
    dispatch(removeAccount({ accountId }));
  };

  // Close the context menu on Escape or any outside click.
  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('mousedown', close);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', close);
      window.removeEventListener('keydown', onKey);
    };
  }, [ctxMenu]);

  return (
    <>
      <div
        data-testid="sidebar-app-rail"
        data-analytics-id="sidebar-app-rail"
        className="scrollbar-hide flex flex-none items-center gap-1.5 overflow-x-auto overflow-y-hidden border-b border-line-subtle px-2 py-2">
        <RailButton
          active={isAgentSelected}
          onClick={selectAgent}
          tooltip={t('accounts.agent')}
          analyticsId="sidebar-app-rail-agent">
          <AgentIcon className="h-5 w-5 rounded-md bg-surface dark:bg-neutral-200" />
        </RailButton>

        {accounts.map(acct => (
          <RailButton
            key={acct.id}
            active={acct.id === selectedId}
            onClick={() => selectAccount(acct.id)}
            onContextMenu={e => openContextMenu(acct.id, e)}
            tooltip={acct.label}
            analyticsId={`sidebar-app-rail-account-${acct.provider}`}
            badge={unreadByAccount[acct.id]}>
            <ProviderIcon provider={acct.provider} className="h-5 w-5 rounded" />
          </RailButton>
        ))}

        <button
          type="button"
          onClick={() => {
            trackEvent('tauri_browser_click', {
              surface: 'sidebar_app_rail',
              action: 'open_add_account',
              provider: 'none',
            });
            setAddOpen(true);
          }}
          data-analytics-id="sidebar-app-rail-add-account"
          data-testid="accounts-add-button"
          className={`group relative flex h-9 flex-none items-center justify-center gap-1.5 rounded-xl border border-dashed border-line-strong text-content-faint transition-colors hover:bg-surface-hover hover:text-content-secondary ${
            showAddLabel ? 'w-auto px-2.5' : 'w-9'
          }`}
          aria-label={t('accounts.addApps')}
          title={t('accounts.addApps')}>
          <svg className="h-4 w-4 flex-none" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
          {showAddLabel && (
            <span className="whitespace-nowrap text-xs font-medium">{t('accounts.addApps')}</span>
          )}
        </button>
      </div>

      <AddAccountModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onPick={handlePickProvider}
        connectedProviders={connectedProviders}
      />

      {ctxMenu && (
        <div
          className="fixed z-50 min-w-[140px] rounded-lg border border-line bg-surface py-1 shadow-strong"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onMouseDown={e => e.stopPropagation()}>
          <button
            type="button"
            data-analytics-id="sidebar-app-rail-disconnect-account"
            onClick={() => void handleLogout(ctxMenu.accountId)}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-coral-600 hover:bg-surface-hover">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
              />
            </svg>
            {t('accounts.disconnect')}
          </button>
        </div>
      )}
    </>
  );
}
