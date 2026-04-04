import { describe, expect, it } from 'vitest';

import { AUTH_MODE_LABELS, FALLBACK_DEFINITIONS, STATUS_STYLES } from '../definitions';

describe('FALLBACK_DEFINITIONS', () => {
  it('contains telegram, discord, and web', () => {
    const ids = FALLBACK_DEFINITIONS.map(d => d.id);
    expect(ids).toContain('telegram');
    expect(ids).toContain('discord');
    expect(ids).toContain('web');
  });

  it('every definition has at least one auth mode', () => {
    for (const def of FALLBACK_DEFINITIONS) {
      expect(def.auth_modes.length).toBeGreaterThan(0);
    }
  });

  it('every definition has at least one capability', () => {
    for (const def of FALLBACK_DEFINITIONS) {
      expect(def.capabilities.length).toBeGreaterThan(0);
    }
  });

  it('telegram has bot_token and managed_dm auth modes', () => {
    const tg = FALLBACK_DEFINITIONS.find(d => d.id === 'telegram')!;
    const modes = tg.auth_modes.map(m => m.mode);
    expect(modes).toContain('bot_token');
    expect(modes).toContain('managed_dm');
  });

  it('discord has bot_token and oauth auth modes', () => {
    const dc = FALLBACK_DEFINITIONS.find(d => d.id === 'discord')!;
    const modes = dc.auth_modes.map(m => m.mode);
    expect(modes).toContain('bot_token');
    expect(modes).toContain('oauth');
  });
});

describe('STATUS_STYLES', () => {
  it('covers all four connection statuses', () => {
    expect(STATUS_STYLES).toHaveProperty('connected');
    expect(STATUS_STYLES).toHaveProperty('connecting');
    expect(STATUS_STYLES).toHaveProperty('disconnected');
    expect(STATUS_STYLES).toHaveProperty('error');
  });

  it('each status has a label and className', () => {
    for (const status of Object.values(STATUS_STYLES)) {
      expect(status.label).toBeTruthy();
      expect(status.className).toBeTruthy();
    }
  });
});

describe('AUTH_MODE_LABELS', () => {
  it('has labels for all standard auth modes', () => {
    expect(AUTH_MODE_LABELS).toHaveProperty('managed_dm');
    expect(AUTH_MODE_LABELS).toHaveProperty('oauth');
    expect(AUTH_MODE_LABELS).toHaveProperty('bot_token');
    expect(AUTH_MODE_LABELS).toHaveProperty('api_key');
  });
});
