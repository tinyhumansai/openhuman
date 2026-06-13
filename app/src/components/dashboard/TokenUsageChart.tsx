import { Bar, BarChart, Legend, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';

import type { CostDashboardDay } from '../../hooks/useCostDashboard';
import { useT } from '../../lib/i18n/I18nContext';
import ChartTooltip from './ChartTooltip';
import { dayOfMonth, formatTokens, longDateLabel, shortDayLabel } from './formatCurrency';

export interface TokenUsageChartProps {
  days: CostDashboardDay[];
}

const INPUT_FILL = '#4A83DD';
const OUTPUT_FILL = '#7BB48E';

interface TokenPoint {
  date: string;
  label: string;
  dayNumber: string;
  input: number;
  output: number;
  total: number;
}

const TokenUsageChart = ({ days }: TokenUsageChartProps) => {
  const { t } = useT();
  const data: TokenPoint[] = days.map(d => ({
    date: d.date,
    label: shortDayLabel(d.date),
    dayNumber: dayOfMonth(d.date),
    input: d.input_tokens,
    output: d.output_tokens,
    total: d.total_tokens,
  }));

  return (
    <div data-testid="token-usage-chart" className="w-full h-56">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
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
            tickFormatter={(v: number) => formatTokens(v)}
          />
          <Tooltip
            cursor={{ fill: 'rgba(150,150,150,0.10)' }}
            content={props => {
              const item = props.payload?.[0]?.payload as TokenPoint | undefined;
              if (!item) return null;
              return (
                <ChartTooltip
                  title={longDateLabel(item.date)}
                  rows={[
                    {
                      label: t('settings.costDashboard.inputTokens'),
                      value: formatTokens(item.input),
                      color: INPUT_FILL,
                    },
                    {
                      label: t('settings.costDashboard.outputTokens'),
                      value: formatTokens(item.output),
                      color: OUTPUT_FILL,
                    },
                    { label: t('settings.costDashboard.tokens'), value: formatTokens(item.total) },
                  ]}
                />
              );
            }}
          />
          <Legend
            formatter={value =>
              String(value) === 'input'
                ? t('settings.costDashboard.inputTokens')
                : t('settings.costDashboard.outputTokens')
            }
            wrapperStyle={{ fontSize: '11px' }}
            iconType="circle"
          />
          <Bar
            dataKey="input"
            stackId="tokens"
            fill={INPUT_FILL}
            radius={[0, 0, 0, 0]}
            isAnimationActive={false}
            maxBarSize={56}
          />
          <Bar
            dataKey="output"
            stackId="tokens"
            fill={OUTPUT_FILL}
            radius={[6, 6, 0, 0]}
            isAnimationActive={false}
            maxBarSize={56}
          />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
};

export default TokenUsageChart;
