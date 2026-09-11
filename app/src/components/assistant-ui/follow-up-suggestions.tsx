'use client';

import { AuiIf, ThreadPrimitive, useAuiState } from '@assistant-ui/react';
import { type FC, useCallback, useEffect, useRef, useState } from 'react';

const FollowupSuggestionsRow: FC = () => {
  const suggestions = useAuiState(s => s.thread.suggestions);
  const scrollRef = useRef<HTMLDivElement>(null);
  const rtlRef = useRef<boolean | null>(null);
  const [fades, setFades] = useState({ left: false, right: false });

  const updateFades = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const maxScroll = el.scrollWidth - el.clientWidth;
    // scrollLeft runs 0..-max in RTL; normalize to hidden width per physical edge.
    const fromStart = Math.abs(el.scrollLeft);
    // getComputedStyle forces a style recalc per scroll event; direction is stable, read it once.
    const rtl = (rtlRef.current ??= window.getComputedStyle(el).direction === 'rtl');
    const [left, right] = rtl
      ? [maxScroll - fromStart, fromStart]
      : [fromStart, maxScroll - fromStart];
    setFades(prev => {
      const next = { left: left > 1, right: right > 1 };
      return prev.left === next.left && prev.right === next.right ? prev : next;
    });
  }, []);

  useEffect(() => {
    updateFades();
    const el = scrollRef.current;
    if (!el?.firstElementChild) return undefined;
    const observer = new ResizeObserver(updateFades);
    observer.observe(el);
    observer.observe(el.firstElementChild);
    return () => observer.disconnect();
  }, [updateFades]);

  const maskImage = `linear-gradient(to right, ${
    fades.left ? 'transparent, black 2rem' : 'black'
  }, ${fades.right ? 'black calc(100% - 2rem), transparent' : 'black'})`;

  return (
    <div
      ref={scrollRef}
      onScroll={updateFades}
      // overflow-x clips both axes; py-1/-my-1 gives focus rings vertical room without changing outer height.
      className="aui-thread-followup-suggestions -my-1 w-full overflow-x-auto py-1 [-ms-overflow-style:none] scrollbar-none [&::-webkit-scrollbar]:hidden"
      style={{ maskImage, WebkitMaskImage: maskImage }}>
      <div className="mx-auto flex min-h-8 w-max items-center gap-2 px-0.5">
        {suggestions.map((suggestion, idx) => (
          <ThreadPrimitive.Suggestion
            key={idx}
            className="aui-thread-followup-suggestion bg-background hover:bg-muted/80 rounded-full border px-3 py-1 text-sm whitespace-nowrap transition-colors ease-in"
            prompt={suggestion.prompt}
            method="replace"
            autoSend>
            {suggestion.title ?? suggestion.prompt}
            {suggestion.label && (
              <span className="aui-thread-followup-suggestion-label text-muted-foreground ms-1">
                {suggestion.label}
              </span>
            )}
          </ThreadPrimitive.Suggestion>
        ))}
      </div>
    </div>
  );
};

export const ThreadFollowupSuggestions: FC = () => (
  <AuiIf
    condition={s => !s.thread.isEmpty && !s.thread.isRunning && s.thread.suggestions.length > 0}>
    <FollowupSuggestionsRow />
  </AuiIf>
);
