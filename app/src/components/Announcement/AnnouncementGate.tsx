import debugFactory from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import { type Announcement, fetchLatestAnnouncement } from '../../services/announcementService';
import { markAnnouncementShown } from '../../store/announcementSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import AnnouncementModal from './AnnouncementModal';

const log = debugFactory('announcement');

/**
 * Fetches the latest active announcement once the user is authenticated and,
 * if it hasn't been seen before (per-user, persisted), shows it once over the
 * app. The modal sits at z-[9998] — just below the harness-init overlay
 * (z-[9999]) — so during first-run setup the init screen covers it and the
 * announcement becomes visible only once init finishes.
 */
export default function AnnouncementGate() {
  const dispatch = useAppDispatch();
  const { snapshot } = useCoreState();
  const isAuthenticated = snapshot.auth.isAuthenticated;
  const userId = snapshot.auth.userId ?? null;
  const shownIds = useAppSelector(state => state.announcement.shownIds);

  const [announcement, setAnnouncement] = useState<Announcement | null>(null);
  // One fetch per (user) auth session — keyed by userId so a user switch refetches.
  const fetchedForRef = useRef<string | null>(null);

  useEffect(() => {
    if (!isAuthenticated) {
      // Reset on sign-out so the next sign-in fetches again. The render guard
      // below hides any stale announcement while signed out (no setState here).
      fetchedForRef.current = null;
      return;
    }

    const key = userId ?? 'authenticated';
    if (fetchedForRef.current === key) {
      return;
    }
    fetchedForRef.current = key;

    let cancelled = false;
    void (async () => {
      try {
        const latest = await fetchLatestAnnouncement();
        if (cancelled || !latest) {
          return;
        }
        setAnnouncement(latest);
        log('fetched announcement %s', latest.id);
      } catch (err) {
        // A missing/failed announcement is never fatal — just don't show one.
        log('fetch failed: %O', err);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [isAuthenticated, userId]);

  if (!isAuthenticated || !announcement || shownIds.includes(announcement.id)) {
    return null;
  }

  const handleDismiss = () => {
    dispatch(markAnnouncementShown(announcement.id));
    setAnnouncement(null);
  };

  return <AnnouncementModal announcement={announcement} onDismiss={handleDismiss} />;
}
