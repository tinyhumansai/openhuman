import type { ReactNode } from 'react';

/**
 * PageSectionHeader — the canonical header for a functional page view: a title
 * (16px semibold) over an optional one-line description (14px muted), with an
 * optional right-aligned action, wrapped in a **card** (rounded border, surface
 * background, soft shadow) so it sits flush with the rest of the app's cards.
 *
 * Render it as the first element inside a page's content column so it inherits
 * the same max-width and centering as the content beneath it — header and body
 * stay aligned. Pass width/centering via `className` (e.g. `mx-auto max-w-2xl`).
 */
export interface PageSectionHeaderProps {
  title: ReactNode;
  /** One-line description of what the view does. */
  description?: ReactNode;
  /** Right-aligned action(s) (e.g. buttons). */
  action?: ReactNode;
  /** Optional chip/tab row rendered inside the card, below the title row. */
  tabs?: ReactNode;
  /** Width / positioning classes (the card chrome is applied internally). */
  className?: string;
  testId?: string;
}

export default function PageSectionHeader({
  title,
  description,
  action,
  tabs,
  className = '',
  testId,
}: PageSectionHeaderProps) {
  return (
    <header
      data-testid={testId}
      className={`rounded-2xl border border-line bg-surface px-4 py-3 shadow-subtle ${className}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-base font-semibold text-content">{title}</h1>
          {description != null && (
            <p className="mt-0.5 text-sm text-content-muted">{description}</p>
          )}
        </div>
        {action != null && <div className="flex-shrink-0">{action}</div>}
      </div>
      {tabs != null && <div className="mt-3">{tabs}</div>}
    </header>
  );
}
