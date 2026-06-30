import type { ReactNode } from 'react';

export interface ChartTooltipRow {
  label: string;
  value: string;
  /** CSS colour for the legend swatch. */
  color?: string;
}

export interface ChartTooltipProps {
  title: string;
  rows: ChartTooltipRow[];
  footer?: ReactNode;
}

/**
 * Shared dark-mode-aware tooltip body for the recharts panels.
 * Recharts' default tooltip is a white box that looks broken on the
 * dashboard's dark background — this component replaces it with a card
 * styled to match the rest of the panel.
 */
const ChartTooltip = ({ title, rows, footer }: ChartTooltipProps) => (
  <div
    role="tooltip"
    data-testid="chart-tooltip"
    className="rounded-lg border border-line bg-surface/95 backdrop-blur-sm shadow-soft px-3 py-2 text-xs text-content">
    <div className="font-medium mb-1 text-content-secondary">{title}</div>
    <ul className="space-y-0.5">
      {rows.map(row => (
        <li key={row.label} className="flex items-center gap-2">
          {row.color && (
            <span
              aria-hidden
              className="inline-block h-2 w-2 rounded-full"
              style={{ backgroundColor: row.color }}
            />
          )}
          <span className="text-content-muted">{row.label}</span>
          <span className="ml-auto tabular-nums font-medium">{row.value}</span>
        </li>
      ))}
    </ul>
    {footer && (
      <div className="mt-1 pt-1 border-t border-line/60 dark:border-line text-[10px] text-content-muted">
        {footer}
      </div>
    )}
  </div>
);

export default ChartTooltip;
