import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import WelcomeStep from '../WelcomeStep';

describe('WelcomeStep', () => {
  it('renders honest eyebrow + confident display title + subtitle', () => {
    renderWithProviders(<WelcomeStep onNext={() => {}} />);
    expect(screen.getByText(/OPENHUMAN · LOCAL BY DEFAULT/)).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { level: 1, name: /Hi\. I'm OpenHuman\./ })
    ).toBeInTheDocument();
    expect(
      screen.getByText(/routes to the cloud when you pick a cloud model/i)
    ).toBeInTheDocument();
  });

  it('exposes a "What leaves my computer?" link', () => {
    renderWithProviders(<WelcomeStep onNext={() => {}} />);
    expect(screen.getByRole('button', { name: 'What leaves my computer?' })).toBeInTheDocument();
  });

  it('fires onNext when the CTA is clicked', () => {
    const onNext = vi.fn();
    renderWithProviders(<WelcomeStep onNext={onNext} />);
    fireEvent.click(screen.getByRole('button', { name: "Let's Start" }));
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it('CTA is always enabled (WelcomeStep has no disabled/loading props)', () => {
    renderWithProviders(<WelcomeStep onNext={() => {}} />);
    expect(screen.getByRole('button', { name: "Let's Start" })).not.toBeDisabled();
  });
});
