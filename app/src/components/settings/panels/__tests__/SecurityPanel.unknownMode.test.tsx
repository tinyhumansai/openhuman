/**
 * SecurityPanel — the unknown-mode fallback.
 *
 * `SecurityPanel.test.tsx` (sibling) is thorough: it covers every mode's label
 * and the whole consent flow. One branch survives it — the
 * `?? MODE_BADGE_VARIANT.consent_pending` fallback, which only runs when
 * `keyringStatus.activeMode` is a value the UI does not know.
 *
 * That is a version-skew path, not dead code: `activeMode` is typed as a plain
 * string and arrives from the core, so a core that grows a new mode hands this
 * panel something absent from `MODE_BADGE_VARIANT`. It has happened once
 * already — #6076 added `local_encrypted_file` and `local_plaintext_file`, and
 * an older UI build against a newer core lands exactly here.
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
 * row), the four consent-outcome modes stay visually distinguishable, and all
 * six modes carry distinct, translated labels — which is the signal this row
 * actually carries.
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

    // The raw mode reaches the label. `MODE_I18N_KEY` deliberately falls
    // through to the raw `activeMode` for an unknown mode, so the lookup still
    // misses and the key shows — rather than silently borrowing the label of a
    // mode this build does know.
    expect(badge.textContent).toContain('hardware_token_v2');
  });

  it('uses a distinct badge styling for each consent-outcome mode', () => {
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

  it('gives every mode a distinct label, including the backend-configured ones', () => {
    // There are six modes and only four badge variants, so the two
    // operator-configured backends (#6076) necessarily share a variant with a
    // consent outcome — `local_encrypted_file` with `os_keyring` (both are a
    // deliberate, encrypted posture) and `local_plaintext_file` with `declined`.
    // The *label* is what has to stay unambiguous, so assert that instead of
    // widening the variant assertion above into something impossible.
    const labels = new Set<string>();
    for (const mode of [
      'os_keyring',
      'local_encrypted',
      'local_encrypted_file',
      'local_plaintext_file',
      'consent_pending',
      'declined',
    ]) {
      cleanup();
      labels.add(render(mode).textContent ?? '');
    }

    expect(labels.size).toBe(6);
    // And none of them fell through to a raw i18n key.
    for (const label of labels) {
      expect(label).not.toContain('keyring.settings.mode.');
    }
  });

  it('never labels the plaintext file backend as encrypted', () => {
    // `file` is plaintext dev-keychain.json. Reusing the `local_encrypted`
    // label here would restate #6076 in the UI layer.
    const badge = render('local_plaintext_file');
    expect(badge.textContent).toBe('Unencrypted file');
  });
});
