/**
 * profileStore — secure storage for ConnectionProfile records.
 *
 * Two backends:
 *   Desktop: localStorage (sufficient for desktop; credentials protected by OS account)
 *   iOS:     TODO(Layer 5) — wire to tauri-plugin-stronghold or tauri-plugin-keychain
 *
 * ConnectionProfile contains the minimum required to select and authenticate a
 * transport: kind, rpcUrl, channelId, tokens, and key material.
 *
 * Key material (devicePrivkey, sessionToken) is sensitive — the iOS backend
 * must store these in the Secure Enclave via Keychain. On desktop, we store
 * in localStorage under the assumption that the device is single-user and
 * protected by OS-level login.
 */
import debug from 'debug';

import { getIsIOS } from '../../lib/platform';

const log = debug('transport:profile-store');

// -- types -------------------------------------------------------------------

export interface ConnectionProfile {
  /** Unique profile identifier. */
  id: string;
  /** Human-readable label, e.g. "Home desktop". */
  label: string;
  /** Transport kind this profile uses. */
  kind: 'local' | 'lan' | 'tunnel' | 'cloud';
  /** LAN or cloud HTTP RPC URL (for lan + cloud kinds). */
  rpcUrl?: string;
  /** Tunnel channel identifier (for tunnel kind). */
  channelId?: string;
  /** Tunnel session token for reconnects (for tunnel kind). */
  sessionToken?: string;
  /** Tunnel pairing token for first-time connect (for tunnel kind). */
  pairingToken?: string;
  /** Core's X25519 public key in base64url (for tunnel kind). */
  corePubkey?: string;
  /**
   * Device's X25519 private key in base64url.
   * SENSITIVE — on iOS this must be stored in Keychain (Layer 5).
   * On desktop we store it in localStorage.
   */
  devicePrivkey?: string;
}

// -- storage key prefix -------------------------------------------------------

const STORAGE_KEY_PREFIX = 'openhuman:transport:profile:';
const INDEX_KEY = 'openhuman:transport:profile:__index__';

// -- desktop backend ---------------------------------------------------------

function desktopList(): string[] {
  try {
    const raw = localStorage.getItem(INDEX_KEY);
    return raw ? (JSON.parse(raw) as string[]) : [];
  } catch {
    return [];
  }
}

function desktopSave(profile: ConnectionProfile): void {
  const ids = desktopList();
  if (!ids.includes(profile.id)) {
    ids.push(profile.id);
    localStorage.setItem(INDEX_KEY, JSON.stringify(ids));
  }
  localStorage.setItem(STORAGE_KEY_PREFIX + profile.id, JSON.stringify(profile));
  log('[profile-store] saved id=%s kind=%s', profile.id, profile.kind);
}

function desktopGet(id: string): ConnectionProfile | null {
  const raw = localStorage.getItem(STORAGE_KEY_PREFIX + id);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as ConnectionProfile;
  } catch {
    return null;
  }
}

function desktopDelete(id: string): void {
  const ids = desktopList().filter(i => i !== id);
  localStorage.setItem(INDEX_KEY, JSON.stringify(ids));
  localStorage.removeItem(STORAGE_KEY_PREFIX + id);
  log('[profile-store] deleted id=%s', id);
}

// -- iOS backend (pragmatic interim) ----------------------------------------
//
// iOS WebView storage is sandboxed per-app by the OS, so localStorage is
// protected from other apps on a non-jailbroken device.
//
// SECURITY TODO(post-Layer-7): migrate to Keychain via tauri-plugin-keychain
// or a custom Swift Tauri command.  Threat model for the interim solution:
//   PROTECTED: other apps (iOS sandbox), remote attackers.
//   NOT PROTECTED: jailbroken device, malicious WebView injection.
// For a v1 demo paired with a sandboxed WKWebView on a stock iOS device this
// is acceptable.  The key material (devicePrivkey, sessionToken) should be
// migrated to the Secure Enclave before public release.

// iOS uses the same localStorage implementation as desktop.  The functions
// are identical because the iOS WKWebView localStorage is app-sandboxed.
// This section is left as a named seam so Layer 7 can swap just the iOS path.

function iosList(): string[] {
  return desktopList();
}

function iosSave(profile: ConnectionProfile): void {
  desktopSave(profile);
  log('[profile-store:ios] saved id=%s kind=%s', profile.id, profile.kind);
}

function iosGet(id: string): ConnectionProfile | null {
  return desktopGet(id);
}

function iosDelete(id: string): void {
  desktopDelete(id);
  log('[profile-store:ios] deleted id=%s', id);
}

// -- platform selector -------------------------------------------------------
// We import getIsIOS() (not the isIOS constant) so that test overrides via
// setTestPlatform() are respected on each call rather than frozen at module
// load time (which is when the isIOS constant is evaluated).
function onIOS(): boolean {
  return getIsIOS();
}

// -- public API --------------------------------------------------------------

/** Save or update a profile. */
export function saveProfile(profile: ConnectionProfile): void {
  if (onIOS()) {
    iosSave(profile);
  } else {
    desktopSave(profile);
  }
}

/** Load a profile by id. Returns null if not found. */
export function getProfile(id: string): ConnectionProfile | null {
  return onIOS() ? iosGet(id) : desktopGet(id);
}

/** List all stored profile IDs. */
export function listProfileIds(): string[] {
  return onIOS() ? iosList() : desktopList();
}

/** Load all stored profiles. */
export function listProfiles(): ConnectionProfile[] {
  const ids = onIOS() ? iosList() : desktopList();
  const getter = onIOS() ? iosGet : desktopGet;
  return ids.map(getter).filter((p): p is ConnectionProfile => p !== null);
}

/** Delete a profile. */
export function deleteProfile(id: string): void {
  if (onIOS()) {
    iosDelete(id);
  } else {
    desktopDelete(id);
  }
}
