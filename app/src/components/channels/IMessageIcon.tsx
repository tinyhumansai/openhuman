import { useId } from 'react';

interface IMessageIconProps {
  /** Tailwind size/color overrides. Defaults to a 20px box, matching the
   * visual weight of the channel-row emojis it sits next to. */
  className?: string;
}

/**
 * Brand mark for the iMessage channel — a green rounded-square tile with a
 * white speech bubble. Drawn as an original, generic message-bubble glyph
 * (not a copy of Apple's proprietary Messages artwork) and inlined as an SVG
 * so it bundles into packaged desktop builds and tints/sizes via Tailwind.
 * The gradient id is generated with `useId` so multiple instances on a page
 * (channel selector + setup modal) don't collide. Decorative: the channel
 * name is rendered as adjacent text.
 */
const IMessageIcon = ({ className = 'w-5 h-5' }: IMessageIconProps) => {
  const gradId = useId();
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      className={className}
      aria-hidden="true">
      <defs>
        <linearGradient id={gradId} x1="10" y1="0" x2="10" y2="20" gradientUnits="userSpaceOnUse">
          <stop stopColor="#5BF675" />
          <stop offset="1" stopColor="#1CB23E" />
        </linearGradient>
      </defs>
      <rect width="20" height="20" rx="5" fill={`url(#${gradId})`} />
      <path
        d="M10 4.6c-3.4 0-6.2 2.1-6.2 4.8 0 1.55.95 2.93 2.43 3.83-.2.86-.74 1.7-1.5 2.32 1.26-.1 2.45-.5 3.42-1.16.58.12 1.2.18 1.85.18 3.43 0 6.2-2.15 6.2-4.97C16.2 6.7 13.43 4.6 10 4.6z"
        fill="#ffffff"
      />
    </svg>
  );
};

export default IMessageIcon;
