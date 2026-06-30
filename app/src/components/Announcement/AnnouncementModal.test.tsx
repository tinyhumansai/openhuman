import { fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Announcement } from '../../services/announcementService';
import { renderWithProviders } from '../../test/test-utils';
import AnnouncementModal from './AnnouncementModal';

function announcement(overrides: Partial<Announcement> = {}): Announcement {
  return {
    id: 'a1',
    title: 'Scheduled maintenance',
    body: 'We will be down tonight.',
    severity: 'INFO',
    cta: null,
    startsAt: null,
    expiresAt: null,
    createdAt: null,
    ...overrides,
  };
}

describe('AnnouncementModal', () => {
  afterEach(() => vi.restoreAllMocks());

  it('renders the title, body, and severity label', () => {
    renderWithProviders(<AnnouncementModal announcement={announcement()} onDismiss={vi.fn()} />);
    expect(screen.getByText('Scheduled maintenance')).toBeInTheDocument();
    expect(screen.getByText('We will be down tonight.')).toBeInTheDocument();
    expect(screen.getByText('Info')).toBeInTheDocument();
  });

  it.each([
    ['INFO', 'Info'],
    ['WARNING', 'Important'],
    ['CRITICAL', 'Critical'],
  ] as const)('maps severity %s to the %s badge', (severity, label) => {
    renderWithProviders(
      <AnnouncementModal announcement={announcement({ severity })} onDismiss={vi.fn()} />
    );
    expect(screen.getByText(label)).toBeInTheDocument();
  });

  it('calls onDismiss when the dismiss button is clicked', () => {
    const onDismiss = vi.fn();
    renderWithProviders(<AnnouncementModal announcement={announcement()} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByTestId('announcement-dismiss'));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('renders no CTA button when there is no cta', () => {
    renderWithProviders(
      <AnnouncementModal announcement={announcement({ cta: null })} onDismiss={vi.fn()} />
    );
    expect(screen.queryByTestId('announcement-cta')).not.toBeInTheDocument();
  });

  it('opens the CTA externally and dismisses when the CTA is clicked', () => {
    const onDismiss = vi.fn();
    const openSpy = vi.spyOn(window, 'open').mockReturnValue(null);
    renderWithProviders(
      <AnnouncementModal
        announcement={announcement({ cta: { label: 'Read more', url: 'https://x.test/' } })}
        onDismiss={onDismiss}
      />
    );

    const cta = screen.getByTestId('announcement-cta');
    expect(cta).toHaveTextContent('Read more');

    fireEvent.click(cta);
    expect(openSpy).toHaveBeenCalledWith('https://x.test/', '_blank', 'noopener,noreferrer');
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
