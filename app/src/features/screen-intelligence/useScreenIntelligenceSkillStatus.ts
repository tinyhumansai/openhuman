/**
 * Derives a skill-card-friendly status for Screen Intelligence,
 * matching the state vocabulary used by third-party skills (Gmail, etc.).
 */
import { useMemo } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import {
  activeStatus,
  enabledStatus,
  offlineStatus,
  setupRequiredStatus,
  type SkillCardStatusDescriptor,
  unsupportedStatus,
} from '../skills/skillCardStatus';

export interface ScreenIntelligenceSkillStatus extends SkillCardStatusDescriptor {
  /** True when all three macOS permissions are granted. */
  allPermissionsGranted: boolean;
  /** True when the platform doesn't support screen intelligence. */
  platformUnsupported: boolean;
}

export function useScreenIntelligenceSkillStatus(): ScreenIntelligenceSkillStatus {
  const { snapshot } = useCoreState();
  const status = snapshot.runtime.screenIntelligence;

  return useMemo(() => {
    // No status yet (core not ready or not in Tauri)
    if (!status) {
      return { ...offlineStatus(), allPermissionsGranted: false, platformUnsupported: false };
    }

    if (!status.platform_supported) {
      return { ...unsupportedStatus(), allPermissionsGranted: false, platformUnsupported: true };
    }

    const { permissions, session, config } = status;
    const allGranted =
      permissions.screen_recording === 'granted' &&
      permissions.accessibility === 'granted' &&
      permissions.input_monitoring === 'granted';

    // Permissions missing — needs setup
    if (!allGranted) {
      return { ...setupRequiredStatus(), allPermissionsGranted: false, platformUnsupported: false };
    }

    // Session active — fully connected
    if (session.active) {
      return { ...activeStatus(), allPermissionsGranted: true, platformUnsupported: false };
    }

    // Permissions granted, enabled in config, but session not active
    if (config.enabled) {
      return { ...enabledStatus(), allPermissionsGranted: true, platformUnsupported: false };
    }

    // Permissions granted but not enabled
    return {
      ...offlineStatus('Disabled'),
      allPermissionsGranted: true,
      platformUnsupported: false,
    };
  }, [status]);
}
