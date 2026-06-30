import type { SkillConnectionStatus } from '../../types/skillStatus';

export interface SkillCardStatusDescriptor {
  connectionStatus: SkillConnectionStatus;
  statusDot: string;
  statusLabel: string;
  statusColor: string;
  ctaLabel: string;
  ctaVariant: 'primary' | 'sage' | 'amber';
}

export function offlineStatus(label = 'Offline', ctaLabel = 'Enable'): SkillCardStatusDescriptor {
  return {
    connectionStatus: 'offline',
    statusDot: 'bg-stone-400',
    statusLabel: label,
    statusColor: 'text-content-muted',
    ctaLabel,
    ctaVariant: 'sage',
  };
}

export function unsupportedStatus(): SkillCardStatusDescriptor {
  return { ...offlineStatus('Unsupported', 'Details'), ctaVariant: 'primary' };
}

export function setupRequiredStatus(): SkillCardStatusDescriptor {
  return {
    connectionStatus: 'setup_required',
    statusDot: 'bg-primary-400',
    statusLabel: 'Setup',
    statusColor: 'text-primary-400',
    ctaLabel: 'Setup',
    ctaVariant: 'primary',
  };
}

export function errorStatus(): SkillCardStatusDescriptor {
  return {
    connectionStatus: 'error',
    statusDot: 'bg-coral-500',
    statusLabel: 'Error',
    statusColor: 'text-coral-400',
    ctaLabel: 'Retry',
    ctaVariant: 'amber',
  };
}

export function activeStatus(label = 'Active'): SkillCardStatusDescriptor {
  return {
    connectionStatus: 'connected',
    statusDot: 'bg-sage-500',
    statusLabel: label,
    statusColor: 'text-sage-400',
    ctaLabel: 'Manage',
    ctaVariant: 'primary',
  };
}

export function transientStatus(label: string): SkillCardStatusDescriptor {
  return {
    connectionStatus: 'connecting',
    statusDot: 'bg-amber-500 animate-pulse',
    statusLabel: label,
    statusColor: 'text-amber-400',
    ctaLabel: 'Manage',
    ctaVariant: 'primary',
  };
}

export function enabledStatus(): SkillCardStatusDescriptor {
  return {
    connectionStatus: 'disconnected',
    statusDot: 'bg-stone-400',
    statusLabel: 'Enabled',
    statusColor: 'text-content-faint',
    ctaLabel: 'Manage',
    ctaVariant: 'primary',
  };
}
