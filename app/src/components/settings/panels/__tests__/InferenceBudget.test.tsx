import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { TeamUsage, TeamUsageInsights } from '../../../../services/api/creditsApi';
import InferenceBudget from '../billing/InferenceBudget';

function buildInsights(overrides: Partial<TeamUsageInsights> = {}): TeamUsageInsights {
  return {
    period: { startDate: '2026-05-01', endDate: '2026-05-31' },
    totals: {
      inferenceUsd: 3,
      integrationsUsd: 2,
      totalUsd: 5,
      inferenceCalls: 100,
      integrationCalls: 10,
    },
    dailySeries: [
      { date: '2026-05-01', inferenceUsd: 1, integrationsUsd: 0.5, totalUsd: 1.5 },
      { date: '2026-05-02', inferenceUsd: 2, integrationsUsd: 1.5, totalUsd: 3.5 },
    ],
    topModels: [{ model: 'claude-sonnet', provider: 'anthropic', spentUsd: 3, calls: 100 }],
    topIntegrations: [{ provider: 'gmail', action: 'send', spentUsd: 2, calls: 10 }],
    ...overrides,
  };
}

function buildTeamUsage(overrides: Partial<TeamUsage> = {}): TeamUsage {
  return {
    remainingUsd: 5,
    cycleBudgetUsd: 10,
    cycleSpentUsd: 5,
    cycleStartDate: '2026-05-01T00:00:00.000Z',
    cycleEndsAt: '2026-05-31T00:00:00.000Z',
    plan: {
      plan: 'PRO',
      name: 'Pro',
      marginPercent: 90,
      payAsYouGoMarginPercent: 0,
      discountVsPayAsYouGoPercent: 50,
    },
    insights: buildInsights(),
    ...overrides,
  };
}

describe('InferenceBudget', () => {
  describe('loading state', () => {
    it('shows Loading… text when isLoadingCredits=true and teamUsage=null', () => {
      render(<InferenceBudget teamUsage={null} isLoadingCredits={true} />);
      expect(screen.getByText('Loading…')).toBeInTheDocument();
    });

    it('shows pulse skeleton bar when isLoadingCredits=true and teamUsage=null', () => {
      const { container } = render(<InferenceBudget teamUsage={null} isLoadingCredits={true} />);
      expect(container.querySelector('.animate-pulse')).toBeInTheDocument();
    });
  });

  describe('null teamUsage (not loading)', () => {
    it('shows unable-to-load message', () => {
      render(<InferenceBudget teamUsage={null} isLoadingCredits={false} />);
      expect(screen.getByText(/Unable to load usage data/i)).toBeInTheDocument();
    });
  });

  describe('with cycleBudgetUsd > 0', () => {
    it('renders remaining / budget text in the header', () => {
      const usage = buildTeamUsage({ remainingUsd: 5, cycleBudgetUsd: 10 });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/\$5\.00 \/ \$10\.00 remaining/i)).toBeInTheDocument();
    });

    it('renders cycle-spent and cycle-ends date', () => {
      const usage = buildTeamUsage({ cycleSpentUsd: 5, cycleEndsAt: '2026-05-31T00:00:00.000Z' });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/Spent \$5\.00 this cycle/i)).toBeInTheDocument();
      expect(screen.getByText(/Cycle ends/i)).toBeInTheDocument();
    });

    it('renders formatCycleEnds as n/a for an invalid date string', () => {
      const usage = buildTeamUsage({ cycleEndsAt: 'not-a-valid-date' });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/Cycle ends n\/a/i)).toBeInTheDocument();
    });

    it('shows coral exhausted warning when remainingUsd <= 0', () => {
      const usage = buildTeamUsage({ remainingUsd: 0, cycleSpentUsd: 10 });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/Included subscription usage is exhausted/i)).toBeInTheDocument();
    });
  });

  describe('with cycleBudgetUsd === 0', () => {
    it('renders "No recurring plan budget" in the header', () => {
      const usage = buildTeamUsage({ cycleBudgetUsd: 0, remainingUsd: 0 });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/No recurring plan budget/i)).toBeInTheDocument();
    });

    it('renders the pay-as-you-go notice card', () => {
      const usage = buildTeamUsage({ cycleBudgetUsd: 0 });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(
        screen.getByText(/does not include a recurring weekly inference budget/i)
      ).toBeInTheDocument();
    });
  });

  describe('plan discount banner', () => {
    it('shows discount text when discountVsPayAsYouGoPercent > 0', () => {
      const usage = buildTeamUsage();
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/50% cheaper per call/i)).toBeInTheDocument();
      expect(screen.getByText(/Pro:/i)).toBeInTheDocument();
    });

    it('does not render discount banner when discountVsPayAsYouGoPercent === 0', () => {
      const usage = buildTeamUsage({
        plan: {
          plan: 'FREE',
          name: 'Free',
          marginPercent: 0,
          payAsYouGoMarginPercent: 0,
          discountVsPayAsYouGoPercent: 0,
        },
      });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.queryByText(/cheaper per call/i)).not.toBeInTheDocument();
    });
  });

  describe('UsageBreakdown', () => {
    it('renders inference and integrations totals', () => {
      const usage = buildTeamUsage();
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/Cycle spend/i)).toBeInTheDocument();
      // "Inference" and "Integrations" each appear once in UsageBreakdown and once in the DailyChart legend
      expect(screen.getAllByText('Inference').length).toBeGreaterThanOrEqual(1);
      expect(screen.getAllByText('Integrations').length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText(/\$5\.00 total/i)).toBeInTheDocument();
    });

    it('renders call counts formatted with toLocaleString', () => {
      const usage = buildTeamUsage({
        insights: buildInsights({
          totals: {
            inferenceUsd: 3,
            integrationsUsd: 2,
            totalUsd: 5,
            inferenceCalls: 1000,
            integrationCalls: 200,
          },
        }),
      });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/1,000 calls/i)).toBeInTheDocument();
    });
  });

  describe('DailyChart', () => {
    it('renders daily spend section when points are present', () => {
      render(<InferenceBudget teamUsage={buildTeamUsage()} isLoadingCredits={false} />);
      expect(screen.getByText(/Daily spend/i)).toBeInTheDocument();
    });

    it('renders legend items for inference and integrations (2 instances each with chart)', () => {
      render(<InferenceBudget teamUsage={buildTeamUsage()} isLoadingCredits={false} />);
      // DailyChart renders legend labels + UsageBreakdown renders column headers = 2 each
      expect(screen.getAllByText('Inference').length).toBeGreaterThanOrEqual(2);
      expect(screen.getAllByText('Integrations').length).toBeGreaterThanOrEqual(2);
    });

    it('does not render chart section when dailySeries is empty', () => {
      const usage = buildTeamUsage({ insights: buildInsights({ dailySeries: [] }) });
      const { container } = render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      // The bar container is only rendered when there are points
      expect(container.querySelector('.h-16')).not.toBeInTheDocument();
    });
  });

  describe('TopModels', () => {
    it('renders top models heading', () => {
      render(<InferenceBudget teamUsage={buildTeamUsage()} isLoadingCredits={false} />);
      expect(screen.getByText('Top models')).toBeInTheDocument();
    });

    it('renders model name and spend', () => {
      render(<InferenceBudget teamUsage={buildTeamUsage()} isLoadingCredits={false} />);
      expect(screen.getByText('claude-sonnet')).toBeInTheDocument();
      expect(screen.getByText(/\$3\.00 · 100/i)).toBeInTheDocument();
    });

    it('falls back to provider name when model is empty string', () => {
      const usage = buildTeamUsage({
        insights: buildInsights({
          topModels: [{ model: '', provider: 'anthropic', spentUsd: 1, calls: 5 }],
        }),
      });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText('anthropic')).toBeInTheDocument();
    });

    it('shows empty-state message when topModels is empty', () => {
      const usage = buildTeamUsage({ insights: buildInsights({ topModels: [] }) });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/No inference usage this cycle/i)).toBeInTheDocument();
    });
  });

  describe('TopIntegrations', () => {
    it('renders top integrations heading', () => {
      render(<InferenceBudget teamUsage={buildTeamUsage()} isLoadingCredits={false} />);
      expect(screen.getByText('Top integrations')).toBeInTheDocument();
    });

    it('renders provider and action together', () => {
      render(<InferenceBudget teamUsage={buildTeamUsage()} isLoadingCredits={false} />);
      expect(screen.getByText(/gmail · send/i)).toBeInTheDocument();
    });

    it('renders provider alone when action is empty', () => {
      const usage = buildTeamUsage({
        insights: buildInsights({
          topIntegrations: [{ provider: 'slack', action: '', spentUsd: 1, calls: 3 }],
        }),
      });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText('slack')).toBeInTheDocument();
      expect(screen.queryByText(/slack ·/i)).not.toBeInTheDocument();
    });

    it('shows empty-state message when topIntegrations is empty', () => {
      const usage = buildTeamUsage({ insights: buildInsights({ topIntegrations: [] }) });
      render(<InferenceBudget teamUsage={usage} isLoadingCredits={false} />);
      expect(screen.getByText(/No integration usage this cycle/i)).toBeInTheDocument();
    });
  });
});
