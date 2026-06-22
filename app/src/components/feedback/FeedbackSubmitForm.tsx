import debugFactory from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { feedbackApi } from '../../services/api/feedbackApi';
import type { CreateFeedbackResult, FeedbackType } from '../../types/feedback';

const log = debugFactory('feedback:submit');

// Mirror the server-side caps (FEEDBACK_TITLE_MAX / FEEDBACK_BODY_MAX).
const TITLE_MAX = 200;
const BODY_MAX = 4000;

type SubmitStatus = 'idle' | 'loading' | 'accepted' | 'rejected' | 'error';

interface FeedbackSubmitFormProps {
  /** Called with the published item when a submission is accepted. */
  onAccepted: (result: CreateFeedbackResult) => void;
}

const INPUT_CLASS =
  'w-full rounded-xl border border-neutral-200 bg-neutral-50 px-4 py-2.5 text-sm text-neutral-900 ' +
  'placeholder:text-neutral-400 transition-all focus:border-primary-500/50 focus:bg-white focus:outline-none ' +
  'focus:ring-2 focus:ring-primary-500/30 dark:border-neutral-700 dark:bg-white/[0.03] dark:text-neutral-100 ' +
  'dark:placeholder:text-neutral-500 dark:focus:bg-white/[0.06]';

export default function FeedbackSubmitForm({ onAccepted }: FeedbackSubmitFormProps) {
  const { t } = useT();
  const [type, setType] = useState<FeedbackType>('feature');
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [status, setStatus] = useState<SubmitStatus>('idle');
  const [message, setMessage] = useState<string | null>(null);

  const canSubmit =
    status !== 'loading' &&
    title.trim().length > 0 &&
    title.trim().length <= TITLE_MAX &&
    body.trim().length > 0 &&
    body.trim().length <= BODY_MAX;

  const handleSubmit = async () => {
    if (!canSubmit) return;
    setStatus('loading');
    setMessage(null);
    try {
      const result = await feedbackApi.submitFeedback({
        type,
        title: title.trim(),
        body: body.trim(),
      });
      if (result.accepted) {
        setStatus('accepted');
        setTitle('');
        setBody('');
        setMessage(t('feedback.submit.success'));
        onAccepted(result);
      } else {
        // Moderation rejected the content — not an error, but not published.
        setStatus('rejected');
        setMessage(result.reason || t('feedback.submit.rejected'));
      }
    } catch (err) {
      log('submit failed type=%s error=%O', type, err);
      setStatus('error');
      setMessage(err instanceof Error ? err.message : t('feedback.submit.error'));
    }
  };

  const messageClass =
    status === 'accepted'
      ? 'text-sage-600 dark:text-sage-400'
      : status === 'rejected'
        ? 'text-amber-600 dark:text-amber-400'
        : 'text-coral-600 dark:text-coral-400';

  return (
    <div className="rounded-2xl border border-neutral-200 bg-white p-6 shadow-soft dark:border-neutral-800 dark:bg-neutral-900 dark:shadow-none">
      <h2 className="font-display text-base font-semibold text-neutral-900 dark:text-neutral-100">
        {t('feedback.submit.heading')}
      </h2>
      <p className="mb-4 mt-0.5 text-xs text-neutral-500 dark:text-neutral-400">
        {t('feedback.submit.subheading')}
      </p>

      <div className="mb-4 grid grid-cols-2 gap-2.5">
        <button
          type="button"
          onClick={() => setType('feature')}
          aria-pressed={type === 'feature'}
          className={`flex items-center justify-center gap-2 rounded-xl border px-4 py-2.5 text-sm font-medium transition-all ${
            type === 'feature'
              ? 'border-primary-500 bg-primary-500/10 text-primary-600 ring-1 ring-primary-500/30 dark:text-primary-400'
              : 'border-neutral-200 text-neutral-500 hover:border-neutral-300 hover:bg-neutral-50 dark:border-neutral-700 dark:hover:bg-white/[0.03]'
          }`}>
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6L12 3zM18.5 14.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7.7-1.8z"
            />
          </svg>
          {t('feedback.type.feature')}
        </button>
        <button
          type="button"
          onClick={() => setType('bug')}
          aria-pressed={type === 'bug'}
          className={`flex items-center justify-center gap-2 rounded-xl border px-4 py-2.5 text-sm font-medium transition-all ${
            type === 'bug'
              ? 'border-coral-500 bg-coral-500/10 text-coral-600 ring-1 ring-coral-500/30 dark:text-coral-400'
              : 'border-neutral-200 text-neutral-500 hover:border-neutral-300 hover:bg-neutral-50 dark:border-neutral-700 dark:hover:bg-white/[0.03]'
          }`}>
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M12 8a4 4 0 00-4 4v2a4 4 0 008 0v-2a4 4 0 00-4-4zM9.5 5.5L8.2 4.2M14.5 5.5l1.3-1.3M8 12.5H4.5M16 12.5h3.5M8 16l-2.8 1.6M16 16l2.8 1.6"
            />
          </svg>
          {t('feedback.type.bug')}
        </button>
      </div>

      <label htmlFor="feedback-title" className="sr-only">
        {t('feedback.submit.titlePlaceholder')}
      </label>
      <input
        id="feedback-title"
        type="text"
        value={title}
        maxLength={TITLE_MAX}
        onChange={e => setTitle(e.target.value)}
        placeholder={t('feedback.submit.titlePlaceholder')}
        disabled={status === 'loading'}
        className={`${INPUT_CLASS} mb-3`}
      />

      <label htmlFor="feedback-body" className="sr-only">
        {t('feedback.submit.bodyPlaceholder')}
      </label>
      <textarea
        id="feedback-body"
        value={body}
        maxLength={BODY_MAX}
        onChange={e => setBody(e.target.value)}
        placeholder={t('feedback.submit.bodyPlaceholder')}
        disabled={status === 'loading'}
        rows={4}
        className={`${INPUT_CLASS} resize-y`}
      />

      <div className="mt-3 flex items-center justify-between gap-3">
        <button
          onClick={handleSubmit}
          disabled={!canSubmit}
          className="rounded-xl bg-primary-500 px-5 py-2.5 text-sm font-semibold text-white shadow-sm transition-colors hover:bg-primary-600 active:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50">
          {status === 'loading' ? '...' : t('feedback.submit.action')}
        </button>
        <div className="flex items-center gap-3">
          {message && <p className={`text-xs ${messageClass}`}>{message}</p>}
          {body.length > 0 && (
            <span className="text-[11px] tabular-nums text-neutral-400 dark:text-neutral-600">
              {body.length}/{BODY_MAX}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
