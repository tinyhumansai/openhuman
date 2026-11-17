import { consumeIdentityFlipSeed, markIdentityFlipSeed } from '../store/userScopedStorage';

/**
 * Decide which async source seeds `userScopedStorage`'s active-user id at
 * boot, before `primeActiveUserId(...)` runs.
 *
 * Three source shapes:
 *   1. Mascot native window — no Tauri IPC, cannot invoke commands.
 *   2. Any remote core mode — cloud, or a gateway this app provisioned in a
 *      container or on another machine. The local `~/.openhuman/active_user.toml`
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
// The marker itself lives with `OPENHUMAN_ACTIVE_USER_ID` in
// `userScopedStorage.ts`: both are unscoped by necessity because they are read
// to *decide* the active user. Re-exported here because this is where the
// bootstrap reads it.
export { consumeIdentityFlipSeed, markIdentityFlipSeed };

/** Every core mode the picker and the gateway section can persist. */
type StoredCoreMode = 'local' | 'cloud' | 'gateway' | null;

interface BootstrapContext {
  isStandaloneNativeWindow: boolean;
  coreMode: StoredCoreMode;
  getActiveUserIdFromCore: () => Promise<string | null>;
  /** Injectable for tests; defaults to the module-level reader. */
  consumeIdentityFlipSeed?: () => string | null;
}

export function shouldSkipLocalActiveUserRead(opts: {
  isStandaloneNativeWindow: boolean;
  coreMode: StoredCoreMode;
}): boolean {
  // `gateway` belongs with `cloud`, not with `local`: the reasoning above is
  // about the local file describing a *different* core's user, and that is
  // just as true of a core in a container as of one at a URL. Treating it as
  // local would prime from a stale id and reintroduce the #4545 restart loop
  // for exactly the users this feature exists for.
  return opts.isStandaloneNativeWindow || opts.coreMode === 'cloud' || opts.coreMode === 'gateway';
}

export function resolveActiveUserBootstrap(ctx: BootstrapContext): Promise<string | null> {
  // A pending flip outranks every other source: it is the id this process was
  // restarted in order to adopt.
  // `?? consumeIdentityFlipSeed()` would fall through to the module reader
  // whenever an injected one returned null, so a test that deliberately says
  // "no flip pending" still hit real storage.
  const readFlipSeed = ctx.consumeIdentityFlipSeed ?? consumeIdentityFlipSeed;
  const flipped = readFlipSeed();
  if (flipped) {
    return Promise.resolve<string | null>(flipped);
  }

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
