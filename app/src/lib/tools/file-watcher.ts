/**
 * Legacy UI-side tools cache watcher removed.
 * Tool/config refresh now comes from core RPC and socket events.
 */
export function startToolsFileWatcher(): void {
  // no-op
}

export function stopToolsFileWatcher(): void {
  // no-op
}

export async function forceToolsCacheRefresh(): Promise<void> {
  // Preserve compatibility for callers that expect a resolved promise.
  return Promise.resolve();
}
