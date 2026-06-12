import { useEffect, useMemo, useState } from 'react';

import AddAccountModal from '../components/accounts/AddAccountModal';
import { AgentIcon, ProviderIcon } from '../components/accounts/providerIcons';
import WebviewHost from '../components/accounts/WebviewHost';
import {
  CustomGifMascot,
  getMascotPalette,
  hexToArgbInt,
  MascotChipAvatar,
  RiveMascot,
} from '../features/human/Mascot';
import { useHumanMascot } from '../features/human/useHumanMascot';
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
import {
  selectCustomMascotGifUrl,
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
} from '../store/mascotSlice';
import type { Account, AccountProvider, ProviderDescriptor } from '../types/accounts';
import { AGENT_ACCOUNT_ID as AGENT_ID } from '../utils/accountsFullscreen';
import Conversations, { AgentChatPanel } from './Conversations';

// Persistence key for face-toggle state across sessions.
const FACE_MODE_KEY = 'chat.faceMode';

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
    data-analytics-id={analyticsId}
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

/**
 * Mascot + TTS panel rendered in face mode (right column of the Assistant
 * surface).  Extracted as a separate component so its hooks only run when
 * face mode is on — keeps the main Accounts component lean when the toggle
 * is off.
 *
 * Phase 6 — reuses the exact same mascot subcomponents and useHumanMascot
 * hook from features/human/ rather than duplicating any logic.
 */
const FaceModePanel = () => {
  const { t } = useT();
  const [speakReplies, setSpeakReplies] = useState<boolean>(() => {
    try {
      const raw = window.localStorage.getItem('human.speakReplies');
      return raw === null ? true : raw === '1';
    } catch {
      return true;
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem('human.speakReplies', speakReplies ? '1' : '0');
    } catch {
      // localStorage may be unavailable in sandboxed contexts.
    }
  }, [speakReplies]);

  const { face, visemeCode } = useHumanMascot({ speakReplies });
  const mascotColor = useAppSelector(selectMascotColor);
  const customPrimary = useAppSelector(selectCustomPrimaryColor);
  const customSecondary = useAppSelector(selectCustomSecondaryColor);
  const customMascotGifUrl = useAppSelector(selectCustomMascotGifUrl);

  const palette = getMascotPalette(mascotColor);
  const primaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customPrimary : palette.bodyFill),
    [mascotColor, customPrimary, palette]
  );
  const secondaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customSecondary : palette.neckShadowColor),
    [mascotColor, customSecondary, palette]
  );

  return (
    <aside
      className="flex min-w-0 flex-1 flex-col items-center justify-center gap-4 bg-stone-50 dark:bg-neutral-900/60 rounded-2xl border border-stone-200/70 dark:border-neutral-800/70 my-3 mr-0 py-4 px-3 overflow-hidden"
      data-testid="face-mode-panel">
      {/* Mascot stage — the dominant element of the "Talk to Tiny" surface */}
      <div className="relative w-full max-w-[460px] aspect-square">
        {customMascotGifUrl ? (
          <CustomGifMascot src={customMascotGifUrl} face={face} />
        ) : (
          <RiveMascot
            face={face}
            primaryColor={primaryColor}
            secondaryColor={secondaryColor}
            visemeCode={visemeCode}
          />
        )}
      </div>

      {/* TTS / speak-replies toggle */}
      <label className="inline-flex cursor-pointer select-none items-center gap-2 rounded-full border border-stone-300 dark:border-neutral-700 bg-white/80 dark:bg-neutral-900/80 px-3 py-1.5 text-xs text-stone-700 dark:text-neutral-200 shadow-soft backdrop-blur-sm">
        <input
          type="checkbox"
          checked={speakReplies}
          onChange={e => setSpeakReplies(e.target.checked)}
          className="cursor-pointer"
          data-testid="speak-replies-toggle"
        />
        {t('voice.pushToTalk')}
      </label>
    </aside>
  );
};

const Accounts = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const accountsById = useAppSelector(state => state.accounts.accounts);
  const order = useAppSelector(state => state.accounts.order);
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);
  const unreadByAccount = useAppSelector(state => state.accounts.unread);
  // Mascot identity — drives the avatar on the "Talk to Tiny" chip so the
  // button advertises the user's actual character (colour / custom GIF) rather
  // than a generic emoji.
  const mascotColor = useAppSelector(selectMascotColor);
  const customPrimary = useAppSelector(selectCustomPrimaryColor);
  const customMascotGifUrl = useAppSelector(selectCustomMascotGifUrl);
  const [addOpen, setAddOpen] = useState(false);
  const [ctxMenu, setCtxMenu] = useState<ContextMenuState | null>(null);

  // Face-mode toggle — persists across sessions.  Face mode only affects the
  // agent-chat surface (external webview accounts ignore it).
  const [faceMode, setFaceMode] = useState<boolean>(() => {
    try {
      return window.localStorage.getItem(FACE_MODE_KEY) === '1';
    } catch {
      return false;
    }
  });

  const toggleFaceMode = () => {
    setFaceMode(prev => {
      const next = !prev;
      try {
        window.localStorage.setItem(FACE_MODE_KEY, next ? '1' : '0');
      } catch {
        // Swallow storage errors.
      }
      return next;
    });
  };

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

  const selectAgent = () => {
    trackEvent('tauri_browser_click', {
      surface: 'chat_right_sidebar',
      action: 'select_agent',
      provider: 'agent',
    });
    dispatch(setActiveAccount(AGENT_ID));
  };
  const selectAccount = (id: string) => {
    const account = accountsById[id];
    if (account) {
      trackEvent('tauri_browser_click', {
        surface: 'chat_right_sidebar',
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
        surface: 'chat_right_sidebar',
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
    <div
      // `h-full` makes this page fill the shell's content box, which bypasses
      className="relative flex h-full gap-3 overflow-hidden"
      data-testid="accounts-page"
      data-analytics-id="chat-right-sidebar">
      {/* Narrow icon rail — always rendered. */}
      <aside className="z-30 flex w-16 flex-none flex-col items-center gap-2 bg-white/60 dark:bg-neutral-900/60 py-3 backdrop-blur-md my-3 ml-3 rounded-2xl border border-stone-200/70 dark:border-neutral-800/70 shadow-soft">
        <RailButton
          active={isAgentSelected}
          onClick={selectAgent}
          tooltip={t('accounts.agent')}
          analyticsId="chat-right-sidebar-agent">
          <AgentIcon className="h-9 w-9 rounded-lg bg-white dark:bg-neutral-200" />
        </RailButton>

        {accounts.map(acct => (
          <RailButton
            key={acct.id}
            active={acct.id === selectedId}
            onClick={() => selectAccount(acct.id)}
            onContextMenu={e => openContextMenu(acct.id, e)}
            tooltip={acct.label}
            analyticsId={`chat-right-sidebar-account-${acct.provider}`}
            badge={unreadByAccount[acct.id]}>
            <ProviderIcon provider={acct.provider} className="h-8 w-8 rounded-md" />
          </RailButton>
        ))}

        <button
          type="button"
          onClick={() => {
            trackEvent('tauri_browser_click', {
              surface: 'chat_right_sidebar',
              action: 'open_add_account',
              provider: 'none',
            });
            setAddOpen(true);
          }}
          data-analytics-id="chat-right-sidebar-add-account"
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

      {/* Floating "Talk to Tiny" face-mode toggle. Kept out of the layout flow
          (absolute) so it never steals vertical space from the chat composer —
          the previous in-flow header strip pushed the input below the viewport. */}
      {isAgentSelected && (
        <button
          type="button"
          onClick={toggleFaceMode}
          data-testid="face-toggle-button"
          aria-pressed={faceMode}
          className={`group absolute right-4 top-4 z-40 inline-flex items-center gap-2 rounded-full border py-1.5 pl-1.5 pr-3 text-xs font-medium shadow-soft backdrop-blur-sm transition-colors ${
            faceMode
              ? 'border-primary-300 bg-primary-50/90 text-primary-700 dark:bg-primary-900/40 dark:text-primary-200'
              : 'border-stone-300/80 bg-white/90 text-stone-600 hover:border-primary-300 hover:text-primary-600 dark:border-neutral-700/80 dark:bg-neutral-900/90 dark:text-neutral-300 dark:hover:text-primary-300'
          }`}
          aria-label={faceMode ? t('assistant.faceMode.turnOff') : t('assistant.faceMode.turnOn')}>
          {/* The mascot wakes up (wiggles) on hover/focus; static at rest. */}
          <span className="origin-bottom transition-transform group-hover:animate-wiggle group-focus-visible:animate-wiggle">
            <MascotChipAvatar
              color={mascotColor}
              customPrimary={customPrimary}
              gifUrl={customMascotGifUrl}
              size={22}
            />
          </span>
          {faceMode ? t('assistant.faceMode.on') : t('assistant.faceMode.off')}
        </button>
      )}

      {/* Main pane
          In face mode (agent selected), the layout is a horizontal split:
          the chat panel on the left and the mascot panel on the right.
          Face mode is ignored when an external webview account is active. */}
      <main
        className={`flex min-w-0 flex-1 gap-3 ${isAgentSelected && faceMode ? 'flex-row' : 'flex-col'}`}>
        {isAgentSelected ? (
          <>
            {/* Agent chat — face mode uses sidebar variant to avoid a second
                thread list; normal mode uses the full-page variant (AgentChatPanel). */}
            <div
              className={`flex min-h-0 min-w-0 flex-col ${faceMode ? 'w-[360px] flex-none' : 'flex-1'}`}>
              {faceMode ? (
                // Face mode: mascot sidebar chat. The toggle floats on the page
                // root (see below) so it never steals height from the composer.
                // `min-h-0` lets the inner message list scroll instead of growing
                // and pushing the composer off-screen.
                <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-stone-200/70 dark:border-neutral-800/70 my-3 mr-0">
                  <Conversations variant="sidebar" />
                </div>
              ) : (
                // `min-h-0` is required so the chat's internal message list owns
                // the overflow (scrolls) rather than expanding and shoving the
                // composer below the viewport on long threads.
                <div className="min-h-0 flex-1 overflow-hidden">
                  <AgentChatPanel />
                </div>
              )}
            </div>
            {/* Mascot + TTS panel — only visible in face mode */}
            {faceMode && <FaceModePanel />}
          </>
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
            type="button"
            data-analytics-id="chat-right-sidebar-disconnect-account"
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
