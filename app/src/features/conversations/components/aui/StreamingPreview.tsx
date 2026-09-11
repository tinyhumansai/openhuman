import { useT } from '../../../../lib/i18n/I18nContext';
import { AuiPlainText } from './AuiPlainText';

/** Maximum trailing characters rendered in a live-streaming preview bubble.
 *  The full response is revealed via `addInferenceResponse` on `chat_done` —
 *  this is purely a ticker-tape affordance to signal progress without jumping
 *  the scroll position as tokens arrive. */
export const STREAMING_PREVIEW_CHARS = 120;

/** The blinking caret at the tail of an in-flight preview. Rendered by
 *  `MessagePartPrimitive.InProgress`, so it disappears the moment the part
 *  scope stops reporting `running` — no separate visibility condition. */
const StreamingCaret = (
  <span className="inline-block w-1 h-3 ml-0.5 align-middle bg-primary-400 animate-pulse" />
);

function PreviewBubble({ content, bordered }: { content: string; bordered?: boolean }) {
  return (
    <div
      className={`rounded-2xl rounded-bl-md px-3 py-1.5 bg-surface-strong/80 dark:bg-surface-muted text-content ${
        bordered ? 'border-l-2 border-primary-400/60' : ''
      }`}>
      <p className="text-xs text-content-secondary font-mono whitespace-pre-wrap wrap-break-word leading-snug">
        {content.length > STREAMING_PREVIEW_CHARS && <span className="text-content-faint">…</span>}
        <AuiPlainText
          text={content.slice(-STREAMING_PREVIEW_CHARS)}
          isRunning
          caret={StreamingCaret}
        />
      </p>
    </div>
  );
}

/**
 * The primary in-flight assistant preview.
 *
 * Reasoning is deliberately NOT rendered here — the rail above renders the same
 * reasoning through `ProcessingTranscriptView`, interleaved with the narration
 * and tool steps it happened between. A separate bubble would show it twice
 * with two lifetimes. The in-flight ANSWER preview is what stays: that is the
 * terminal response streaming in.
 */
export function StreamingAssistantPreview({ content }: { content: string }) {
  if (content.length === 0) return null;
  return (
    <div className="flex justify-start">
      <div className="relative w-fit max-w-[75%]">
        <PreviewBubble content={content} />
      </div>
    </div>
  );
}

export interface ParallelBranchStream {
  requestId: string;
  content: string;
  thinking: string;
}

/**
 * Concurrent (forked) turns on the same thread, each in its own labeled bubble
 * so they do not collide with the primary stream above.
 */
export function ParallelBranchPreviews({ branches }: { branches: ParallelBranchStream[] }) {
  const { t } = useT();
  return (
    <>
      {branches.map(branch =>
        branch.content.length > 0 || branch.thinking.length > 0 ? (
          <div key={branch.requestId} className="flex justify-start">
            <div className="relative w-fit max-w-[75%]">
              <div className="mb-1 flex items-center gap-1.5 text-[10px] font-medium uppercase tracking-wide text-primary-500 dark:text-primary-400">
                <span className="inline-block w-1.5 h-1.5 rounded-full bg-primary-400 animate-pulse" />
                <span>{t('chat.parallelBranchLabel')}</span>
              </div>
              {branch.content.length > 0 && <PreviewBubble content={branch.content} bordered />}
            </div>
          </div>
        ) : null
      )}
    </>
  );
}

/**
 * The three-dot placeholder shown between "turn accepted" and "first token".
 *
 * Suppressed by the caller once streaming output has started — the preview
 * bubble above takes over as the activity indicator from that point.
 */
export function TypingIndicator() {
  return (
    <div className="flex justify-start">
      <div className="bg-surface-strong/80 dark:bg-surface-muted rounded-2xl rounded-bl-md px-4 py-3">
        <div className="flex items-center gap-1">
          <span className="w-1.5 h-1.5 rounded-full bg-surface-muted dark:bg-surface-muted/600 animate-bounce [animation-delay:0ms]" />
          <span className="w-1.5 h-1.5 rounded-full bg-surface-muted dark:bg-surface-muted/600 animate-bounce [animation-delay:150ms]" />
          <span className="w-1.5 h-1.5 rounded-full bg-surface-muted dark:bg-surface-muted/600 animate-bounce [animation-delay:300ms]" />
        </div>
      </div>
    </div>
  );
}
