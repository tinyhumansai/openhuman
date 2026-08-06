import { type ReactNode } from 'react';

export interface ExpandableResourceRowProps {
  id: string;
  expanded: boolean;
  onToggle: () => void;
  summary: ReactNode;
  trailingContent?: ReactNode;
  children: ReactNode;
  className?: string;
  expandedClassName?: string;
  summaryClassName?: string;
  detailClassName?: string;
}

export default function ExpandableResourceRow({
  id,
  expanded,
  onToggle,
  summary,
  trailingContent,
  children,
  className,
  expandedClassName,
  summaryClassName,
  detailClassName,
}: ExpandableResourceRowProps) {
  const toggleId = `${id}-toggle`;
  const detailId = `${id}-details`;
  const rowClassName = [className, expanded ? expandedClassName : undefined]
    .filter(Boolean)
    .join(' ');
  const chevronClassName = [
    trailingContent ? undefined : 'mt-0.5',
    'h-4 w-4 shrink-0 text-content-faint transition-transform',
    expanded ? 'rotate-180' : undefined,
  ]
    .filter(Boolean)
    .join(' ');
  const chevron = (
    <svg
      aria-hidden="true"
      className={chevronClassName}
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
    </svg>
  );

  return (
    <div className={rowClassName || undefined}>
      <button
        id={toggleId}
        type="button"
        aria-expanded={expanded}
        aria-controls={detailId}
        onClick={onToggle}
        className={summaryClassName}>
        {summary}
        {trailingContent ? (
          <div className="flex shrink-0 flex-col items-end gap-2">
            {trailingContent}
            {chevron}
          </div>
        ) : (
          chevron
        )}
      </button>

      {expanded && (
        <div id={detailId} role="region" aria-labelledby={toggleId} className={detailClassName}>
          {children}
        </div>
      )}
    </div>
  );
}
