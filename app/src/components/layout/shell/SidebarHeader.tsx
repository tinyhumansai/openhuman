import { useLocation, useNavigate } from 'react-router-dom';

import { registry } from '../../../lib/commands/registry';
import { useT } from '../../../lib/i18n/I18nContext';
import { settingsNavState } from '../../settings/modal/settingsOverlay';
import { Tooltip } from '../../ui';
import { useRootSidebar } from './RootShellLayout';
import { useHomeNav } from './useHomeNav';

const ICON_BTN =
  'flex h-7 w-7 flex-none items-center justify-center rounded-md text-content-muted transition-colors hover:bg-surface-hover hover:text-content-secondary';

/**
 * Thin utility header at the top of the root sidebar: jump Home, open keyboard
 * shortcuts, open Settings, and collapse the sidebar. Language is chosen from
 * Settings, not here.
 */
export default function SidebarHeader() {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const { hide } = useRootSidebar();
  const handleHome = useHomeNav();

  return (
    // Right-aligned so the macOS traffic lights (top-left, overlay title bar)
    // sit in the empty left space — the icons stay clear of the window controls
    // and inline with them (no extra top padding).
    <div className="flex items-center justify-end gap-1 px-2 py-1.5">
      <div className="flex items-center gap-0.5">
        {/* Home shortcut (replaces the former wallet shortcut). */}
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

        {/* Keyboard shortcuts — one-click open of the help directory (also ? / ⌘/). */}
        <Tooltip label={t('shortcuts.title')}>
          <button
            type="button"
            onClick={() => registry.runAction('meta.keyboard-shortcuts')}
            className={ICON_BTN}
            data-analytics-id="sidebar-header-shortcuts"
            aria-label={t('shortcuts.title')}>
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.8}
                d="M4 6h16a1 1 0 011 1v10a1 1 0 01-1 1H4a1 1 0 01-1-1V7a1 1 0 011-1z"
              />
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.8}
                d="M7 10h.01M11 10h.01M15 10h.01M17 10h.01M7 13h.01M9 16h6"
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
