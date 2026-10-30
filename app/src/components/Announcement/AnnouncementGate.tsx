import debugFactory from 'debug';
import { useEffect, useState } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import { type Announcement, fetchLatestAnnouncement } from '../../services/announcementService';
import { markAnnouncementShown } from '../../store/announcementSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import AnnouncementModal from './AnnouncementModal';

const log = debugFactory('announcement');

/**
 * Mount a fresh announcement fetch state for each authenticated session.
 * Returning null on sign-out unmounts the inner gate even when the same user
 * and profile signs in again, preventing old results from flashing on re-login.
 */
export default function AnnouncementGate() {
  const { snapshot } = useCoreState();
  if (!snapshot.auth.isAuthenticated) return null;

  // A user or credential-profile switch must also reset a previous session's
  // fetched result, even while both snapshots remain authenticated.
  const identity = JSON.stringify([snapshot.auth.userId ?? null, snapshot.auth.profileId ?? null]);
  return <AuthenticatedAnnouncementGate key={identity} />;
}

/**
 * Fetch the latest active announcement once per mounted authentication session.
 * The mount boundary also cancels writes from previous identities and sessions.
 */
function AuthenticatedAnnouncementGate() {
  const dispatch = useAppDispatch();
  const shownIds = useAppSelector(state => state.announcement.shownIds);
  const [announcement, setAnnouncement] = useState<Announcement | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const latest = await fetchLatestAnnouncement();
        if (cancelled || !latest) return;
        setAnnouncement(latest);
        log('fetched announcement %s', latest.id);
      } catch (err) {
        // Missing announcements and transient requests are not fatal.
        log('fetch failed: %O', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (!announcement || shownIds.includes(announcement.id)) return null;

  const handleDismiss = () => {
    dispatch(markAnnouncementShown(announcement.id));
    setAnnouncement(null);
  };

  return <AnnouncementModal announcement={announcement} onDismiss={handleDismiss} />;
}
