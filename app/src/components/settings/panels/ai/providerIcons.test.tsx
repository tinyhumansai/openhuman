/**
 * The icon map is keyed by provider slug, so a key that matches no real
 * provider is a mark that never renders and nobody notices. These pin the keys
 * against the actual provider list rather than against a copy of it.
 */
import { describe, expect, it } from 'vitest';

import { BUILTIN_CLOUD_PROVIDER_SLUGS } from '../builtinCloudProviders';
import { PROVIDER_ICON_SLUGS, providerIcon } from './providerIcons';

/** Slugs the panel renders that are not built-in cloud providers. */
const NON_CLOUD_SLUGS = ['openhuman', 'claude-code', 'ollama', 'lmstudio', 'omlx'];

describe('providerIcons', () => {
  it('keys every icon to a slug the panel actually renders', () => {
    const known = new Set<string>([...BUILTIN_CLOUD_PROVIDER_SLUGS, ...NON_CLOUD_SLUGS]);
    const orphans = PROVIDER_ICON_SLUGS.filter(slug => !known.has(slug));

    expect(orphans, 'icon keyed to a slug no provider uses').toEqual([]);
  });

  it('returns null for an unknown custom endpoint, so the caller can letter it', () => {
    expect(providerIcon('a-provider-that-does-not-exist', 'h-4 w-4')).toBeNull();
  });

  it('returns a rendered mark for every built-in and local provider', () => {
    for (const slug of [...BUILTIN_CLOUD_PROVIDER_SLUGS, ...NON_CLOUD_SLUGS]) {
      expect(providerIcon(slug, 'h-4 w-4'), slug).toHaveProperty('type');
    }
  });
});
