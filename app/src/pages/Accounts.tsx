import debugFactory from 'debug';
import { useEffect, useMemo, useState } from 'react';
import { useParams } from 'react-router-dom';

import {
  CustomGifMascot,
  getMascotPalette,
  hexToArgbInt,
  RiveMascot,
} from '../features/human/Mascot';
import { useHumanMascot } from '../features/human/useHumanMascot';
import { usePrewarmMostRecentAccount } from '../hooks/usePrewarmMostRecentAccount';
import { useT } from '../lib/i18n/I18nContext';
import { startWebviewAccountService } from '../services/webviewAccountService';
import { setActiveAccount } from '../store/accountsSlice';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  selectCustomMascotGifUrl,
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
} from '../store/mascotSlice';
import type { Account } from '../types/accounts';
import { AGENT_ACCOUNT_ID as AGENT_ID } from '../utils/accountsFullscreen';
import Conversations, { AgentChatPanel } from './Conversations';

// Persistence key for face-toggle state across sessions.
const FACE_MODE_KEY = 'chat.faceMode';
const debug = debugFactory('accounts');

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
      className="flex min-w-0 flex-1 flex-col items-center justify-center gap-4 bg-surface-muted dark:bg-surface/60 rounded-2xl border border-line/70 dark:border-line/70 my-3 mr-0 py-4 px-3 overflow-hidden"
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
      <label className="inline-flex cursor-pointer select-none items-center gap-2 rounded-full border border-line-strong bg-surface/80 px-3 py-1.5 text-xs text-content-secondary shadow-soft backdrop-blur-sm">
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
  const dispatch = useAppDispatch();
  const { threadId } = useParams<{ threadId?: string }>();
  const accountsById = useAppSelector(state => state.accounts.accounts);
  const order = useAppSelector(state => state.accounts.order);
  const activeAccountId = useAppSelector(state => state.accounts.activeAccountId);

  const [faceMode] = useState<boolean>(() => {
    try {
      const stored = window.localStorage.getItem(FACE_MODE_KEY);
      if (stored === '1') {
        window.localStorage.removeItem(FACE_MODE_KEY);
      }
      return false;
    } catch {
      return false;
    }
  });

  useEffect(() => {
    startWebviewAccountService();
  }, []);

  useEffect(() => {
    if (!threadId) return;
    debug('[chat][route] selecting agent for thread route thread=%s', threadId);
    dispatch(setActiveAccount(AGENT_ID));
  }, [dispatch, threadId]);

  // Issue #1233 — prewarm the MRU account once on mount so its CEF profile
  // and provider page are warm before the user actually clicks the rail.
  // Skipped for power users with many accounts to bound the spawn cost.
  // The accounts array snapshot is captured by the hook at first render.
  const accounts: Account[] = useMemo(
    () => order.map(id => accountsById[id]).filter((a): a is Account => Boolean(a)),
    [order, accountsById]
  );
  usePrewarmMostRecentAccount({ accounts, accountsById, activeAccountId });

  const selectedId = activeAccountId ?? AGENT_ID;
  const isAgentSelected = selectedId === AGENT_ID;

  return (
    <div
      // `h-full` makes this page fill the shell's content box edge-to-edge.
      className="relative flex h-full overflow-hidden"
      data-testid="accounts-page"
      data-analytics-id="chat-right-sidebar">
      {/* "Talk to Tiny" face-mode toggle — hidden (kept for potential re-enable). */}

      {/* Main pane. In face mode (agent selected) it's a horizontal split with
          the mascot panel. Connected-app CEF views are hosted above this page
          by the desktop shell, so the routed chat panel must only mount while
          the agent is active; its thread effects own `/chat/:threadId`. */}
      {isAgentSelected && faceMode ? (
        <main className="flex min-w-0 flex-1 flex-row gap-3">
          <div className="flex min-h-0 w-[360px] flex-none flex-col">
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-line/70 dark:border-line/70 my-3 mr-0">
              <Conversations variant="sidebar" />
            </div>
          </div>
          <FaceModePanel />
        </main>
      ) : (
        <main className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
          {isAgentSelected && (
            <div className="min-h-0 flex-1 overflow-hidden">
              <AgentChatPanel />
            </div>
          )}
        </main>
      )}
    </div>
  );
};

export default Accounts;
