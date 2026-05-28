export interface WorkspaceLinkTarget {
  path: string;
}

const WORKSPACE_SCHEME_RE = /^(?:workspace|openhuman-workspace):/i;
const WINDOWS_DRIVE_RE = /^[a-z]:\//i;

export function parseWorkspaceHref(rawHref?: string | null): WorkspaceLinkTarget | null {
  if (!rawHref) return null;
  const trimmed = rawHref.trim();
  if (!WORKSPACE_SCHEME_RE.test(trimmed)) return null;

  const rawPath = trimmed.replace(WORKSPACE_SCHEME_RE, '').replace(/^\/+/, '');
  if (!rawPath || rawPath.includes('\0')) return null;

  let decoded: string;
  try {
    decoded = decodeURIComponent(rawPath);
  } catch {
    return null;
  }
  if (decoded.includes('\0')) return null;

  const normalized = decoded.replace(/\\/g, '/').replace(/^\/+/, '');
  if (!normalized || WINDOWS_DRIVE_RE.test(normalized)) return null;

  const parts = normalized.split('/').filter(Boolean);
  if (parts.length === 0) return null;
  if (parts.some(part => part === '.' || part === '..' || part.includes(':'))) {
    return null;
  }

  return { path: parts.join('/') };
}

export function isWorkspaceHref(rawHref?: string | null): boolean {
  return parseWorkspaceHref(rawHref) !== null;
}
