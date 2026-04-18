type UsageRefreshListener = () => void;

const listeners = new Set<UsageRefreshListener>();

export function subscribeUsageRefresh(listener: UsageRefreshListener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function requestUsageRefresh(): void {
  for (const listener of listeners) {
    listener();
  }
}
