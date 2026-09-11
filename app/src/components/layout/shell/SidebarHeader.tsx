import { LuKeyboard, LuPanelLeftClose, LuSearch, LuSettings } from 'react-icons/lu';
import { useNavigate } from 'react-router-dom';

import { registry } from '../../../lib/commands/registry';
import { useT } from '../../../lib/i18n/I18nContext';
import { Button, SidebarHeader as SidebarHeaderShell, Tooltip } from '../../ui';
import { useRootSidebar } from './RootShellLayout';

/**
 * Header footprint layered on `<Button variant="tertiary" iconOnly>`: 28px
 * square (no `size` maps to that) with the muted resting colour these utility
 * icons use. Focus ring, transition and hover fill come from the primitive.
 */
const ICON_BTN = 'h-7 w-7 flex-none rounded-md text-content-muted hover:text-content-secondary';

/**
 * Thin utility header at the top of the root sidebar: keyboard shortcuts,
 * Feedback, Settings, and collapse. Language is chosen from Settings, not here.
 *
 * Feedback moved up here from the sidebar footer row: it is a utility action
 * like the other three, not a destination the user returns to, and the footer
 * is now only the connectivity/version strip.
 */
export default function SidebarHeader() {
  const { t } = useT();
  const navigate = useNavigate();
  const { hide } = useRootSidebar();

  return (
    // The primitive's header slot supplies the px-3/pb-2/pt-3 band; this only
    // turns it into a right-aligned row. Right-aligned so the macOS traffic
    // lights (top-left, overlay title bar) sit in the empty left space — the
    // icons stay clear of the window controls and inline with them.
    //
    // The icons deliberately do NOT move to meet the lights: this row's
    // vertical rhythm is the sidebar's, shared with `SidebarNav` below it, and
    // pulling it up to the window's edge to chase a platform control would bend
    // the app's own spacing around one OS's chrome. The lights are moved to
    // this line instead, via `trafficLightPosition` in `tauri.conf.json`. That
    // file is JSON and cannot hold a comment, so the reasoning lives here,
    // beside the row it has to agree with.
    //
    // The target is fixed and computable: this row's centre is pt-3 (12px) plus
    // half of ICON_BTN's h-7 (14px) = 26px from the window top, and the sidebar
    // starts flush at that top — `SidebarProvider`/`Sidebar` add no inset.
    //
    // `y` is NOT that centre, and is not a simple gap either. tao positions the
    // lights by resizing the title-bar container: `inset_traffic_lights`
    // (tao `platform_impl/macos/view.rs`) sets the container's height to
    // buttonHeight + y and pins it to the window top, but only rewrites each
    // button's origin.x — origin.y is left alone. So the space above a button
    // works out as `y − b`, where `b` is whatever offset the button already had
    // inside that container. `b` is AppKit's and is not knowable from here,
    // which is why `y` is tuned by looking at the window rather than solved:
    // 20 sat visibly high, 28 is the correction.
    //
    // `x: 20` is the conventional macOS left inset. Supplying it is unavoidable
    // — the config takes a position, so `y` cannot be set alone without moving
    // this into Rust and reading the existing frame.
    //
    // Re-check this if `pt-3` or `ICON_BTN`'s height ever changes: the target
    // moves with them, and nothing fails loudly when the two disagree.
    //
    // `data-tauri-drag-region` lives directly on the primitive (rather than a
    // wrapping div in `AppSidebar`) so the header band is draggable window
    // chrome without an extra hand-rolled layout element.
    //
    // Its value is `"deep"`, not the bare attribute. Tauri's injected `drag.js`
    // reads a bare region (React renders the JSX shorthand as `="true"`) as
    // direct-hit-only — `return el === composedPath[0]` — so with the icon row
    // nested below, only this element's own uncovered box dragged and the
    // wrapper's box did not. `"deep"` makes the subtree draggable, which also
    // keeps any non-button content added to that row later draggable. The icons
    // are unaffected: `isDragRegion` bails out on a clickable element before it
    // reaches the `deep` branch, so every button here still clicks. Bare is only
    // correct on a band with no children — see `WindowDragBar`.
    <SidebarHeaderShell
      data-tauri-drag-region="deep"
      className="flex-row items-center justify-end gap-1">
      <div className="flex items-center gap-0.5">
        {/* Keyboard shortcuts — one-click open of the help directory (also ? / ⌘/). */}
        <Tooltip label={t('shortcuts.title')}>
          <Button
            variant="tertiary"
            iconOnly
            onClick={() => registry.runAction('meta.keyboard-shortcuts')}
            className={ICON_BTN}
            analyticsId="sidebar-header-shortcuts"
            aria-label={t('shortcuts.title')}>
            <LuKeyboard className="h-4 w-4" />
          </Button>
        </Tooltip>

        {/* Global search — opens the ⌘K command palette. Goes through the
            command registry rather than a local handler so this button and the
            keyboard shortcut are literally the same action: one definition
            (`meta.command-palette` in `lib/commands/globalActions.ts`) owns the
            label, the shortcut and the handler, and the palette cannot end up
            opening one way and not the other.

            This slot held Share Feedback, which navigated to `/feedback`. That
            page is a settings panel now (`/settings/feedback`), reachable from
            the settings sidebar — and from this palette. */}
        <Tooltip label={t('shortcuts.action.commandPalette')}>
          <Button
            variant="tertiary"
            iconOnly
            onClick={() => registry.runAction('meta.command-palette')}
            className={ICON_BTN}
            analyticsId="sidebar-header-command-palette"
            aria-label={t('shortcuts.action.commandPalette')}>
            <LuSearch className="h-4 w-4" />
          </Button>
        </Tooltip>

        <Tooltip label={t('nav.settings')}>
          <Button
            variant="tertiary"
            iconOnly
            onClick={() => navigate('/settings')}
            className={ICON_BTN}
            analyticsId="sidebar-header-settings"
            aria-label={t('nav.settings')}>
            <LuSettings className="h-4 w-4" />
          </Button>
        </Tooltip>

        {/* Collapse the sidebar — sits on the right, next to Settings. */}
        <Tooltip label={t('chat.hideSidebar')}>
          <Button
            variant="tertiary"
            iconOnly
            onClick={hide}
            className={ICON_BTN}
            analyticsId="sidebar-header-collapse"
            aria-label={t('chat.hideSidebar')}>
            <LuPanelLeftClose className="h-4 w-4" />
          </Button>
        </Tooltip>
      </div>
    </SidebarHeaderShell>
  );
}
