export function redactRpcUrlForLog(url: string): string {
  try {
    const parsed = new URL(url);
    parsed.username = '';
    parsed.password = '';
    parsed.search = '';
    parsed.hash = '';
    return parsed.origin + parsed.pathname;
  } catch {
    return '[invalid-url]';
  }
}
