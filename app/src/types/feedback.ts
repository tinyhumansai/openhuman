/**
 * Feedback board types.
 *
 * Mirrors the backend `SerializedFeedback` DTO and the `/feedback` REST contract
 * (see `backend-alphahuman/src/controllers/feedback/serialize.ts`). Dates are
 * ISO strings; `myVote` is computed server-side for the authenticated caller.
 */

export type FeedbackType = 'feature' | 'bug';

/** Statuses a client can observe on the public board. */
export type FeedbackStatus = 'open' | 'planned' | 'completed' | 'closed';

/** Board orderings exposed by the list endpoint. */
export type FeedbackSort = 'hot' | 'top' | 'new';

/** -1 downvote, 0 no/retracted vote, 1 upvote. */
export type FeedbackVoteValue = 1 | -1 | 0;

export interface FeedbackGithub {
  issueNumber: number | null;
  issueUrl: string | null;
}

export interface FeedbackItem {
  id: string;
  type: FeedbackType;
  title: string;
  body: string;
  status: FeedbackStatus;
  /** ObjectId of the author. */
  createdBy: string;
  /** Author's display name (firstName, else username); null when unknown. */
  createdByName: string | null;
  upvoteCount: number;
  downvoteCount: number;
  /** upvoteCount - downvoteCount. */
  score: number;
  /** Time-decayed ranking score backing the "hot" sort. */
  rankScore: number;
  commentCount: number;
  github: FeedbackGithub | null;
  /** The caller's current vote on this item. */
  myVote: FeedbackVoteValue;
  createdAt: string;
  updatedAt: string;
}

export interface FeedbackListParams {
  type?: FeedbackType;
  status?: FeedbackStatus;
  sort?: FeedbackSort;
  page?: number;
  limit?: number;
}

export interface FeedbackListResult {
  items: FeedbackItem[];
  total: number;
  page: number;
  limit: number;
}

export interface FeedbackComment {
  id: string;
  /** ObjectId of the comment's author. */
  user: string;
  /** Author's display name (firstName, else username); null when unknown. */
  userName: string | null;
  body: string;
  createdAt: string;
}

export interface FeedbackDetail {
  feedback: FeedbackItem;
  comments: FeedbackComment[];
}

export interface CreateFeedbackInput {
  type: FeedbackType;
  title: string;
  body: string;
}

/**
 * The quality gate's verdict on a draft. Distinct from moderation: this is
 * "we could not act on this", not "you were flagged".
 *
 * `block` never reaches the board — the submit endpoint applies the same rules,
 * so the composer's check is a courtesy, not the enforcement point. `warn` is
 * published; the reason is a nudge to improve it.
 */
export type FeedbackQualityTier = 'block' | 'warn' | 'pass';

export interface FeedbackQuality {
  tier: FeedbackQualityTier;
  /** Shown to the submitter. Empty on `pass`. */
  reason: string;
}

/**
 * Result of a submission. `accepted` is false when the moderation gate rejects
 * the content — in that case `feedback` is null and `reason` explains why.
 * `quality` carries the gate's verdict when the submission was published.
 */
export interface CreateFeedbackResult {
  accepted: boolean;
  reason: string;
  feedback: FeedbackItem | null;
  quality?: FeedbackQuality;
}
