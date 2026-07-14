import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { DISCORD_INVITE_URL } from '../../utils/links';
import MedullaOverviewPanel from './MedullaOverviewPanel';

// Pass-through translator so assertions can target the i18n keys directly.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const openUrl = vi.fn();
vi.mock('../../utils/openUrl', () => ({ openUrl: (url: string) => openUrl(url) }));

describe('MedullaOverviewPanel', () => {
  it('renders the Medulla teaser copy and CTA', () => {
    render(<MedullaOverviewPanel />);
    expect(screen.getByTestId('orch-medulla')).toBeInTheDocument();
    expect(screen.getByText('orchPage.medulla.badge')).toBeInTheDocument();
    expect(screen.getByText('orchPage.medulla.title')).toBeInTheDocument();
    expect(screen.getByText('orchPage.medulla.body')).toBeInTheDocument();
    expect(screen.getByText('orchPage.medulla.subscriberNote')).toBeInTheDocument();
    expect(screen.getByTestId('orch-medulla-discord')).toBeInTheDocument();
  });

  it('opens the Discord invite when the CTA is clicked', () => {
    render(<MedullaOverviewPanel />);
    fireEvent.click(screen.getByTestId('orch-medulla-discord'));
    expect(openUrl).toHaveBeenCalledWith(DISCORD_INVITE_URL);
  });
});
