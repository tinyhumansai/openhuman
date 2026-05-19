import { describe, expect, it } from 'vitest';

import { CHANNEL_ICONS } from '../MessagingPanel';

describe('MessagingPanel CHANNEL_ICONS', () => {
  it('includes icons for every shipped channel slug', () => {
    // The backend `channels::controllers::definitions::all_channel_definitions`
    // emits these icon slugs; the Messaging panel must render an emoji for
    // each or the channel row gets a blank gap. Pin them by key so a renamed
    // slug on either side fails this test instead of a silent visual break.
    expect(CHANNEL_ICONS.telegram).toBe('✈️');
    expect(CHANNEL_ICONS.discord).toBe('🎮');
    expect(CHANNEL_ICONS.web).toBe('🌐');
  });

  it('includes Lark/Feishu and DingTalk icons (#2048)', () => {
    // Regression for #2048 — adding the channel definitions without the
    // matching icon entries produced a blank chip next to each channel in
    // the Messaging settings panel.
    expect(CHANNEL_ICONS.lark).toBe('🪶');
    expect(CHANNEL_ICONS.dingtalk).toBe('🔔');
  });

  it('has no duplicate emoji values (icons remain visually distinct)', () => {
    // Two channels sharing the same emoji would make their rows visually
    // indistinguishable in the panel. Asserting uniqueness here catches
    // the easy copy-paste mistake at test time.
    const values = Object.values(CHANNEL_ICONS);
    const unique = new Set(values);
    expect(unique.size).toBe(values.length);
  });
});
