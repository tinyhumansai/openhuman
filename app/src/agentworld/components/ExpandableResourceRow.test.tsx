import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, test, vi } from 'vitest';

import ExpandableResourceRow from './ExpandableResourceRow';

describe('ExpandableResourceRow', () => {
  test('renders a collapsed disclosure button without details', () => {
    render(
      <ExpandableResourceRow
        id="resource-one"
        expanded={false}
        onToggle={vi.fn()}
        summary={<span>Resource one</span>}>
        <p>Resource details</p>
      </ExpandableResourceRow>
    );

    const toggle = screen.getByRole('button', { name: 'Resource one' });
    expect(toggle).toHaveAttribute('id', 'resource-one-toggle');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(toggle).toHaveAttribute('aria-controls', 'resource-one-details');
    expect(screen.queryByRole('region')).not.toBeInTheDocument();
    expect(screen.queryByText('Resource details')).not.toBeInTheDocument();
  });

  test('renders labelled details and applies expanded composition classes', () => {
    const { container } = render(
      <ExpandableResourceRow
        id="resource-two"
        expanded
        onToggle={vi.fn()}
        summary={<span>Resource two</span>}
        className="resource-row"
        expandedClassName="resource-row-expanded"
        summaryClassName="resource-summary"
        detailClassName="resource-detail">
        <p>Resource details</p>
      </ExpandableResourceRow>
    );

    const toggle = screen.getByRole('button', { name: 'Resource two' });
    const details = screen.getByRole('region');
    expect(container.firstChild).toHaveClass('resource-row', 'resource-row-expanded');
    expect(toggle).toHaveClass('resource-summary');
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(details).toHaveAttribute('id', 'resource-two-details');
    expect(details).toHaveAttribute('aria-labelledby', 'resource-two-toggle');
    expect(details).toHaveClass('resource-detail');
    expect(details).toHaveTextContent('Resource details');
  });

  test('owns the chevron and delegates toggling', async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();
    render(
      <ExpandableResourceRow
        id="resource-three"
        expanded
        onToggle={onToggle}
        summary={<span>Resource three</span>}>
        <p>Resource details</p>
      </ExpandableResourceRow>
    );

    const toggle = screen.getByRole('button', { name: 'Resource three' });
    const chevron = toggle.querySelector('svg');
    expect(chevron).toHaveAttribute('aria-hidden', 'true');
    expect(chevron).toHaveClass('mt-0.5');
    expect(chevron).toHaveClass('rotate-180');

    await user.click(toggle);
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  test('stacks optional trailing content above its chevron', () => {
    render(
      <ExpandableResourceRow
        id="resource-four"
        expanded={false}
        onToggle={vi.fn()}
        summary={<span>Resource four</span>}
        trailingContent={<span>Updated recently</span>}>
        <p>Resource details</p>
      </ExpandableResourceRow>
    );

    const toggle = screen.getByRole('button', { name: 'Resource four Updated recently' });
    const trailingContent = screen.getByText('Updated recently');
    expect(trailingContent.parentElement).toHaveClass(
      'flex',
      'shrink-0',
      'flex-col',
      'items-end',
      'gap-2'
    );
    const chevron = toggle.querySelector('svg');
    expect(trailingContent.nextElementSibling).toBe(chevron);
    expect(chevron).not.toHaveClass('mt-0.5');
  });
});
