const STORAGE_KEY = 'openhuman_device_fingerprint_v1';

/**
 * Stable anonymous id for referral abuse signals (optional body/header on backend).
 */
export function getOrCreateDeviceFingerprint(): string {
  try {
    let v = localStorage.getItem(STORAGE_KEY);
    if (!v) {
      v =
        typeof crypto !== 'undefined' && 'randomUUID' in crypto
          ? crypto.randomUUID()
          : `fp_${Date.now()}_${Math.random().toString(36).slice(2, 12)}`;
      localStorage.setItem(STORAGE_KEY, v);
    }
    return v;
  } catch {
    return `fp_ephemeral_${Date.now()}`;
  }
}
