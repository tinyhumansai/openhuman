/** Tiny colour helpers for blending palette tints (all values are 0xRRGGBB). */

function channels(color: number): [number, number, number] {
  return [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff];
}

function pack(red: number, green: number, blue: number): number {
  const clamp = (value: number): number => Math.max(0, Math.min(255, Math.round(value)));
  return (clamp(red) << 16) | (clamp(green) << 8) | clamp(blue);
}

/** Linearly blend two colours; `amount` 0 returns `colorA`, 1 returns `colorB`. */
export function mixColor(colorA: number, colorB: number, amount: number): number {
  const [redA, greenA, blueA] = channels(colorA);
  const [redB, greenB, blueB] = channels(colorB);
  return pack(
    redA + (redB - redA) * amount,
    greenA + (greenB - greenA) * amount,
    blueA + (blueB - blueA) * amount
  );
}

/** Scale a colour's brightness (`factor` < 1 darkens, > 1 lightens). */
export function shadeColor(color: number, factor: number): number {
  const [red, green, blue] = channels(color);
  return pack(red * factor, green * factor, blue * factor);
}
