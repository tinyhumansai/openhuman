import { useMemo, useState } from 'react';

interface MemoryHeatmapProps {
  /** Array of document/relation timestamps (unix epoch seconds). */
  timestamps: number[];
  loading?: boolean;
}

const MONTHS = 8;
const DAYS_PER_WEEK = 7;
const CELL_GAP = 2;
const DAY_LABELS = ['', 'Mon', '', 'Wed', '', 'Fri', ''];

const INTENSITY_COLORS = [
  'rgba(255,255,255,0.04)', // 0 events
  'rgba(74,131,221,0.25)', // 1
  'rgba(74,131,221,0.45)', // 2-3
  'rgba(74,131,221,0.65)', // 4-6
  'rgba(74,131,221,0.85)', // 7+
];

function getIntensity(count: number): number {
  if (count === 0) return 0;
  if (count === 1) return 1;
  if (count <= 3) return 2;
  if (count <= 6) return 3;
  return 4;
}

function dateToKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

function formatDate(date: Date): string {
  return date.toLocaleDateString('en-US', {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });
}

export function MemoryHeatmap({ timestamps, loading }: MemoryHeatmapProps) {
  const [hoveredCell, setHoveredCell] = useState<{
    date: Date;
    count: number;
    x: number;
    y: number;
  } | null>(null);

  const { grid, monthLabels, totalEvents, maxDailyCount, totalWeeks } = useMemo(() => {
    // The window: 6 months ago through today
    const today = new Date();
    today.setHours(0, 0, 0, 0);

    const rangeStart = new Date(today);
    rangeStart.setMonth(rangeStart.getMonth() - MONTHS);
    rangeStart.setDate(1); // start of that month

    // Align to the Sunday of rangeStart's week
    const startDate = new Date(rangeStart);
    startDate.setDate(startDate.getDate() - startDate.getDay());

    // Count timestamps that fall anywhere (not limited to the 6-month window)
    // — this means ingesting old data still lights up that old date.
    const countMap = new Map<string, number>();
    let total = 0;
    let maxCount = 0;

    for (const ts of timestamps) {
      const date = new Date(ts > 9999999999 ? ts : ts * 1000);
      const key = dateToKey(date);
      const prev = countMap.get(key) ?? 0;
      const next = prev + 1;
      countMap.set(key, next);
      // Only count towards total/max if inside our display range
      if (date >= startDate && date <= today) {
        total++;
        if (next > maxCount) maxCount = next;
      }
    }

    // Build grid
    const cells: { date: Date; count: number; weekIdx: number; dayIdx: number }[] = [];
    const months: { label: string; weekIdx: number }[] = [];
    let lastMonth = -1;
    let weekIdx = 0;

    const cursor = new Date(startDate);
    while (cursor <= today) {
      const d = cursor.getDay(); // 0=Sun ... 6=Sat

      if (d === 0 && cells.length > 0) weekIdx++;

      const cellDate = new Date(cursor);
      const key = dateToKey(cellDate);
      cells.push({ date: cellDate, count: countMap.get(key) ?? 0, weekIdx, dayIdx: d });

      // Track month labels (on the first Sunday-row cell of each new month)
      if (cellDate.getMonth() !== lastMonth && d === 0) {
        lastMonth = cellDate.getMonth();
        months.push({ label: cellDate.toLocaleDateString('en-US', { month: 'short' }), weekIdx });
      }

      cursor.setDate(cursor.getDate() + 1);
    }

    return {
      grid: cells,
      monthLabels: months,
      totalEvents: total,
      maxDailyCount: maxCount,
      totalWeeks: weekIdx + 1,
    };
  }, [timestamps]);

  // Dynamic cell size: fill available width (parent is ~100%).
  // We use a viewBox + 100% width so SVG scales to fit container.
  const DAY_LABEL_WIDTH = 28;
  const cellSize = 11;
  const svgWidth = DAY_LABEL_WIDTH + totalWeeks * (cellSize + CELL_GAP) + CELL_GAP;
  const svgHeight = DAYS_PER_WEEK * (cellSize + CELL_GAP) + 22;

  if (loading) {
    return (
      <div className="rounded-xl border border-stone-200 bg-stone-50 p-5">
        <h3 className="text-sm font-semibold text-stone-900 mb-3">Ingestion Activity</h3>
        <div className="h-28 rounded-lg bg-stone-200 animate-pulse" />
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-stone-200 bg-stone-50 p-5">
      <div className="flex items-center justify-between mb-3">
        <div>
          <h3 className="text-sm font-semibold text-stone-900">Ingestion Activity</h3>
          <p className="text-xs text-stone-500 mt-0.5">
            {totalEvents} event{totalEvents !== 1 ? 's' : ''} over the last {MONTHS} months
            {maxDailyCount > 0 && <> · peak: {maxDailyCount}/day</>}
          </p>
        </div>
        <div className="flex items-center gap-1 text-[10px] text-stone-500">
          <span>Less</span>
          {INTENSITY_COLORS.map((color, i) => (
            <div
              key={i}
              className="w-[10px] h-[10px] rounded-[2px]"
              style={{ backgroundColor: color }}
            />
          ))}
          <span>More</span>
        </div>
      </div>

      <svg
        width="100%"
        viewBox={`0 0 ${svgWidth} ${svgHeight}`}
        preserveAspectRatio="xMinYMin meet"
        className="block">
        {/* Day labels */}
        {DAY_LABELS.map((label, i) =>
          label ? (
            <text
              key={i}
              x={0}
              y={22 + i * (cellSize + CELL_GAP) + cellSize * 0.75}
              fontSize={9}
              fill="rgba(0,0,0,0.4)"
              style={{ userSelect: 'none' }}>
              {label}
            </text>
          ) : null
        )}

        {/* Month labels */}
        {monthLabels.map((m, i) => (
          <text
            key={i}
            x={DAY_LABEL_WIDTH + m.weekIdx * (cellSize + CELL_GAP)}
            y={12}
            fontSize={9}
            fill="rgba(0,0,0,0.4)"
            style={{ userSelect: 'none' }}>
            {m.label}
          </text>
        ))}

        {/* Cells */}
        {grid.map((cell, i) => {
          const x = DAY_LABEL_WIDTH + cell.weekIdx * (cellSize + CELL_GAP);
          const y = 18 + cell.dayIdx * (cellSize + CELL_GAP);
          const intensity = getIntensity(cell.count);

          return (
            <rect
              key={i}
              x={x}
              y={y}
              width={cellSize}
              height={cellSize}
              rx={2}
              fill={INTENSITY_COLORS[intensity]}
              stroke={
                hoveredCell?.date.getTime() === cell.date.getTime()
                  ? 'rgba(255,255,255,0.4)'
                  : 'transparent'
              }
              strokeWidth={1}
              style={{ cursor: 'pointer', transition: 'fill 0.1s' }}
              onMouseEnter={e => {
                const rect = (e.target as SVGRectElement).getBoundingClientRect();
                setHoveredCell({
                  date: cell.date,
                  count: cell.count,
                  x: rect.left + rect.width / 2,
                  y: rect.top,
                });
              }}
              onMouseLeave={() => setHoveredCell(null)}
            />
          );
        })}
      </svg>

      {/* Tooltip */}
      {hoveredCell && (
        <div
          className="fixed z-50 px-2 py-1 rounded-md bg-white border border-stone-200 text-[11px] text-stone-900 shadow-lg pointer-events-none"
          style={{ left: hoveredCell.x, top: hoveredCell.y - 32, transform: 'translateX(-50%)' }}>
          <span className="font-medium">
            {hoveredCell.count} event{hoveredCell.count !== 1 ? 's' : ''}
          </span>{' '}
          <span className="text-stone-400">on {formatDate(hoveredCell.date)}</span>
        </div>
      )}
    </div>
  );
}
