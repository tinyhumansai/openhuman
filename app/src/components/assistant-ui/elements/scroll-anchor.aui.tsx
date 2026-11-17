'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { Button } from '@/components/assistant-ui/ui/button';
import { ThreadPrimitive } from '@assistant-ui/react';
import { ArrowDownIcon } from 'lucide-react';

/**
 * Runtime scroll-anchor pill. The viewport owns visibility and the jump; the
 * library resumes following when the reader returns to the end.
 * Positioned above the composer so expanding its cards moves the pill with it.
 */
export function ScrollAnchor({ label, className }: { label: string; className?: string }) {
  return (
    <ThreadPrimitive.ScrollToBottom asChild behavior="instant">
      <Button
        data-slot="scroll-anchor"
        data-analytics-id="chat-scroll-to-bottom"
        variant="outline"
        size="sm"
        className={cn(
          'aui-thread-scroll-to-bottom bg-background dark:bg-background shadow-soft absolute -top-11 left-1/2 z-20 h-auto w-fit -translate-x-1/2 gap-1.5 rounded-full px-3.5 py-1.5 text-xs disabled:invisible',
          className
        )}>
        <ArrowDownIcon aria-hidden className="size-3 opacity-60" />
        {label}
      </Button>
    </ThreadPrimitive.ScrollToBottom>
  );
}
