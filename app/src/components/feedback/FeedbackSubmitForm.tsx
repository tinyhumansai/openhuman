import debugFactory from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { feedbackApi } from '../../services/api/feedbackApi';
import { messageForApiError } from '../../services/apiError';
import type { CreateFeedbackResult, FeedbackQuality, FeedbackType } from '../../types/feedback';
import { Button, TextArea, TextField } from '../ui';

const log = debugFactory('feedback:submit');

// Mirror the server-side caps (FEEDBACK_TITLE_MAX / FEEDBACK_BODY_MAX).
const TITLE_MAX = 200;
const BODY_MAX = 4000;

// Long enough that a burst of typing is one call, short enough that the hint
// still arrives while the user is looking at what they wrote.
const VALIDATE_DEBOUNCE_MS = 300;

// Shared by the hint and the `aria-describedby` that points submit at it.
const QUALITY_HINT_ID = 'feedback-quality-hint';

type SubmitStatus = 'idle' | 'loading' | 'accepted' | 'rejected' | 'error';

interface FeedbackSubmitFormProps {
  /** Called with the published item when a submission is accepted. */
  onAccepted: (result: CreateFeedbackResult) => void;
}

const INPUT_CLASS = 'w-full rounded-xl bg-surface-muted px-4 py-2.5';

export default function FeedbackSubmitForm({ onAccepted }: FeedbackSubmitFormProps) {
  const { t } = useT();
  const [type, setType] = useState<FeedbackType>('feature');
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [status, setStatus] = useState<SubmitStatus>('idle');
  const [message, setMessage] = useState<string | null>(null);
  // The verdict is stored against the draft it was computed for, so a verdict
  // for text the user has since changed is simply not the current one — no
  // clearing pass, and a stale `block` can never disable submit for a draft it
  // was never about. `submittedQuality` is kept apart so clearing the form
  // after a warned submission does not clear the advice it came back with.
  const [verdict, setVerdict] = useState<{ draft: string; quality: FeedbackQuality } | null>(null);
  const [submittedQuality, setSubmittedQuality] = useState<FeedbackQuality | null>(null);

  const draftTitle = title.trim();
  const draftBody = body.trim();
  const withinCaps = draftTitle.length <= TITLE_MAX && draftBody.length <= BODY_MAX;
  const validatable = Boolean(draftTitle && draftBody && withinCaps);
  const draftKey = JSON.stringify([type, draftTitle, draftBody]);

  // The server rejects a blocked submission anyway — `POST /feedback` runs the
  // same rules — so this is a courtesy that saves a round trip on text the user
  // can still fix, not the enforcement point.
  useEffect(() => {
    if (!validatable) return;

    // A superseded check must not write at all. Keying the verdict only guards
    // the *read*: if an older call answers after a newer one, an unguarded
    // write replaces a correct verdict with one that no longer matches the
    // draft, and the hint disappears until the user types again.
    let cancelled = false;
    const timer = setTimeout(() => {
      feedbackApi
        .validateFeedback({ type, title: draftTitle, body: draftBody })
        .then(quality => {
          if (!cancelled) setVerdict({ draft: draftKey, quality });
        })
        .catch(() => {
          // The check is advisory. If it cannot run, say nothing and let the
          // submit path be the judge rather than blocking on our own outage.
          // `feedbackApi` already logged the failure with its cause.
          log('validate unavailable, leaving the draft unjudged type=%s', type);
        });
    }, VALIDATE_DEBOUNCE_MS);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [validatable, draftKey, type, draftTitle, draftBody]);

  const draftQuality = verdict?.draft === draftKey ? verdict.quality : null;
  const hint = submittedQuality ?? draftQuality;
  // `pass` has nothing to say, and neither does an empty reason: rendering one
  // would be an empty paragraph that still shifts the layout.
  const visibleHint = hint && hint.tier !== 'pass' && hint.reason ? hint : null;
  // Never disable submit without saying why. A `block` we cannot explain would
  // be a dead end, so let it through and take the server's refusal, which
  // carries the reason. Enforcement is server-side either way.
  const blocked = draftQuality?.tier === 'block' && Boolean(draftQuality.reason);

  const canSubmit =
    status !== 'loading' &&
    !blocked &&
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
        // A warned item is published; the advice outlives the text it was about.
        setSubmittedQuality(result.quality?.tier === 'warn' ? result.quality : null);
        onAccepted(result);
      } else {
        // Moderation rejected the content — not an error, but not published.
        setStatus('rejected');
        setSubmittedQuality(null);
        setMessage(result.reason || t('feedback.submit.rejected'));
      }
    } catch (err) {
      // No error payload: on a quality block the message is the server's
      // account of the user's own draft, and it is already on screen below.
      log('submit failed type=%s', type);
      setStatus('error');
      setSubmittedQuality(null);
      setMessage(messageForApiError(err, t('feedback.submit.error')));
    }
  };

  const messageClass =
    status === 'accepted'
      ? 'text-sage-600 dark:text-sage-400'
      : status === 'rejected'
        ? 'text-amber-600 dark:text-amber-400'
        : 'text-coral-600 dark:text-coral-400';

  return (
    <div className="rounded-2xl border border-line bg-surface p-6 shadow-soft dark:shadow-none">
      <h2 className="font-title text-base font-semibold text-content">
        {t('feedback.submit.heading')}
      </h2>
      <p className="mb-4 mt-0.5 text-xs text-content-muted">{t('feedback.submit.subheading')}</p>

      <div className="mb-4 grid grid-cols-2 gap-2.5">
        <Button
          variant="secondary"
          size="lg"
          onClick={() => {
            // Changing the type is an edit like any other: the advice the last
            // submission came back with is no longer about what is on screen.
            setType('feature');
            setSubmittedQuality(null);
          }}
          aria-pressed={type === 'feature'}
          className={
            type === 'feature'
              ? 'border-primary-500 bg-primary-500/10 text-primary-600 ring-1 ring-primary-500/30 dark:text-primary-400'
              : 'text-content-muted'
          }>
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6L12 3zM18.5 14.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7.7-1.8z"
            />
          </svg>
          {t('feedback.type.feature')}
        </Button>
        <Button
          variant="secondary"
          size="lg"
          onClick={() => {
            setType('bug');
            setSubmittedQuality(null);
          }}
          aria-pressed={type === 'bug'}
          tone={type === 'bug' ? 'danger' : 'default'}
          className={type === 'bug' ? 'ring-1 ring-coral-500/30' : 'text-content-muted'}>
          <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.8}
              d="M12 8a4 4 0 00-4 4v2a4 4 0 008 0v-2a4 4 0 00-4-4zM9.5 5.5L8.2 4.2M14.5 5.5l1.3-1.3M8 12.5H4.5M16 12.5h3.5M8 16l-2.8 1.6M16 16l2.8 1.6"
            />
          </svg>
          {t('feedback.type.bug')}
        </Button>
      </div>

      <label htmlFor="feedback-title" className="sr-only">
        {t('feedback.submit.titlePlaceholder')}
      </label>
      <TextField
        id="feedback-title"
        type="text"
        value={title}
        maxLength={TITLE_MAX}
        onChange={e => {
          setTitle(e.target.value);
          setSubmittedQuality(null);
        }}
        placeholder={t('feedback.submit.titlePlaceholder')}
        disabled={status === 'loading'}
        className={`${INPUT_CLASS} mb-3`}
      />

      <label htmlFor="feedback-body" className="sr-only">
        {t('feedback.submit.bodyPlaceholder')}
      </label>
      <TextArea
        id="feedback-body"
        value={body}
        maxLength={BODY_MAX}
        onChange={e => {
          // Typing again is the user acting on the last advice; drop it.
          setBody(e.target.value);
          setSubmittedQuality(null);
        }}
        placeholder={t('feedback.submit.bodyPlaceholder')}
        disabled={status === 'loading'}
        rows={4}
        className={`${INPUT_CLASS} resize-y`}
      />

      {/* Nothing moves focus here and the hint arrives ~300ms after typing
          stops, so without a live region a blocked submitter hears the button
          go disabled with no reason given. The region is mounted
          unconditionally: one inserted at the same moment as its text gives
          assistive tech no change to observe, and on `block` it is the only
          announcement path there is — `aria-describedby` cannot cover for it,
          because a disabled button is not focusable. Same shape as
          `SystemDiagnostics.tsx` / `DeveloperOptionsPanel.tsx`. */}
      <div role="status" aria-live="polite" aria-atomic="true">
        {visibleHint && (
          <p
            id={QUALITY_HINT_ID}
            data-testid="feedback-quality-hint"
            data-tier={visibleHint.tier}
            // `block` is the harder outcome, so it gets the louder colour.
            className={`mt-2 text-xs ${
              visibleHint.tier === 'block'
                ? 'text-primary-600 dark:text-primary-400'
                : 'text-content-muted'
            }`}>
            {visibleHint.reason}
          </p>
        )}
      </div>

      <div className="mt-3 flex items-center justify-between gap-3">
        <Button
          variant="primary"
          size="lg"
          onClick={handleSubmit}
          disabled={!canSubmit}
          aria-describedby={visibleHint ? QUALITY_HINT_ID : undefined}>
          {status === 'loading' ? '...' : t('feedback.submit.action')}
        </Button>
        <div className="flex items-center gap-3">
          {message && <p className={`text-xs ${messageClass}`}>{message}</p>}
          {body.length > 0 && (
            <span className="text-[11px] tabular-nums text-content-faint">
              {body.length}/{BODY_MAX}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
