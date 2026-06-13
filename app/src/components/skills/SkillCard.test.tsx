import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import UnifiedSkillCard from './SkillCard';

vi.mock('../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string) => key }),
}));

describe('UnifiedSkillCard stable test hooks', () => {
  it('renders row and primary action test ids', () => {
    const onCtaClick = vi.fn();

    render(
      <UnifiedSkillCard
        icon={<span />}
        title="Calendar"
        description="Connect calendar context"
        ctaLabel="Install"
        testId="skill-row-calendar"
        ctaTestId="skill-install-calendar"
        onCtaClick={onCtaClick}
      />
    );

    expect(screen.getByTestId('skill-row-calendar')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('skill-install-calendar'));
    expect(onCtaClick).toHaveBeenCalledTimes(1);
  });

  it('renders secondary action test ids', () => {
    const onUninstall = vi.fn();

    render(
      <UnifiedSkillCard
        icon={<span />}
        title="Calendar"
        description="Connect calendar context"
        ctaLabel="Open"
        onCtaClick={vi.fn()}
        secondaryActions={[
          {
            label: 'Uninstall',
            icon: <span />,
            testId: 'skill-uninstall-calendar',
            onClick: onUninstall,
          },
        ]}
      />
    );

    fireEvent.click(screen.getByTitle('skills.card.moreActions'));
    fireEvent.click(screen.getByTestId('skill-uninstall-calendar'));
    expect(onUninstall).toHaveBeenCalledTimes(1);
  });
});
