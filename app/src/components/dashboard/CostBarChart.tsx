import {
  Bar,
  BarChart,
  Cell,
  LabelList,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

import type { CostDashboardDay } from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import { intlLocale } from '../../lib/i18n/intlLocale';
import ChartTooltip from './ChartTooltip';
import { dayOfMonth, formatCurrency, longDateLabel, shortDayLabel } from './formatCurrency';

interface CostBarChartProps {
  days: CostDashboardDay[];
  currency: string;
}

const NORMAL_FILL = '#4A83DD';

interface ChartPoint {
  date: string;
  label: string;
  dayNumber: string;
  cost: number;
  requestCount: number;
  isToday: boolean;
}

const CostBarChart = ({ days, currency }: CostBarChartProps) => {
  const { t, locale } = useT();
  const numberLocale = intlLocale(locale);
  const todayDate = days.length > 0 ? days[days.length - 1].date : null;

  const chartData: ChartPoint[] = days.map(day => ({
    date: day.date,
    label: shortDayLabel(day.date, numberLocale),
    dayNumber: dayOfMonth(day.date, numberLocale),
    cost: day.cost_usd || 0,
    requestCount: day.request_count,
    isToday: day.date === todayDate,
  }));

  const hasSpend = chartData.some(point => point.cost > 0);

  return (
    <div data-testid="cost-bar-chart" className="w-full">
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
              height={24}
            />
            <YAxis
              stroke="currentColor"
              fontSize={11}
              tickLine={false}
              axisLine={false}
              width={52}
              tick={{ fill: 'currentColor', opacity: 0.7 }}
              domain={[0, hasSpend ? 'auto' : 1]}
              tickFormatter={(v: number) =>
                formatCurrency(v, currency, { precise: true, locale: numberLocale })
              }
            />
            <Tooltip
              cursor={{ fill: 'rgba(150,150,150,0.10)' }}
              content={props => {
                const item = props.payload?.[0]?.payload as ChartPoint | undefined;
                if (!item) return null;
                return (
                  <ChartTooltip
                    title={longDateLabel(item.date, numberLocale)}
                    rows={[
                      {
                        label: t('settings.costDashboard.cost'),
                        value: formatCurrency(item.cost, currency, { locale: numberLocale }),
                        color: NORMAL_FILL,
                      },
                      {
                        label: t('settings.costDashboard.requests'),
                        value: String(item.requestCount),
                      },
                    ]}
                  />
                );
              }}
            />
            <Bar dataKey="cost" radius={[6, 6, 0, 0]} isAnimationActive={false} maxBarSize={56}>
              {chartData.map(entry => (
                <Cell
                  key={entry.date}
                  fill={NORMAL_FILL}
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
