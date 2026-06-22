import debugFactory from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useUser } from '../../hooks/useUser';
import { useT } from '../../lib/i18n/I18nContext';
import { feedbackApi } from '../../services/api/feedbackApi';
import type { FeedbackComment } from '../../types/feedback';

const log = debugFactory('feedback:comments');

const COMMENT_MAX = 4000;

interface FeedbackCommentsProps {
  feedbackId: string;
  /** Bumps the parent item's comment count after a comment is posted. */
  onCommentAdded: () => void;
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ''
    : date.toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}

/** Short, stable label for an author whose id we have but whose name we don't. */
function authorLabel(userId: string): string {
  return userId.length > 6 ? `@${userId.slice(-6)}` : `@${userId}`;
}

export default function FeedbackComments({ feedbackId, onCommentAdded }: FeedbackCommentsProps) {
  const { t } = useT();
  const { user } = useUser();
  const [comments, setComments] = useState<FeedbackComment[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [input, setInput] = useState('');
  const [posting, setPosting] = useState(false);
  const [postError, setPostError] = useState<string | null>(null);
  const loadRequestIdRef = useRef(0);

  useEffect(() => {
    const requestId = ++loadRequestIdRef.current;
    setIsLoading(true);
    setLoadError(null);
    feedbackApi
      .getFeedback(feedbackId)
      .then(detail => {
        if (requestId !== loadRequestIdRef.current) return;
        setComments(detail.comments);
      })
      .catch((error: unknown) => {
        if (requestId !== loadRequestIdRef.current) return;
        log('load comments failed id=%s error=%O', feedbackId, error);
        setLoadError(error instanceof Error ? error.message : t('feedback.comments.loadError'));
      })
      .finally(() => {
        if (requestId === loadRequestIdRef.current) setIsLoading(false);
      });
    return () => {
      loadRequestIdRef.current += 1;
    };
  }, [feedbackId, t]);

  const handlePost = async () => {
    const trimmed = input.trim();
    if (!trimmed || posting) return;
    setPosting(true);
    setPostError(null);
    try {
      const comment = await feedbackApi.addComment(feedbackId, trimmed);
      setComments(prev => [...prev, comment]);
      setInput('');
      onCommentAdded();
    } catch (error) {
      log('post comment failed id=%s error=%O', feedbackId, error);
      setPostError(error instanceof Error ? error.message : t('feedback.comments.postError'));
    } finally {
      setPosting(false);
    }
  };

  return (
    <div className="mt-3 space-y-3 border-t border-neutral-200 pt-3 dark:border-neutral-800">
      {isLoading ? (
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{t('common.loading')}</p>
      ) : loadError ? (
        <p className="text-xs text-coral-600 dark:text-coral-400">{loadError}</p>
      ) : comments.length === 0 ? (
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          {t('feedback.comments.empty')}
        </p>
      ) : (
        <ul className="space-y-2">
          {comments.map(comment => {
            const isMine = user?._id === comment.user;
            return (
              <li
                key={comment.id}
                className="rounded-xl bg-neutral-50 px-3 py-2 dark:bg-white/[0.03]">
                <div className="flex items-center gap-2 text-xs text-neutral-400 dark:text-neutral-500">
                  <span className="font-medium text-neutral-600 dark:text-neutral-300">
                    {isMine
                      ? t('feedback.comments.you')
                      : comment.userName?.trim() || authorLabel(comment.user)}
                  </span>
                  <span>·</span>
                  <span>{formatDate(comment.createdAt)}</span>
                </div>
                <p className="mt-1 whitespace-pre-wrap break-words text-sm text-neutral-700 dark:text-neutral-300">
                  {comment.body}
                </p>
              </li>
            );
          })}
        </ul>
      )}

      <div className="flex items-start gap-2">
        <textarea
          value={input}
          maxLength={COMMENT_MAX}
          onChange={e => setInput(e.target.value)}
          placeholder={t('feedback.comments.placeholder')}
          disabled={posting}
          rows={2}
          className="flex-1 resize-y rounded-xl border border-neutral-200 bg-neutral-50 px-3 py-2 text-sm text-neutral-900 placeholder:text-neutral-400 transition-all focus:border-primary-500/50 focus:bg-white focus:outline-none focus:ring-2 focus:ring-primary-500/30 dark:border-neutral-700 dark:bg-white/[0.03] dark:text-neutral-100 dark:placeholder:text-neutral-500"
        />
        <button
          type="button"
          onClick={handlePost}
          disabled={posting || !input.trim()}
          className="whitespace-nowrap rounded-xl bg-primary-500 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary-600 active:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50">
          {posting ? '...' : t('feedback.comments.post')}
        </button>
      </div>
      {postError && <p className="text-xs text-coral-600 dark:text-coral-400">{postError}</p>}
    </div>
  );
}
