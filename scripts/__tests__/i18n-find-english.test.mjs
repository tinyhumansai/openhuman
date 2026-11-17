import assert from 'node:assert/strict';
import { test } from 'node:test';

import { looksEnglish } from '../i18n-find-english.ts';

test('looksEnglish flags a value with two English-only function words', () => {
  assert.equal(looksEnglish('Save the file before you close it.'), true);
  assert.equal(looksEnglish('Save the file before you close it.', 'de'), true);
});

test('looksEnglish ignores English-list words that are ordinary Turkish words', () => {
  assert.equal(looksEnglish('Not: can sıkıcı olabilir', 'tr'), false);
  assert.equal(looksEnglish('Not: may ekleyin', 'tr'), false);
  assert.equal(looksEnglish('Has üzüm must', 'tr'), false);
  assert.equal(looksEnglish('Had aşıldı, don uyarısı', 'tr'), false);
});

test('looksEnglish still flags real English left in a Turkish value', () => {
  assert.equal(looksEnglish('Note: you can not undo this.', 'tr'), true);
});

test('Turkish-only exclusions do not leak into other locales', () => {
  assert.equal(looksEnglish('Not: can sıkıcı olabilir', 'de'), true);
});
