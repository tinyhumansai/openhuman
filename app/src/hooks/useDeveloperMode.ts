/**
 * useDeveloperMode — runtime developer-mode gate.
 *
 * Developer surfaces (Settings › Developer & Diagnostics, dev-only settings
 * search entries, the Intelligence "council" tab) are now always visible, so
 * this hook unconditionally returns `true`.  The previous opt-in toggle in
 * Settings › About has been removed.
 *
 * Gating was always UI-only.  The Rust `SecurityPolicy` / autonomy-tier
 * enforcement in the core is authoritative and is unaffected by this.
 */
export function useDeveloperMode(): boolean {
  return true;
}
