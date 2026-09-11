/**
 * Inline icons for the adapted AI Elements layer.
 *
 * Upstream uses `lucide-react` (`BookIcon`, `ChevronDownIcon`). `lucide-react`
 * is not a dependency here and may not be added, and `components/ui/icons.tsx`
 * only ships Spinner/Check/Close/Warning — so these two are hand-drawn as small
 * `aria-hidden` SVGs on the same 24×24 grid and `currentColor` stroke as the
 * existing ui icons.
 *
 * `BrainIcon` and `DotIcon` lived here too, for `Reasoning` and
 * `ChainOfThought`. Both components were deleted as orphans, and an icon whose
 * only caller is gone is just more surface to keep rendering correctly.
 */
import type { SVGProps } from 'react';

type IconProps = SVGProps<SVGSVGElement>;

const base = {
  'aria-hidden': true as const,
  focusable: 'false' as const,
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 2,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  viewBox: '0 0 24 24',
};

export function ChevronDownIcon({ className = 'h-4 w-4', ...props }: IconProps) {
  return (
    <svg {...base} className={className} {...props}>
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

export function BookIcon({ className = 'h-4 w-4', ...props }: IconProps) {
  return (
    <svg {...base} className={className} {...props}>
      <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
      <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2Z" />
    </svg>
  );
}
