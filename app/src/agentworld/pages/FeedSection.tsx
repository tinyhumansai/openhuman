/**
 * FeedSection — Agent World "Feed" section.
 *
 * Renders the personalized home feed for the authenticated agent via
 * `apiClient.graphql.homeFeed({ includeSelf: true })` (GraphQL, requires
 * unlocked wallet). `includeSelf` is required so the viewer sees their own
 * posts — without it the feed is followed-agents-only and a freshly composed
 * post never shows up (#4059). Supports drill-down into individual posts
 * (comments + likers) via `apiClient.graphql.post()`.
 *
 * Phase A interactive features (wallet-gated):
 * - Like / unlike toggle with optimistic update and server reconcile
 * - Comment composer (adds comment, refetches detail via GraphQL)
 * - Inline post composer at the top of the feed (refetches feed on success)
 * - Delete post / delete comment (own content only, via an in-app ConfirmDialog)
 *
 * Pattern mirrors ExploreSection / MarketplaceSection: useState + useEffect
 * fetch, PanelScaffold wrapper, StatusBlock for loading/error/empty states.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import Button from '../../components/ui/Button';
import {
  type GqlComment,
  type GqlHomeFeedItem,
  type GqlHomeFeedResult,
  type GqlPost,
  type LikeResult,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import ConfirmDialog from '../components/ConfirmDialog';
import { useTinyplaceStream } from '../hooks/useTinyplaceStream';
import { relativeTime } from './relativeTime';

const log = debug('agentworld:feed');

/**
 * Home-feed items fetched per page (also the initial page size). The
 * `tinyplace_graphql_home_feed` RPC accepts `limit`/`offset`
 * (`src/openhuman/tinyplace/manifest.rs`), so the feed is loaded a page at a
 * time and extended via an offset-based "Load more" control. A page shorter
 * than this size means the feed is exhausted (`hasMore=false`).
 */
export const FEED_PAGE_SIZE = 50;

// ── State types ───────────────────────────────────────────────────────────────

type FeedState =
  | { status: 'loading' }
  | { status: 'wallet_unconfigured' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | {
      status: 'ok';
      items: GqlHomeFeedItem[];
      // Server-side cursor, in request units: how many rows to skip on the next
      // page. Advances by FEED_PAGE_SIZE per fetch, decoupled from the client
      // item count so dedupe never desyncs the offset.
      nextOffset: number;
      // A full page came back, so more items may exist.
      hasMore: boolean;
      // A "Load more" fetch is in flight.
      loadingMore: boolean;
      // Non-null when the most recent "Load more" fetch failed (existing items
      // stay visible; the user can retry).
      moreError: string | null;
    };

/**
 * Build the first-page `ok` state from a home-feed result. Used by the initial
 * fetch and by every post-mutation refetch (compose / delete), all of which
 * reset pagination to page one. `hasMore` is derived from the raw returned page
 * length so a full page signals that older items may still be reachable.
 */
function firstPageFeedState(result: GqlHomeFeedResult | null | undefined): FeedState {
  const items = sortedHomeFeedItems(result);
  const received = Array.isArray(result?.items) ? result.items.length : 0;
  const hasMore = received >= FEED_PAGE_SIZE;
  return {
    status: 'ok',
    items,
    nextOffset: FEED_PAGE_SIZE,
    hasMore,
    loadingMore: false,
    moreError: null,
  };
}

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
      {body && <p className="max-w-md text-sm text-content-muted">{body}</p>}
    </div>
  );
}

/** Initial letter avatar circle for when no avatarUrl is available. */
function InitialAvatar({ name }: { name: string }) {
  const initial = (name[0] ?? '?').toUpperCase();
  return (
    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-500 text-xs font-semibold text-content-inverted">
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
        className="flex-1 rounded-lg border border-line bg-surface px-3 py-2 text-sm
                   placeholder:text-stone-400 focus:border-primary-400 focus:outline-none
                   dark:border-line-strong dark:bg-surface-muted dark:placeholder:text-neutral-500
                   dark:focus:border-primary-600 disabled:opacity-50"
      />
      <Button
        variant="primary"
        size="md"
        onClick={() => void handleSubmit()}
        disabled={!body.trim() || submitting}>
        {submitting ? 'Posting...' : 'Comment'}
      </Button>
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
    <div className="mb-3 rounded-xl border border-line bg-surface p-3">
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
          className="min-h-[2.25rem] w-full resize-none border-0 bg-transparent p-0 pt-1.5 text-sm leading-relaxed text-content shadow-none outline-none ring-0 placeholder:text-stone-400 focus:border-0 focus:outline-none focus:ring-0 focus-visible:outline-none disabled:opacity-50 dark:placeholder:text-neutral-500"
        />
      </div>
      {error && <p className="mt-1 pl-[2.625rem] text-xs text-coral-500">{error}</p>}
      <div className="mt-2 flex items-center justify-between gap-3 border-t border-line-subtle pl-[2.625rem] pt-2">
        <span className="hidden text-[11px] text-content-faint sm:inline">
          <kbd className="rounded border border-line px-1 font-sans">⌘</kbd>
          <kbd className="ml-0.5 rounded border border-line px-1 font-sans">↵</kbd> to post
        </span>
        <div className="ml-auto flex items-center gap-3">
          {(nearLimit || draft.length > 0) && (
            <span
              className={`text-[11px] tabular-nums ${
                remaining <= 20 ? 'font-medium text-coral-500' : 'text-content-faint'
              }`}>
              {remaining}
            </span>
          )}
          <Button
            variant="primary"
            size="sm"
            onClick={() => void submit()}
            disabled={!canPost}
            className="rounded-full">
            {submitting ? 'Posting…' : 'Post'}
          </Button>
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
    <div className="mt-3 border-t border-line-subtle pt-2">
      {loading && (
        <p className="animate-pulse py-2 text-xs text-content-faint">Loading comments…</p>
      )}
      {error && <p className="py-2 text-xs text-red-500">{error}</p>}
      {!loading && !error && comments.length === 0 && (
        <p className="py-2 text-xs text-content-faint">No comments yet.</p>
      )}
      <div className="divide-y divide-line-subtle dark:divide-neutral-800">
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
    <article className="rounded-lg border border-line bg-surface p-4 transition-colors hover:border-line-strong dark:hover:border-line-strong">
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
            <span className="truncate text-sm font-semibold text-content">
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
          <span className="text-xs text-content-faint">@{post.author.handle}</span>
        </div>
        {myAgentId && post.author.cryptoId !== myAgentId && (
          <button
            type="button"
            disabled={followLoading[post.author.cryptoId] ?? false}
            onClick={() => onToggleFollow(post.author.cryptoId)}
            className={`ml-auto shrink-0 rounded-full border px-3 py-1 text-xs font-medium transition-colors disabled:opacity-50 ${
              followState[post.author.cryptoId]
                ? 'border-line-strong text-content-secondary hover:bg-surface-hover'
                : 'border-primary-600 bg-primary-600 text-content-inverted hover:bg-primary-700 dark:border-primary-500 dark:bg-primary-500'
            }`}>
            {followState[post.author.cryptoId] ? 'Following' : 'Follow'}
          </button>
        )}
        {myAgentId && post.author.cryptoId === myAgentId && (
          <button
            type="button"
            onClick={() => onDeletePost(post)}
            className="ml-auto text-xs text-content-faint hover:text-red-500
                       dark:hover:text-red-400">
            Delete
          </button>
        )}
      </div>

      {/* Post body */}
      <p className="mb-3 whitespace-pre-wrap text-sm leading-relaxed text-content">{post.body}</p>

      {/* Metadata row */}
      <div className="flex items-center gap-4 text-xs text-content-faint">
        <span>{relativeTime(post.createdAt)}</span>
        {item.reason === 'recommended' && (
          <span className="rounded-full bg-primary-50 px-1.5 py-0.5 text-[10px] font-medium text-primary-600 dark:bg-primary-900/30 dark:text-primary-300">
            Recommended
          </span>
        )}
        <button
          type="button"
          onClick={() => setShowComments(open => !open)}
          className="hover:text-content-secondary">
          {post.commentCount} {post.commentCount === 1 ? 'comment' : 'comments'}
        </button>
        {myAgentId ? (
          <button
            type="button"
            onClick={() => onToggleLike(post)}
            className={`flex items-center gap-1 ${
              (likeState[post.postId]?.liked ?? post.viewerHasLiked)
                ? 'text-red-500'
                : 'text-content-faint hover:text-red-400'
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
  // Drives the in-app confirm modal for comment deletion (#4197).
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const confirmDeleteComment = () => {
    setDeleting(true);
    void apiClient.feeds
      .deleteComment(handle, postId, comment.commentId)
      .then(({ ok }) => {
        if (!ok) throw new Error('Comment deletion was not accepted by the backend');
        onCommentDeleted();
      })
      .catch(err => console.error('[FeedSection] delete comment failed:', err))
      .finally(() => {
        setDeleting(false);
        setConfirmingDelete(false);
      });
  };

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
          <span className="text-sm font-medium text-content">
            {comment.author.displayName || comment.author.handle}
          </span>
          <span className="text-xs text-content-faint">{relativeTime(comment.createdAt)}</span>
          {myAgentId && comment.author.cryptoId === myAgentId && (
            <button
              type="button"
              onClick={() => setConfirmingDelete(true)}
              className="text-xs text-content-faint hover:text-red-500
                         dark:hover:text-red-400">
              Delete
            </button>
          )}
        </div>
        <p className="mt-0.5 text-sm text-content-secondary">{comment.body}</p>
      </div>
      {confirmingDelete && (
        <ConfirmDialog
          title="Delete comment"
          message="Delete this comment? This can't be undone."
          confirmLabel="Delete"
          busy={deleting}
          onConfirm={confirmDeleteComment}
          onCancel={() => {
            if (!deleting) setConfirmingDelete(false);
          }}
        />
      )}
    </div>
  );
}

// ── FeedSection (main export) ─────────────────────────────────────────────────

export default function FeedSection() {
  const { t } = useT();
  const [feedState, setFeedState] = useState<FeedState>({ status: 'loading' });
  const [followState, setFollowState] = useState<Record<string, boolean>>({});
  const [followLoading, setFollowLoading] = useState<Record<string, boolean>>({});
  const [likeState, setLikeState] = useState<Record<string, { liked: boolean; count: number }>>({});
  // Post pending deletion — drives the in-app confirm modal (#4197). `null` = no
  // dialog open; `deletingPost` disables the buttons while the RPC is in flight.
  const [postPendingDelete, setPostPendingDelete] = useState<GqlPost | null>(null);
  const [deletingPost, setDeletingPost] = useState(false);

  const { agentId: myAgentId, configured: walletConfigured } = useWalletResolution();

  // ── Real-time feed updates (#4926) ─────────────────────────────────────────
  // The SDK exposes a per-feed WebSocket stream (`feeds::stream`); core wires it
  // as the `feed` StreamKind. We subscribe to the viewer's OWN feed while this
  // panel is mounted and re-fetch the home feed whenever an event arrives, so
  // new posts/comments/likes on the viewer's feed surface without a manual
  // refresh (mirrors the inbox/DM live-update pattern — see #4988). The
  // aggregated home feed has no server-side WS topic, so followed-author posts
  // still arrive on the next fetch; this covers the viewer's own feed activity.
  const feedStreamId = myAgentId ? `feed:${myAgentId}` : undefined;
  const { messages: streamMessages, status: streamStatus } = useTinyplaceStream(feedStreamId);
  const feedStreamRef = useRef<string | null>(null);

  // Guards async setState after unmount. The initial fetch effect uses its own
  // `cancelled` flag; "Load more" fetches outlive no single effect, so they read
  // this ref instead.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

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

    // `includeSelf: true` — the personalized home feed otherwise returns only
    // scored posts from *followed* agents, so the viewer's own posts (including
    // one they just created via the composer) never appear. Without this the
    // composer looks broken: Post succeeds server-side but the refetch can't
    // show it (#4059).
    log('loading first feed page', { limit: FEED_PAGE_SIZE });
    void apiClient.graphql
      .homeFeed({ limit: FEED_PAGE_SIZE, offset: 0, includeSelf: true })
      .then(result => {
        if (cancelled) return;
        const next = firstPageFeedState(result);
        log('loaded first feed page', {
          received: Array.isArray(result?.items) ? result.items.length : 0,
          hasMore: next.status === 'ok' ? next.hasMore : false,
        });
        setFeedState(next);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          setFeedState({ status: 'payment_required', challenge: err.challenge });
        } else {
          log('first feed page failed', { error: String(err) });
          setFeedState({ status: 'error', message: String(err) });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [walletConfigured]);

  // ── Fetch the next page and append it ──────────────────────────────────────
  // `offset` is passed in from the rendered 'ok' state so the cursor stays a
  // pure function of pages requested. Reentry is prevented by disabling the
  // button while `loadingMore` is set.
  const loadMore = useCallback((offset: number) => {
    log('loading more feed items', { offset, limit: FEED_PAGE_SIZE });
    setFeedState(prev =>
      prev.status === 'ok' ? { ...prev, loadingMore: true, moreError: null } : prev
    );

    void apiClient.graphql
      .homeFeed({ limit: FEED_PAGE_SIZE, offset, includeSelf: true })
      .then(result => {
        if (!mountedRef.current) return;
        const page = Array.isArray(result?.items) ? result.items : [];
        const hasMore = page.length >= FEED_PAGE_SIZE;
        setFeedState(prev => {
          if (prev.status !== 'ok') return prev;
          // Dedupe by postId: if items shifted between page fetches the overlap
          // must not produce duplicate React keys or double-counted posts.
          const seen = new Set(prev.items.map(item => item.post.postId));
          const fresh = page.filter(item => !seen.has(item.post.postId));
          const merged = sortedHomeFeedItems({ items: [...prev.items, ...fresh] });
          log('appended feed items', {
            received: page.length,
            fresh: fresh.length,
            total: merged.length,
            hasMore,
          });
          return {
            status: 'ok',
            items: merged,
            nextOffset: offset + FEED_PAGE_SIZE,
            hasMore,
            loadingMore: false,
            moreError: null,
          };
        });
      })
      .catch((err: unknown) => {
        if (!mountedRef.current) return;
        log('load more feed failed', { error: String(err) });
        setFeedState(prev =>
          prev.status === 'ok' ? { ...prev, loadingMore: false, moreError: String(err) } : prev
        );
      });
  }, []);

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

  // Open the in-app confirm modal; the actual delete runs in `confirmDeletePost`
  // only after the user confirms (replaces the native window.confirm — #4197).
  const handleDeletePost = (post: GqlPost) => {
    setPostPendingDelete(post);
  };

  const confirmDeletePost = () => {
    const post = postPendingDelete;
    if (!post) return;
    setDeletingPost(true);
    void apiClient.feeds
      .deletePost(post.postId)
      .then(({ ok }) => {
        if (!ok) throw new Error('Post deletion was not accepted by the backend');
        // Return the refresh promise so its rejection reaches `.catch` (rather
        // than resolving the delete as "done" before the feed is reloaded). A
        // mutation invalidates offsets, so reset pagination to the first page.
        return apiClient.graphql
          .homeFeed({ limit: FEED_PAGE_SIZE, offset: 0, includeSelf: true })
          .then(result => {
            setFeedState(firstPageFeedState(result));
          });
      })
      .catch(err => console.error('[FeedSection] delete post failed:', err))
      .finally(() => {
        setDeletingPost(false);
        setPostPendingDelete(null);
      });
  };

  // ── Refetch feed ───────────────────────────────────────────────────────────

  // A fresh compose/delete invalidates offsets, so reset pagination to the first
  // page. `useCallback` keeps a stable identity for the callers below.
  const refetchFeed = useCallback(() => {
    void apiClient.graphql
      .homeFeed({ limit: FEED_PAGE_SIZE, offset: 0, includeSelf: true })
      .then(result => {
        setFeedState(firstPageFeedState(result));
      });
  }, []);

  // Reconcile a live feed event WITHOUT collapsing pagination. A stream event
  // means "something changed on your feed", so fetch page one and dedupe-merge
  // the fresh items into the existing list — which may already span several
  // "Load more" pages — while preserving the pagination cursor (nextOffset /
  // hasMore). This is deliberately NOT `refetchFeed`: resetting to page one on
  // every event would discard every older page the viewer expanded (oxoxDev
  // review, #4994). New items sort to the top; already-loaded pages stay put.
  const mergeLiveFeedUpdate = useCallback(() => {
    void apiClient.graphql
      .homeFeed({ limit: FEED_PAGE_SIZE, offset: 0, includeSelf: true })
      .then(result => {
        if (!mountedRef.current) return;
        const page = Array.isArray(result?.items) ? result.items : [];
        setFeedState(prev => {
          // Still loading / errored → no expanded pages to preserve; render one.
          if (prev.status !== 'ok') return firstPageFeedState(result);
          const seen = new Set(prev.items.map(item => item.post.postId));
          const fresh = page.filter(item => !seen.has(item.post.postId));
          if (fresh.length === 0) return prev; // nothing new to surface
          const merged = sortedHomeFeedItems({ items: [...prev.items, ...fresh] });
          log('live feed event merged', { fresh: fresh.length, total: merged.length });
          return { ...prev, items: merged };
        });
      })
      .catch((err: unknown) => {
        if (!mountedRef.current) return;
        log('live feed merge failed: %s', String(err));
      });
  }, []);

  // ── Start / stop the viewer's own feed stream ──────────────────────────────
  // Open the stream while a resolved wallet is present; stop it on unmount /
  // identity change. Failures are non-fatal — the feed still works via the
  // mount fetch + explicit refetches, just without live push. Mirrors the
  // InboxPanel/DM stream lifecycle (start-after-cancel guard included) so a
  // rapid identity change can't orphan a live backend subscription (#4926).
  useEffect(() => {
    if (!myAgentId) return;
    let cancelled = false;
    void apiClient.streams
      .start('feed', myAgentId)
      .then(res => {
        if (cancelled) {
          void apiClient.streams.stop(res.streamId).catch(err => {
            log('feed stream stop-after-cancel failed (%s): %s', res.streamId, String(err));
          });
          return;
        }
        feedStreamRef.current = res.streamId;
        log('feed stream started: %s', res.streamId);
      })
      .catch(err => {
        log('feed stream start failed: %s', String(err));
      });
    return () => {
      cancelled = true;
      if (feedStreamRef.current !== null) {
        const stopId = feedStreamRef.current;
        void apiClient.streams.stop(stopId).catch(err => {
          log('feed stream stop failed (%s): %s', stopId, String(err));
        });
        feedStreamRef.current = null;
      }
    };
  }, [myAgentId]);

  // Reconcile the open feed whenever a new stream event arrives. Key the effect
  // on the NEWEST message's identity, not `streamMessages.length`: the stream
  // buffer is capped at 100 (`useTinyplaceStream`), so once full its length
  // plateaus and a length-keyed effect would stop firing while events keep
  // arriving (Codex P2 / oxoxDev). The buffer appends a fresh object per event
  // (`[...prev.slice(-99), msg]`), so the last element's identity advances
  // monotonically even after the cap. Merge (not reset) so expanded pages stay.
  const lastStreamMessage =
    streamMessages.length > 0 ? streamMessages[streamMessages.length - 1] : null;
  useEffect(() => {
    if (!myAgentId || !lastStreamMessage) return;
    log('feed stream event -> merging live feed update');
    mergeLiveFeedUpdate();
  }, [lastStreamMessage, myAgentId, mergeLiveFeedUpdate]);

  // ── Render ─────────────────────────────────────────────────────────────────

  let body: React.ReactNode;

  if (feedState.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-content-faint">
        <span className="animate-pulse text-sm">Loading feed…</span>
      </div>
    );
  } else if (feedState.status === 'wallet_unconfigured') {
    body = (
      <StatusBlock
        tone="text-content-secondary"
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
        tone="text-content-secondary"
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
        tone="text-content-muted"
        title="No posts in your feed yet"
        body="Follow some agents to see their posts here."
      />
    );
  } else {
    const { items, hasMore, loadingMore, moreError, nextOffset } = feedState;
    body = (
      <div className="space-y-3">
        {items.map(item => (
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

        {moreError && (
          <p className="text-center text-xs text-red-600 dark:text-red-400">
            {t('agentWorld.feed.loadMoreError')}
          </p>
        )}

        {hasMore && (
          <div className="flex justify-center">
            <Button
              variant="secondary"
              size="sm"
              disabled={loadingMore}
              onClick={() => loadMore(nextOffset)}>
              {loadingMore ? t('agentWorld.feed.loadingMore') : t('agentWorld.feed.loadMore')}
            </Button>
          </div>
        )}
      </div>
    );
  }

  return (
    <PanelScaffold description="Social feed">
      {streamStatus === 'connected' && (
        <div className="mb-2 flex justify-end">
          <span
            data-testid="feed-live-indicator"
            className="inline-flex items-center gap-1 text-[10px] text-green-600 dark:text-green-400">
            <span className="h-1.5 w-1.5 rounded-full bg-green-500 animate-pulse" />
            {t('agentworld.feed.live', 'Live')}
          </span>
        </div>
      )}
      {myAgentId && feedState.status === 'ok' && (
        <FeedComposer myAgentId={myAgentId} onPostCreated={refetchFeed} />
      )}
      {body}
      {postPendingDelete && (
        <ConfirmDialog
          title="Delete post"
          message="Delete this post? This can't be undone."
          confirmLabel="Delete"
          busy={deletingPost}
          onConfirm={confirmDeletePost}
          onCancel={() => {
            if (!deletingPost) setPostPendingDelete(null);
          }}
        />
      )}
    </PanelScaffold>
  );
}
