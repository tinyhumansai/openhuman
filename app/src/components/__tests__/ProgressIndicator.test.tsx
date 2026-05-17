import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import ProgressIndicator from '../ProgressIndicator';

describe('ProgressIndicator', () => {
  it('renders the correct number of step dots', () => {
    render(<ProgressIndicator currentStep={0} totalSteps={3} />);
    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(3);
  });

  it('container has role="list" and aria-label', () => {
    render(<ProgressIndicator currentStep={1} totalSteps={4} />);
    const list = screen.getByRole('list');
    expect(list).toBeInTheDocument();
    expect(list).toHaveAttribute('aria-label', 'Progress steps');
  });

  it('each step has an accessible label', () => {
    render(<ProgressIndicator currentStep={0} totalSteps={3} />);
    expect(screen.getByRole('listitem', { name: 'Step 1 of 3' })).toBeInTheDocument();
    expect(screen.getByRole('listitem', { name: 'Step 2 of 3' })).toBeInTheDocument();
    expect(screen.getByRole('listitem', { name: 'Step 3 of 3' })).toBeInTheDocument();
  });

  it('marks the current step with aria-current="step"', () => {
    render(<ProgressIndicator currentStep={1} totalSteps={3} />);
    const currentDot = screen.getByRole('listitem', { name: 'Step 2 of 3' });
    expect(currentDot).toHaveAttribute('aria-current', 'step');
  });

  it('does not set aria-current on non-current steps', () => {
    render(<ProgressIndicator currentStep={1} totalSteps={3} />);
    const step1 = screen.getByRole('listitem', { name: 'Step 1 of 3' });
    const step3 = screen.getByRole('listitem', { name: 'Step 3 of 3' });
    expect(step1).not.toHaveAttribute('aria-current');
    expect(step3).not.toHaveAttribute('aria-current');
  });

  it('does not use tablist/tab roles (non-interactive indicator)', () => {
    render(<ProgressIndicator currentStep={0} totalSteps={3} />);
    expect(screen.queryByRole('tablist')).not.toBeInTheDocument();
    expect(screen.queryByRole('tab')).not.toBeInTheDocument();
  });

  it('does not set aria-selected on any dot', () => {
    render(<ProgressIndicator currentStep={0} totalSteps={3} />);
    screen.getAllByRole('listitem').forEach(dot => {
      expect(dot).not.toHaveAttribute('aria-selected');
    });
  });

  it('applies active style class to the current step', () => {
    render(<ProgressIndicator currentStep={2} totalSteps={3} />);
    const activeDot = screen.getByRole('listitem', { name: 'Step 3 of 3' });
    expect(activeDot).toHaveClass('bg-stone-800');
  });

  it('applies inactive style class to non-current steps', () => {
    render(<ProgressIndicator currentStep={0} totalSteps={3} />);
    const inactiveDot = screen.getByRole('listitem', { name: 'Step 2 of 3' });
    expect(inactiveDot).toHaveClass('bg-stone-300');
  });

  it('renders a single step correctly', () => {
    render(<ProgressIndicator currentStep={0} totalSteps={1} />);
    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(1);
    expect(items[0]).toHaveAttribute('aria-current', 'step');
    expect(items[0]).toHaveAttribute('aria-label', 'Step 1 of 1');
  });
});
