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
    className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white/95 dark:bg-neutral-900/95 backdrop-blur-sm shadow-soft px-3 py-2 text-xs text-stone-800 dark:text-neutral-100">
    <div className="font-medium mb-1 text-stone-700 dark:text-neutral-200">{title}</div>
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
          <span className="text-stone-500 dark:text-neutral-400">{row.label}</span>
          <span className="ml-auto tabular-nums font-medium">{row.value}</span>
        </li>
      ))}
    </ul>
    {footer && (
      <div className="mt-1 pt-1 border-t border-stone-200/60 dark:border-neutral-800 text-[10px] text-stone-500 dark:text-neutral-400">
        {footer}
      </div>
    )}
  </div>
);

export default ChartTooltip;
