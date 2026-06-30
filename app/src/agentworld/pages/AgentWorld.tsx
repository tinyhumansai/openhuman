/**
 * AgentWorld — section host for the Tiny.Place integration.
 *
 * The section navigation lives in the root app sidebar's dynamic region (the
 * "session sidebar"), projected there via `SidebarContent` — the same pattern
 * as Brain. The active section fills the content pane flush. The section name
 * is carried by the sidebar (no per-section page title), so sections render
 * their own body chrome via `PanelScaffold`.
 *
 * Sub-navigation keys: agentWorld.explore (+ future section keys).
 */
import { Navigate, Route, Routes, useLocation, useNavigate } from 'react-router-dom';

import { SidebarContent } from '../../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../../components/layout/TwoPaneNav';
import { useT } from '../../lib/i18n/I18nContext';
import WalletAddressChip from '../components/WalletAddressChip';
import BountiesSection from './BountiesSection';
import DirectorySection from './DirectorySection';
import ExploreSection from './ExploreSection';
import FeedSection from './FeedSection';
import IdentitiesSection from './IdentitiesSection';
import JobsSection from './JobsSection';
import LedgerSection from './LedgerSection';
import MarketplaceSection from './MarketplaceSection';
import MessagingSection from './MessagingSection';
import ProfilesSection from './ProfilesSection';
import WorldSection from './WorldSection';

// Sub-nav section definition (one per section).
interface AgentWorldSection {
  slug: string;
  labelKey: string;
  iconPath: string;
}

/** Small inline icon helper for the sidebar nav (matches Brain's). */
const navIcon = (d: string) => (
  <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={d} />
  </svg>
);

// === AGENT-WORLD SECTIONS (append one per section) ===
// Format: { slug: '<path-segment>', labelKey: 'agentWorld.<name>', iconPath: '<svg d>' }
// Fan-out agents: add a row here AND a <Route> below AND an i18n key.
// Sidebar order: Feed first, then Messages, then the rest; Profiles sits at the
// end. Marketplace, Jobs and Explore are intentionally OMITTED from the sidebar
// (their routes still exist below so existing flows / deep links remain
// reachable) — hidden, not removed. Jobs is superseded by Bounties.
const SECTIONS: AgentWorldSection[] = [
  {
    slug: 'world',
    labelKey: 'agentWorld.world',
    iconPath:
      'M3 9.75L12 4l9 5.75M5.25 10.75V19h13.5v-8.25M9 19v-4.5h6V19M8 12h.01M12 10h.01M16 12h.01',
  },
  {
    slug: 'feed',
    labelKey: 'agentWorld.feed',
    iconPath:
      'M19 20H5a2 2 0 01-2-2V6a2 2 0 012-2h10a2 2 0 012 2v1m2 13a2 2 0 01-2-2V7m2 13a2 2 0 002-2V9a2 2 0 00-2-2h-2m-4-3H9M7 16h6M7 8h6v4H7V8z',
  },
  {
    slug: 'messaging',
    labelKey: 'agentWorld.messaging',
    iconPath:
      'M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z',
  },
  {
    slug: 'ledger',
    labelKey: 'agentWorld.ledger',
    iconPath:
      'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-3 7h3m-3 4h3m-6-4h.01M9 16h.01',
  },
  {
    slug: 'bounties',
    labelKey: 'agentWorld.bounties',
    iconPath:
      'M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 11-18 0 9 9 0 0118 0z',
  },
  {
    slug: 'directory',
    labelKey: 'agentWorld.directory',
    iconPath:
      'M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197M13 7a4 4 0 11-8 0 4 4 0 018 0z',
  },
  {
    slug: 'identities',
    labelKey: 'agentWorld.identities',
    iconPath:
      'M10 6H5a2 2 0 00-2 2v9a2 2 0 002 2h14a2 2 0 002-2V8a2 2 0 00-2-2h-5m-4 0V5a2 2 0 114 0v1m-4 0a2 2 0 104 0',
  },
  {
    slug: 'profiles',
    labelKey: 'agentWorld.profiles',
    iconPath:
      'M5.121 17.804A13.937 13.937 0 0112 16c2.5 0 4.847.655 6.879 1.804M15 10a3 3 0 11-6 0 3 3 0 016 0z',
  },
];

export default function AgentWorld() {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();

  // Derive the active slug from the current sub-path
  // e.g. /agent-world/explore → 'explore'
  const pathParts = location.pathname.split('/');
  const activeSlug = pathParts[pathParts.length - 1] || 'feed';
  const activeSection = activeSlug === 'agent-world' ? 'world' : activeSlug;
  const isWorld = activeSection === 'world';

  return (
    <div className="h-full">
      {/* The Tiny.Place section navigation lives in the root app sidebar's
          dynamic region (the session sidebar), projected via SidebarContent. */}
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <TwoPaneNav
            ariaLabel={t('nav.agentWorld')}
            selected={activeSection}
            onSelect={slug => navigate(`/agent-world/${slug}`)}
            groups={[
              {
                items: SECTIONS.map(section => ({
                  value: section.slug,
                  label: t(section.labelKey),
                  icon: navIcon(section.iconPath),
                })),
              },
            ]}
            header={
              <div className="space-y-2">
                <p className="min-w-0 text-[11px] leading-relaxed text-content-muted">
                  {t('agentWorld.description')}
                </p>
                <WalletAddressChip />
              </div>
            }
          />
        </div>
      </SidebarContent>
      {/* Card surface around the active section so the section chrome and its
          inner cards sit on a framed panel (matching Brain) instead of floating
          flush on the bare shell background. */}
      <div className={isWorld ? 'h-full w-full p-0' : 'mx-auto h-full w-full max-w-6xl p-4'}>
        <div
          className={
            isWorld
              ? 'h-full overflow-hidden bg-black'
              : 'h-full overflow-hidden rounded-2xl border border-line bg-surface shadow-soft'
          }>
          <Routes>
            <Route index element={<Navigate to="/agent-world/world" replace />} />
            <Route path="world" element={<WorldSection />} />
            <Route path="feed" element={<FeedSection />} />
            <Route path="ledger" element={<LedgerSection />} />
            <Route path="jobs" element={<JobsSection />} />
            <Route path="bounties" element={<BountiesSection />} />
            <Route path="explore" element={<ExploreSection />} />
            {/* === AGENT-WORLD SECTION ROUTES (append one per section) === */}
            <Route path="directory" element={<DirectorySection />} />
            <Route path="profiles" element={<ProfilesSection />} />
            <Route path="identities" element={<IdentitiesSection />} />
            <Route path="marketplace" element={<MarketplaceSection />} />
            <Route path="messaging" element={<MessagingSection />} />
            <Route path="*" element={<Navigate to="/agent-world/world" replace />} />
          </Routes>
        </div>
      </div>
    </div>
  );
}
