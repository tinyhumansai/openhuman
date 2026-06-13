import { describe, expect, it } from 'vitest';

import { SCHEDULE_PRESET_VALUES, SCHEDULE_PRESETS } from './schedulePresets';

describe('SCHEDULE_PRESETS', () => {
  it('is non-empty', () => {
    expect(SCHEDULE_PRESETS.length).toBeGreaterThan(0);
  });

  it('all values are valid 5-field cron expressions', () => {
    for (const preset of SCHEDULE_PRESETS) {
      const fields = preset.value.trim().split(/\s+/);
      expect(fields).toHaveLength(5);
    }
  });

  it('has no duplicate cron expression values', () => {
    const values = SCHEDULE_PRESETS.map(p => p.value);
    const unique = new Set(values);
    expect(unique.size).toBe(values.length);
  });

  it('has no duplicate labelKeys', () => {
    const keys = SCHEDULE_PRESETS.map(p => p.labelKey);
    const unique = new Set(keys);
    expect(unique.size).toBe(keys.length);
  });
});

describe('SCHEDULE_PRESET_VALUES', () => {
  it('contains all preset values from SCHEDULE_PRESETS', () => {
    for (const preset of SCHEDULE_PRESETS) {
      expect(SCHEDULE_PRESET_VALUES.has(preset.value)).toBe(true);
    }
  });

  it('has same size as SCHEDULE_PRESETS', () => {
    expect(SCHEDULE_PRESET_VALUES.size).toBe(SCHEDULE_PRESETS.length);
  });
});
