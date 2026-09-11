/**
 * Sources — adapted from `sources.tsx` in
 * https://github.com/vercel/ai-elements (Apache License 2.0).
 *
 * Changes made in this port:
 * - `@/registry/default/ui/collapsible` -> OpenHuman's Collapsible primitives
 *   from `../ui`; `@/lib/utils` cn -> `../../lib/cn`.
 * - lucide `BookIcon` / `ChevronDownIcon` -> inline SVGs in `./icons`
 *   (`lucide-react` is not a dependency here).
 * - shadcn semantic colours -> OpenHuman design tokens, and the
 *   `tailwindcss-animate` enter/exit utilities dropped in favour of the
 *   `animate-fade-in` already baked into `CollapsibleContent`.
 * - `SourcesProps` is typed off `CollapsibleRoot` rather than `'div'` so
 *   `open`/`defaultOpen` type-check; the component names are upstream's.
 * - user-facing strings go through `useT()`.
 */
import type { ComponentProps } from 'react';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { CollapsibleContent, CollapsibleRoot, CollapsibleTrigger } from '../ui';
import { BookIcon, ChevronDownIcon } from './icons';

export type SourcesProps = ComponentProps<typeof CollapsibleRoot>;

export const Sources = ({ className, ...props }: SourcesProps) => (
  <CollapsibleRoot
    data-slot="sources"
    className={cn('not-prose mb-4 text-xs text-primary-500', className)}
    {...props}
  />
);

export type SourcesTriggerProps = ComponentProps<typeof CollapsibleTrigger> & { count: number };

export const SourcesTrigger = ({ className, count, children, ...props }: SourcesTriggerProps) => {
  const { t } = useT();

  return (
    <CollapsibleTrigger
      data-slot="sources-trigger"
      className={cn(
        'flex w-full items-center justify-start gap-2 px-0 py-0 text-xs font-normal',
        'text-primary-500 hover:bg-transparent',
        className
      )}
      {...props}>
      {children ?? (
        <>
          <p className="font-medium">
            {t('chat.sources.usedCount', 'Used {n} sources').replace('{n}', String(count))}
          </p>
          <ChevronDownIcon className="h-4 w-4" />
        </>
      )}
    </CollapsibleTrigger>
  );
};

export type SourcesContentProps = ComponentProps<typeof CollapsibleContent>;

export const SourcesContent = ({ className, ...props }: SourcesContentProps) => (
  <CollapsibleContent
    data-slot="sources-content"
    className={cn('mt-3 flex w-fit flex-col gap-2 px-0 pb-0 text-xs outline-hidden', className)}
    {...props}
  />
);

export type SourceProps = ComponentProps<'a'>;

export const Source = ({ href, title, children, className, ...props }: SourceProps) => (
  <a
    data-slot="source"
    className={cn('flex items-center gap-2 text-primary-500', className)}
    href={href}
    rel="noreferrer"
    target="_blank"
    title={title}
    {...props}>
    {children ?? (
      <>
        <BookIcon className="h-4 w-4" />
        <span className="block font-medium">{title}</span>
      </>
    )}
  </a>
);
