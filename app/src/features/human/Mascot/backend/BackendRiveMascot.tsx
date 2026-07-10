// Renders a backend-served Rive mascot selected by id.
//
// Fetches the manifest (cached), then resolves the .riv binary through the
// version-keyed cache (`rivCache` → IndexedDB) so the backend is only hit when
// the mascot's version changes. While the binary loads — or if it fails — we
// fall back to the bundled default mascot so the Human stage is never blank.
import debug from 'debug';
import { type FC, useEffect, useState } from 'react';

import { getCachedMascotDetail, loadMascotRivBuffer } from '../../../../services/mascotService';
import type { MascotFace } from '../Ghosty';
import { RiveMascot } from '../RiveMascot';
import { isRiveMascotDetail } from './types';

const log = debug('human:mascot:backend-rive');

export interface BackendRiveMascotProps {
  mascotId: string;
  face?: MascotFace;
  size?: number | string;
  primaryColor?: number;
  secondaryColor?: number;
  visemeCode?: string;
  idlePoseRotation?: boolean;
}

export const BackendRiveMascot: FC<BackendRiveMascotProps> = ({ mascotId, ...riveProps }) => {
  // Callers key this component by mascotId so a new selection remounts it with
  // fresh state — meaning the effect never has to synchronously reset here, it
  // only resolves the buffer (or marks failure) asynchronously.
  const [buffer, setBuffer] = useState<ArrayBuffer | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const detail = await getCachedMascotDetail(mascotId);
        if (cancelled) return;
        if (!isRiveMascotDetail(detail)) {
          // Selected id is an SVG mascot — not renderable here. Fall back.
          log('mascot %s is not a rive mascot; using default', mascotId);
          setFailed(true);
          return;
        }
        const buf = await loadMascotRivBuffer(detail);
        if (!cancelled) setBuffer(buf);
      } catch (err) {
        if (!cancelled) {
          log('failed to load backend rive mascot %s: %o', mascotId, err);
          setFailed(true);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [mascotId]);

  // Default mascot while loading or on failure; the custom buffer once ready.
  // Key the instance so the Rive runtime cleanly reinitialises when the buffer
  // arrives (or when switching mascots) rather than mutating a live context.
  if (failed || !buffer) return <RiveMascot key="default" {...riveProps} />;
  return <RiveMascot key={`buf-${mascotId}`} {...riveProps} buffer={buffer} />;
};
