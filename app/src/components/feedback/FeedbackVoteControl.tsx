import debugFactory from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { feedbackApi } from '../../services/api/feedbackApi';
import type { FeedbackItem, FeedbackVoteValue } from '../../types/feedback';

const log = debugFactory('feedback:vote');

interface FeedbackVoteControlProps {
  item: FeedbackItem;
  /** Called with the authoritative item after each successful vote, and with
   * the optimistic item immediately on click so the row updates without lag. */
  onVoted: (updated: FeedbackItem) => void;
}

/**
 * Apply a vote transition locally so the UI updates before the server responds.
 * Mirrors the backend tally recomputation (counts derived from the vote set).
 */
function applyOptimisticVote(item: FeedbackItem, next: FeedbackVoteValue): FeedbackItem {
  let { upvoteCount, downvoteCount } = item;
  if (item.myVote === 1) upvoteCount -= 1;
  if (item.myVote === -1) downvoteCount -= 1;
  if (next === 1) upvoteCount += 1;
  if (next === -1) downvoteCount += 1;
  return { ...item, upvoteCount, downvoteCount, score: upvoteCount - downvoteCount, myVote: next };
}

export default function FeedbackVoteControl({ item, onVoted }: FeedbackVoteControlProps) {
  const { t } = useT();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState(false);

  const castVote = async (direction: 1 | -1) => {
    if (pending) return;
    // Clicking the active direction again retracts the vote.
    const next: FeedbackVoteValue = item.myVote === direction ? 0 : direction;
    const previous = item;

    setPending(true);
    setError(false);
    onVoted(applyOptimisticVote(item, next));

    try {
      const updated = await feedbackApi.voteFeedback(item.id, next);
      onVoted(updated);
    } catch (err) {
      log('vote failed id=%s value=%d error=%O', item.id, next, err);
      onVoted(previous); // roll back to the pre-click state
      setError(true);
    } finally {
      setPending(false);
    }
  };

  const upActive = item.myVote === 1;
  const downActive = item.myVote === -1;

  // Reddit-style vote pill: solid arrows, orange upvote / periwinkle downvote,
  // the pill tinting to match the active direction.
  const pillTint = upActive
    ? 'bg-orange-500/10'
    : downActive
      ? 'bg-indigo-500/10'
      : 'bg-neutral-100 dark:bg-white/[0.04]';
  const countColor = error
    ? 'text-coral-500'
    : upActive
      ? 'text-orange-500'
      : downActive
        ? 'text-indigo-400'
        : 'text-neutral-700 dark:text-neutral-200';

  return (
    <div
      className={`flex w-9 flex-shrink-0 flex-col items-center gap-0.5 rounded-full py-1 transition-colors ${pillTint}`}>
      <button
        type="button"
        onClick={() => castVote(1)}
        disabled={pending}
        aria-pressed={upActive}
        aria-label={t('feedback.vote.up')}
        title={t('feedback.vote.up')}
        className={`flex h-6 w-6 items-center justify-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
          upActive
            ? 'text-orange-500'
            : 'text-neutral-400 hover:bg-orange-500/10 hover:text-orange-500 dark:text-neutral-500'
        }`}>
        <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 20 20">
          <path d="M10 3.5a1 1 0 01.78.375l5.5 6.5A1 1 0 0115.5 12H13v4a1 1 0 01-1 1H8a1 1 0 01-1-1v-4H4.5a1 1 0 01-.78-1.625l5.5-6.5A1 1 0 0110 3.5z" />
        </svg>
      </button>

      <span className={`text-sm font-bold tabular-nums ${countColor}`} aria-live="polite">
        {item.score}
      </span>

      <button
        type="button"
        onClick={() => castVote(-1)}
        disabled={pending}
        aria-pressed={downActive}
        aria-label={t('feedback.vote.down')}
        title={t('feedback.vote.down')}
        className={`flex h-6 w-6 items-center justify-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
          downActive
            ? 'text-indigo-400'
            : 'text-neutral-400 hover:bg-indigo-500/10 hover:text-indigo-400 dark:text-neutral-500'
        }`}>
        <svg className="h-4 w-4 rotate-180" fill="currentColor" viewBox="0 0 20 20">
          <path d="M10 3.5a1 1 0 01.78.375l5.5 6.5A1 1 0 0115.5 12H13v4a1 1 0 01-1 1H8a1 1 0 01-1-1v-4H4.5a1 1 0 01-.78-1.625l5.5-6.5A1 1 0 0110 3.5z" />
        </svg>
      </button>
    </div>
  );
}
