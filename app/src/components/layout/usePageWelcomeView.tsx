import { type ReactNode, useCallback, useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { SidebarContent } from './shell/SidebarSlot';
import TwoPaneNav from './TwoPaneNav';

/** Small inline stroke-icon helper matching the other sidebar navs. */
const navIcon = (d: string) => (
  <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={d} />
  </svg>
);

/** Check-circle glyph, shared by every Welcome sidebar entry. */
const WELCOME_ICON = navIcon('M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z');

export type PageWelcomeViewId = 'welcome' | 'main';

export interface UsePageWelcomeViewOptions {
  /** Accessible label for the two-item sidebar nav. */
  ariaLabel: string;
  /** Label for the Welcome entry. */
  welcomeLabel: string;
  /** Label for the functional (main) entry. */
  mainLabel: string;
  /** Icon (SVG path `d`) for the functional entry. */
  mainIconPath: string;
  /** Optional extra header rendered above the nav (e.g. a subtitle). */
  header?: ReactNode;
}

export interface PageWelcomeView {
  /** Current view — `welcome` (default landing) or `main`. */
  view: PageWelcomeViewId;
  /** Switch views (updates `?view=`). */
  setView: (v: PageWelcomeViewId) => void;
  /** The sidebar nav element to render once inside the page. */
  nav: ReactNode;
}

/**
 * Give a single-view page (Flows, Notifications, …) the same "Welcome landing
 * first" shape as the sidebar pages that have real sub-navs: a two-item sidebar
 * nav (Welcome · <main>) projected into the shell, driven by `?view=`, defaulting
 * to the Welcome landing.
 */
export function usePageWelcomeView(opts: UsePageWelcomeViewOptions): PageWelcomeView {
  const { ariaLabel, welcomeLabel, mainLabel, mainIconPath, header } = opts;
  const location = useLocation();
  const navigate = useNavigate();

  const view: PageWelcomeViewId =
    new URLSearchParams(location.search).get('view') === 'main' ? 'main' : 'welcome';

  const setView = useCallback(
    (v: PageWelcomeViewId) => {
      const params = new URLSearchParams(location.search);
      if (v === 'main') params.set('view', 'main');
      else params.delete('view');
      const search = params.toString();
      navigate({ pathname: location.pathname, search: search ? `?${search}` : '' });
    },
    [location.pathname, location.search, navigate]
  );

  const nav = useMemo(
    () => (
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <TwoPaneNav
            ariaLabel={ariaLabel}
            selected={view}
            onSelect={v => setView(v as PageWelcomeViewId)}
            groups={[
              {
                items: [
                  { value: 'welcome', label: welcomeLabel, icon: WELCOME_ICON },
                  { value: 'main', label: mainLabel, icon: navIcon(mainIconPath) },
                ],
              },
            ]}
            header={header}
          />
        </div>
      </SidebarContent>
    ),
    [ariaLabel, welcomeLabel, mainLabel, mainIconPath, header, view, setView]
  );

  return { view, setView, nav };
}
