/**
 * Shared Vitest mocks for screen-intelligence / autocomplete / voice status hooks.
 * Import this module first in Skills page tests so `Skills` does not require `CoreStateProvider`.
 */
import { vi } from 'vitest';

/** Shared offline-shaped fields for skill status hook mocks (avoid drift across hooks). */
const offlineStatusBase = {
  connectionStatus: 'offline' as const,
  statusDot: 'bg-stone-400',
  statusLabel: 'Offline',
  statusColor: 'text-stone-500',
  ctaLabel: 'Enable',
  ctaVariant: 'sage' as const,
};

vi.mock('../features/screen-intelligence/useScreenIntelligenceSkillStatus', () => ({
  useScreenIntelligenceSkillStatus: () => ({
    ...offlineStatusBase,
    allPermissionsGranted: false,
    platformUnsupported: false,
  }),
}));

vi.mock('../features/autocomplete/useAutocompleteSkillStatus', () => ({
  useAutocompleteSkillStatus: () => ({ ...offlineStatusBase, platformUnsupported: false }),
}));

vi.mock('../features/voice/useVoiceSkillStatus', () => ({
  useVoiceSkillStatus: () => ({
    ...offlineStatusBase,
    sttModelMissing: false,
    voiceStatus: null,
    serverStatus: null,
  }),
}));
