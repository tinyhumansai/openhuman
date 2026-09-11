import type { ReactNode } from 'react';

import { cn } from '../../lib/cn';
import { type ContentWidth, contentWidthVariants } from './contentWidth';

/**
 * PageSectionHeader — the canonical header for a functional page view: a title
 * (16px semibold) over an optional one-line description (14px muted), with an
 * optional right-aligned action, wrapped in a **card** (rounded border, surface
 * background, soft shadow) so it sits flush with the rest of the app's cards.
 *
 * Render it as the first element inside a page's content column so it inherits
 * the same max-width and centering as the content beneath it — header and body
 * stay aligned. Use `width` for the named scale (`sm` / `md` / `lg` / `full`,
 * see `contentWidth.ts`); `className` remains for one-off overrides and still
 * merges (via `cn`, last-wins) on top of it.
 */
interface PageSectionHeaderProps {
  title: ReactNode;
  /** One-line description of what the view does. */
  description?: ReactNode;
  /** Right-aligned action(s) (e.g. buttons). */
  action?: ReactNode;
  /** Optional chip/tab row rendered inside the card, below the title row. */
  tabs?: ReactNode;
  /**
   * Cap the header's width and center it (`mx-auto`). Defaults to `'full'` —
   * today's behavior, where the caller supplies width/centering via
   * `className` (still supported; see `Notifications.tsx`'s
   * `mx-auto max-w-3xl`, equivalent to `width="lg"`).
   */
  width?: ContentWidth;
  /** Extra classes on the card (merged via `cn`, last-wins over `width`). */
  className?: string;
  testId?: string;
}

export default function PageSectionHeader({
  title,
  description,
  action,
  tabs,
  width = 'full',
  className = '',
  testId,
}: PageSectionHeaderProps) {
  return (
    <header
      data-testid={testId}
      className={cn(
        'rounded-2xl border border-line bg-surface px-4 py-3 shadow-subtle',
        width !== 'full' && ['mx-auto w-full', contentWidthVariants({ width })],
        className
      )}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-base font-semibold text-content">{title}</h1>
          {description != null && (
            <p className="mt-0.5 text-sm text-content-muted">{description}</p>
          )}
        </div>
        {action != null && <div className="shrink-0">{action}</div>}
      </div>
      {tabs != null && <div className="mt-3">{tabs}</div>}
    </header>
  );
}
