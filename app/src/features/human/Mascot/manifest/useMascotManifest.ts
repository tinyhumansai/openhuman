/**
 * Load the published mascot manifest and resolve the active mascot entry from
 * the user's `selectedMascotId` Redux preference, with a render-only fallback
 * to the default (`ready`) mascot while none is selected. Shared by the Human
 * stage and the settings picker so both agree on which mascot is current.
 */
import { useEffect, useState } from 'react';
import { useDispatch, useSelector } from 'react-redux';

import {
  selectCustomMascotGifUrl,
  selectSelectedMascotId,
  setSelectedMascotId,
} from '../../../../store/mascotSlice';
import { defaultMascot, fetchMascotManifest, findMascot } from './manifestService';
import type { MascotManifest, MascotManifestEntry } from './types';

export interface UseMascotManifestResult {
  manifest: MascotManifest | null;
  /** The selected mascot, or the default when none is chosen / found yet. */
  entry: MascotManifestEntry | null;
  loading: boolean;
  error: Error | null;
}

export function useMascotManifest(): UseMascotManifestResult {
  const dispatch = useDispatch();
  const selectedMascotId = useSelector(selectSelectedMascotId);
  const customMascotGifUrl = useSelector(selectCustomMascotGifUrl);
  const [manifest, setManifest] = useState<MascotManifest | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const m = await fetchMascotManifest();
        if (cancelled) return;
        setManifest(m);
        setError(null);
        setLoading(false);
      } catch (err) {
        if (cancelled) return;
        setError(err instanceof Error ? err : new Error(String(err)));
        setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const selectedEntry = manifest ? findMascot(manifest, selectedMascotId) : undefined;
  const fallbackEntry = manifest ? (defaultMascot(manifest) ?? null) : null;
  const entry = selectedEntry ?? fallbackEntry;

  useEffect(() => {
    if (!manifest || !selectedMascotId || customMascotGifUrl || selectedEntry) return;
    dispatch(setSelectedMascotId(fallbackEntry?.id ?? null));
  }, [customMascotGifUrl, dispatch, fallbackEntry?.id, manifest, selectedEntry, selectedMascotId]);

  return { manifest, entry, loading, error };
}
