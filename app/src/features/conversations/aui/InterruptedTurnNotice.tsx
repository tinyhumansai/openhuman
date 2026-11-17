/**
 * End-of-transcript marker for a turn that was cut off — the core crashed or
 * restarted mid-reply — rather than stopped by the user (that is
 * `StoppedRunSlot`). `chatRuntime.interruptedAssistantByThread` is hydrated
 * from the turn-state snapshot with whatever partial text the turn had
 * streamed; without this the partial was recorded and never shown, so a
 * crashed turn looked like it had simply never answered.
 *
 * Styled like the stopped-run element so the two read as siblings. It
 * disappears on its own when a new turn starts (the slice clears the entry).
 */
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppSelector } from '../../../store/hooks';

export function InterruptedTurnNotice() {
  const { t } = useT();
  const interrupted = useAppSelector(state => {
    const threadId = state.thread.selectedThreadId;
    if (!threadId) return null;
    return state.chatRuntime?.interruptedAssistantByThread?.[threadId] ?? null;
  });
  if (!interrupted) return null;
  const partial = interrupted.content.trim();
  return (
    <div
      data-testid="interrupted-turn"
      data-slot="interrupted-run"
      role="status"
      className="flex w-full flex-col gap-2 px-2">
      {partial.length > 0 && (
        <p className="text-foreground/80 text-[13.5px] leading-relaxed whitespace-pre-wrap">
          {partial}
        </p>
      )}
      <div className="flex items-center gap-2">
        <span className="bg-foreground/[0.04] text-foreground/55 inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 font-mono text-[11px]">
          <span aria-hidden className="size-1.5 rounded-full bg-amber-500" />
          {t('chat.message.interrupted')}
        </span>
        <span className="text-foreground/45 text-xs">{t('chat.message.interruptedDetail')}</span>
      </div>
    </div>
  );
}

export default InterruptedTurnNotice;
