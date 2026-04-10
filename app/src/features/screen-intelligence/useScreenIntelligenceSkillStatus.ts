/**
 * Derives a skill-card-friendly status for Screen Intelligence,
 * matching the state vocabulary used by third-party skills (Gmail, etc.).
 */
import { useMemo } from 'react';

import type { SkillConnectionStatus } from '../../lib/skills/types';
import { useCoreState } from '../../providers/CoreStateProvider';

export interface ScreenIntelligenceSkillStatus {
  connectionStatus: SkillConnectionStatus;
  statusDot: string;
  statusLabel: string;
  statusColor: string;
  ctaLabel: string;
  ctaVariant: 'primary' | 'sage' | 'amber';
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
      return {
        connectionStatus: 'offline' as SkillConnectionStatus,
        statusDot: 'bg-stone-400',
        statusLabel: 'Offline',
        statusColor: 'text-stone-500',
        ctaLabel: 'Enable',
        ctaVariant: 'sage' as const,
        allPermissionsGranted: false,
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
        allPermissionsGranted: false,
        platformUnsupported: true,
      };
    }

    const { permissions, session, config } = status;
    const allGranted =
      permissions.screen_recording === 'granted' &&
      permissions.accessibility === 'granted' &&
      permissions.input_monitoring === 'granted';

    // Permissions missing — needs setup
    if (!allGranted) {
      return {
        connectionStatus: 'setup_required' as SkillConnectionStatus,
        statusDot: 'bg-primary-400',
        statusLabel: 'Setup',
        statusColor: 'text-primary-400',
        ctaLabel: 'Setup',
        ctaVariant: 'primary' as const,
        allPermissionsGranted: false,
        platformUnsupported: false,
      };
    }

    // Session active — fully connected
    if (session.active) {
      return {
        connectionStatus: 'connected' as SkillConnectionStatus,
        statusDot: 'bg-sage-500',
        statusLabel: 'Active',
        statusColor: 'text-sage-400',
        ctaLabel: 'Manage',
        ctaVariant: 'primary' as const,
        allPermissionsGranted: true,
        platformUnsupported: false,
      };
    }

    // Permissions granted, enabled in config, but session not active
    if (config.enabled) {
      return {
        connectionStatus: 'disconnected' as SkillConnectionStatus,
        statusDot: 'bg-stone-400',
        statusLabel: 'Enabled',
        statusColor: 'text-stone-400',
        ctaLabel: 'Manage',
        ctaVariant: 'primary' as const,
        allPermissionsGranted: true,
        platformUnsupported: false,
      };
    }

    // Permissions granted but not enabled
    return {
      connectionStatus: 'offline' as SkillConnectionStatus,
      statusDot: 'bg-stone-400',
      statusLabel: 'Disabled',
      statusColor: 'text-stone-500',
      ctaLabel: 'Enable',
      ctaVariant: 'sage' as const,
      allPermissionsGranted: true,
      platformUnsupported: false,
    };
  }, [status]);
}
