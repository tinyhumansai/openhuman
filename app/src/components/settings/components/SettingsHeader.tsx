import type { ReactNode } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsBackButton from './SettingsBackButton';

interface BreadcrumbItem {
  label: string;
  onClick?: () => void;
}

/**
 * @deprecated For settings panels use {@link SettingsPanel}
 * (`components/settings/layout/SettingsPanel`), which bakes in a consistent
 * title, back button, sub-nav and spacing. This component now only backs the
 * couple of standalone non-settings page shells (e.g. Webhooks, Intelligence)
 * that live outside the `/settings` route tree and therefore can't derive a
 * title from the settings route registry. Do not add new settings call sites.
 */
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

const SettingsHeader = ({
  className = '',
  title,
  showBackButton = false,
  onBack,
  action,
}: SettingsHeaderProps) => {
  const { t } = useT();

  return (
    <div className={`px-5 pt-5 pb-3 ${className}`}>
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center min-w-0">
          {/* Route-aware back button (hidden when not applicable). */}
          {showBackButton && <SettingsBackButton onBack={onBack} />}

          {/* Title */}
          <h2 className="text-base font-semibold text-content">{title ?? t('nav.settings')}</h2>
        </div>

        {action && <div className="flex-shrink-0">{action}</div>}
      </div>
    </div>
  );
};

export default SettingsHeader;
