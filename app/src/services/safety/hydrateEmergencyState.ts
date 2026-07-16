import type { Dispatch } from '@reduxjs/toolkit';

import { hydrateHalt } from '../../store/safetySlice';
import { emergencyStatus } from '../api/emergencyApi';

/**
 * Fetches the authoritative halt state from the core and dispatches
 * `hydrateHalt` into the Redux store. Errors are caught and logged so a
 * degraded core never crashes the boot path.
 *
 * Extracted from AppShellDesktop's boot-hydration effect so it can be
 * unit-tested in isolation without rendering the full component tree.
 */
export async function hydrateEmergencyState(dispatch: Dispatch): Promise<void> {
  try {
    const status = await emergencyStatus();
    dispatch(hydrateHalt(status));
  } catch (err) {
    console.warn('[emergency] status hydration failed', err);
  }
}
