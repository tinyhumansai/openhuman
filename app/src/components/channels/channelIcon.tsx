import type { ReactElement } from 'react';

import DingTalkIcon from './DingTalkIcon';
import IMessageIcon from './IMessageIcon';
import LarkIcon from './LarkIcon';
import YuanbaoIcon from './YuanbaoIcon';

/**
 * Channels whose brand mark is a component (inline SVG or a bundled logo image)
 * rather than an emoji glyph. Keyed by `ChannelDefinition.icon` — the same
 * string the core sends in `channels/controllers/definitions.rs`.
 */
const ICON_COMPONENTS: Record<string, (props: { className?: string }) => ReactElement> = {
  yuanbao: YuanbaoIcon,
  lark: LarkIcon,
  dingtalk: DingTalkIcon,
  imessage: IMessageIcon,
};

/**
 * Emoji icons for channels without a dedicated brand mark, rendered as plain
 * text. Keyed by `ChannelDefinition.icon`.
 */
const ICON_EMOJI: Record<string, string> = { telegram: '✈️', discord: '🎮', web: '🌐', mcp: '🔌' };

/**
 * Render the brand icon for a channel, keyed by `ChannelDefinition.icon`.
 *
 * Component-backed marks (Yuanbao SVG, Lark/DingTalk logos) take precedence
 * over emoji glyphs. Unknown icons render `null` so callers lay out around a
 * possibly-absent icon rather than reserving space for a blank.
 *
 * This is the single source of truth for channel icons — both `ChannelSelector`
 * and `ChannelSetupModal` consume it, so an icon can never be defined in one
 * surface but missing in the other (the bug that left Lark/DingTalk blank).
 */
export function renderChannelIcon(icon: string, className = 'w-5 h-5'): ReactElement | null {
  const IconComponent = ICON_COMPONENTS[icon];
  if (IconComponent) {
    return <IconComponent className={className} />;
  }
  const emoji = ICON_EMOJI[icon];
  return emoji ? (
    <span aria-hidden="true" className="text-base">
      {emoji}
    </span>
  ) : null;
}
