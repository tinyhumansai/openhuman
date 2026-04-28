import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { useAutocompleteSkillStatus } from '../useAutocompleteSkillStatus';

vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: vi.fn() }));

type AutocompleteRuntime = {
  platform_supported: boolean;
  running: boolean;
  enabled: boolean;
  last_error?: string | null;
};

function mockSnapshot(autocomplete: AutocompleteRuntime | null): void {
  vi.mocked(useCoreState).mockReturnValue({ snapshot: { runtime: { autocomplete } } } as ReturnType<
    typeof useCoreState
  >);
}

describe('useAutocompleteSkillStatus (5.2 — autocomplete settings status)', () => {
  it('returns offline + Enable CTA when no runtime status is available yet', () => {
    mockSnapshot(null);
    const { result } = renderHook(() => useAutocompleteSkillStatus());
    expect(result.current.connectionStatus).toBe('offline');
    expect(result.current.statusLabel).toBe('Offline');
    expect(result.current.ctaLabel).toBe('Enable');
    expect(result.current.platformUnsupported).toBe(false);
  });

  it('returns Unsupported when the platform reports the runtime is unsupported', () => {
    mockSnapshot({ platform_supported: false, running: false, enabled: false });
    const { result } = renderHook(() => useAutocompleteSkillStatus());
    expect(result.current.connectionStatus).toBe('offline');
    expect(result.current.statusLabel).toBe('Unsupported');
    expect(result.current.ctaLabel).toBe('Details');
    expect(result.current.platformUnsupported).toBe(true);
  });

  it('returns Active + Manage CTA when the runtime is running (overrides stale errors)', () => {
    mockSnapshot({
      platform_supported: true,
      running: true,
      enabled: true,
      last_error: 'stale: should not surface',
    });
    const { result } = renderHook(() => useAutocompleteSkillStatus());
    expect(result.current.connectionStatus).toBe('connected');
    expect(result.current.statusLabel).toBe('Active');
    expect(result.current.ctaLabel).toBe('Manage');
  });

  it('returns Error + Retry CTA when not running and an error is present', () => {
    mockSnapshot({
      platform_supported: true,
      running: false,
      enabled: true,
      last_error: 'permission denied',
    });
    const { result } = renderHook(() => useAutocompleteSkillStatus());
    expect(result.current.connectionStatus).toBe('error');
    expect(result.current.statusLabel).toBe('Error');
    expect(result.current.ctaLabel).toBe('Retry');
    expect(result.current.ctaVariant).toBe('amber');
  });

  it('returns Enabled (disconnected) + Manage when enabled but not running and no error', () => {
    mockSnapshot({ platform_supported: true, running: false, enabled: true, last_error: null });
    const { result } = renderHook(() => useAutocompleteSkillStatus());
    expect(result.current.connectionStatus).toBe('disconnected');
    expect(result.current.statusLabel).toBe('Enabled');
    expect(result.current.ctaLabel).toBe('Manage');
  });

  it('returns Disabled + Enable CTA when not enabled and not running', () => {
    mockSnapshot({ platform_supported: true, running: false, enabled: false, last_error: null });
    const { result } = renderHook(() => useAutocompleteSkillStatus());
    expect(result.current.connectionStatus).toBe('offline');
    expect(result.current.statusLabel).toBe('Disabled');
    expect(result.current.ctaLabel).toBe('Enable');
    expect(result.current.ctaVariant).toBe('sage');
  });
});
