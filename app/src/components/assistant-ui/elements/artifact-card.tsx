'use client';

/**
 * assistant-ui's artifact-card element: a compact row for a generated file
 * (document, presentation, ...) — icon, title, and either a "writing" shimmer
 * with a live word count, or the settled metadata line once it's done.
 *
 * Vendored from the assistant-ui `elements-artifact-card` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-artifact-card.json).
 * Changes from upstream:
 * - `cn` import path and `./surfaces` resolved through this app's alias.
 * - `writingLabel` prop (English default supplied by the caller via
 *   `useT()`) replaces the hardcoded "Writing" string.
 * - `icon` prop (defaults to `FileTextIcon`) so a presentation, image, or
 *   other artifact kind can swap the glyph instead of always showing a
 *   document icon.
 * - `onOpen` replaces upstream's implicit "the whole card is a link" with an
 *   explicit handler; the card renders as a `<button>` when present.
 */
import { mono, paper, ShimmerLabel } from '@/components/assistant-ui/elements/surfaces';
import { cn } from '@/components/assistant-ui/lib/utils';
import { useT } from '@/lib/i18n/I18nContext';
import { ArrowUpRightIcon, FileTextIcon } from 'lucide-react';
import type { ComponentProps, ElementType } from 'react';

export interface ArtifactCardProps extends Omit<
  ComponentProps<'div'>,
  'children' | 'title' | 'meta' | 'generating' | 'words'
> {
  title: string;
  meta: string;
  generating?: boolean;
  words?: number;
  writingLabel?: string;
  icon?: ElementType;
  onOpen?: () => void;
}

export function ArtifactCard({
  title,
  meta,
  generating = false,
  words = 0,
  writingLabel = 'Writing',
  icon: Icon = FileTextIcon,
  onOpen,
  className,
  ...props
}: ArtifactCardProps) {
  const cardClassName = cn(
    paper,
    'group flex w-full cursor-pointer items-center gap-3 rounded-[20px] p-3.5 text-start transition-transform duration-150 hover:-translate-y-px active:scale-[0.98]',
    className
  );

  if (onOpen) {
    return (
      <button
        data-slot="artifact-card"
        type="button"
        onClick={onOpen}
        className={cardClassName}
        {...(props as ComponentProps<'button'>)}>
        <ArtifactCardBody
          title={title}
          meta={meta}
          generating={generating}
          words={words}
          writingLabel={writingLabel}
          Icon={Icon}
        />
      </button>
    );
  }

  return (
    <div data-slot="artifact-card" className={cardClassName} {...props}>
      <ArtifactCardBody
        title={title}
        meta={meta}
        generating={generating}
        words={words}
        writingLabel={writingLabel}
        Icon={Icon}
      />
    </div>
  );
}

function ArtifactCardBody({
  title,
  meta,
  generating,
  words,
  writingLabel,
  Icon,
}: {
  title: string;
  meta: string;
  generating: boolean;
  words: number;
  writingLabel: string;
  Icon: ElementType;
}) {
  const { t } = useT();
  const wordsText = t(
    words === 1 ? 'elements.artifactCard.wordsOne' : 'elements.artifactCard.wordsOther',
    words === 1 ? '{count} word' : '{count} words'
  ).replace('{count}', String(words));
  return (
    <>
      <span className="bg-foreground/[0.05] text-foreground/45 flex size-9 shrink-0 items-center justify-center rounded-xl">
        <Icon className={cn('size-4', generating && 'animate-pulse motion-reduce:animate-none')} />
      </span>
      <div className="min-w-0 flex-1">
        <p className="truncate text-[13.5px] font-medium">{title}</p>
        {generating ? (
          <p className={cn(mono, 'text-foreground/40 flex items-center gap-1')}>
            <ShimmerLabel className="relative inline-block leading-none">
              {writingLabel}
            </ShimmerLabel>
            <span>·</span>
            <span className="tabular-nums">{wordsText}</span>
          </p>
        ) : (
          <p
            className={cn(
              mono,
              'fade-in blur-in-[2px] animate-in text-foreground/40 duration-300 motion-reduce:animate-none'
            )}>
            {meta}
          </p>
        )}
      </div>
      <ArrowUpRightIcon className="text-foreground/35 size-3.5 opacity-0 transition-opacity group-hover:opacity-100" />
    </>
  );
}
