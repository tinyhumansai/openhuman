/**
 * HarnessGlyph — a small brand mark for the agent harness driving an instance
 * (Claude / Codex / Gemini), plus an OpenHuman mark for internal windows. Used
 * as the leading glyph in roster rows.
 *
 * The colors here are deliberate brand/identity hues (not surface chrome), so
 * they use literal palette classes per the project's "meaningful content color"
 * guidance rather than semantic tokens.
 */
import type { HarnessType } from '../../lib/orchestration/orchestrationClient';

export type GlyphKind = HarnessType | 'openhuman';

export interface HarnessGlyphProps {
  harness: GlyphKind;
  className?: string;
}

const GLYPH: Record<GlyphKind, { label: string; tone: string }> = {
  claude: { label: 'C', tone: 'bg-[#c96442] text-white' },
  codex: { label: 'Cx', tone: 'bg-content text-surface' },
  gemini: { label: 'G', tone: 'bg-ocean-500 text-white' },
  openhuman: { label: 'OH', tone: 'bg-sage-500 text-white' },
};

export default function HarnessGlyph({
  harness,
  className,
}: HarnessGlyphProps): React.ReactElement {
  const { label, tone } = GLYPH[harness];
  return (
    <span
      aria-hidden
      data-testid="harness-glyph"
      data-harness={harness}
      className={`flex h-6 w-6 flex-none items-center justify-center rounded-md font-mono text-[11px] font-bold ${tone} ${className ?? ''}`}>
      {label}
    </span>
  );
}
