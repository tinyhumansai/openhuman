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

// The card fills the pane; the fields inside it do not. A title line and a
// description are prose, and prose past ~80 characters per line is measurably
// harder to read -- a full-pane textarea also makes a short body look like a
// mistake. 68ch keeps both comfortable without leaving the field looking stunted.
const INPUT_CLASS = 'w-full max-w-[68ch] rounded-xl bg-surface-muted px-4 py-2.5';

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
      {/* The type toggle used to be two full-width `size="lg"` buttons stacked
          above the fields: a binary property of the draft, rendered larger and
          louder than the title, the body and Submit combined, so the form read
          as "pick one of two things" and the primary action read as dead.
          It is now a pill beside the heading -- the same shape and placement
          the billing panel gives its monthly/annual switch, which is the same
          kind of control: one bit that qualifies the thing below it. */}
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="max-w-[52ch]">
          <h2 className="font-title text-base font-semibold text-content">
            {t('feedback.submit.heading')}
          </h2>
          <p className="mt-0.5 text-xs text-content-muted">{t('feedback.submit.subheading')}</p>
        </div>

        <div
          role="group"
          aria-label={t('feedback.submit.heading')}
          className="inline-flex w-fit shrink-0 rounded-full bg-surface-subtle p-1 ring-1 ring-line">
          <Button
            variant={type === 'feature' ? 'primary' : 'tertiary'}
            size="sm"
            className="rounded-full"
            onClick={() => {
              // Changing the type is an edit like any other: the advice the last
              // submission came back with is no longer about what is on screen.
              // Clicking the pill that is already selected is not an edit, so it
              // must not discard advice an accepted-with-warning submission just
              // produced -- easier to hit now that these are adjacent pills.
              if (type === 'feature') return;
              setType('feature');
              setSubmittedQuality(null);
            }}
            aria-pressed={type === 'feature'}>
            {t('feedback.type.feature')}
          </Button>
          <Button
            variant={type === 'bug' ? 'primary' : 'tertiary'}
            size="sm"
            className="rounded-full"
            onClick={() => {
              if (type === 'bug') return;
              setType('bug');
              setSubmittedQuality(null);
            }}
            aria-pressed={type === 'bug'}>
            {t('feedback.type.bug')}
          </Button>
        </div>
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
