/**
 * Small presentational building blocks shared by the Orchestration sub-pages
 * (Connections / Discover / Usage). Pure, no data access.
 */
import type { ReactNode } from 'react';

/** A titled card surface used to group a panel section. */
export function SectionCard({
  title,
  description,
  action,
  children,
  testId,
}: {
  title?: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  testId?: string;
}) {
  return (
    <section
      className="rounded-xl border border-line bg-surface p-4 shadow-soft"
      data-testid={testId}>
      {(title || action) && (
        <div className="mb-3 flex items-start justify-between gap-3">
          <div className="min-w-0">
            {title && <h3 className="text-sm font-semibold text-content">{title}</h3>}
            {description && <p className="mt-0.5 text-xs text-content-muted">{description}</p>}
          </div>
          {action}
        </div>
      )}
      {children}
    </section>
  );
}

/** A single metric tile: a big value over a muted label, with an optional hint. */
export function StatTile({
  label,
  value,
  hint,
  testId,
}: {
  label: ReactNode;
  value: ReactNode;
  hint?: ReactNode;
  testId?: string;
}) {
  return (
    <div className="rounded-xl border border-line bg-surface p-4 shadow-soft" data-testid={testId}>
      <p className="text-xs font-medium uppercase tracking-wide text-content-muted">{label}</p>
      <p className="mt-1 text-2xl font-semibold tabular-nums text-content">{value}</p>
      {hint && <p className="mt-1 text-[11px] text-content-faint">{hint}</p>}
    </div>
  );
}
