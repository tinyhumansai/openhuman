/**
 * Which voice entry point the Human tab renders in its chat card (#5399).
 *
 * - `realtime` — the "Start voice chat" control (ElevenLabs Agents session).
 * - `push-to-talk` — the classic tap-and-speak mic composer.
 * - `both` — both, stacked, for comparing the two paths.
 */
export type HumanVoiceEntry = 'realtime' | 'push-to-talk' | 'both';

/**
 * Resolve the entry point from the two build flags. Pure so the precedence rule
 * is testable without a build: `showBoth` wins over `realtimeEnabled`, because a
 * build that deliberately asks to see both should not have one of them hidden by
 * the other flag's rollback state.
 */
export function resolveHumanVoiceEntry(flags: {
  realtimeEnabled: boolean;
  showBoth: boolean;
}): HumanVoiceEntry {
  if (flags.showBoth) return 'both';
  return flags.realtimeEnabled ? 'realtime' : 'push-to-talk';
}
