/**
 * SecurityPanel — the unknown-mode fallback.
 *
 * `SecurityPanel.test.tsx` (sibling) is thorough: 18 tests, 100% line and
 * function coverage. One branch survives it — the
 * `?? MODE_BADGE_VARIANT.consent_pending` fallback at `SecurityPanel.tsx:24-25`,
 * which only runs when `keyringStatus.activeMode` is a value the UI does not
 * know.
 *
 * That is a version-skew path, not dead code: `activeMode` is typed as a plain
 * string and arrives from the core, so a core that grows a fifth mode hands this
 * panel something absent from `MODE_BADGE_VARIANT`.
 *
 * **The fallback itself cannot be asserted, and this file does not pretend to.**
 * `Badge` declares `defaultVariants: { variant: 'neutral' }` (`ui/Badge.tsx:22`)
 * and `consent_pending` maps to `'neutral'`, so an undefined variant and the
 * fallback render byte-identical classes. Deleting the `??` changes nothing
 * observable — I verified that by removing it, and dropped the test that
 * claimed to cover it rather than keep one that always passes. The `??` is
 * harmless belt-and-braces; it is not load-bearing.
 *
 * What IS asserted below: an unknown mode still renders (no crash, no blank
 * row), and the four known modes stay visually distinguishable — which is the
 * signal this row actually carries.
 */
import { cleanup } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import SecurityPanel from '../SecurityPanel';

vi.mock('../../../../services/keyringApi', () => ({
  retryKeyringProbe: vi.fn(),
  decideKeyringConsent: vi.fn(),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

const mockUseCoreState = vi.fn();
vi.mock('../../../../providers/CoreStateProvider', () => ({
  useCoreState: () => mockUseCoreState(),
}));

function render(activeMode: string) {
  mockUseCoreState.mockReturnValue({
    snapshot: {
      keyringStatus: {
        activeMode,
        available: true,
        backendName: 'macOS Keychain',
        failureReason: null,
      },
    },
  });
  const { container } = renderWithProviders(<SecurityPanel />);
  // Scope to THIS mount's container: RTL appends a new one per render, so a
  // document-wide query would keep returning the first mount's badge.
  const badge = container.querySelector('[data-slot="badge"]');
  if (!badge) throw new Error('no badge rendered');
  return badge;
}

afterEach(() => cleanup());

describe('SecurityPanel — unknown storage mode', () => {
  it('still renders the mode the core reported, even when unrecognised', () => {
    const badge = render('hardware_token_v2');

    // The raw mode reaches the label (via a missing i18n key, so the key shows).
    expect(badge.textContent).toContain('hardware_token_v2');
  });

  it('uses a distinct badge styling for each mode it does know', () => {
    const seen = new Map<string, string>();
    for (const mode of ['os_keyring', 'local_encrypted', 'consent_pending', 'declined']) {
      cleanup();
      seen.set(mode, render(mode).className);
    }

    // Four modes, four different variants — if two collapsed to the same class
    // the badge would stop distinguishing "stored in the OS keyring" from
    // "declined", which is the whole signal this row carries.
    expect(new Set(seen.values()).size).toBe(4);
  });
});
