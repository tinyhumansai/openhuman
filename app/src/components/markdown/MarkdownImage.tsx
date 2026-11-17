import type { ComponentPropsWithoutRef } from 'react';

/**
 * Inline images in chat markdown (e.g. meme GIFs added to a reply). Capped in
 * height so a tall template cannot take over the thread, lazy-loaded, and
 * fetched without a referrer so third-party hosts do not learn which chat or
 * page displayed them.
 */
export function MarkdownImage({ className, alt, ...props }: ComponentPropsWithoutRef<'img'>) {
  return (
    <img
      {...props}
      alt={alt ?? ''}
      loading="lazy"
      decoding="async"
      referrerPolicy="no-referrer"
      className={['my-2 block h-auto max-h-72 max-w-full rounded-lg object-contain', className]
        .filter(Boolean)
        .join(' ')}
    />
  );
}
