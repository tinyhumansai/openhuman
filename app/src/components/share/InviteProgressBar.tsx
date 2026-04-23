interface InviteProgressBarProps {
  /** Number of invites the user has successfully converted. */
  converted: number;
  /** Total capacity (usually the number of invite codes the user owns). */
  total: number;
  /** Optional cap for how far the progress bar fills (defaults to `total`). */
  goal?: number;
}

/**
 * Horizontal progress indicator that turns the abstract "5 invite codes"
 * into a visible, gamified goal. Users get a dopamine nudge as the bar
 * fills, which empirically lifts referral conversion.
 */
export default function InviteProgressBar({ converted, total, goal }: InviteProgressBarProps) {
  const cap = Math.max(1, goal ?? total);
  const clamped = Math.max(0, Math.min(converted, cap));
  const pct = Math.min(100, Math.round((clamped / cap) * 100));
  const remaining = Math.max(0, cap - clamped);

  const message =
    clamped === 0
      ? 'Invite your first friend to unlock your first reward.'
      : clamped >= cap
        ? 'You maxed out your invite rewards — thank you!'
        : `${remaining} more ${remaining === 1 ? 'invite' : 'invites'} to go.`;

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between text-[11px] font-medium uppercase tracking-wider text-stone-400">
        <span>Your invite streak</span>
        <span>
          {clamped}/{cap}
        </span>
      </div>
      <div
        className="h-2.5 w-full rounded-full bg-stone-100"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={cap}
        aria-valuenow={clamped}>
        <div
          className="h-full rounded-full bg-gradient-to-r from-primary-500 via-primary-400 to-sage-500 transition-all duration-500"
          style={{ width: `${pct}%` }}
        />
      </div>
      <p className="text-xs text-stone-500">{message}</p>
    </div>
  );
}
