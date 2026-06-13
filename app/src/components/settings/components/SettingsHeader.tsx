import type { ReactNode } from 'react';
import { useInRouterContext, useLocation } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useSettingsLayout } from '../layout/SettingsLayoutContext';

interface BreadcrumbItem {
  label: string;
  onClick?: () => void;
}

interface SettingsHeaderProps {
  className?: string;
  title?: string;
  showBackButton?: boolean;
  onBack?: () => void;
  /**
   * Accepted for backward compatibility but no longer rendered — the two-pane
   * sidebar replaced breadcrumb navigation. Call sites are cleaned up
   * incrementally.
   */
  breadcrumbs?: BreadcrumbItem[];
  /**
   * Optional right-aligned action (e.g. a refresh or pair-device button).
   * Rendered at the end of the header row so panels keep the canonical
   * "SettingsHeader as first child" structure instead of wrapping the header
   * in an ad-hoc flex row.
   */
  action?: ReactNode;
}

/**
 * Resolve the current pathname without throwing when the header is rendered
 * outside a `<Router>` (e.g. isolated settings-panel unit tests). Inside the
 * app the header always sits within the router, so this returns the real path;
 * with no router it falls back to '' which callers treat as a top-level route.
 *
 * Split into its own component so `useLocation` is only ever called when a
 * router is actually present — keeping the rules-of-hooks contract intact.
 */
const SettingsHeader = (props: SettingsHeaderProps) => {
  const inRouter = useInRouterContext();
  return inRouter ? (
    <RoutedSettingsHeader {...props} />
  ) : (
    <SettingsHeaderView {...props} pathname="" />
  );
};

const RoutedSettingsHeader = (props: SettingsHeaderProps) => {
  const { pathname } = useLocation();
  return <SettingsHeaderView {...props} pathname={pathname} />;
};

const SettingsHeaderView = ({
  className = '',
  title,
  showBackButton = false,
  onBack,
  action,
  pathname,
}: SettingsHeaderProps & { pathname: string }) => {
  const { t } = useT();
  const { inTwoPaneShell } = useSettingsLayout();

  // These panels are also embedded outside /settings — e.g. Brain
  // (`/brain?tab=memory-data`) and Connections (`/connections?tab=llm`). There
  // the host page's own sidebar owns navigation, and the panel's `onBack`
  // (sourced from useSettingsNavigation, which has no settings slug on those
  // routes) would navigate away from the host. Suppress the back button when
  // embedded outside the settings route tree.
  const isSettingsPath = pathname.startsWith('/settings');
  const showBack = showBackButton && !!onBack && (isSettingsPath || !inTwoPaneShell);

  // Inside the settings two-pane shell, top-level destinations (/settings/<slug>)
  // hide the back button on wide viewports — the sidebar provides navigation.
  // Nested pages (team/manage/:id, agents/edit/:id, …) keep it at all widths.
  const isTopLevel = pathname.split('/').filter(Boolean).length <= 2;
  const backButtonClass =
    inTwoPaneShell && isTopLevel
      ? 'md:hidden w-6 h-6 flex items-center justify-center rounded-full hover:bg-stone-100 dark:bg-neutral-800 dark:hover:bg-neutral-800 transition-colors mr-2'
      : 'w-6 h-6 flex items-center justify-center rounded-full hover:bg-stone-100 dark:bg-neutral-800 dark:hover:bg-neutral-800 transition-colors mr-2';

  return (
    <div className={`px-5 pt-5 pb-3 ${className}`}>
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center min-w-0">
          {/* Back button */}
          {showBack && onBack && (
            <button onClick={onBack} className={backButtonClass} aria-label={t('common.back')}>
              <svg
                className="w-4 h-4 text-stone-500 dark:text-neutral-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M15 19l-7-7 7-7"
                />
              </svg>
            </button>
          )}

          {/* Title */}
          <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
            {title ?? t('nav.settings')}
          </h2>
        </div>

        {action && <div className="flex-shrink-0">{action}</div>}
      </div>
    </div>
  );
};

export default SettingsHeader;
