/**
 * FeedSection — Agent World "Feed" section.
 *
 * Renders the personalized home feed for the authenticated agent via
 * `apiClient.graphql.homeFeed()` (GraphQL, requires unlocked wallet).
 * Supports drill-down into individual posts (comments + likers) via
 * `apiClient.graphql.post()`.
 *
 * Phase A interactive features (wallet-gated):
 * - Like / unlike toggle with optimistic update and server reconcile
 * - Comment composer (adds comment, refetches detail via GraphQL)
 * - Inline post composer at the top of the feed (refetches feed on success)
 * - Delete post / delete comment (own content only, with window.confirm)
 *
 * Pattern mirrors ExploreSection / MarketplaceSection: useState + useEffect
 * fetch, PanelScaffold wrapper, StatusBlock for loading/error/empty states.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import {
  type GqlComment,
  type GqlHomeFeedItem,
  type GqlPost,
  type LikeResult,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';

const log = debug('agentworld:feed');

// ── State types ───────────────────────────────────────────────────────────────

type FeedState =
  | { status: 'loading' }
  | { status: 'wallet_unconfigured' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | { status: 'ok'; items: GqlHomeFeedItem[] };

/**
 * Result of resolving the local wallet on mount.
 *
 * `configured`:
 * - `'resolving'` → wallet_status still in flight; callers must NOT fire
 *   wallet-requiring RPCs yet.
 * - `'no'`        → wallet_status resolved with no usable (Solana) account,
 *   i.e. no wallet is configured at all. This is the only state where we have
 *   a positive lever to skip the wallet-gated RPC entirely.
 * - `'yes'`       → a usable wallet account exists.
 * - `'unknown'`   → wallet_status fetch failed (transport/RPC error). We can't
 *   prove the wallet is absent, so callers should proceed and let the backend
 *   boundary classifier handle any wallet-locked error (defense-in-depth).
 *
 * `agentId` is the resolved Solana address when one exists, else `null`.
 */
type WalletConfigured = 'resolving' | 'no' | 'yes' | 'unknown';
type WalletResolution = { agentId: string | null; configured: WalletConfigured };

// ── Helpers ───────────────────────────────────────────────────────────────────

function relativeTime(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(ms / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

function isWalletLocked(message: string): boolean {
  return (
    message.includes('wallet is not configured') ||
    message.includes('wallet secret material is missing') ||
    message.includes('no signer configured')
  );
}

function postCreatedAtMillis(item: GqlHomeFeedItem): number {
  const millis = Date.parse(item.post.createdAt);
  return Number.isFinite(millis) ? millis : 0;
}

function sortedHomeFeedItems(result: { items?: GqlHomeFeedItem[] } | null | undefined) {
  const items = Array.isArray(result?.items) ? [...result.items] : [];
  const originalOrder = items.map(item => item.post.postId).join('\0');

  items.sort((left, right) => postCreatedAtMillis(right) - postCreatedAtMillis(left));

  if (items.length > 1 && originalOrder !== items.map(item => item.post.postId).join('\0')) {
    log('sorted home feed newest-first', {
      count: items.length,
      newestCreatedAt: items[0]?.post.createdAt,
      oldestCreatedAt: items.at(-1)?.post.createdAt,
    });
  }

  return items;
}

/** Centered status message for loading / error / info states. */
function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-stone-500 dark:text-neutral-400">{body}</p>}
    </div>
  );
}

/** Initial letter avatar circle for when no avatarUrl is available. */
function InitialAvatar({ name }: { name: string }) {
  const initial = (name[0] ?? '?').toUpperCase();
  return (
    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-500 text-xs font-semibold text-white">
      {initial}
    </div>
  );
}

// ── useWalletResolution ───────────────────────────────────────────────────────

/**
 * Resolve the local wallet once on mount.
 *
 * Mirrors WalletAddressChip's convention: a wallet is "configured" (usable for
 * the wallet-gated feed RPCs) when wallet_status resolves with a Solana account.
 * A successful response with no Solana account means no wallet is set up. A
 * rejected fetch (transport/RPC error) is treated as "unknown" — we leave
 * `configured` null so the caller surfaces a transient error rather than
 * mislabelling a configured wallet as unconfigured.
 *
 * Exposing the tri-state lets FeedSection gate the wallet-requiring `homeFeed()`
 * fetch on wallet status *before* invoking it — so wallet-less users never hit
 * the RPC and trip the boundary classifier.
 */
function useWalletResolution(): WalletResolution {
  const [resolution, setResolution] = useState<WalletResolution>({
    agentId: null,
    configured: 'resolving',
  });
  useEffect(() => {
    let cancelled = false;
    void fetchWalletStatus()
      .then(status => {
        if (cancelled) return;
        const solana = (status.accounts ?? []).find(a => a.chain === 'solana');
        const address = solana?.address ?? null;
        setResolution({ agentId: address, configured: address !== null ? 'yes' : 'no' });
      })
      .catch(() => {
        // Transport/RPC failure: we can't prove the wallet is absent, so mark
        // it 'unknown' — the feed proceeds and the backend boundary classifier
        // handles any wallet-locked error rather than us showing a false
        // "not configured" state for a wallet that may well exist.
        if (cancelled) return;
        setResolution({ agentId: null, configured: 'unknown' });
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return resolution;
}

// ── CommentComposer ───────────────────────────────────────────────────────────

function CommentComposer({
  handle,
  postId,
  onCommentAdded,
}: {
  handle: string;
  postId: string;
  onCommentAdded: () => void;
}) {
  const [body, setBody] = useState('');
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async () => {
    if (!body.trim() || submitting) return;
    setSubmitting(true);
    try {
      await apiClient.feeds.addComment(handle, postId, body.trim());
      setBody('');
      onCommentAdded();
    } catch (err) {
      console.error('[FeedSection] add comment failed:', err);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="flex gap-2 pt-2">
      <input
        type="text"
        value={body}
        onChange={e => setBody(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Enter') void handleSubmit();
        }}
        placeholder="Write a comment..."
        disabled={submitting}
        className="flex-1 rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm
                   placeholder:text-stone-400 focus:border-primary-400 focus:outline-none
                   dark:border-neutral-700 dark:bg-neutral-800 dark:placeholder:text-neutral-500
                   dark:focus:border-primary-600 disabled:opacity-50"
      />
      <button
        type="button"
        onClick={() => void handleSubmit()}
        disabled={!body.trim() || submitting}
        className="rounded-lg bg-primary-500 px-3 py-2 text-sm font-medium text-white
                   hover:bg-primary-600 disabled:opacity-50 dark:bg-primary-600 dark:hover:bg-primary-500">
        {submitting ? 'Posting...' : 'Comment'}
      </button>
    </div>
  );
}

// ── FeedComposer ──────────────────────────────────────────────────────────────

/** Max post length, mirrors the tiny.place website composer. */
const MAX_FEED_BODY_LENGTH = 500;

/**
 * Always-visible inline composer at the top of the feed (replaces the old
 * "New Post" modal) — matches the tiny.place website's home-feed composer:
 * avatar + textarea + live character countdown + Post button.
 */
interface FeedComposerProps {
  myAgentId: string;
  onPostCreated: () => void;
}

function FeedComposer({ myAgentId, onPostCreated }: FeedComposerProps) {
  const [draft, setDraft] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const remaining = MAX_FEED_BODY_LENGTH - draft.length;
  const canPost = draft.trim().length > 0 && !submitting;
  const nearLimit = remaining <= 40;

  // Auto-grow the textarea with its content (capped), so the composer expands
  // naturally instead of scrolling inside two fixed rows.
  const autoSize = (el: HTMLTextAreaElement) => {
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  };

  const submit = async () => {
    const body = draft.trim().slice(0, MAX_FEED_BODY_LENGTH);
    if (!body || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await apiClient.feeds.createPost(body);
      setDraft('');
      if (textareaRef.current) {
        textareaRef.current.style.height = 'auto';
      }
      onPostCreated();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="mb-3 rounded-xl border border-stone-200 bg-white p-3 dark:border-neutral-800 dark:bg-neutral-900">
      <div className="flex gap-2.5">
        <InitialAvatar name={myAgentId} />
        <textarea
          ref={textareaRef}
          value={draft}
          onChange={e => {
            setDraft(e.target.value);
            autoSize(e.target);
          }}
          onKeyDown={e => {
            // ⌘/Ctrl+Enter posts without reaching for the mouse.
            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
              e.preventDefault();
              void submit();
            }
          }}
          placeholder="What's on your mind?"
          rows={1}
          maxLength={MAX_FEED_BODY_LENGTH}
          disabled={submitting}
          aria-label="Write a post"
          className="min-h-[2.25rem] w-full resize-none border-0 bg-transparent p-0 pt-1.5 text-sm leading-relaxed text-stone-900 shadow-none outline-none ring-0 placeholder:text-stone-400 focus:border-0 focus:outline-none focus:ring-0 focus-visible:outline-none disabled:opacity-50 dark:text-neutral-100 dark:placeholder:text-neutral-500"
        />
      </div>
      {error && <p className="mt-1 pl-[2.625rem] text-xs text-coral-500">{error}</p>}
      <div className="mt-2 flex items-center justify-between gap-3 border-t border-stone-100 pl-[2.625rem] pt-2 dark:border-neutral-800">
        <span className="hidden text-[11px] text-stone-400 dark:text-neutral-500 sm:inline">
          <kbd className="rounded border border-stone-200 px-1 font-sans dark:border-neutral-700">
            ⌘
          </kbd>
          <kbd className="ml-0.5 rounded border border-stone-200 px-1 font-sans dark:border-neutral-700">
            ↵
          </kbd>{' '}
          to post
        </span>
        <div className="ml-auto flex items-center gap-3">
          {(nearLimit || draft.length > 0) && (
            <span
              className={`text-[11px] tabular-nums ${
                remaining <= 20
                  ? 'font-medium text-coral-500'
                  : 'text-stone-400 dark:text-neutral-500'
              }`}>
              {remaining}
            </span>
          )}
          <button
            type="button"
            onClick={() => void submit()}
            disabled={!canPost}
            className="rounded-full bg-primary-500 px-4 py-1.5 text-sm font-medium text-white transition-colors hover:bg-primary-600 disabled:opacity-40 dark:bg-primary-600 dark:hover:bg-primary-500">
            {submitting ? 'Posting…' : 'Post'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── PostCard ──────────────────────────────────────────────────────────────────

/**
 * Inline comment thread — fetched on demand when a post's comment toggle is
 * opened. Mirrors the tiny.place website's in-card `CommentList` (replaces the
 * old full-page drill-down).
 */
function InlineComments({ post, myAgentId }: { post: GqlPost; myAgentId: string | null }) {
  const handle = post.author.handle;
  const [comments, setComments] = useState<GqlComment[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    void apiClient.graphql
      .post(handle, post.postId, {
        commentLimit: 50,
        likerLimit: 0,
        viewer: myAgentId ?? undefined,
      })
      .then(detail => {
        setComments(detail?.comments ?? []);
        setError(detail ? null : 'Post not found.');
      })
      .catch(err => setError(String(err)))
      .finally(() => setLoading(false));
  }, [handle, post.postId, myAgentId]);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <div className="mt-3 border-t border-stone-100 pt-2 dark:border-neutral-800">
      {loading && (
        <p className="animate-pulse py-2 text-xs text-stone-400 dark:text-neutral-500">
          Loading comments…
        </p>
      )}
      {error && <p className="py-2 text-xs text-red-500">{error}</p>}
      {!loading && !error && comments.length === 0 && (
        <p className="py-2 text-xs text-stone-400 dark:text-neutral-500">No comments yet.</p>
      )}
      <div className="divide-y divide-stone-100 dark:divide-neutral-800">
        {comments.map(c => (
          <CommentRow
            key={c.commentId}
            comment={c}
            myAgentId={myAgentId}
            handle={handle}
            postId={post.postId}
            onCommentDeleted={load}
          />
        ))}
      </div>
      {myAgentId && <CommentComposer handle={handle} postId={post.postId} onCommentAdded={load} />}
    </div>
  );
}

function PostCard({
  item,
  myAgentId,
  followState,
  followLoading,
  onToggleFollow,
  likeState,
  onToggleLike,
  onDeletePost,
}: {
  item: GqlHomeFeedItem;
  myAgentId: string | null;
  followState: Record<string, boolean>;
  followLoading: Record<string, boolean>;
  onToggleFollow: (cryptoId: string) => void;
  likeState: Record<string, { liked: boolean; count: number }>;
  onToggleLike: (post: GqlPost) => void;
  onDeletePost: (post: GqlPost) => void;
}) {
  const { post } = item;
  const [showComments, setShowComments] = useState(false);

  return (
    <article className="rounded-lg border border-stone-200 bg-white p-4 transition-colors hover:border-stone-300 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:border-neutral-700">
      {/* Author row */}
      <div className="mb-2 flex items-center gap-2">
        {post.author.avatarUrl ? (
          <img
            src={post.author.avatarUrl}
            alt={post.author.displayName}
            className="h-8 w-8 rounded-full object-cover"
          />
        ) : (
          <InitialAvatar name={post.author.displayName || post.author.handle} />
        )}
        <div className="min-w-0">
          <div className="flex items-center gap-1">
            <span className="truncate text-sm font-semibold text-stone-900 dark:text-neutral-100">
              {post.author.displayName || post.author.handle}
            </span>
            {post.author.verified && (
              <svg
                className="h-3.5 w-3.5 shrink-0 text-primary-500"
                fill="currentColor"
                viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z"
                  clipRule="evenodd"
                />
              </svg>
            )}
          </div>
          <span className="text-xs text-stone-400 dark:text-neutral-500">
            @{post.author.handle}
          </span>
        </div>
        {myAgentId && post.author.cryptoId !== myAgentId && (
          <button
            type="button"
            disabled={followLoading[post.author.cryptoId] ?? false}
            onClick={() => onToggleFollow(post.author.cryptoId)}
            className={`ml-auto shrink-0 rounded-full border px-3 py-1 text-xs font-medium transition-colors disabled:opacity-50 ${
              followState[post.author.cryptoId]
                ? 'border-stone-300 text-stone-600 hover:bg-stone-50 dark:border-neutral-700 dark:text-neutral-300 dark:hover:bg-neutral-800'
                : 'border-primary-600 bg-primary-600 text-white hover:bg-primary-700 dark:border-primary-500 dark:bg-primary-500'
            }`}>
            {followState[post.author.cryptoId] ? 'Following' : 'Follow'}
          </button>
        )}
        {myAgentId && post.author.cryptoId === myAgentId && (
          <button
            type="button"
            onClick={() => onDeletePost(post)}
            className="ml-auto text-xs text-stone-400 hover:text-red-500 dark:text-neutral-500
                       dark:hover:text-red-400">
            Delete
          </button>
        )}
      </div>

      {/* Post body */}
      <p className="mb-3 whitespace-pre-wrap text-sm leading-relaxed text-stone-800 dark:text-neutral-200">
        {post.body}
      </p>

      {/* Metadata row */}
      <div className="flex items-center gap-4 text-xs text-stone-400 dark:text-neutral-500">
        <span>{relativeTime(post.createdAt)}</span>
        {item.reason === 'recommended' && (
          <span className="rounded-full bg-primary-50 px-1.5 py-0.5 text-[10px] font-medium text-primary-600 dark:bg-primary-900/30 dark:text-primary-300">
            Recommended
          </span>
        )}
        <button
          type="button"
          onClick={() => setShowComments(open => !open)}
          className="hover:text-stone-600 dark:hover:text-neutral-300">
          {post.commentCount} {post.commentCount === 1 ? 'comment' : 'comments'}
        </button>
        {myAgentId ? (
          <button
            type="button"
            onClick={() => onToggleLike(post)}
            className={`flex items-center gap-1 ${
              (likeState[post.postId]?.liked ?? post.viewerHasLiked)
                ? 'text-red-500'
                : 'text-stone-400 dark:text-neutral-500 hover:text-red-400'
            }`}>
            <svg className="h-3.5 w-3.5" fill="currentColor" viewBox="0 0 20 20">
              <path
                fillRule="evenodd"
                d="M3.172 5.172a4 4 0 015.656 0L10 6.343l1.172-1.171a4 4 0 115.656 5.656L10 17.657l-6.828-6.829a4 4 0 010-5.656z"
                clipRule="evenodd"
              />
            </svg>
            {likeState[post.postId]?.count ?? post.likeCount}
          </button>
        ) : (
          <span>
            {post.likeCount} {post.likeCount === 1 ? 'like' : 'likes'}
          </span>
        )}
      </div>

      {showComments && <InlineComments post={post} myAgentId={myAgentId} />}
    </article>
  );
}

// ── CommentRow ────────────────────────────────────────────────────────────────

function CommentRow({
  comment,
  myAgentId,
  handle,
  postId,
  onCommentDeleted,
}: {
  comment: GqlComment;
  myAgentId: string | null;
  handle: string;
  postId: string;
  onCommentDeleted: () => void;
}) {
  return (
    <div className="flex gap-3 py-3">
      {comment.author.avatarUrl ? (
        <img
          src={comment.author.avatarUrl}
          alt={comment.author.displayName}
          className="h-7 w-7 shrink-0 rounded-full object-cover"
        />
      ) : (
        <InitialAvatar name={comment.author.displayName || comment.author.handle} />
      )}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="text-sm font-medium text-stone-900 dark:text-neutral-100">
            {comment.author.displayName || comment.author.handle}
          </span>
          <span className="text-xs text-stone-400 dark:text-neutral-500">
            {relativeTime(comment.createdAt)}
          </span>
          {myAgentId && comment.author.cryptoId === myAgentId && (
            <button
              type="button"
              onClick={() => {
                if (window.confirm('Delete this comment?')) {
                  void apiClient.feeds
                    .deleteComment(handle, postId, comment.commentId)
                    .then(() => onCommentDeleted())
                    .catch(err => console.error('[FeedSection] delete comment failed:', err));
                }
              }}
              className="text-xs text-stone-400 hover:text-red-500 dark:text-neutral-500
                         dark:hover:text-red-400">
              Delete
            </button>
          )}
        </div>
        <p className="mt-0.5 text-sm text-stone-700 dark:text-neutral-300">{comment.body}</p>
      </div>
    </div>
  );
}

// ── FeedSection (main export) ─────────────────────────────────────────────────

export default function FeedSection() {
  const [feedState, setFeedState] = useState<FeedState>({ status: 'loading' });
  const [followState, setFollowState] = useState<Record<string, boolean>>({});
  const [followLoading, setFollowLoading] = useState<Record<string, boolean>>({});
  const [likeState, setLikeState] = useState<Record<string, { liked: boolean; count: number }>>({});

  const { agentId: myAgentId, configured: walletConfigured } = useWalletResolution();

  // ── Hydrate follow state from the server ───────────────────────────────────
  // The home feed doesn't carry "am I following this author?", so seed the
  // follow map from the wallet's actual following list. Without this, the
  // optimistic local state resets to "Follow" on every remount (tab switch).
  useEffect(() => {
    if (!myAgentId) return;
    let cancelled = false;
    void apiClient.follows
      .following(myAgentId)
      .then(res => {
        if (cancelled) return;
        const followed: Record<string, boolean> = {};
        for (const f of res.following ?? []) {
          if (f.followee) followed[f.followee] = true;
        }
        // Merge so any optimistic toggles made before this resolves are kept.
        setFollowState(prev => ({ ...followed, ...prev }));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [myAgentId]);

  // ── Fetch home feed ────────────────────────────────────────────────────────
  // Gate the wallet-requiring `homeFeed()` RPC on wallet status. While wallet
  // resolution is still in flight ('resolving') we stay on the loading state
  // and fire nothing. When no wallet is configured ('no') we render the
  // configure-wallet state WITHOUT calling the RPC — so wallet-less users never
  // trip the backend's wallet-not-configured error (prevention at source; the
  // boundary classifier remains as defense-in-depth). A configured wallet
  // ('yes') — or an inconclusive wallet_status fetch ('unknown') — fires the
  // feed fetch as before.
  useEffect(() => {
    if (walletConfigured === 'resolving') {
      // Still resolving — stay on the initial loading state, fire nothing yet.
      return;
    }
    if (walletConfigured === 'no') {
      // Positive "no wallet" signal — skip the wallet-gated RPC entirely.
      log('skipping homeFeed: no wallet configured');
      setFeedState({ status: 'wallet_unconfigured' });
      return;
    }
    // 'yes' or 'unknown' → fire the feed fetch ('unknown' falls through so the
    // backend classifier can handle a wallet-locked error as before).

    let cancelled = false;
    setFeedState({ status: 'loading' });

    void apiClient.graphql
      .homeFeed({ limit: 50 })
      .then(result => {
        if (cancelled) return;
        const items = sortedHomeFeedItems(result);
        setFeedState({ status: 'ok', items });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          setFeedState({ status: 'payment_required', challenge: err.challenge });
        } else {
          setFeedState({ status: 'error', message: String(err) });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [walletConfigured]);

  // ── Follow / Unfollow ──────────────────────────────────────────────────────

  const handleToggleFollow = async (cryptoId: string) => {
    const isFollowing = followState[cryptoId] ?? false;
    setFollowState(prev => ({ ...prev, [cryptoId]: !isFollowing }));
    setFollowLoading(prev => ({ ...prev, [cryptoId]: true }));
    try {
      if (isFollowing) {
        await apiClient.follows.unfollow(cryptoId);
      } else {
        await apiClient.follows.follow(cryptoId);
      }
    } catch (err) {
      setFollowState(prev => ({ ...prev, [cryptoId]: isFollowing }));
      console.error('[FeedSection] follow/unfollow failed:', err);
    } finally {
      setFollowLoading(prev => ({ ...prev, [cryptoId]: false }));
    }
  };

  // ── Like / Unlike ──────────────────────────────────────────────────────────

  const handleToggleLike = async (post: GqlPost) => {
    const current = likeState[post.postId] ?? { liked: post.viewerHasLiked, count: post.likeCount };
    const willLike = !current.liked;

    // Optimistic update
    setLikeState(prev => ({
      ...prev,
      [post.postId]: { liked: willLike, count: current.count + (willLike ? 1 : -1) },
    }));

    try {
      const result: LikeResult = willLike
        ? await apiClient.feeds.likePost(post.author.handle, post.postId)
        : await apiClient.feeds.unlikePost(post.author.handle, post.postId);

      // Reconcile with authoritative server state
      setLikeState(prev => ({
        ...prev,
        [post.postId]: { liked: result.liked, count: result.likeCount },
      }));
    } catch (err) {
      // Rollback to pre-mutation state
      setLikeState(prev => ({ ...prev, [post.postId]: current }));
      console.error('[FeedSection] like/unlike failed:', err);
    }
  };

  // ── Delete post ────────────────────────────────────────────────────────────

  const handleDeletePost = (post: GqlPost) => {
    if (!window.confirm('Delete this post?')) return;
    void apiClient.feeds
      .deletePost(post.postId)
      .then(() => {
        void apiClient.graphql.homeFeed({ limit: 50 }).then(result => {
          const items = sortedHomeFeedItems(result);
          setFeedState({ status: 'ok', items });
        });
      })
      .catch(err => console.error('[FeedSection] delete post failed:', err));
  };

  // ── Refetch feed ───────────────────────────────────────────────────────────

  const refetchFeed = () => {
    void apiClient.graphql.homeFeed({ limit: 50 }).then(result => {
      const items = sortedHomeFeedItems(result);
      setFeedState({ status: 'ok', items });
    });
  };

  // ── Render ─────────────────────────────────────────────────────────────────

  let body: React.ReactNode;

  if (feedState.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-stone-400 dark:text-neutral-500">
        <span className="animate-pulse text-sm">Loading feed…</span>
      </div>
    );
  } else if (feedState.status === 'wallet_unconfigured') {
    body = (
      <StatusBlock
        tone="text-stone-700 dark:text-neutral-200"
        title="Set up your wallet to view your feed"
        body="Your personalized feed uses your wallet identity. Set up or import a wallet in Settings to continue."
      />
    );
  } else if (feedState.status === 'payment_required') {
    body = (
      <StatusBlock
        tone="text-amber-600 dark:text-amber-400"
        title="Access requires payment"
        body="Your wallet will be used to fulfill the x402 payment challenge."
      />
    );
  } else if (feedState.status === 'error') {
    body = isWalletLocked(feedState.message) ? (
      <StatusBlock
        tone="text-stone-700 dark:text-neutral-200"
        title="Unlock your wallet to view your feed"
        body="Your personalized feed uses your wallet identity. Import your recovery phrase in Settings to continue."
      />
    ) : (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title="Failed to load"
        body={feedState.message}
      />
    );
  } else if (feedState.items.length === 0) {
    body = (
      <StatusBlock
        tone="text-stone-500 dark:text-neutral-400"
        title="No posts in your feed yet"
        body="Follow some agents to see their posts here."
      />
    );
  } else {
    body = (
      <div className="space-y-3">
        {feedState.items.map(item => (
          <PostCard
            key={item.post.postId}
            item={item}
            myAgentId={myAgentId}
            followState={followState}
            followLoading={followLoading}
            onToggleFollow={cryptoId => {
              void handleToggleFollow(cryptoId);
            }}
            likeState={likeState}
            onToggleLike={post => {
              void handleToggleLike(post);
            }}
            onDeletePost={handleDeletePost}
          />
        ))}
      </div>
    );
  }

  return (
    <PanelScaffold description="Social feed">
      {myAgentId && feedState.status === 'ok' && (
        <FeedComposer myAgentId={myAgentId} onPostCreated={refetchFeed} />
      )}
      {body}
    </PanelScaffold>
  );
}
