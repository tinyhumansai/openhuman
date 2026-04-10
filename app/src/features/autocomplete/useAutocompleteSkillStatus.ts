/**
 * Derives a skill-card-friendly status for Text Auto-Complete,
 * matching the state vocabulary used by third-party skills (Gmail, etc.).
 */
import { useMemo } from 'react';

import type { SkillConnectionStatus } from '../../lib/skills/types';
import { useCoreState } from '../../providers/CoreStateProvider';

export interface AutocompleteSkillStatus {
  connectionStatus: SkillConnectionStatus;
  statusDot: string;
  statusLabel: string;
  statusColor: string;
  ctaLabel: string;
  ctaVariant: 'primary' | 'sage' | 'amber';
  /** True when the platform doesn't support autocomplete. */
  platformUnsupported: boolean;
}

export function useAutocompleteSkillStatus(): AutocompleteSkillStatus {
  const { snapshot } = useCoreState();
  const status = snapshot.runtime.autocomplete;

  return useMemo(() => {
    // No status yet (core not ready or not in Tauri)
    if (!status) {
      return {
        connectionStatus: 'offline' as SkillConnectionStatus,
        statusDot: 'bg-stone-400',
        statusLabel: 'Offline',
        statusColor: 'text-stone-500',
        ctaLabel: 'Enable',
        ctaVariant: 'sage' as const,
        platformUnsupported: false,
      };
    }

    if (!status.platform_supported) {
      return {
        connectionStatus: 'offline' as SkillConnectionStatus,
        statusDot: 'bg-stone-400',
        statusLabel: 'Unsupported',
        statusColor: 'text-stone-500',
        ctaLabel: 'Details',
        ctaVariant: 'primary' as const,
        platformUnsupported: true,
      };
    }

    // Running — fully active (checked before error so a stale last_error
    // doesn't mask a successfully running service)
    if (status.running) {
      return {
        connectionStatus: 'connected' as SkillConnectionStatus,
        statusDot: 'bg-sage-500',
        statusLabel: 'Active',
        statusColor: 'text-sage-400',
        ctaLabel: 'Manage',
        ctaVariant: 'primary' as const,
        platformUnsupported: false,
      };
    }

    // Error state (only when not running)
    if (status.last_error) {
      return {
        connectionStatus: 'error' as SkillConnectionStatus,
        statusDot: 'bg-coral-500',
        statusLabel: 'Error',
        statusColor: 'text-coral-400',
        ctaLabel: 'Retry',
        ctaVariant: 'amber' as const,
        platformUnsupported: false,
      };
    }

    // Enabled in config but not running
    if (status.enabled) {
      return {
        connectionStatus: 'disconnected' as SkillConnectionStatus,
        statusDot: 'bg-stone-400',
        statusLabel: 'Enabled',
        statusColor: 'text-stone-400',
        ctaLabel: 'Manage',
        ctaVariant: 'primary' as const,
        platformUnsupported: false,
      };
    }

    // Not enabled
    return {
      connectionStatus: 'offline' as SkillConnectionStatus,
      statusDot: 'bg-stone-400',
      statusLabel: 'Disabled',
      statusColor: 'text-stone-500',
      ctaLabel: 'Enable',
      ctaVariant: 'sage' as const,
      platformUnsupported: false,
    };
  }, [status]);
}
