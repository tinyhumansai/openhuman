/**
 * Derives a skill-card-friendly status for Text Auto-Complete,
 * matching the state vocabulary used by third-party skills (Gmail, etc.).
 */
import { useMemo } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import {
  activeStatus,
  enabledStatus,
  errorStatus,
  offlineStatus,
  type SkillCardStatusDescriptor,
  unsupportedStatus,
} from '../skills/skillCardStatus';

export interface AutocompleteSkillStatus extends SkillCardStatusDescriptor {
  /** True when the platform doesn't support autocomplete. */
  platformUnsupported: boolean;
}

export function useAutocompleteSkillStatus(): AutocompleteSkillStatus {
  const { snapshot } = useCoreState();
  const status = snapshot.runtime.autocomplete;

  return useMemo(() => {
    // No status yet (core not ready or not in Tauri)
    if (!status) {
      return { ...offlineStatus(), platformUnsupported: false };
    }

    if (!status.platform_supported) {
      return { ...unsupportedStatus(), platformUnsupported: true };
    }

    // Running — fully active (checked before error so a stale last_error
    // doesn't mask a successfully running service)
    if (status.running) {
      return { ...activeStatus(), platformUnsupported: false };
    }

    // Error state (only when not running)
    if (status.last_error) {
      return { ...errorStatus(), platformUnsupported: false };
    }

    // Enabled in config but not running
    if (status.enabled) {
      return { ...enabledStatus(), platformUnsupported: false };
    }

    // Not enabled
    return { ...offlineStatus('Disabled'), platformUnsupported: false };
  }, [status]);
}
