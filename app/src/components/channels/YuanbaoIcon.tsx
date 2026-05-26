import { useId } from 'react';

interface YuanbaoIconProps {
  /**
   * Tailwind size + color overrides. Defaults to a 20px box, matching
   * the visual weight of the channel-row emojis it sits next to.
   */
  className?: string;
}

/**
 * Brand mark for the Yuanbao channel. Inlined as an SVG component so it
 * can be tinted / sized via Tailwind without round-tripping through an
 * `<img>` element. `clipPath` ids are generated with `useId` so multiple
 * instances on the same page (channel selector + setup modal) don't
 * collide in the DOM.
 */
const YuanbaoIcon = ({ className = 'w-5 h-5' }: YuanbaoIconProps) => {
  const clipId = useId();
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      width="20"
      height="20"
      viewBox="0 0 20 20"
      fill="none"
      className={className}
      aria-hidden="true">
      <g clipPath={`url(#${clipId})`}>
        <path
          d="M10 20C15.5228 20 20 15.5228 20 10C20 4.47715 15.5228 0 10 0C4.47715 0 0 4.47715 0 10C0 15.5228 4.47715 20 10 20Z"
          fill="#00CC70"
        />
        <path
          d="M16.5659 7.16636C15.0693 5.53753 12.4048 5.53174 10.6148 7.15372C8.82475 8.77569 6.16026 8.7699 4.66365 7.14107C3.21394 5.56335 3.39146 3.04056 5.03135 1.41016C1.60302 4.66517 1.1858 9.82084 4.13265 13.0274C7.12586 16.285 12.4548 16.2961 16.0349 13.0522C17.8249 11.4302 18.0625 8.79518 16.5659 7.16636Z"
          fill="white"
        />
        <path
          d="M8.9865 10.6631C8.9865 12.1702 8.68728 12.4531 8.01194 12.4531C7.3366 12.4531 7.03738 12.1702 7.03738 10.6631C7.03738 9.15593 7.3366 8.87305 8.01194 8.87305C8.68728 8.87305 8.9865 9.15593 8.9865 10.6631Z"
          fill="black"
        />
        <path
          d="M12.3379 12.4478C12.1414 12.4478 11.948 12.3556 11.8263 12.1829L11.0372 11.0634C10.8892 10.8532 10.8844 10.5741 11.0256 10.3596L11.8148 9.15963C12.0044 8.87095 12.3926 8.79088 12.6808 8.98052C12.9689 9.17016 13.0495 9.55788 12.8599 9.84656L12.3047 10.691L12.8483 11.4622C13.0474 11.7446 12.98 12.1349 12.6976 12.3335C12.5881 12.411 12.4622 12.4478 12.3379 12.4478Z"
          fill="black"
        />
      </g>
      <defs>
        <clipPath id={clipId}>
          <rect width="20" height="20" rx="10" fill="white" />
        </clipPath>
      </defs>
    </svg>
  );
};

export default YuanbaoIcon;
