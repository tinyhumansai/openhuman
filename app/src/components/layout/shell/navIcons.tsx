import type { IconType } from 'react-icons';
import {
  LuActivity,
  LuBrain,
  LuGift,
  LuHouse,
  LuKeyboard,
  LuMegaphone,
  LuMessageCircle,
  LuNetwork,
  LuSettings,
  LuUserRound,
  LuWallet,
  LuWaypoints,
  LuWorkflow,
} from 'react-icons/lu';

/**
 * Icon set for the static sidebar navigation, keyed by `NavTab.id`.
 *
 * Was 199 lines of hand-drawn `<path d="…">` line art; now a lookup into
 * `react-icons/lu` (lucide, already a dependency and already used at 20+
 * other sites in this repo — see AGENTS.md). Keeping the `<NavIcon id=… />`
 * shape means every call site (`SidebarNav`, `CollapsedNavRail`, `AppSidebar`)
 * is unchanged; only this file's body did.
 */

const ICONS: Record<string, IconType> = {
  home: LuHouse,
  human: LuUserRound,
  chat: LuMessageCircle,
  feedback: LuMegaphone,
  connections: LuNetwork,
  activity: LuActivity,
  settings: LuSettings,
  flows: LuWorkflow,
  orchestration: LuWaypoints,
  wallet: LuWallet,
  brain: LuBrain,
  keyboard: LuKeyboard,
  rewards: LuGift,
};

interface NavIconProps {
  id: string;
  className?: string;
}

export function NavIcon({ id, className = 'w-5 h-5' }: NavIconProps) {
  const Icon = ICONS[id];
  if (!Icon) return null;
  return <Icon className={className} aria-hidden="true" />;
}
