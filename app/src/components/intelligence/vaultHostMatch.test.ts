/**
 * Unit tests for cross-host vault detection (#4278).
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { isVaultLocalToThisDevice, normalizeOs, resolveVaultHostMatch } from './vaultHostMatch';

const isTauriMock = vi.fn();
const platformMock = vi.fn();

vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: () => isTauriMock() }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => platformMock() }));

describe('normalizeOs', () => {
  it('folds known aliases to canonical tokens', () => {
    expect(normalizeOs('macos')).toBe('macos');
    expect(normalizeOs('Darwin')).toBe('macos');
    expect(normalizeOs('win32')).toBe('windows');
    expect(normalizeOs('Windows')).toBe('windows');
    expect(normalizeOs('linux')).toBe('linux');
  });

  it('returns undefined for empty / unknown values', () => {
    expect(normalizeOs(undefined)).toBeUndefined();
    expect(normalizeOs(null)).toBeUndefined();
    expect(normalizeOs('')).toBeUndefined();
    expect(normalizeOs('plan9')).toBeUndefined();
  });
});

describe('isVaultLocalToThisDevice', () => {
  it('is true only when both OSes are known and equal', () => {
    expect(isVaultLocalToThisDevice('macos', 'macos')).toBe(true);
    expect(isVaultLocalToThisDevice('linux', 'linux')).toBe(true);
  });

  it('is false when the OSes differ', () => {
    expect(isVaultLocalToThisDevice('macos', 'linux')).toBe(false);
    expect(isVaultLocalToThisDevice('linux', 'windows')).toBe(false);
  });

  it('defaults to local (true) when either OS is unknown — never block on missing signal', () => {
    expect(isVaultLocalToThisDevice(undefined, 'macos')).toBe(true);
    expect(isVaultLocalToThisDevice('macos', undefined)).toBe(true);
    expect(isVaultLocalToThisDevice(undefined, undefined)).toBe(true);
  });
});

describe('resolveVaultHostMatch', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('treats a vault as local when the core reports no host_os (older core)', async () => {
    isTauriMock.mockReturnValue(true);
    const m = await resolveVaultHostMatch(undefined);
    expect(m).toEqual({ local: true });
    expect(platformMock).not.toHaveBeenCalled();
  });

  it('treats a vault as local outside Tauri', async () => {
    isTauriMock.mockReturnValue(false);
    const m = await resolveVaultHostMatch('linux');
    expect(m).toEqual({ local: true, hostOs: 'linux' });
    expect(platformMock).not.toHaveBeenCalled();
  });

  it('flags cross-host when the core OS differs from this device', async () => {
    isTauriMock.mockReturnValue(true);
    platformMock.mockResolvedValue('macos');
    const m = await resolveVaultHostMatch('linux');
    expect(m).toEqual({ local: false, hostOs: 'linux' });
  });

  it('reports local when the core OS matches this device', async () => {
    isTauriMock.mockReturnValue(true);
    platformMock.mockResolvedValue('linux');
    const m = await resolveVaultHostMatch('linux');
    expect(m).toEqual({ local: true, hostOs: 'linux' });
  });

  it('falls back to local when the platform probe throws', async () => {
    isTauriMock.mockReturnValue(true);
    platformMock.mockRejectedValue(new Error('no os'));
    const m = await resolveVaultHostMatch('windows');
    expect(m).toEqual({ local: true, hostOs: 'windows' });
  });
});
