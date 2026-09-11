/**
 * `/pair` must stay reachable — including from an already-paired phone.
 *
 * openhuman#5479 ("Cannot Pair iPhone") is a wire-contract failure on the
 * desktop side: `[devices/tunnel] parse tunnel:register ack failed: missing
 * field 'channelId'`. Nothing in this file can fix that — it is Rust, in
 * `src/openhuman/security/devices/tunnel_client.rs`. What this file protects is
 * the thing that turns that bug from "retry it" into "the phone is bricked":
 * the route that lets a user pair again.
 *
 * `/pair` is the ONLY mobile route not wrapped in `RequirePairing`
 * (`AppRoutesIOS.tsx:57`). That is deliberate and load-bearing — a phone whose
 * saved profile points at a core it can no longer reach is "paired" as far as
 * `isPaired()` is concerned (`listProfiles().length > 0`), so wrapping `/pair`
 * or letting the catch-all swallow it would strand exactly the user who most
 * needs to re-pair. `AppRoutesIOS.test.tsx` covers `/pair` while UNPAIRED; this
 * covers the paired case, which is the one that regresses silently.
 *
 * The pairing screen's own behaviour (QR happy path, expiry, bad URL, missing
 * fields, transport unhealthy, camera errors, retry) is already covered by
 * `pages/ios/PairScreen.test.tsx`. Not repeated here.
 */
import { render, screen } from '@testing-library/react';
import { MemoryRouter, useParams } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('./features/human/HumanPage', () => ({
  default: () => <div data-testid="page-human">human</div>,
}));
// Renders the route param back out. A stub that discards params makes
// `/chat/thread-abc`, `/chat/anything` and a route that dropped the
// `:threadId` segment entirely produce identical DOM — so the deep-link test
// below would pass without observing the thread id at all. Mirrors the
// technique already used in `AppRoutes.connections-flows.test.tsx`.
vi.mock('./pages/Accounts', () => {
  const ChatProbe = () => {
    const { threadId } = useParams();
    return <div data-testid="page-chat">{`chat:${threadId ?? ''}`}</div>;
  };
  return { default: ChatProbe };
});
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

/** A saved profile is all `isPaired()` looks at — not whether the core answers. */
const SAVED_PROFILE = [{ id: 'profile-1', label: 'Desk' }];

describe('AppRoutesIOS — re-pairing escape hatch', () => {
  beforeEach(() => listProfiles.mockReset());
  afterEach(() => vi.clearAllMocks());

  it('serves /pair to an already-paired phone rather than bouncing it to /human', () => {
    listProfiles.mockReturnValue(SAVED_PROFILE);

    renderAt('/pair');

    expect(screen.getByTestId('page-pair')).toBeInTheDocument();
    expect(screen.queryByTestId('page-human')).not.toBeInTheDocument();
  });

  it('still serves /pair when the saved profile is stale or broken', () => {
    // `isPaired()` is `listProfiles().length > 0` (AppRoutesIOS.tsx:29) — it
    // never asks whether the core is reachable. A phone holding a profile for a
    // desktop that is gone is indistinguishable from a healthy one here, and it
    // is precisely the phone that must be able to re-scan a QR code.
    listProfiles.mockReturnValue([{ id: 'dead-core', label: 'Old laptop' }]);

    renderAt('/pair');

    expect(screen.getByTestId('page-pair')).toBeInTheDocument();
  });

  it('shows no tab bar on /pair, paired or not', () => {
    // The tab bar navigates to paired-only surfaces. Rendering it over the
    // pairing screen offers exits that bounce straight back to /pair.
    for (const profiles of [[], SAVED_PROFILE]) {
      listProfiles.mockReturnValue(profiles);
      const { unmount } = renderAt('/pair');
      expect(screen.getByTestId('page-pair')).toBeInTheDocument();
      expect(screen.queryByTestId('mobile-tab-bar')).not.toBeInTheDocument();
      unmount();
    }
  });
});

describe('AppRoutesIOS — deep links a paired phone must honour', () => {
  beforeEach(() => listProfiles.mockReset());
  afterEach(() => vi.clearAllMocks());

  it('opens a specific thread at /chat/:threadId', () => {
    // The bare /chat form is covered by AppRoutesIOS.test.tsx; the optional
    // `:threadId` segment is what a notification deep link actually carries,
    // and a mismatch there lands the user on the catch-all instead.
    listProfiles.mockReturnValue(SAVED_PROFILE);

    renderAt('/chat/thread-abc');

    // The segment must survive to the page, not merely match the route: this is
    // the half a notification deep link depends on.
    expect(screen.getByTestId('page-chat')).toHaveTextContent('chat:thread-abc');
    expect(screen.getByTestId('mobile-tab-bar')).toBeInTheDocument();
    expect(screen.queryByTestId('page-human')).not.toBeInTheDocument();
  });

  it('bounces a thread deep link to /pair when the phone is unpaired', () => {
    listProfiles.mockReturnValue([]);

    renderAt('/chat/thread-abc');

    expect(screen.getByTestId('page-pair')).toBeInTheDocument();
    expect(screen.queryByTestId('page-chat')).not.toBeInTheDocument();
  });
});
