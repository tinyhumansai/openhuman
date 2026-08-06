import { useT } from '../../../lib/i18n/I18nContext';
import { BubbleMarkdown } from './AgentMessageBubble';

/**
 * The partial assistant reply left behind by an INTERRUPTED turn — the core
 * process that was streaming it exited before `chat_done`, so no final message
 * was ever committed to the durable thread log (restore-fidelity fix 2).
 *
 * Unlike the live streaming preview, this is a SETTLED buffer: it renders as a
 * static agent bubble (no pulsing cursor, full Markdown like a finished answer)
 * carrying an "Interrupted" marker, plus the hidden reasoning it had streamed in
 * a collapsed block. It is deliberately NOT written into the durable message
 * list — it is a restore-time surfacing of what the agent had produced, so the
 * user sees the partial work instead of a blank turn.
 */
export function InterruptedAnswer({ content, thinking }: { content: string; thinking: string }) {
  const { t } = useT();
  const trimmedContent = content.trim();
  const trimmedThinking = thinking.trim();
  // Nothing persisted to show — render nothing rather than an empty marked bubble.
  if (!trimmedContent && !trimmedThinking) return null;

  return (
    <div className="flex justify-start" data-testid="interrupted-answer">
      <div className="relative w-fit max-w-[75%] space-y-1">
        {trimmedThinking ? (
          <details className="mb-0.5 rounded-lg bg-surface-subtle px-3 py-1.5 text-xs text-content-secondary dark:bg-surface-muted">
            <summary className="flex cursor-pointer items-center gap-1.5 select-none">
              <span aria-hidden className="text-[10px] leading-none">
                💭
              </span>
              <span>{t('chat.thinking')}</span>
            </summary>
            <pre className="mt-1.5 font-sans text-[11px] break-words whitespace-pre-wrap text-content-muted">
              {trimmedThinking}
            </pre>
          </details>
        ) : null}
        <div className="rounded-2xl rounded-bl-md border-l-2 border-amber-400/70 bg-surface-strong/80 px-3 py-2 text-content dark:bg-surface-muted">
          <div
            className="mb-1 flex items-center gap-1.5 text-[10px] font-medium tracking-wide text-amber-600 uppercase dark:text-amber-300"
            data-testid="interrupted-answer-marker">
            <span aria-hidden className="text-[11px] leading-none">
              ⚠
            </span>
            <span>{t('intelligence.agentWork.status.interrupted')}</span>
          </div>
          {trimmedContent ? <BubbleMarkdown content={trimmedContent} /> : null}
        </div>
      </div>
    </div>
  );
}
