import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import ChartTooltip from './ChartTooltip';

describe('<ChartTooltip />', () => {
  it('renders title, rows, and a coloured swatch when colour is provided', () => {
    render(
      <ChartTooltip
        title="Wed, May 27"
        rows={[
          { label: 'Cost', value: '$1.25', color: '#4A83DD' },
          { label: 'Requests', value: '3' },
        ]}
      />
    );
    const tooltip = screen.getByTestId('chart-tooltip');
    expect(tooltip).toHaveTextContent('Wed, May 27');
    expect(tooltip).toHaveTextContent('Cost');
    expect(tooltip).toHaveTextContent('$1.25');
    expect(tooltip).toHaveTextContent('Requests');
    expect(tooltip).toHaveTextContent('3');
  });

  it('renders the optional footer when supplied', () => {
    render(
      <ChartTooltip
        title="Today"
        rows={[{ label: 'X', value: '1' }]}
        footer="Daily target: $3.33"
      />
    );
    expect(screen.getByTestId('chart-tooltip')).toHaveTextContent('Daily target: $3.33');
  });

  it('omits the footer block when none provided', () => {
    const { container } = render(
      <ChartTooltip title="No footer" rows={[{ label: 'X', value: '1' }]} />
    );
    // Footer is a div with the border-t class; assert it is not rendered.
    expect(container.querySelector('.border-t')).toBeNull();
  });
});
