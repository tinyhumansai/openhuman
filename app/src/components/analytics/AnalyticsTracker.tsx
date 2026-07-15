import { useEffect } from 'react';
import { useLocation } from 'react-router-dom';

import { trackPageView } from '../../services/analytics';

/** Standard route-view tracker. Mount once inside the active router. */
export function AnalyticsPageTracker() {
  const { pathname } = useLocation();
  useEffect(() => {
    trackPageView(pathname);
  }, [pathname]);
  return null;
}
