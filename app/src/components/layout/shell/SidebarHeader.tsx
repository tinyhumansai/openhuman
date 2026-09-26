import { FaDiscord } from 'react-icons/fa6';
import { LuPanelLeftClose, LuSearch, LuSettings } from 'react-icons/lu';
import { useNavigate } from 'react-router-dom';

import { registry } from '../../../lib/commands/registry';
import { useT } from '../../../lib/i18n/I18nContext';
import { DISCORD_INVITE_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import { Button, SidebarHeader as SidebarHeaderShell, Tooltip } from '../../ui';
import { useRootSidebar } from './RootShellLayout';
import { isWindowsDesktop } from './WindowsWindowControls';

/** The community destination opened by the Discord button. */
export const DISCORD_URL = DISCORD_INVITE_URL;

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
    // The primitive supplies the horizontal and bottom inset; this overrides
    // its top inset to align the row to the title-bar centreline, then turns it
    // into a right-aligned row on macOS. The traffic lights sit in the empty
    // left space. Windows has no controls in this corner, so center the row.
    //
    // On macOS the icons deliberately do NOT move to meet the lights: this row's
    // vertical rhythm is the sidebar's, shared with `SidebarNav` below it, and
    // pulling it up to the window's edge to chase a platform control would bend
    // the app's own spacing around one OS's chrome. The lights are moved to
    // this line instead, via `trafficLightPosition` in `tauri.conf.json`. That
    // file is JSON and cannot hold a comment, so the reasoning lives here,
    // beside the row it has to agree with.
    //
    // The native traffic lights sit optically 4px above the title-bar band's
    // mathematical centre. The sidebar starts 8px from the window edge, then
    // 7px top padding plus half of ICON_BTN's h-7 (14px) puts the icon centre
    // on the tuned 29px optical line.
    // The native cluster is deliberately nudged toward the window corner:
    // x 22 / y 32 keeps it inside the 88px collapsed rail while leaving more
    // breathing room below it before the centred navigation controls begin.
    //
    // `y` is NOT that centre, and is not a simple gap either. tao positions the
    // lights by resizing the title-bar container: `inset_traffic_lights`
    // (tao `platform_impl/macos/view.rs`) sets the container's height to
    // buttonHeight + y and pins it to the window top, but only rewrites each
    // button's origin.x — origin.y is left alone. So the space above a button
    // works out as `y − b`, where `b` is whatever offset the button already had
    // inside that container. `b` is AppKit's and is not knowable from here,
    // which is why `y` is tuned by looking at the window rather than solved:
    // 20 sat visibly high, 28 was the correction before the 8px sidebar inset.
    //
    // `x: 22` is the collapsed-rail optical correction. Supplying it is unavoidable
    // — the config takes a position, so `y` cannot be set alone without moving
    // this into Rust and reading the existing frame.
    //
    // Re-check this if the 7px top inset or `ICON_BTN`'s height ever changes: the target
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
      className={`flex-row items-center gap-1 pt-[7px] ${isWindowsDesktop() ? 'justify-center' : 'justify-end'}`}>
      <div className="flex items-center gap-0.5">
        {/* Community Discord — opens the invite in the system browser. This slot
            held the keyboard-shortcuts help; that directory is still one
            keystroke away (? / ⌘/), while the community had no door at all. */}
        <Tooltip label={t('nav.discord')}>
          <Button
            variant="tertiary"
            iconOnly
            onClick={() => void openUrl(DISCORD_URL).catch(() => {})}
            className={ICON_BTN}
            analyticsId="sidebar-header-discord"
            aria-label={t('nav.discord')}>
            <FaDiscord className="h-4 w-4" />
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
