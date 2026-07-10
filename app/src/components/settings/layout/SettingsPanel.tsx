import type { ReactNode } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import PanelPage, { type PanelPageTab } from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { findEntryById } from '../settingsRouteRegistry';
import SettingsSubNav from './SettingsSubNav';

export interface SettingsPanelProps<T extends string = string> {
  /**
   * Override the panel title. Defaults to the active route's registry title, so
   * most panels omit it. Supply it for dynamic sub-pages (profile/agent
   * editors, team management) that don't map 1:1 to a registry entry.
   */
  title?: ReactNode;
  /** Optional muted sub-title shown below the title. */
  description?: ReactNode;
  /** Right-aligned header action(s) (e.g. an "Add" button). */
  action?: ReactNode;

  /** In-panel chip tabs. Omit for a single-body panel (use `children`). */
  tabs?: PanelPageTab<T>[];
  /** Active tab id (controlled). */
  value?: T;
  /** Called with the chip id when a tab is selected. */
  onChange?: (id: T) => void;
  /** Accessible label for the chip row. */
  tabsAriaLabel?: string;
  /** Prefix for each chip's `data-testid` (`${prefix}-${id}`). */
  tabsTestIdPrefix?: string;

  /** Single-body content (when there are no `tabs`). */
  children?: ReactNode;
  testId?: string;
}

/**
 * The single template for every Settings page. Wraps {@link PanelPage} and bakes
 * in the conventions so panels stop drifting:
 *
 * - A consistent visible **title** (auto-derived from the route registry), with
 *   the optional `action` aligned on the same row and the `description` beneath.
 * - The route-aware back button (hidden in the two-pane shell on wide viewports).
 * - The sibling **sub-nav** pill row rendered *inside* the header — so the order
 *   is always title → description → tabs → body, on every panel.
 * - Canonical full-width body spacing (`p-4 space-y-5`) and `z-10`.
 *
 * Use it for the routed panel only; embedded sub-panels (tab bodies) keep
 * rendering headerless content.
 */
export default function SettingsPanel<T extends string = string>({
  title,
  description,
  action,
  tabs,
  value,
  onChange,
  tabsAriaLabel,
  tabsTestIdPrefix,
  children,
  testId,
}: SettingsPanelProps<T>) {
  const { t } = useT();
  const { currentRoute, navigateBack } = useSettingsNavigation();

  const entry = findEntryById(currentRoute);
  const resolvedTitle = title ?? (entry ? t(entry.titleKey) : t('nav.settings'));

  const leading = <SettingsBackButton onBack={navigateBack} />;

  // Family pill row (e.g. Account → Team / Privacy / …). Renders null when the
  // active route has no siblings, so it costs nothing on standalone panels.
  // Tucked tight under the title/description to match in-panel chip spacing.
  const subNav = <SettingsSubNav className="flex flex-wrap gap-1.5 pt-2" />;

  if (tabs && tabs.length > 0) {
    return (
      <PanelPage<T>
        className="z-10"
        testId={testId}
        title={resolvedTitle}
        description={description}
        leading={leading}
        action={action}
        headerExtra={subNav}
        tabs={tabs}
        value={value}
        onChange={onChange}
        tabsAriaLabel={tabsAriaLabel}
        tabsTestIdPrefix={tabsTestIdPrefix}
      />
    );
  }

  return (
    <PanelPage
      className="z-10"
      testId={testId}
      title={resolvedTitle}
      description={description}
      leading={leading}
      action={action}
      headerExtra={subNav}>
      {children}
    </PanelPage>
  );
}
