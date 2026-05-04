import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { BILLING_DASHBOARD_URL, DISCORD_INVITE_URL } from '../../../utils/links';
import { openUrl } from '../../../utils/openUrl';
import {
  DiscordBanner,
  EarlyBirdyBanner,
  PromotionalCreditsBanner,
  UsageLimitBanner,
} from '../HomeBanners';

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

describe('HomeBanners', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('opens the billing dashboard through openUrl from the usage limit banner', () => {
    render(
      <UsageLimitBanner
        tone="warning"
        icon="⏳"
        title="Limit"
        message="Usage is capped."
        ctaLabel="Buy top-up credits"
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Buy top-up credits' }));

    expect(openUrl).toHaveBeenCalledWith('https://tinyhumans.ai/dashboard');
  });

  it('renders danger tone styles for UsageLimitBanner', () => {
    render(
      <UsageLimitBanner
        tone="danger"
        icon="⚠️"
        title="Out of Usage"
        message="You are out of budget."
        ctaLabel="Get a subscription"
      />
    );
    expect(screen.getByText('Out of Usage')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Get a subscription' }));
    expect(openUrl).toHaveBeenCalledWith(BILLING_DASHBOARD_URL);
  });

  it('opens the billing dashboard through openUrl from the promotional credits banner', () => {
    render(<PromotionalCreditsBanner promoCredits={12} />);

    fireEvent.click(screen.getByRole('button', { name: 'get a subscription' }));

    expect(openUrl).toHaveBeenCalledWith('https://tinyhumans.ai/dashboard');
  });

  it('opens the Discord invite through openUrl from the Discord banner', () => {
    render(<DiscordBanner />);

    fireEvent.click(screen.getByRole('button', { name: /join our discord/i }));

    expect(openUrl).toHaveBeenCalledWith(DISCORD_INVITE_URL);
  });

  describe('EarlyBirdyBanner', () => {
    it('renders the discount code and headline', () => {
      render(<EarlyBirdyBanner />);

      expect(screen.getByText('The first 1,000 users get 60% off.')).toBeInTheDocument();
      expect(screen.getByText('EARLYBIRDY')).toBeInTheDocument();
    });

    it('opens the billing dashboard when the subscription link is clicked', () => {
      render(<EarlyBirdyBanner />);

      fireEvent.click(screen.getByRole('button', { name: /first subscription/i }));

      expect(openUrl).toHaveBeenCalledWith(BILLING_DASHBOARD_URL);
    });

    it('does not render a dismiss button when onDismiss is not provided', () => {
      render(<EarlyBirdyBanner />);

      expect(
        screen.queryByRole('button', { name: /dismiss early bird banner/i })
      ).not.toBeInTheDocument();
    });

    it('renders an accessible dismiss button when onDismiss is provided', () => {
      const onDismiss = vi.fn();
      render(<EarlyBirdyBanner onDismiss={onDismiss} />);

      expect(
        screen.getByRole('button', { name: /dismiss early bird banner/i })
      ).toBeInTheDocument();
    });

    it('calls onDismiss when the dismiss button is clicked', () => {
      const onDismiss = vi.fn();
      render(<EarlyBirdyBanner onDismiss={onDismiss} />);

      fireEvent.click(screen.getByRole('button', { name: /dismiss early bird banner/i }));

      expect(onDismiss).toHaveBeenCalledOnce();
    });

    it('does not call openUrl when the dismiss button is clicked', () => {
      const onDismiss = vi.fn();
      render(<EarlyBirdyBanner onDismiss={onDismiss} />);

      fireEvent.click(screen.getByRole('button', { name: /dismiss early bird banner/i }));

      expect(openUrl).not.toHaveBeenCalled();
    });
  });
});
