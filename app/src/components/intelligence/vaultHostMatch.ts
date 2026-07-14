/**
 * Cross-host vault awareness (issue #4278).
 *
 * `openhuman-core` owns a single local filesystem. The memory-tree / Obsidian
 * vault physically lives under the CORE host's filesystem, and the vault RPCs
 * return that host's absolute path (`content_root_abs`) plus the core's OS
 * (`host_os`). When a frontend attaches from a DIFFERENT OS than the core, that
 * path cannot be opened or registered locally — "Reveal Folder" / "Open in
 * Obsidian" would act on a path that does not exist on this machine.
 *
 * These helpers compare the core's OS against this frontend's OS so the local
 * file integrations can be disabled with a clear message instead of silently
 * firing a doomed deep link or a cryptic opener failure.
 *
 * Backward compatibility: when `host_os` is absent (older core) or this
 * frontend's OS cannot be determined, treat the vault as local — never block a
 * single-host user on missing signal.
 */
import { platform } from '@tauri-apps/plugin-os';

import { isTauri } from '../../utils/tauriCommands/common';

/** Canonical OS tokens shared by Rust `std::env::consts::OS` and plugin-os. */
export type OsToken = 'macos' | 'linux' | 'windows';

/**
 * Normalize an OS string to a canonical token. Rust's `std::env::consts::OS`
 * and `@tauri-apps/plugin-os` both emit `"macos"`/`"linux"`/`"windows"`, but we
 * defensively fold common aliases (`darwin`, `win32`, …) so a future source
 * change can't silently break the comparison.
 */
export function normalizeOs(os: string | undefined | null): OsToken | undefined {
  if (!os) return undefined;
  const v = os.trim().toLowerCase();
  if (v === 'macos' || v === 'darwin' || v === 'mac' || v === 'osx') return 'macos';
  if (v === 'windows' || v === 'win' || v === 'win32') return 'windows';
  if (v === 'linux') return 'linux';
  return undefined;
}

/**
 * Pure decision: is a vault whose path lives on `hostOs` local to a frontend
 * running on `clientOs`?
 *
 * Returns `true` (local — safe to open) when either OS is unknown, so a missing
 * signal never disables local actions for a genuine single-host setup.
 */
export function isVaultLocalToThisDevice(
  hostOs: string | undefined | null,
  clientOs: string | undefined | null
): boolean {
  const host = normalizeOs(hostOs);
  const client = normalizeOs(clientOs);
  if (!host || !client) return true;
  return host === client;
}

/** Resolved cross-host state for a vault response. */
export interface VaultHostMatch {
  /** True when the vault path is openable on this device (same OS, or unknown). */
  local: boolean;
  /** Normalized core host OS, when known. */
  hostOs?: OsToken;
}

/**
 * Resolve the cross-host state for a vault response carrying `host_os`.
 *
 * Reads this frontend's OS via `@tauri-apps/plugin-os`. Outside Tauri, or if
 * the platform probe throws, falls back to "local" so non-desktop / preview
 * contexts are never blocked.
 */
export async function resolveVaultHostMatch(
  hostOs: string | undefined | null
): Promise<VaultHostMatch> {
  const normalizedHost = normalizeOs(hostOs);
  if (!normalizedHost) return { local: true };
  if (!isTauri()) return { local: true, hostOs: normalizedHost };
  try {
    const clientOs = await platform();
    return { local: isVaultLocalToThisDevice(normalizedHost, clientOs), hostOs: normalizedHost };
  } catch {
    // Can't determine this device's OS — don't block local actions.
    return { local: true, hostOs: normalizedHost };
  }
}
