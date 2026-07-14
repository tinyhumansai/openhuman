/**
 * Decide which async source seeds `userScopedStorage`'s active-user id at
 * boot, before `primeActiveUserId(...)` runs.
 *
 * Three source shapes:
 *   1. Mascot/notch native windows — no Tauri IPC, cannot invoke commands.
 *   2. Cloud/remote core mode — the local `~/.openhuman/active_user.toml`
 *      is either empty (no prior local session) or bound to a prior LOCAL
 *      session's user id. In both cases it doesn't match the REMOTE core's
 *      authenticated user, and priming from it overwrites the correct
 *      `localStorage` seed that `handleIdentityFlip` writes just before
 *      `restartApp`. That mismatch drives the infinite
 *      `identityFlip → restartApp` restart loop reported in #4545.
 *   3. Local core mode — read the Rust `active_user.toml` via IPC. This is
 *      the profile-independent source of truth the local sidecar writes
 *      atomically during `auth_store_session` (#900).
 *
 * Cases (1) and (2) resolve `null`; `primeActiveUserId(null)` then preserves
 * the existing `localStorage` seed rather than wiping it. See
 * `userScopedStorage.ts::primeActiveUserId` and the "cloud-mode reload
 * survival" test.
 */
export interface BootstrapContext {
  isStandaloneNativeWindow: boolean;
  coreMode: 'local' | 'cloud' | null;
  getActiveUserIdFromCore: () => Promise<string | null>;
}

export function shouldSkipLocalActiveUserRead(opts: {
  isStandaloneNativeWindow: boolean;
  coreMode: 'local' | 'cloud' | null;
}): boolean {
  return opts.isStandaloneNativeWindow || opts.coreMode === 'cloud';
}

export function resolveActiveUserBootstrap(ctx: BootstrapContext): Promise<string | null> {
  if (
    shouldSkipLocalActiveUserRead({
      isStandaloneNativeWindow: ctx.isStandaloneNativeWindow,
      coreMode: ctx.coreMode,
    })
  ) {
    return Promise.resolve<string | null>(null);
  }
  return ctx.getActiveUserIdFromCore();
}
