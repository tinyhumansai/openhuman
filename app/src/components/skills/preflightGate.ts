// Parse and present the structured error message that the
// `openhuman.skills_run` RPC returns when a skill's `[github]`
// preflight gate refuses the run.
//
// Backend contract (see src/openhuman/skills/schemas.rs's
// `spawn_skill_run_background` → preflight branch and
// src/openhuman/skills/preflight.rs's `GithubGateError::tag` /
// `to_user_message`): the error string is shaped as
//
//   `[preflight:<gate>:<tag>] <user-readable body>`
//
// where `<gate>` is `github` today and `<tag>` is one of:
//
//   composio_github_missing | git_binary_missing | git_user_name_missing |
//   git_user_email_missing  | identity_mismatch  | composio_identity_unresolved
//
// Free-form RPC errors (anything else) parse as `gate: null`
// `body: <raw>` so the caller can fall back to a generic error
// pill. The split is what lets the runner UI surface the gate
// failure as a distinct status (pill + remediation) rather than
// blending it into the generic "Run failed to start" surface.

const PREFLIGHT_PREFIX_RE = /^\[preflight:([a-z0-9_-]+):([a-z0-9_-]+)\]\s+/i;

/** Parsed shape of a backend RPC error returned by `openhuman.skills_run`. */
export interface SkillRunError {
  /** `'github'` when this is a github-gate failure; `null` for any other error. */
  gate: string | null;
  /**
   * Short machine tag (e.g. `'identity_mismatch'`). Stable across versions —
   * see the rustdoc on `GithubGateError::tag()`. `null` for non-gate errors.
   */
  tag: string | null;
  /** User-readable body with the gate prefix stripped. */
  body: string;
}

/**
 * Parse the message string returned by the `openhuman.skills_run` RPC
 * error path (or thrown by `skillsApi.runSkill`). Anything that matches
 * the `[preflight:<gate>:<tag>]` prefix becomes a structured gate
 * failure; anything else falls through with `gate: null` so the caller
 * can render the raw text.
 *
 * Idempotent: calling this twice on the same message is fine (the
 * second call sees no prefix and returns the same body unchanged).
 */
export function parseSkillRunError(message: string | undefined | null): SkillRunError {
  const raw = (message ?? '').toString();
  const m = PREFLIGHT_PREFIX_RE.exec(raw);
  if (!m) {
    return { gate: null, tag: null, body: raw };
  }
  return {
    gate: m[1].toLowerCase(),
    tag: m[2].toLowerCase(),
    body: raw.slice(m[0].length),
  };
}

/**
 * True when this error is a github-gate failure. Convenience for
 * the rendering layer that wants a single boolean rather than
 * matching on the `gate` string.
 */
export function isGithubGateFailure(err: SkillRunError): boolean {
  return err.gate === 'github';
}
