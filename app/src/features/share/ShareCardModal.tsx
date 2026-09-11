import { useCallback, useEffect, useRef, useState } from 'react';

import { trackAnalyticsEvent } from '../../components/analytics';
import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';
import { useT } from '../../lib/i18n/I18nContext';
import { openUrl } from '../../utils/openUrl';
import { draftShareHeadline } from './shareCaption';
import {
  CARD_HEIGHT,
  CARD_WIDTH,
  cardToPngBlob,
  renderShareCardToCanvas,
  type ShareCardFallbacks,
} from './shareCard';
import { buildShareCaption, type ShareCaptionTemplates } from './shareContent';
import { buildLinkedInShareUrl, buildTweetIntentUrl, SHARE_LANDING_URL } from './shareTargets';

const LOG_PREFIX = '[share-modal]';

/** Marketing URL printed on the card footer. */
const CARD_URL = 'tinyhumans.ai';

interface ShareCardModalProps {
  /** The assistant output being shared (already image-stripped plain text). */
  content: string;
  /** Display name of the agent/profile that produced the output. */
  agentName: string;
  /** Optional thread id, forwarded to inference for log grouping. */
  threadId?: string;
  onClose: () => void;
}

type CopyState = 'idle' | 'image' | 'caption';

/**
 * Preview-and-share modal for the share cards feature (#5006). Drafts a
 * headline (LLM with deterministic fallback), renders a branded PNG card, lets
 * the user edit the caption, and opens the X / LinkedIn composer or copies the
 * image. All rules that could touch inference live in `shareCaption.ts`; this
 * component is presentation + wiring only.
 */
export function ShareCardModal({ content, agentName, threadId, onClose }: ShareCardModalProps) {
  const { t } = useT();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [headline, setHeadline] = useState('');
  const [caption, setCaption] = useState('');
  const [drafting, setDrafting] = useState(true);
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const [imageError, setImageError] = useState(false);
  const [linkedInHint, setLinkedInHint] = useState(false);

  // Localized fallback text for the offline/empty-headline paths in
  // shareContent.ts / shareCard.ts. Kept in a ref (refreshed every render) so
  // the drafting effect below can read the latest translation without taking
  // a `t` dependency that would re-trigger the LLM draft RPC on locale change.
  const captionTemplatesRef = useRef<ShareCaptionTemplates>({
    emptyFallback: '',
    withHeadline: '',
  });
  captionTemplatesRef.current = {
    emptyFallback: t('share.defaultCaption'),
    withHeadline: t('share.captionWithHeadline'),
  };

  // Draft the headline once on open; degrade to the offline fallback on any
  // failure (draftShareHeadline never rejects).
  useEffect(() => {
    let cancelled = false;
    void draftShareHeadline(content, threadId).then(drafted => {
      if (cancelled) return;
      setHeadline(drafted);
      setCaption(buildShareCaption(drafted, captionTemplatesRef.current));
      setDrafting(false);
      console.debug(`${LOG_PREFIX} headline ready len=${drafted.length}`);
    });
    return () => {
      cancelled = true;
    };
  }, [content, threadId]);

  // (Re)paint the card whenever the headline settles, and also on locale
  // change so an empty-headline / empty-agent-name fallback repaints localized.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || drafting) return;
    const cardFallbacks: ShareCardFallbacks = {
      headline: t('share.defaultHeadline'),
      agentName: t('share.defaultAgentName'),
    };
    try {
      renderShareCardToCanvas(canvas, { headline, agentName, brandUrl: CARD_URL }, cardFallbacks);
      setImageError(false);
    } catch (err) {
      console.debug(`${LOG_PREFIX} render failed: ${String(err)}`);
      setImageError(true);
    }
  }, [headline, agentName, drafting, t]);

  const flashCopy = useCallback((state: Exclude<CopyState, 'idle'>) => {
    setCopyState(state);
    setTimeout(() => setCopyState('idle'), 2000);
  }, []);

  const handleShareX = useCallback(() => {
    const url = buildTweetIntentUrl(caption, SHARE_LANDING_URL);
    trackAnalyticsEvent('chat_message_shared', { destination: 'x' });
    void openUrl(url).catch(err => console.debug(`${LOG_PREFIX} openUrl x failed: ${String(err)}`));
  }, [caption]);

  const handleShareLinkedIn = useCallback(async () => {
    // LinkedIn's share endpoint ignores caption text, so copy it for pasting.
    try {
      await navigator.clipboard.writeText(caption);
      setLinkedInHint(true);
    } catch {
      // Non-fatal: the composer still opens, the user just retypes the caption.
    }
    trackAnalyticsEvent('chat_message_shared', { destination: 'linkedin' });
    void openUrl(buildLinkedInShareUrl(SHARE_LANDING_URL)).catch(err =>
      console.debug(`${LOG_PREFIX} openUrl linkedin failed: ${String(err)}`)
    );
  }, [caption]);

  const handleCopyImage = useCallback(async () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    try {
      const blob = await cardToPngBlob(canvas);
      const canWriteImage =
        typeof ClipboardItem !== 'undefined' && typeof navigator.clipboard?.write === 'function';
      if (canWriteImage) {
        try {
          await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
          flashCopy('image');
          trackAnalyticsEvent('chat_message_shared', { destination: 'copy_image' });
          return;
        } catch (clipboardErr) {
          // Clipboard write rejected (permission denied, MIME unsupported, or
          // WebView restrictions): fall through to the download fallback below
          // instead of surfacing an image error for a card that rendered fine.
          console.debug(
            `${LOG_PREFIX} clipboard image write rejected; falling back to download err_type=${
              clipboardErr instanceof Error ? clipboardErr.name : typeof clipboardErr
            }`
          );
        }
      }
      // Fallback: trigger a download so the user still gets the PNG.
      downloadBlob(blob, 'openhuman-share.png');
      flashCopy('image');
      trackAnalyticsEvent('chat_message_shared', { destination: 'save_image' });
    } catch (err) {
      console.debug(`${LOG_PREFIX} copy image failed: ${String(err)}`);
      setImageError(true);
    }
  }, [flashCopy]);

  const handleCopyCaption = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(caption);
      flashCopy('caption');
    } catch (err) {
      console.debug(`${LOG_PREFIX} copy caption failed: ${String(err)}`);
    }
  }, [caption, flashCopy]);

  return (
    <ModalShell
      onClose={onClose}
      titleId="share-card-title"
      title={t('share.modalTitle')}
      subtitle={t('share.modalSubtitle')}
      maxWidthClassName="max-w-lg"
      contentClassName="px-5 py-4 space-y-4">
      <div
        className="relative overflow-hidden rounded-xl border border-line-subtle bg-surface-muted"
        style={{ aspectRatio: `${CARD_WIDTH} / ${CARD_HEIGHT}` }}>
        {drafting ? (
          <div
            className="absolute inset-0 flex items-center justify-center text-sm text-content-muted"
            data-testid="share-card-drafting">
            {t('share.drafting')}
          </div>
        ) : null}
        <canvas
          ref={canvasRef}
          data-testid="share-card-canvas"
          aria-label={t('share.cardAlt')}
          className={`h-full w-full ${drafting ? 'opacity-0' : 'opacity-100'} transition-opacity`}
        />
        {imageError ? (
          <div className="absolute inset-0 flex items-center justify-center px-4 text-center text-sm text-coral-600 dark:text-coral-300">
            {t('share.imageError')}
          </div>
        ) : null}
      </div>

      <div className="space-y-1.5">
        <label htmlFor="share-caption" className="text-xs font-medium text-content-secondary">
          {t('share.captionLabel')}
        </label>
        <textarea
          id="share-caption"
          value={caption}
          onChange={e => setCaption(e.target.value)}
          rows={3}
          disabled={drafting}
          placeholder={t('share.captionPlaceholder')}
          className="w-full resize-none rounded-xl border border-line bg-surface px-3 py-2 text-sm text-content placeholder:text-content-faint focus:outline-hidden focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50"
        />
      </div>

      <p className="rounded-xl bg-surface-subtle px-3 py-2 text-xs text-content-muted">
        {t('share.privacyNote')}
      </p>

      {linkedInHint ? (
        <p className="text-xs text-sage-700 dark:text-sage-300">{t('share.linkedInHint')}</p>
      ) : null}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="primary"
          size="sm"
          analyticsId="chat-share-x"
          disabled={drafting}
          onClick={handleShareX}>
          {t('share.shareX')}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          analyticsId="chat-share-linkedin"
          disabled={drafting}
          onClick={() => void handleShareLinkedIn()}>
          {t('share.shareLinkedIn')}
        </Button>
        <Button
          variant="tertiary"
          size="sm"
          analyticsId="chat-share-copy-image"
          disabled={drafting}
          onClick={() => void handleCopyImage()}>
          {copyState === 'image' ? t('share.copiedImage') : t('share.copyImage')}
        </Button>
        <Button
          variant="tertiary"
          size="sm"
          analyticsId="chat-share-copy-caption"
          disabled={drafting}
          onClick={() => void handleCopyCaption()}>
          {copyState === 'caption' ? t('share.copiedCaption') : t('share.copyCaption')}
        </Button>
      </div>
    </ModalShell>
  );
}

function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}
