import { useEffect, useMemo, useState } from 'react';

import AddAccountModal from '../components/accounts/AddAccountModal';
import { AgentIcon, ProviderIcon } from '../components/accounts/providerIcons';
// import RespondQueuePanel from '../components/accounts/RespondQueuePanel';
import WebviewHost from '../components/accounts/WebviewHost';
// [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
// import { isWelcomeLocked } from '../lib/coreState/store';
// [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
// import { useCoreState } from '../providers/CoreStateProvider';
import { usePrewarmMostRecentAccount } from '../hooks/usePrewarmMostRecentAccount';
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
      active ? 'bg-primary-50 ring-2 ring-primary-500' : 'hover:bg-stone-100 hover:scale-105'
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
  const dispatch = useAppDispatch();
  const accountsById = useAppSelector(state => state.accounts.accounts);
  const order = useAppSelector(state => state.accounts.order);
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  const unreadByAccount = useAppSelector(state => state.accounts.unread);
  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // const { snapshot } = useCoreState();
  // const welcomeLocked = isWelcomeLocked(snapshot);
  // Respond-queue selectors disabled while RespondQueuePanel is hidden.
  // const respondQueue = useAppSelector(state => state.providerSurfaces.queue);
  // const respondQueueCount = useAppSelector(state => state.providerSurfaces.count);
  // const respondQueueStatus = useAppSelector(state => state.providerSurfaces.status);
  // const respondQueueError = useAppSelector(state => state.providerSurfaces.error);

  const [addOpen, setAddOpen] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState | null>(null);

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

  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // Welcome lockdown (#883) — force the Agent pane while the welcome
  // conversation is in progress so the user cannot jump to a connected
  // account webview. The rail is hidden below, so this is belt-and-
  // suspenders in case an external caller toggles `activeAccountId`.
  // useEffect(() => {
  //   if (welcomeLocked && activeAccountId !== AGENT_ID) {
  //     dispatch(setActiveAccount(AGENT_ID));
  //   }
  // }, [welcomeLocked, activeAccountId, dispatch]);

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

  // [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
  // While welcome-locked, derive the effective selection directly from
  // `welcomeLocked` so the first paint after a lock flip never renders the
  // stale `activeAccountId`. The post-paint `useEffect` above still
  // syncs Redux so other consumers observe the forced selection.
  // const selectedId = welcomeLocked ? AGENT_ID : (activeAccountId ?? AGENT_ID);
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
    <div className="relative flex h-full gap-3 overflow-hidden">
      {/* Narrow icon rail — always rendered. */}
      {/* [#1123] welcomeLocked guard removed — welcome-agent onboarding replaced by Joyride walkthrough */}
      <aside className="z-30 flex w-16 flex-none flex-col items-center gap-2 bg-white/60 py-3 backdrop-blur-md my-3 ml-3 rounded-2xl border border-stone-200/70 shadow-soft">
        <RailButton active={isAgentSelected} onClick={selectAgent} tooltip="Agent">
          <AgentIcon className="h-9 w-9 rounded-lg" />
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
          className="group relative mt-2 flex h-11 w-11 items-center justify-center rounded-xl border border-dashed border-stone-300 text-stone-400 hover:z-50 hover:bg-stone-50 hover:text-stone-600"
          aria-label="Add app">
          <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
          </svg>
          {/* Issue #1284 — see RailButton for why the tooltip sits below
              the icon instead of to the right. */}
          <span className="pointer-events-none absolute left-1/2 top-full mt-1 -translate-x-1/2 whitespace-nowrap rounded-md bg-stone-900 px-2 py-1 text-xs text-white opacity-0 shadow-md transition-opacity group-hover:opacity-100 z-50">
            Add app
          </span>
        </button>
      </aside>

      {/* Main pane */}
      <main className="flex min-w-0 flex-1 flex-col">
        {isAgentSelected ? (
          <div className="flex h-full min-w-0">
            <div className="min-w-0 flex-1">
              <AgentChatPanel />
            </div>
            {/* Respond queue side panel hidden for now — bring back when
                the cross-provider surface is ready to ship. */}
            {/* <RespondQueuePanel
              items={respondQueue}
              count={respondQueueCount}
              status={respondQueueStatus}
              error={respondQueueError}
              onRefresh={() => {
                void dispatch(fetchRespondQueue());
              }}
            /> */}
          </div>
        ) : active ? (
          <div className="flex-1 py-3 pr-3">
            <WebviewHost accountId={active.id} provider={active.provider} />
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center text-sm text-stone-400">
            Select or add an app to get started.
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
          className="fixed z-50 min-w-[140px] rounded-lg border border-stone-200 bg-white py-1 shadow-strong"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onMouseDown={e => e.stopPropagation()}>
          <button
            onClick={() => void handleLogout(ctxMenu.accountId)}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-coral-600 hover:bg-stone-100">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
              />
            </svg>
            Logout
          </button>
        </div>
      )}
    </div>
  );
};

export default Accounts;
