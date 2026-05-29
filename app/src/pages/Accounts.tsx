import { useEffect, useMemo, useState } from 'react';

import AddAccountModal from '../components/accounts/AddAccountModal';
import { AgentIcon, ProviderIcon } from '../components/accounts/providerIcons';
import RespondQueuePanel from '../components/accounts/RespondQueuePanel';
import WebviewHost from '../components/accounts/WebviewHost';
import { usePrewarmMostRecentAccount } from '../hooks/usePrewarmMostRecentAccount';
import { useT } from '../lib/i18n/I18nContext';
import { trackEvent } from '../services/analytics';
import {
  hideWebviewAccount,
  purgeWebviewAccount,
  showWebviewAccount,
  startWebviewAccountService,
} from '../services/webviewAccountService';
import {
  addAccount,
  removeAccount,
  setActiveAccount,
  setLastActiveAccount,
} from '../store/accountsSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { fetchRespondQueue } from '../store/providerSurfaceSlice';
import type { Account, AccountProvider, ProviderDescriptor } from '../types/accounts';
import { AGENT_ACCOUNT_ID as AGENT_ID } from '../utils/accountsFullscreen';
import { AgentChatPanel } from './Conversations';

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
  badge?: number;
  children: React.ReactNode;
}

const RailButton = ({
  active,
  onClick,
  onContextMenu,
  tooltip,
  badge,
  children,
}: RailButtonProps) => (
  <button
    onClick={onClick}
    onContextMenu={onContextMenu}
    // Issue #1284 — `hover:z-50` lifts the entire button (and its tooltip
    // child) above sibling rail buttons during hover. Without it, the
    // `hover:scale-105` transform on a non-active button establishes its
    // own stacking context that traps the tooltip's `z-50` inside it,
    // and a later sibling button (next in DOM order) paints over the
    // tooltip rectangle. Belt-and-suspenders for the active-button case
    // too, where ring-2 + bg-primary-50 don't transform but the lifted
    // z still helps tooltips render cleanly above neighbours.
    className={`group relative flex h-11 w-11 items-center justify-center rounded-xl transition-all hover:z-50 ${
      active
        ? 'bg-primary-50 ring-2 ring-primary-500'
        : 'hover:bg-stone-100 dark:hover:bg-neutral-800/60 hover:scale-105'
    }`}
    aria-label={tooltip}>
    {children}
    {badge && badge > 0 ? (
      <span className="absolute -right-0.5 -top-0.5 flex min-w-[16px] items-center justify-center rounded-full bg-coral-500 px-1 text-[9px] font-semibold text-white">
        {badge > 99 ? '99+' : badge}
      </span>
    ) : null}
    {/* Issue #1284 — tooltip sits BELOW the icon (`top-full`) so it stays
        inside the HTML-only rail region. The native CEF webview is
        composited above the HTML layer to the right of the rail, so a
        right-anchored tooltip is hidden behind the webview the moment a
        provider is open and DOM z-index can't lift it. Below-icon keeps
        the tooltip near the cursor and never blocks the icon being
        hovered (it briefly overlays the next icon down, which clears as
        soon as the user moves the cursor). */}
    <span className="pointer-events-none absolute left-1/2 top-full mt-1 -translate-x-1/2 whitespace-nowrap rounded-md bg-stone-900 px-2 py-1 text-xs text-white opacity-0 shadow-md transition-opacity group-hover:opacity-100 z-50">
      {tooltip}
    </span>
  </button>
);

interface ContextMenuState {
  accountId: string;
  x: number;
  y: number;
}

const Accounts = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const accountsById = useAppSelector(state => state.accounts.accounts);
  const order = useAppSelector(state => state.accounts.order);
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  const unreadByAccount = useAppSelector(state => state.accounts.unread);
  // Respond-queue selectors enabled
  const respondQueue = useAppSelector(state => state.providerSurfaces.queue);
  const respondQueueCount = useAppSelector(state => state.providerSurfaces.count);
  const respondQueueStatus = useAppSelector(state => state.providerSurfaces.status);
  const respondQueueError = useAppSelector(state => state.providerSurfaces.error);

  const [addOpen, setAddOpen] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState | null>(null);
  // Respond Queue is a power-user surface (pending replies from connected
  // accounts). Hidden by default so the agent chat gets full width; users
  // can toggle it open from the header. (#XXXX)
  const [respondQueueOpen, setRespondQueueOpen] = useState(false);

  useEffect(() => {
    startWebviewAccountService();
  }, []);

  // Issue #1233 — prewarm the MRU account once on mount so its CEF profile
  // and provider page are warm before the user actually clicks the rail.
  // Skipped for power users with many accounts to bound the spawn cost.
  // The accounts array snapshot is captured by the hook at first render.
  const accounts: Account[] = useMemo(
    () => order.map(id => accountsById[id]).filter((a): a is Account => Boolean(a)),
    [order, accountsById]
  );
  usePrewarmMostRecentAccount({ accounts, accountsById, activeAccountId });

  useEffect(() => {
    void dispatch(fetchRespondQueue());
    const id = window.setInterval(() => {
      void dispatch(fetchRespondQueue({ silent: true }));
    }, 10_000);
    return () => window.clearInterval(id);
  }, [dispatch]);

  const connectedProviders = useMemo(
    () => new Set<AccountProvider>(accounts.map(a => a.provider)),
    [accounts]
  );

  const selectedId = activeAccountId ?? AGENT_ID;
  const active = selectedId === AGENT_ID ? null : (accountsById[selectedId] ?? null);
  const isAgentSelected = selectedId === AGENT_ID;

  // The child Tauri webview is a native view composited above the HTML
  // canvas, so DOM z-index can't put React overlays on top of it. Hide
  // the active webview while any overlay (add-account modal or the
  // right-click context menu) is open and restore it on close. No-op
  // when the agent pane is selected (pure HTML).
  const activeId = active?.id ?? null;
  const overlayOpen = addOpen || ctxMenu !== null;
  useEffect(() => {
    if (!activeId) return;
    if (overlayOpen) {
      void hideWebviewAccount(activeId);
    } else {
      void showWebviewAccount(activeId);
    }
  }, [overlayOpen, activeId]);

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
    // Issue #1233 — record this real-account selection in the persisted
    // MRU pointer so the next session can prewarm it. Agent selections
    // never reach this code path (separate `selectAgent` callback below).
    dispatch(setLastActiveAccount(id));
  };

  const selectAgent = () => dispatch(setActiveAccount(AGENT_ID));
  const selectAccount = (id: string) => {
    dispatch(setActiveAccount(id));
    dispatch(setLastActiveAccount(id));
  };

  const openContextMenu = (accountId: string, e: React.MouseEvent) => {
    e.preventDefault();
    setCtxMenu({ accountId, x: e.clientX, y: e.clientY });
  };

  const handleLogout = async (accountId: string) => {
    setCtxMenu(null);
    try {
      await purgeWebviewAccount(accountId);
    } catch {
      // Purge failures are already logged by the service; still drop the
      // account from the UI so the user isn't stuck with a zombie icon.
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
    <div className="relative flex h-full gap-3 overflow-hidden" data-testid="accounts-page">
      {/* Narrow icon rail — always rendered. */}
      <aside className="z-30 flex w-16 flex-none flex-col items-center gap-2 bg-white/60 dark:bg-neutral-900/60 py-3 backdrop-blur-md my-3 ml-3 rounded-2xl border border-stone-200/70 dark:border-neutral-800/70 shadow-soft">
        <RailButton active={isAgentSelected} onClick={selectAgent} tooltip={t('accounts.agent')}>
          <AgentIcon className="h-9 w-9 rounded-lg bg-white dark:bg-neutral-200" />
        </RailButton>

        {accounts.map(acct => (
          <RailButton
            key={acct.id}
            active={acct.id === selectedId}
            onClick={() => selectAccount(acct.id)}
            onContextMenu={e => openContextMenu(acct.id, e)}
            tooltip={acct.label}
            badge={unreadByAccount[acct.id]}>
            <ProviderIcon provider={acct.provider} className="h-8 w-8 rounded-md" />
          </RailButton>
        ))}

        <button
          onClick={() => setAddOpen(true)}
          data-testid="accounts-add-button"
          className="group relative mt-2 flex h-11 w-11 items-center justify-center rounded-xl border border-dashed border-stone-300 dark:border-neutral-700 text-stone-400 dark:text-neutral-500 hover:z-50 hover:bg-stone-50 dark:hover:bg-neutral-800/60 hover:text-stone-600 dark:hover:text-neutral-300"
          aria-label={t('accounts.addAccount')}>
          <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
          {/* Issue #1284 — see RailButton for why the tooltip sits below
              the icon instead of to the right. */}
          <span className="pointer-events-none absolute left-1/2 top-full mt-1 -translate-x-1/2 whitespace-nowrap rounded-md bg-stone-900 px-2 py-1 text-xs text-white opacity-0 shadow-md transition-opacity group-hover:opacity-100 z-50">
            {t('accounts.addAccount')}
          </span>
        </button>
      </aside>

      {/* Main pane */}
      <main className="flex min-w-0 flex-1 flex-col">
        {isAgentSelected ? (
          <div className="flex h-full min-w-0">
            <div className="relative min-w-0 flex-1">
              <AgentChatPanel />
              {/* Floating toggle to reveal the Respond Queue side panel.
                  Pinned top-right inside the chat pane so it doesn't shift
                  layout when the panel opens/closes. */}
              <button
                type="button"
                onClick={() => setRespondQueueOpen(prev => !prev)}
                className="absolute right-4 top-4 z-10 inline-flex items-center gap-1 rounded-full border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1 text-[11px] font-medium text-stone-600 dark:text-neutral-300 shadow-soft hover:bg-stone-50 dark:hover:bg-neutral-800/60"
                title={
                  respondQueueOpen
                    ? t('accounts.respondQueue.hide')
                    : t('accounts.respondQueue.show')
                }>
                <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
                  />
                </svg>
                <span>
                  {t('accounts.respondQueue.title')}
                  {respondQueueCount > 0 && ` (${respondQueueCount})`}
                </span>
              </button>
            </div>
            {/* Respond queue side panel — hidden by default; reveals via toggle */}
            {respondQueueOpen && (
              <RespondQueuePanel
                items={respondQueue}
                count={respondQueueCount}
                status={respondQueueStatus}
                error={respondQueueError}
                onRefresh={() => {
                  void dispatch(fetchRespondQueue());
                }}
              />
            )}
          </div>
        ) : active ? (
          <div className="flex-1 py-3 pr-3">
            <WebviewHost accountId={active.id} provider={active.provider} />
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center text-sm text-stone-400 dark:text-neutral-500">
            {t('accounts.noAccounts')}
          </div>
        )}
      </main>

      <AddAccountModal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onPick={handlePickProvider}
        connectedProviders={connectedProviders}
      />

      {ctxMenu && (
        <div
          className="fixed z-50 min-w-[140px] rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 py-1 shadow-strong"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onMouseDown={e => e.stopPropagation()}>
          <button
            onClick={() => void handleLogout(ctxMenu.accountId)}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-coral-600 hover:bg-stone-100 dark:hover:bg-neutral-800/60">
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
    </div>
  );
};

export default Accounts;
