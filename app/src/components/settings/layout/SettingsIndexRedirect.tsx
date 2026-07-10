import { Navigate, useLocation } from 'react-router-dom';

import { useMediaQuery } from '../../../hooks/useMediaQuery';

/**
 * /settings index behavior:
 *  - wide (md+): redirect to the first sidebar destination so the content
 *    pane is never empty;
 *  - narrow: render nothing — the sidebar itself is the index page (the
 *    classic drill-down home list).
 *
 * The media query re-evaluates on resize, so widening a window parked at
 * /settings auto-selects the first item.
 */
const SettingsIndexRedirect = () => {
  const isWide = useMediaQuery('(min-width: 768px)');
  const location = useLocation();
  // Preserve nav state (notably the desktop modal's `backgroundLocation`) so the
  // index redirect doesn't drop the backdrop. Harmless on iOS.
  if (isWide) return <Navigate to="/settings/account" replace state={location.state} />;
  return null;
};

export default SettingsIndexRedirect;
