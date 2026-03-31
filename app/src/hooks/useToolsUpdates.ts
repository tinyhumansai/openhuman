/**
 * React hook to listen for tools updates and trigger re-renders
 *
 * Components using this hook will automatically re-render when TOOLS.md
 * is updated and the cache is refreshed.
 */
import { useEffect, useState } from 'react';

import { forceToolsCacheRefresh } from '../lib/tools/file-watcher';

interface ToolsUpdateEvent {
  timestamp: number;
}

/**
 * Hook to listen for tools updates
 * @returns timestamp of last tools update (triggers re-renders)
 */
export function useToolsUpdates(): number {
  const [lastUpdate, setLastUpdate] = useState<number>(0);

  useEffect(() => {
    const handleToolsUpdate = (event: CustomEvent<ToolsUpdateEvent>) => {
      console.log('🔔 Tools update detected, triggering component re-render');
      setLastUpdate(event.detail.timestamp);
    };

    // Listen for tools-updated events
    window.addEventListener('tools-updated', handleToolsUpdate as EventListener);

    return () => {
      window.removeEventListener('tools-updated', handleToolsUpdate as EventListener);
    };
  }, []);

  return lastUpdate;
}

/**
 * Hook to get a callback that forces tools refresh
 * @returns function to manually trigger tools refresh
 */
export function useForceToolsRefresh(): () => Promise<void> {
  const forceRefresh = async () => {
    return forceToolsCacheRefresh();
  };

  return forceRefresh;
}
