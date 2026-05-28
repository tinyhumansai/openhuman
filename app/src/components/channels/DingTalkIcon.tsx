interface DingTalkIconProps {
  /** Tailwind size/color overrides. Defaults to a 20px box, matching the
   * visual weight of the channel-row emojis it sits next to. */
  className?: string;
}

/**
 * Brand mark for the DingTalk (钉钉) channel. The official logo is a raster
 * asset served from the public dir (`/dingtalk.png`) so it bundles into
 * packaged desktop builds — same pattern as `/alpha.svg` in `providerIcons`.
 * Decorative: the channel name is rendered as adjacent text, so `alt` is empty.
 */
const DingTalkIcon = ({ className = 'w-5 h-5' }: DingTalkIconProps) => (
  <img src="/dingtalk.png" alt="" className={className} draggable={false} />
);

export default DingTalkIcon;
