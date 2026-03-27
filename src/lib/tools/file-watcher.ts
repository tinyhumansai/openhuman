/**
 * File watcher for TOOLS.md to automatically invalidate cache when updated
 *
 * This module sets up automatic cache invalidation whenever the bundled TOOLS.md
 * file is updated, ensuring the UI reflects the latest tool data immediately.
 */
import { clearAICache, loadAIConfig } from '../ai/loader';
import { clearToolsCache } from '../ai/tools/loader';

// Track file modification time to detect changes
let lastModifiedTime: number | null = null;
let watcherInterval: ReturnType<typeof setInterval> | null = null;

/**
 * Start watching TOOLS.md file for changes
 * When changes are detected, automatically clear cache and reload
 */
export function startToolsFileWatcher(): void {
  if (watcherInterval) {
    console.log('🔍 TOOLS.md file watcher already running');
    return;
  }

  console.log('🔍 Starting TOOLS.md file watcher...');

  // Check for file changes every 2 seconds
  watcherInterval = setInterval(checkForToolsChanges, 2000);
}

/**
 * Stop the file watcher
 */
export function stopToolsFileWatcher(): void {
  if (watcherInterval) {
    clearInterval(watcherInterval);
    watcherInterval = null;
    console.log('🔍 TOOLS.md file watcher stopped');
  }
}

/**
 * Check if TOOLS.md file has been modified and trigger cache refresh
 */
async function checkForToolsChanges(): Promise<void> {
  try {
    // Check if the bundled TOOLS.md has changed by fetching from public directory
    const response = await fetch('/src-tauri/ai/TOOLS.md');
    if (!response.ok) return;

    const content = await response.text();
    const contentHash = simpleHash(content);

    // Check if content has changed
    if (lastModifiedTime === null) {
      lastModifiedTime = contentHash;
      return;
    }

    if (lastModifiedTime !== contentHash) {
      console.log('📝 TOOLS.md file changed detected - refreshing cache...');
      lastModifiedTime = contentHash;

      // Clear cache and trigger reload
      clearToolsCache();
      clearAICache();

      // Preload the new tools and AI config data
      try {
        await loadAIConfig();
        console.log('✅ AI configuration cache refreshed successfully');

        // Dispatch custom event for components to react to
        window.dispatchEvent(
          new CustomEvent('tools-updated', { detail: { timestamp: Date.now() } })
        );
      } catch (error) {
        console.error('❌ Failed to reload AI config after file change:', error);
      }
    }
  } catch (_err) {
    // Silently ignore errors (file not found, network issues, etc.)
    // This prevents console spam during development
  }
}

/**
 * Simple hash function for content change detection
 */
function simpleHash(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = (hash << 5) - hash + char;
    hash = hash & hash; // Convert to 32-bit integer
  }
  return hash;
}

/**
 * Force a cache refresh (useful for manual triggers)
 */
export async function forceToolsCacheRefresh(): Promise<void> {
  console.log('🔄 Forcing tools cache refresh...');
  clearToolsCache();
  clearAICache();
  lastModifiedTime = null; // Reset to trigger next check
  await loadAIConfig();
}
