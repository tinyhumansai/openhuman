import { useInRouterContext, useLocation } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useSettingsLayout } from '../layout/SettingsLayoutContext';

interface SettingsBackButtonProps {
  /** Invoked when the button is pressed (typically `navigateBack`). */
  onBack?: () => void;
}

/**
 * Route-aware back button shared by {@link SettingsHeader} and panels built on
 * {@link PanelScaffold}. Encapsulates the visibility rules so both render the
 * exact same affordance:
 *
 * - Hidden entirely when there's no `onBack`, or when a panel is embedded
 *   *outside* the settings route tree (e.g. Brain / Connections host their own
 *   navigation and the settings `onBack` would jump away from the host).
 * - Inside the two-pane settings shell, top-level destinations
 *   (`/settings/<slug>`) hide it on md+ — the sidebar owns navigation there —
 *   while nested pages keep it at every width.
 *
 * Returns `null` when it should not show, so callers can drop it into a slot
 * unconditionally.
 */
const SettingsBackButton = ({ onBack }: SettingsBackButtonProps) => {
  const inRouter = useInRouterContext();
  return inRouter ? (
    <RoutedSettingsBackButton onBack={onBack} />
  ) : (
    <SettingsBackButtonView onBack={onBack} pathname="" />
  );
};

const RoutedSettingsBackButton = ({ onBack }: SettingsBackButtonProps) => {
  const { pathname } = useLocation();
  return <SettingsBackButtonView onBack={onBack} pathname={pathname} />;
};

const SettingsBackButtonView = ({
  onBack,
  pathname,
}: SettingsBackButtonProps & { pathname: string }) => {
  const { t } = useT();
  const { inTwoPaneShell } = useSettingsLayout();

  const isSettingsPath = pathname.startsWith('/settings');
  const show = !!onBack && (isSettingsPath || !inTwoPaneShell);
  if (!show) return null;

  const isTopLevel = pathname.split('/').filter(Boolean).length <= 2;
  const className =
    inTwoPaneShell && isTopLevel
      ? 'md:hidden w-6 h-6 flex items-center justify-center rounded-full hover:bg-surface-hover dark:bg-surface-muted transition-colors mr-2'
      : 'w-6 h-6 flex items-center justify-center rounded-full hover:bg-surface-hover dark:bg-surface-muted transition-colors mr-2';

  return (
    <button onClick={onBack} className={className} aria-label={t('common.back')}>
      <svg
        className="w-4 h-4 text-content-muted"
        fill="none"
        stroke="currentColor"
        viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" />
      </svg>
    </button>
  );
};

export default SettingsBackButton;
