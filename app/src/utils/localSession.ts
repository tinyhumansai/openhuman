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
