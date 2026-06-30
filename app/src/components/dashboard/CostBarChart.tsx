import {
  Bar,
  BarChart,
  Cell,
  LabelList,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

import type { CostDashboardDay } from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import ChartTooltip from './ChartTooltip';
import { dayOfMonth, formatCurrency, longDateLabel, shortDayLabel } from './formatCurrency';

export interface CostBarChartProps {
  days: CostDashboardDay[];
  currency: string;
  /** Monthly budget in USD; used to derive a daily target. */
  budgetLimitMonthlyUsd: number;
  /** Fraction of daily target that flips bar colour to amber. */
  warnThreshold: number;
  /** Fraction of daily target that flips bar colour to red. */
  alertThreshold: number;
}

const NORMAL_FILL = '#4A83DD';
const WARN_FILL = '#F5A524';
const ALERT_FILL = '#E5484D';
const TARGET_STROKE = '#94A3B8';

export function colorForCost(
  cost: number,
  dailyTarget: number,
  warnThreshold: number,
  alertThreshold: number
): string {
  if (dailyTarget <= 0) return NORMAL_FILL;
  const ratio = cost / dailyTarget;
  if (ratio >= alertThreshold) return ALERT_FILL;
  if (ratio >= warnThreshold) return WARN_FILL;
  return NORMAL_FILL;
}

interface ChartPoint {
  date: string;
  label: string;
  dayNumber: string;
  cost: number;
  requestCount: number;
  fill: string;
  isToday: boolean;
}

const CostBarChart = ({
  days,
  currency,
  budgetLimitMonthlyUsd,
  warnThreshold,
  alertThreshold,
}: CostBarChartProps) => {
  const { t } = useT();
  const dailyTarget = budgetLimitMonthlyUsd > 0 ? budgetLimitMonthlyUsd / 30 : 0;
  const todayDate = days.length > 0 ? days[days.length - 1].date : null;

  const chartData: ChartPoint[] = days.map(day => ({
    date: day.date,
    label: shortDayLabel(day.date),
    dayNumber: dayOfMonth(day.date),
    cost: Number(day.cost_usd.toFixed(4)),
    requestCount: day.request_count,
    fill: colorForCost(day.cost_usd, dailyTarget, warnThreshold, alertThreshold),
    isToday: day.date === todayDate,
  }));

  return (
    <div data-testid="cost-bar-chart" className="w-full">
      {dailyTarget > 0 && (
        <div className="text-[11px] text-content-muted mb-1 flex items-center gap-1.5">
          <span
            aria-hidden
            className="inline-block h-px w-3 border-t border-dashed border-stone-400 dark:border-neutral-500"
          />
          <span>
            {`${t('settings.costDashboard.dailyTarget')}: ${formatCurrency(dailyTarget, currency)}`}
          </span>
        </div>
      )}
      <div className="w-full h-56">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={chartData} margin={{ top: 16, right: 8, left: 0, bottom: 0 }}>
            <XAxis
              dataKey="label"
              stroke="currentColor"
              fontSize={11}
              tickLine={false}
              axisLine={false}
              tick={{ fill: 'currentColor', opacity: 0.7 }}
            />
            <XAxis
              dataKey="dayNumber"
              xAxisId="day"
              stroke="currentColor"
              fontSize={10}
              tickLine={false}
              axisLine={false}
              tick={{ fill: 'currentColor', opacity: 0.45 }}
              height={14}
            />
            <YAxis
              stroke="currentColor"
              fontSize={11}
              tickLine={false}
              axisLine={false}
              width={52}
              tick={{ fill: 'currentColor', opacity: 0.7 }}
              tickFormatter={(v: number) => formatCurrency(v, currency)}
            />
            <Tooltip
              cursor={{ fill: 'rgba(150,150,150,0.10)' }}
              content={props => {
                const item = props.payload?.[0]?.payload as ChartPoint | undefined;
                if (!item) return null;
                return (
                  <ChartTooltip
                    title={longDateLabel(item.date)}
                    rows={[
                      {
                        label: t('settings.costDashboard.cost'),
                        value: formatCurrency(item.cost, currency),
                        color: item.fill,
                      },
                      {
                        label: t('settings.costDashboard.requests'),
                        value: String(item.requestCount),
                      },
                    ]}
                    footer={
                      dailyTarget > 0
                        ? `${t('settings.costDashboard.dailyTarget')}: ${formatCurrency(dailyTarget, currency)}`
                        : undefined
                    }
                  />
                );
              }}
            />
            {dailyTarget > 0 && (
              <ReferenceLine
                y={dailyTarget}
                stroke={TARGET_STROKE}
                strokeDasharray="4 4"
                strokeWidth={1}
                ifOverflow="extendDomain"
              />
            )}
            <Bar dataKey="cost" radius={[6, 6, 0, 0]} isAnimationActive={false} maxBarSize={56}>
              {chartData.map(entry => (
                <Cell
                  key={entry.date}
                  fill={entry.fill}
                  stroke={entry.isToday ? '#0F172A' : 'transparent'}
                  strokeOpacity={entry.isToday ? 0.15 : 0}
                />
              ))}
              <LabelList
                dataKey="isToday"
                position="top"
                content={({ x, y, width, value }) => {
                  if (!value) return null;
                  const cx = Number(x ?? 0) + Number(width ?? 0) / 2;
                  const cy = Math.max(0, Number(y ?? 0) - 6);
                  return (
                    <g>
                      <rect
                        x={cx - 22}
                        y={cy - 12}
                        width={44}
                        height={14}
                        rx={7}
                        ry={7}
                        fill="#4A83DD"
                        fillOpacity={0.12}
                      />
                      <text
                        x={cx}
                        y={cy - 2}
                        textAnchor="middle"
                        fontSize={9}
                        fontWeight={600}
                        fill="#4A83DD">
                        {t('settings.costDashboard.todayBadge')}
                      </text>
                    </g>
                  );
                }}
              />
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
};

export default CostBarChart;
