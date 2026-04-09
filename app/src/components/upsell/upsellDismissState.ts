const STORAGE_PREFIX = 'openhuman:upsell:';

export function dismissBanner(bannerId: string): void {
  localStorage.setItem(`${STORAGE_PREFIX}${bannerId}`, JSON.stringify({ dismissedAt: Date.now() }));
}

export function shouldShowBanner(bannerId: string, cooldownMs: number): boolean {
  const raw = localStorage.getItem(`${STORAGE_PREFIX}${bannerId}`);
  if (!raw) return true;
  try {
    const { dismissedAt } = JSON.parse(raw) as { dismissedAt: number };
    return Date.now() - dismissedAt > cooldownMs;
  } catch {
    return true;
  }
}
