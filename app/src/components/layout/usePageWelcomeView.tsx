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

/** `'welcome'` / `'main'`, plus any extra sub-page value the caller declares. */
export type PageWelcomeViewId = string;

/** An additional sub-page beyond Welcome · Main (e.g. Workflows' Runs / Discoveries). */
export interface PageWelcomeExtraItem {
  /** `?view=` value + nav selection id. */
  value: string;
  /** Sidebar label. */
  label: string;
  /** Icon (SVG path `d`). */
  iconPath: string;
}

export interface UsePageWelcomeViewOptions {
  /** Accessible label for the sidebar nav. */
  ariaLabel: string;
  /** Label for the Welcome entry. */
  welcomeLabel: string;
  /** Label for the functional (main) entry. */
  mainLabel: string;
  /** Icon (SVG path `d`) for the functional entry. */
  mainIconPath: string;
  /** Optional extra header rendered above the nav (e.g. a subtitle). */
  header?: ReactNode;
  /** Optional extra sub-pages listed after `main` in the nav. */
  extraItems?: PageWelcomeExtraItem[];
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
  const { ariaLabel, welcomeLabel, mainLabel, mainIconPath, header, extraItems } = opts;
  const location = useLocation();
  const navigate = useNavigate();

  const validViews = useMemo(
    () => new Set<string>(['main', ...(extraItems ?? []).map(i => i.value)]),
    [extraItems]
  );

  const raw = new URLSearchParams(location.search).get('view') ?? '';
  const view: PageWelcomeViewId = validViews.has(raw) ? raw : 'welcome';

  const setView = useCallback(
    (v: PageWelcomeViewId) => {
      const params = new URLSearchParams(location.search);
      if (v === 'welcome') params.delete('view');
      else params.set('view', v);
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
                  ...(extraItems ?? []).map(item => ({
                    value: item.value,
                    label: item.label,
                    icon: navIcon(item.iconPath),
                  })),
                ],
              },
            ]}
            header={header}
          />
        </div>
      </SidebarContent>
    ),
    [ariaLabel, welcomeLabel, mainLabel, mainIconPath, header, extraItems, view, setView]
  );

  return { view, setView, nav };
}
