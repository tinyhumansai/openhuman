export const LOCAL_SESSION_USER_ID = 'local';

export const LOCAL_SESSION_USER = {
  _id: LOCAL_SESSION_USER_ID,
  id: LOCAL_SESSION_USER_ID,
  name: 'Local User',
  email: 'local@openhuman.local',
};

function base64UrlEncode(value: object): string {
  return globalThis
    .btoa(JSON.stringify(value))
    .replace(/=/g, '')
    .replace(/\+/g, '-')
    .replace(/\//g, '_');
}

export function createLocalSessionToken(nowMs = Date.now()): string {
  const now = Math.floor(nowMs / 1000);
  return [
    base64UrlEncode({ alg: 'none', typ: 'JWT' }),
    base64UrlEncode({
      sub: LOCAL_SESSION_USER_ID,
      user_id: LOCAL_SESSION_USER_ID,
      iat: now,
      exp: now + 31536000,
    }),
    'local',
  ].join('.');
}

export function isLocalSessionToken(token: string | null | undefined): boolean {
  if (!token) return false;
  const parts = token.split('.');
  return parts.length === 3 && parts[2] === 'local';
}

/** The credential kinds the core reports in `auth.credential`. */
export type CoreCredentialKind = 'session' | 'api-key' | 'local';

interface HostedAccountSnapshot {
  auth: { isAuthenticated: boolean; credential?: CoreCredentialKind | string | null };
  sessionToken: string | null;
}

/**
 * Whether the signed-in credential is backed by a TinyHumans account, so the
 * hosted account surfaces (usage, billing, team, announcements, invites) can
 * answer. False for the offline local profile and when signed out. The core
 * refuses those calls anyway (`BACKEND_UNAVAILABLE:`); this only spares the
 * round trip. Falls back to the session token shape for older cores that do
 * not report `auth.credential`.
 */
export function hasHostedAccount(snapshot: HostedAccountSnapshot): boolean {
  if (!snapshot.auth.isAuthenticated) return false;
  const credential = snapshot.auth.credential;
  if (credential) return credential !== 'local';
  return !isLocalSessionToken(snapshot.sessionToken);
}
