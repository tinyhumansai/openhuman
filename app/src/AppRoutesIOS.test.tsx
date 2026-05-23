import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Stub out the surfaces the mobile shell routes to so we can mount
// `<AppRoutesIOS />` without dragging the full Redux + provider tree along.
vi.mock('./features/human/HumanPage', () => ({
  default: () => <div data-testid="page-human">human</div>,
}));
vi.mock('./pages/Accounts', () => ({ default: () => <div data-testid="page-chat">chat</div> }));
vi.mock('./pages/Settings', () => ({
  default: () => <div data-testid="page-settings">settings</div>,
}));
vi.mock('./pages/ios/PairScreen', () => ({
  PairScreen: () => <div data-testid="page-pair">pair</div>,
}));
vi.mock('./components/ios/MobileTabBar', () => ({
  default: () => <nav data-testid="mobile-tab-bar">tabs</nav>,
}));

const listProfiles = vi.fn();
vi.mock('./services/transport/profileStore', () => ({ listProfiles: () => listProfiles() }));

const AppRoutesIOS = (await import('./AppRoutesIOS')).default;

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <AppRoutesIOS />
    </MemoryRouter>
  );

describe('AppRoutesIOS', () => {
  beforeEach(() => listProfiles.mockReset());
  afterEach(() => vi.clearAllMocks());

  describe('unpaired (no saved profile)', () => {
    beforeEach(() => listProfiles.mockReturnValue([]));

    it('redirects unknown paths to /pair', () => {
      renderAt('/');
      expect(screen.getByTestId('page-pair')).toBeInTheDocument();
    });

    it('renders the PairScreen at /pair', () => {
      renderAt('/pair');
      expect(screen.getByTestId('page-pair')).toBeInTheDocument();
    });

    it('bounces /human back to /pair when no profile exists', () => {
      renderAt('/human');
      expect(screen.getByTestId('page-pair')).toBeInTheDocument();
      expect(screen.queryByTestId('page-human')).not.toBeInTheDocument();
    });
  });

  describe('paired (profile exists)', () => {
    beforeEach(() => listProfiles.mockReturnValue([{ id: 'p1' }]));

    it('renders HumanPage with the mobile tab bar', () => {
      renderAt('/human');
      expect(screen.getByTestId('page-human')).toBeInTheDocument();
      expect(screen.getByTestId('mobile-tab-bar')).toBeInTheDocument();
    });

    it('renders the chat surface at /chat', () => {
      renderAt('/chat');
      expect(screen.getByTestId('page-chat')).toBeInTheDocument();
      expect(screen.getByTestId('mobile-tab-bar')).toBeInTheDocument();
    });

    it('renders Settings at /settings/devices via nested route', () => {
      renderAt('/settings/devices');
      expect(screen.getByTestId('page-settings')).toBeInTheDocument();
      expect(screen.getByTestId('mobile-tab-bar')).toBeInTheDocument();
    });

    it('redirects unknown paths to /human when paired', () => {
      renderAt('/');
      expect(screen.getByTestId('page-human')).toBeInTheDocument();
    });
  });
});
