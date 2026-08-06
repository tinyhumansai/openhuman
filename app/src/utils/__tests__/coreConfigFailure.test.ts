import { describe, expect, it } from 'vitest';

import en from '../../lib/i18n/en';
import { CORE_CONFIG_UNREADABLE_I18N_KEY, isCoreConfigUnreadableError } from '../coreConfigFailure';

// The verbatim chain a container core emits when its workspace volume carries a
// config.toml owned by a different uid than the runtime process.
const REPORTED =
  'Failed to read config file: /home/openhuman/.openhuman/config.toml ' +
  '[config owner mismatch] (file uid=0 gid=0 mode=0600; process euid=10001 egid=10001): ' +
  'Permission denied (os error 13)';

describe('isCoreConfigUnreadableError', () => {
  it('matches the reported container failure', () => {
    expect(isCoreConfigUnreadableError(REPORTED)).toBe(true);
  });

  it('matches the pre-ownership-diagnostics shape still shipping in older cores', () => {
    expect(
      isCoreConfigUnreadableError(
        'Failed to read config file: /home/openhuman/.openhuman/config.toml: Permission denied (os error 13)'
      )
    ).toBe(true);
  });

  it('matches the Windows denial and the snapshot-reload context line', () => {
    expect(
      isCoreConfigUnreadableError(
        'Failed to read config file: C:\\Users\\u\\.openhuman\\users\\local\\config.toml: Access is denied. (os error 5)'
      )
    ).toBe(true);
    expect(
      isCoreConfigUnreadableError(
        'reading config.toml from /home/openhuman/.openhuman/config.toml: Permission denied (os error 13)'
      )
    ).toBe(true);
  });

  it('does not match an unrelated errno that merely shares a prefix', () => {
    // `os error 13` unanchored also matches `os error 130`, and `os error 5`
    // matches `os error 50`/`512`. The loader always emits the parenthesised
    // form, so the classifier keys on that.
    for (const errno of ['os error 130', 'os error 50', 'os error 512']) {
      expect(
        isCoreConfigUnreadableError(
          `Failed to read config file: /home/openhuman/.openhuman/config.toml: Some other failure (${errno})`
        )
      ).toBe(false);
    }
  });

  it('requires BOTH the config-read context and a denial signal', () => {
    // Permission failure from another subsystem keeps its own message.
    expect(isCoreConfigUnreadableError('opening keychain failed: Permission denied')).toBe(false);
    // Config-read failure that is not a denial (missing file, parse) is a
    // different fault with a different remedy.
    expect(
      isCoreConfigUnreadableError(
        'Failed to read config file: /home/openhuman/.openhuman/config.toml: No such file or directory (os error 2)'
      )
    ).toBe(false);
  });

  it('normalises its own input, so callers may pass a raw or lowered message', () => {
    expect(isCoreConfigUnreadableError(REPORTED.toLowerCase())).toBe(true);
    expect(isCoreConfigUnreadableError(REPORTED.toUpperCase())).toBe(true);
  });

  it('is safe on empty input', () => {
    expect(isCoreConfigUnreadableError(null)).toBe(false);
    expect(isCoreConfigUnreadableError(undefined)).toBe(false);
    expect(isCoreConfigUnreadableError('')).toBe(false);
  });
});

describe('CORE_CONFIG_UNREADABLE_I18N_KEY', () => {
  it('resolves to real English copy so the UI never renders a bare key', () => {
    const copy = (en as Record<string, string>)[CORE_CONFIG_UNREADABLE_I18N_KEY];
    expect(copy).toBeTruthy();
    // `toBeTruthy` alone would pass on a value that is literally the key, which
    // is exactly the "bare key reached the UI" case this test exists to rule out.
    expect(copy).not.toBe(CORE_CONFIG_UNREADABLE_I18N_KEY);
    // The person reading the sign-in screen cannot act on a container path, a
    // uid, or an errno, and the path is the runtime host's, not theirs.
    expect(copy).not.toMatch(/\/home\/openhuman|os error|uid=/);
    // Hedged, not asserted: the classifier also matches denials with no
    // ownership marker (older cores, Windows DACLs), where a flat "is owned by
    // another user" claim would be wrong.
    expect(copy).toMatch(/may/i);
    // Repo i18n rule: no em dashes in translation values.
    expect(copy).not.toContain('\u2014');
  });
});
