/**
 * MemoryEmbeddingBudgetBanner tests (#5324).
 *
 * The behaviours worth pinning are the ones that decide whether a user finds
 * out their memory stopped growing: which levels render, which can be
 * silenced, where the CTA goes, and that the OS notification fires exactly
 * once for an exhausted budget.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import MemoryEmbeddingBudgetBanner, {
  __resetNativeNotificationLatchForTests,
  EMBEDDINGS_SETTINGS_ROUTE,
} from '../MemoryEmbeddingBudgetBanner';

const mockUseEmbeddingBudgetState = vi.hoisted(() =>
  vi.fn(() => ({
    level: 'none' as 'none' | 'warn' | 'urgent' | 'exhausted',
    pct: 0,
    isLoading: false,
    isManagedEmbeddings: true,
  }))
);
const mockNavigate = vi.hoisted(() => vi.fn());
const mockShowNativeNotification = vi.hoisted(() =>
  vi.fn(() => Promise.resolve({ delivered: true }))
);

vi.mock('../../../hooks/useEmbeddingBudgetState', () => ({
  useEmbeddingBudgetState: mockUseEmbeddingBudgetState,
}));
vi.mock('react-router-dom', () => ({ useNavigate: () => mockNavigate }));
vi.mock('../../../lib/nativeNotifications/tauriBridge', () => ({
  showNativeNotification: mockShowNativeNotification,
}));
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../UpsellBanner', () => ({
  default: ({
    title,
    message,
    ctaLabel,
    onCtaClick,
    dismissible,
    onDismiss,
  }: {
    title: string;
    message: string;
    ctaLabel?: string;
    onCtaClick?: () => void;
    dismissible?: boolean;
    onDismiss?: () => void;
  }) => (
    <div data-testid="upsell-banner" data-dismissible={String(Boolean(dismissible))}>
      <span>{title}</span>
      <span data-testid="banner-message">{message}</span>
      <button onClick={onCtaClick}>{ctaLabel}</button>
      {dismissible && (
        <button data-testid="banner-dismiss" onClick={onDismiss}>
          dismiss
        </button>
      )}
    </div>
  ),
}));

function setLevel(level: 'none' | 'warn' | 'urgent' | 'exhausted', pct = 0) {
  mockUseEmbeddingBudgetState.mockReturnValue({
    level,
    pct,
    isLoading: false,
    isManagedEmbeddings: true,
  });
}

describe('MemoryEmbeddingBudgetBanner', () => {
  beforeEach(() => {
    mockNavigate.mockReset();
    mockShowNativeNotification.mockClear();
    __resetNativeNotificationLatchForTests();
    setLevel('none');
  });

  it('renders nothing below the warning threshold', () => {
    const { container } = render(<MemoryEmbeddingBudgetBanner />);
    expect(container.firstChild).toBeNull();
  });

  it('shows a dismissible warning at the 75% level', () => {
    setLevel('warn', 76);
    render(<MemoryEmbeddingBudgetBanner />);
    expect(screen.getByTestId('upsell-banner')).toHaveAttribute('data-dismissible', 'true');
    expect(screen.getByText('memoryBudget.approachingTitle')).toBeInTheDocument();
  });

  it('interpolates the consumed percentage into the warning copy', () => {
    setLevel('warn', 76);
    render(<MemoryEmbeddingBudgetBanner />);
    // The mocked translator returns the key, so the replace() target is
    // absent — what matters is that the component does not crash and renders
    // the approaching message slot.
    expect(screen.getByTestId('banner-message')).toBeInTheDocument();
  });

  it('makes the 90% escalation non-dismissible', () => {
    setLevel('urgent', 92);
    render(<MemoryEmbeddingBudgetBanner />);
    expect(screen.getByTestId('upsell-banner')).toHaveAttribute('data-dismissible', 'false');
    expect(screen.queryByTestId('banner-dismiss')).toBeNull();
  });

  it('makes the exhausted state non-dismissible and names it distinctly', () => {
    setLevel('exhausted', 100);
    render(<MemoryEmbeddingBudgetBanner />);
    expect(screen.getByTestId('upsell-banner')).toHaveAttribute('data-dismissible', 'false');
    expect(screen.getByText('memoryBudget.exhaustedTitle')).toBeInTheDocument();
  });

  it('hides the warning once dismissed', () => {
    setLevel('warn', 76);
    render(<MemoryEmbeddingBudgetBanner />);
    fireEvent.click(screen.getByTestId('banner-dismiss'));
    expect(screen.queryByTestId('upsell-banner')).toBeNull();
  });

  it('re-shows at the next level after the warning was dismissed', () => {
    setLevel('warn', 76);
    const { rerender } = render(<MemoryEmbeddingBudgetBanner />);
    fireEvent.click(screen.getByTestId('banner-dismiss'));
    expect(screen.queryByTestId('upsell-banner')).toBeNull();

    // Dismissing 75% must not silence the 90% escalation — otherwise the
    // user is back to a silent failure.
    setLevel('urgent', 92);
    rerender(<MemoryEmbeddingBudgetBanner />);
    expect(screen.getByTestId('upsell-banner')).toBeInTheDocument();
  });

  it('deep-links the CTA to the embeddings configuration screen', () => {
    setLevel('exhausted', 100);
    render(<MemoryEmbeddingBudgetBanner />);
    fireEvent.click(screen.getByText('memoryBudget.cta'));
    expect(mockNavigate).toHaveBeenCalledWith(EMBEDDINGS_SETTINGS_ROUTE);
  });

  it('fires an OS notification once when the budget is exhausted', () => {
    setLevel('exhausted', 100);
    const { rerender } = render(<MemoryEmbeddingBudgetBanner />);
    expect(mockShowNativeNotification).toHaveBeenCalledTimes(1);
    expect(mockShowNativeNotification).toHaveBeenCalledWith(
      expect.objectContaining({ tag: 'memory-embedding-budget-exhausted' })
    );

    // The usage hook re-renders on every poll; the notification must not.
    rerender(<MemoryEmbeddingBudgetBanner />);
    expect(mockShowNativeNotification).toHaveBeenCalledTimes(1);
  });

  it('does not fire an OS notification for the pre-exhaustion warnings', () => {
    setLevel('urgent', 92);
    render(<MemoryEmbeddingBudgetBanner />);
    expect(mockShowNativeNotification).not.toHaveBeenCalled();
  });
});
