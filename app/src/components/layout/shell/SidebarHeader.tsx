import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { settingsNavState } from '../../settings/modal/settingsOverlay';
import { Tooltip } from '../../ui';
import { useRootSidebar } from './RootShellLayout';
import { useHomeNav } from './useHomeNav';

const ICON_BTN =
  'flex h-7 w-7 flex-none items-center justify-center rounded-md text-stone-500 transition-colors hover:bg-stone-100 hover:text-stone-700 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-200';

/**
 * Thin utility header at the top of the root sidebar: jump Home, open the
 * wallet, open Settings, and collapse the sidebar. Language is chosen from
 * Settings, not here.
 */
export default function SidebarHeader() {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const { hide } = useRootSidebar();
  const handleHome = useHomeNav();

  return (
    <div className="flex items-center justify-between gap-1 px-2 py-1.5">
      <Tooltip label={t('nav.home')}>
        <button
          type="button"
          onClick={handleHome}
          className={ICON_BTN}
          data-analytics-id="sidebar-header-home"
          aria-label={t('nav.home')}>
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-4 0a2 2 0 01-2-2v-4a2 2 0 012-2h2a2 2 0 012 2v4a2 2 0 01-2 2h-2z"
            />
          </svg>
        </button>
      </Tooltip>

      <div className="flex items-center gap-0.5">
        {/* Wallet shortcut — one-click access to wallet balances. */}
        <Tooltip label={t('nav.wallet')}>
          <button
            type="button"
            onClick={() => navigate('/settings/wallet-balances', settingsNavState(location))}
            className={ICON_BTN}
            data-analytics-id="sidebar-header-wallet"
            aria-label={t('nav.wallet')}>
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.8}
                d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H6a3 3 0 00-3 3v8a3 3 0 003 3z"
              />
            </svg>
          </button>
        </Tooltip>

        <Tooltip label={t('nav.settings')}>
          <button
            type="button"
            onClick={() => navigate('/settings', settingsNavState(location))}
            className={ICON_BTN}
            data-analytics-id="sidebar-header-settings"
            aria-label={t('nav.settings')}>
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.8}
                d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
              />
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.8}
                d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
              />
            </svg>
          </button>
        </Tooltip>

        {/* Collapse the sidebar — sits on the right, next to Settings. */}
        <Tooltip label={t('chat.hideSidebar')}>
          <button
            type="button"
            onClick={hide}
            className={ICON_BTN}
            data-analytics-id="sidebar-header-collapse"
            aria-label={t('chat.hideSidebar')}>
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.8}
                d="M15 19l-7-7 7-7M20 5v14"
              />
            </svg>
          </button>
        </Tooltip>
      </div>
    </div>
  );
}
