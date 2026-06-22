const normalizePathname = (pathname: string): string =>
  pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname;

export function missingHashRedirectTarget(pathname: string, search: string): string {
  if (normalizePathname(pathname) === '/auth') {
    return `/#/auth${search}`;
  }

  return `${pathname}${search}#/`;
}
