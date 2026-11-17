import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

import { chatErrorCopyText } from './chatErrorCopy';
import en from './i18n/en';

const table: Record<string, string> = {
  ...(en as Record<string, string>),
  'chat_error.timeout': 'LOCALIZED timeout',
};
const t = (key: string, fallback?: string) => table[key] ?? fallback ?? key;

describe('chatErrorCopyText', () => {
  it('renders a known key from the translation table', () => {
    expect(chatErrorCopyText({ message: 'english', copy_key: 'chat_error.timeout' }, t)).toBe(
      'LOCALIZED timeout'
    );
  });

  it('appends the retry hint and the quoted provider detail', () => {
    const text = chatErrorCopyText(
      {
        message: 'english',
        copy_key: 'chat_error.managed_rate_limited',
        copy_params: { retry_after_secs: 45, provider: 'openhuman', detail: 'slow down' },
      },
      t
    );
    expect(text).toBe(
      'Your AI provider is rate-limiting requests. You can retry in this thread. Try again in 45 seconds.\n\n> slow down'
    );
  });

  it.each([
    [0, 'You can retry immediately.'],
    [1, 'Try again in 1 second.'],
    [89, 'Try again in 89 seconds.'],
    [90, 'Try again in about 2 minutes.'],
    [61 * 60, 'Try again in about 61 minutes.'],
    [60 * 60 + 1, 'Try again in about 61 minutes.'],
  ])('formats a %i second retry hint like the core', (secs, hint) => {
    const text = chatErrorCopyText(
      {
        message: 'english',
        copy_key: 'chat_error.rate_limited',
        copy_params: { retry_after_secs: secs },
      },
      t
    );
    expect(text.endsWith(` ${hint}`)).toBe(true);
  });

  it('falls back to message for an unknown key', () => {
    expect(chatErrorCopyText({ message: 'english', copy_key: 'chat_error.nope' }, t)).toBe(
      'english'
    );
  });

  it('falls back to message for a key outside the chat_error namespace', () => {
    expect(chatErrorCopyText({ message: 'english', copy_key: 'nav.home' }, t)).toBe('english');
  });

  it('falls back to message when there is no key', () => {
    expect(chatErrorCopyText({ message: 'english' }, t)).toBe('english');
  });

  it('has an English string for every provider and host failure-copy key', () => {
    const hostTablePath = resolve(
      process.cwd(),
      '../crates/openhuman-core/src/inference/failure_copy/table.rs'
    );
    const providerTablePath = resolve(
      process.cwd(),
      '../vendor/tinyagents/vendor/tinyinference/crates/tinyinference-llm/src/failure/copy.rs'
    );
    const failureCopySources = [
      readFileSync(hostTablePath, 'utf8'),
      readFileSync(providerTablePath, 'utf8'),
    ].join('\n');
    const keys = [...failureCopySources.matchAll(/"(chat_error\.[a-z_]+)"/g)].map(m => m[1]);
    expect(keys.length).toBeGreaterThanOrEqual(28);
    const missing = keys.filter(key => !(key in (en as Record<string, string>)));
    expect(missing).toEqual([]);
  });
});
