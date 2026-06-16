/**
 * Deterministic member → colour mapping for the agent-team surface (#3374).
 *
 * A team member should read as the *same* colour everywhere it appears — the
 * owner left-border on a task card, the roster chip in the header, and the
 * avatar in the activity rail. Tailwind can't express a per-member dynamic
 * class (the class names aren't known at build time, so they'd be purged), so
 * the colour is resolved to a house-palette hex here and applied via inline
 * style at the call site — the same approach `memory-workspace.css` already
 * uses for the ocean accent.
 *
 * The palette is drawn from the project's semantic tokens (ocean / sage / amber
 * / coral / sky / lavender) so the board never introduces an off-brand hue.
 * Assignment is stable per member id: the same id always maps to the same
 * colour, and ids spread across the palette before repeating.
 */

/** House-palette hues used to distinguish team members. Order is the cycle. */
const MEMBER_PALETTE = [
  '#4A83DD', // ocean
  '#34C759', // sage
  '#E8A728', // amber
  '#EF4444', // coral
  '#0EA5E9', // sky (darkened for contrast on white)
  '#9B8AFB', // lavender accent
] as const;

/** Stable, order-independent hash of a member id → palette bucket. */
function hashId(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) {
    // Classic 31-multiplier string hash; `| 0` keeps it in 32-bit int range.
    hash = (hash * 31 + id.charCodeAt(i)) | 0;
  }
  return Math.abs(hash);
}

/**
 * Resolve a member id to a stable house-palette hex colour.
 *
 * @param memberId The member's durable id. Empty/undefined → first palette hue.
 */
export function memberColor(memberId: string | null | undefined): string {
  if (!memberId) return MEMBER_PALETTE[0];
  return MEMBER_PALETTE[hashId(memberId) % MEMBER_PALETTE.length];
}

/**
 * A translucent tint of a member's colour, for chip/badge backgrounds. Appends
 * an alpha byte to the 6-digit hex (e.g. `#4A83DD` + `1f` → ~12% opacity).
 *
 * @param memberId The member's durable id.
 * @param alphaHex Two-hex-digit alpha suffix. Defaults to `1f` (~12%).
 */
export function memberTint(memberId: string | null | undefined, alphaHex = '1f'): string {
  return `${memberColor(memberId)}${alphaHex}`;
}
