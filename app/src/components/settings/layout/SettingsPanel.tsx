import type { ReactNode } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import PanelPage, { type PanelPageTab } from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { findEntryById } from '../settingsRouteRegistry';
import { useSettingsLayout } from './SettingsLayoutContext';
import SettingsSubNav from './SettingsSubNav';
import SettingsTabbedPage from './SettingsTabbedPage';

interface SettingsPanelProps<T extends string = string> {
  /**
   * Replaces the default `space-y-5` body wrapper. A panel whose main region is
   * meant to fill the height (a live log, a list) needs to be a flex column
   * rather than a stack of margins, and cannot express that from the outside.
   */
  bodyClassName?: string;
  /**
   * Forwarded to {@link SettingsTabbedPage}. Pass `false` when the panel owns
   * its own scroll region: the page wrapper then has a definite height, which
   * is what a `flex-1` child needs in order to fill it. Without this the log
   * region below sized to its content and left the rest of the pane blank.
   */
  scrollable?: boolean;
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
 * The single template for every Settings page. Wraps
 * {@link SettingsTabbedPage} and bakes in the conventions so panels stop
 * drifting:
 *
 * - A consistent visible **title** (auto-derived from the route registry), with
 *   the optional `action` aligned on the same row and the `description` beneath.
 * - The route-aware back button (hidden in the two-pane shell on wide viewports).
 * - The sibling **sub-nav** pill row rendered *inside* the header — so the order
 *   is always title → description → sub-nav → tabs → body, on every panel.
 * - Canonical body spacing and the page's `p-4` gutter.
 *
 * It wrapped `PanelPage` until the settings pages were brought onto the
 * Connections pages' layout: `PanelPage`'s header is a small title over a
 * hairline, while `SettingsTabbedPage` gives the page a real 2xl heading and a
 * full-bleed divider. Both hosts render the same panels, so having two chromes
 * meant the same panel looked like a different page depending on how it was
 * reached.
 *
 * `headerless` still delegates to `PanelPage`: that mode exists for the
 * Connections pane, which draws the page header itself, so this must render
 * body-only chrome and nothing that would double it.
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
  bodyClassName,
  scrollable,
}: SettingsPanelProps<T>) {
  const { t } = useT();
  const { currentRoute, navigateBack } = useSettingsNavigation();
  const { headerless } = useSettingsLayout();

  // Headerless: a host (the Connections pane) already renders a page header
  // above this panel, so render just the tabs/body without title/description/
  // sub-nav to avoid a doubled header.
  if (headerless) {
    // `scrollable` / `bodyClassName` are forwarded here too. They were declared
    // on the props but only wired into the routed return, so a headerless host
    // passing either got the PanelPage defaults with no type error and no
    // warning -- the same silent-drop failure the `children` comment below
    // describes. `PanelPage` calls the body class `contentClassName`.
    if (tabs && tabs.length > 0) {
      return (
        <PanelPage<T>
          className="z-10"
          testId={testId}
          action={action}
          tabs={tabs}
          value={value}
          onChange={onChange}
          tabsAriaLabel={tabsAriaLabel}
          tabsTestIdPrefix={tabsTestIdPrefix}
          scrollable={scrollable}
        />
      );
    }
    return (
      <PanelPage
        className="z-10"
        testId={testId}
        action={action}
        scrollable={scrollable}
        contentClassName={bodyClassName}>
        {children}
      </PanelPage>
    );
  }

  const entry = findEntryById(currentRoute);
  const resolvedTitle = title ?? (entry ? t(entry.titleKey) : t('nav.settings'));

  const leading = <SettingsBackButton onBack={navigateBack} />;

  // Family pill row (e.g. Account → Team / Privacy / …). Renders null when the
  // active route has no siblings, so it costs nothing on standalone panels.
  const subNav = <SettingsSubNav className="flex flex-wrap gap-1.5" />;

  const tabList = tabs ?? [];
  const active = tabList.length > 0 ? (tabList.find(tab => tab.id === value) ?? tabList[0]) : null;

  // A tab's `description` was rendered by the scaffold `PanelPage` gave each
  // body. Nothing renders it here, so keep it above the body rather than
  // dropping it — a silently unrendered prop is how a page loses its only
  // explanation of what a tab does.
  const body = active ? (
    <div className={active.contentClassName || undefined}>
      {active.description != null && (
        <p className="pb-3 text-sm text-content-muted">{active.description}</p>
      )}
      {active.content}
      {children}
    </div>
  ) : (
    <div className={bodyClassName ?? 'space-y-5'}>{children}</div>
  );

  return (
    // `p-4` is the page gutter the divider's `-mx-4` bleeds through; `z-10`
    // is carried over from the PanelPage era, where panels stacked over the
    // shell background.
    <div className="relative z-10 h-full p-4" data-testid={testId}>
      <SettingsTabbedPage<T>
        title={resolvedTitle}
        description={description}
        leading={leading}
        headerAction={action}
        headerExtra={subNav}
        tabs={
          active
            ? tabList.map(tab => ({ id: tab.id, label: tab.label, testId: tab.chipTestId }))
            : undefined
        }
        value={active ? active.id : undefined}
        onChange={onChange}
        tabsAriaLabel={tabsAriaLabel}
        tabsTestIdPrefix={tabsTestIdPrefix}
        scrollable={scrollable}>
        {body}
      </SettingsTabbedPage>
    </div>
  );
}
