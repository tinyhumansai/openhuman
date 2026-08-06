/**
 * Recognise the one core failure that reads like a transient sign-in glitch but
 * is a permanent, environment-level fault: the core cannot open its own
 * `config.toml`.
 *
 * The Rust chain looks like
 *   `Failed to read config file: /home/openhuman/.openhuman/config.toml
 *    [config owner mismatch] (file uid=0 …): Permission denied (os error 13)`
 * and, before this module existed, was painted verbatim into the Welcome
 * screen (absolute path included) with no classification and no recovery hint,
 * while the OAuth path bucketed it as `'other'` -> "Sign-in failed. Please try
 * again." presenting a permanent fault as a retryable one.
 *
 * Every config-dependent RPC fails the same way, so no amount of retrying
 * helps; the fix is always on the runtime host.
 */

/** Context lines the Rust loader wraps a config read in. */
const CONFIG_READ_ANCHOR = /failed to read config file|reading config\.toml from/;

/**
 * OS denial signals, unix and Windows.
 *
 * The errno alternatives are parenthesised because that is the exact shape
 * `io::Error`'s Display always produces (`… (os error 13)`), and an unanchored
 * `os error 5` would also match `os error 50` / `os error 512`.
 */
const PERMISSION_SIGNAL = /permission denied|access is denied|\(os error 13\)|\(os error 5\)/;

/** Marker the core appends when the file's uid differs from the process euid. */
const OWNER_MISMATCH_MARKER = 'config owner mismatch';

/**
 * True when `message` is a core config-read denial rather than an unrelated
 * permission error. Requires BOTH the config-read context and a denial signal
 * so a permission failure from any other subsystem keeps its own message.
 *
 * Accepts a raw (un-lowercased) message; callers need not normalise.
 */
export const isCoreConfigUnreadableError = (message: string | null | undefined): boolean => {
  const lowered = (message ?? '').toLowerCase();
  if (!CONFIG_READ_ANCHOR.test(lowered)) {
    return false;
  }
  return PERMISSION_SIGNAL.test(lowered) || lowered.includes(OWNER_MISMATCH_MARKER);
};

/**
 * i18n key for the user-facing copy. The message itself lives in the locale
 * files rather than here so every language gets it and the i18n gates can see
 * it; this module only decides *whether* the failure applies.
 *
 * The copy is deliberately hedged. A denial carrying the ownership marker is
 * definitely a uid mismatch, but the classifier also matches the bare
 * permission shape emitted by cores predating that marker, and on Windows the
 * same shape can come from a DACL or an antivirus lock. Asserting "is owned by
 * another user" would send those users chasing the wrong remedy.
 */
export const CORE_CONFIG_UNREADABLE_I18N_KEY = 'welcome.coreConfigUnreadable';
