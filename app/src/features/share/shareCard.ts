/**
 * Branded share-card rendering for issue #5006.
 *
 * The card is drawn with the raw Canvas 2D API (no html2canvas / screenshot) so
 * the output is deterministic, dependency-free, and on-brand. Layout maths that
 * doesn't need a live canvas (word wrap, which elements show) is factored into
 * pure functions so it can be unit-tested; `paintShareCard` issues the actual
 * draw calls and is exercised against a mock context in tests.
 *
 * Brand tokens mirror app/tailwind.config.js: ocean primary #4A83DD, Inter for
 * body copy, Cabinet Grotesk for the wordmark (falls back to Inter/sans if the
 * font is not yet loaded in the webview).
 */

/** 16:9 — the aspect ratio both X and LinkedIn preview cleanly. */
export const CARD_WIDTH = 1200;
export const CARD_HEIGHT = 675;

const OCEAN = '#4A83DD';
const OCEAN_DEEP = '#2C5AA8';
const INK = '#0E1726';

const LOG_PREFIX = '[share-card]';

interface ShareCardData {
  /** Punchy headline describing what the agent did. */
  headline: string;
  /** Agent / profile name shown in the footer. */
  agentName: string;
  /** Optional stat chip, e.g. "12s" or "3 files". */
  stat?: string;
  /** Marketing URL printed in the footer. */
  brandUrl: string;
}

interface ShareCardModel {
  headlineLines: string[];
  agentName: string;
  stat: string | null;
  brandUrl: string;
}

/**
 * Greedy word-wrap by character budget. Deterministic and canvas-free so the
 * layout is unit-testable. Words longer than `maxChars` are hard-split. Caps at
 * `maxLines`, ellipsising the final line when content overflows.
 */
export function wrapLines(text: string, maxChars: number, maxLines: number): string[] {
  const words = text.trim().split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let current = '';

  const pushWord = (word: string) => {
    if (!current) {
      current = word;
    } else if (current.length + 1 + word.length <= maxChars) {
      current += ` ${word}`;
    } else {
      lines.push(current);
      current = word;
    }
  };

  for (let word of words) {
    while (word.length > maxChars) {
      // Hard-split an over-long token (e.g. a URL slug).
      if (current) {
        lines.push(current);
        current = '';
      }
      lines.push(word.slice(0, maxChars));
      word = word.slice(maxChars);
    }
    pushWord(word);
  }
  if (current) lines.push(current);

  if (lines.length <= maxLines) return lines;
  const clipped = lines.slice(0, maxLines);
  const last = clipped[maxLines - 1];
  clipped[maxLines - 1] =
    last.length > maxChars - 1 ? `${last.slice(0, maxChars - 1)}…` : `${last}…`;
  return clipped;
}

/** Roughly how many headline characters fit one line at the card's title size. */
const HEADLINE_CHARS_PER_LINE = 26;
const HEADLINE_MAX_LINES = 4;

/** Localized fallbacks for {@link computeCardModel}. See `share.default*` in `en.ts`. */
export interface ShareCardFallbacks {
  /** Shown when `data.headline` is empty/blank. */
  headline: string;
  /** Shown when `data.agentName` is empty/blank. */
  agentName: string;
}

/** English defaults, used when a caller doesn't pass locale-aware fallbacks (e.g. tests). */
const DEFAULT_CARD_FALLBACKS: ShareCardFallbacks = {
  headline: 'Look what my OpenHuman agent just did',
  agentName: 'My agent',
};

/**
 * Builds the pure layout model for a card: wrapped headline lines plus the
 * normalised footer/stat fields. No canvas required.
 *
 * `fallbacks` should be sourced from `useT()` at the UI boundary so non-English
 * users don't get English text on the card; it defaults to English for callers
 * that don't have a locale (tests, non-UI callers).
 */
export function computeCardModel(
  data: ShareCardData,
  fallbacks: ShareCardFallbacks = DEFAULT_CARD_FALLBACKS
): ShareCardModel {
  const headline = data.headline.trim() || fallbacks.headline;
  return {
    headlineLines: wrapLines(headline, HEADLINE_CHARS_PER_LINE, HEADLINE_MAX_LINES),
    agentName: data.agentName.trim() || fallbacks.agentName,
    stat: data.stat?.trim() ? data.stat.trim() : null,
    brandUrl: data.brandUrl.trim(),
  };
}

/**
 * Minimal structural subset of `CanvasRenderingContext2D` that `paintShareCard`
 * uses. Declaring it lets tests pass a lightweight mock and lets us paint
 * without depending on a real DOM canvas.
 */
export interface CardPaintContext {
  fillStyle: string | CanvasGradient;
  strokeStyle: string | CanvasGradient;
  lineWidth: number;
  font: string;
  textBaseline: string;
  textAlign: string;
  globalAlpha: number;
  fillRect(x: number, y: number, w: number, h: number): void;
  fillText(text: string, x: number, y: number): void;
  beginPath(): void;
  arc(x: number, y: number, r: number, start: number, end: number): void;
  fill(): void;
  roundRect?(x: number, y: number, w: number, h: number, radius: number): void;
  rect(x: number, y: number, w: number, h: number): void;
  createLinearGradient(x0: number, y0: number, x1: number, y1: number): CanvasGradient;
}

const FONT_SANS =
  'Inter, "Cabinet Grotesk", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
const FONT_DISPLAY = '"Cabinet Grotesk", Inter, sans-serif';

/**
 * Paints the full card onto a 2D context sized `CARD_WIDTH`×`CARD_HEIGHT`.
 * Pure with respect to `data` — same input paints the same pixels.
 */
export function paintShareCard(
  ctx: CardPaintContext,
  data: ShareCardData,
  fallbacks: ShareCardFallbacks = DEFAULT_CARD_FALLBACKS
): void {
  const model = computeCardModel(data, fallbacks);

  // Background: ocean vertical gradient.
  const bg = ctx.createLinearGradient(0, 0, 0, CARD_HEIGHT);
  bg.addColorStop(0, OCEAN);
  bg.addColorStop(1, OCEAN_DEEP);
  ctx.fillStyle = bg;
  ctx.fillRect(0, 0, CARD_WIDTH, CARD_HEIGHT);

  const pad = 72;

  // Brand wordmark: a dot + "OpenHuman".
  ctx.beginPath();
  ctx.fillStyle = '#FFFFFF';
  ctx.arc(pad + 12, pad + 6, 12, 0, Math.PI * 2);
  ctx.fill();
  ctx.textBaseline = 'alphabetic';
  ctx.textAlign = 'left';
  ctx.font = `700 34px ${FONT_DISPLAY}`;
  ctx.fillStyle = '#FFFFFF';
  ctx.fillText('OpenHuman', pad + 36, pad + 18);

  // Headline block.
  ctx.font = `700 62px ${FONT_SANS}`;
  ctx.fillStyle = '#FFFFFF';
  const lineHeight = 74;
  const blockHeight = model.headlineLines.length * lineHeight;
  let y = Math.max(pad + 150, (CARD_HEIGHT - blockHeight) / 2);
  for (const line of model.headlineLines) {
    ctx.fillText(line, pad, y);
    y += lineHeight;
  }

  // Optional stat chip below the headline.
  if (model.stat) {
    const chipY = y + 8;
    ctx.globalAlpha = 0.18;
    ctx.fillStyle = '#FFFFFF';
    drawRoundedRect(ctx, pad, chipY, 260, 56, 28);
    ctx.globalAlpha = 1;
    ctx.fillStyle = '#FFFFFF';
    ctx.font = `600 30px ${FONT_SANS}`;
    ctx.fillText(model.stat, pad + 28, chipY + 38);
  }

  // Footer: agent name + marketing URL.
  ctx.font = `600 30px ${FONT_SANS}`;
  ctx.fillStyle = 'rgba(255,255,255,0.92)';
  ctx.fillText(model.agentName, pad, CARD_HEIGHT - pad + 6);
  if (model.brandUrl) {
    ctx.textAlign = 'right';
    ctx.font = `500 28px ${FONT_SANS}`;
    ctx.fillStyle = 'rgba(255,255,255,0.82)';
    ctx.fillText(model.brandUrl, CARD_WIDTH - pad, CARD_HEIGHT - pad + 6);
    ctx.textAlign = 'left';
  }

  // Faint ink vignette in the bottom-right for depth (kept subtle).
  ctx.globalAlpha = 0.06;
  ctx.fillStyle = INK;
  ctx.fillRect(CARD_WIDTH - 4, 0, 4, CARD_HEIGHT);
  ctx.globalAlpha = 1;
}

function drawRoundedRect(
  ctx: CardPaintContext,
  x: number,
  y: number,
  w: number,
  h: number,
  radius: number
): void {
  ctx.beginPath();
  if (typeof ctx.roundRect === 'function') {
    ctx.roundRect(x, y, w, h, radius);
  } else {
    ctx.rect(x, y, w, h);
  }
  ctx.fill();
}

/**
 * Renders the card into a real DOM canvas (sizing it and acquiring the 2D
 * context). Throws if the context can't be acquired. Browser/Tauri only.
 */
export function renderShareCardToCanvas(
  canvas: HTMLCanvasElement,
  data: ShareCardData,
  fallbacks: ShareCardFallbacks = DEFAULT_CARD_FALLBACKS
): void {
  console.debug(`${LOG_PREFIX} render start w=${CARD_WIDTH} h=${CARD_HEIGHT}`);
  canvas.width = CARD_WIDTH;
  canvas.height = CARD_HEIGHT;
  const ctx = canvas.getContext('2d');
  if (!ctx) {
    console.debug(`${LOG_PREFIX} render failed: no 2d context`);
    throw new Error('2d canvas context unavailable');
  }
  try {
    paintShareCard(ctx as unknown as CardPaintContext, data, fallbacks);
    console.debug(`${LOG_PREFIX} render ok`);
  } catch (err) {
    console.debug(
      `${LOG_PREFIX} render failed err_type=${err instanceof Error ? err.name : typeof err}`
    );
    throw err;
  }
}

/** Serialises a canvas to a PNG blob. Rejects if encoding fails. */
export function cardToPngBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  console.debug(`${LOG_PREFIX} png export start`);
  return new Promise((resolve, reject) => {
    canvas.toBlob(blob => {
      if (blob) {
        console.debug(`${LOG_PREFIX} png export ok bytes=${blob.size}`);
        resolve(blob);
      } else {
        console.debug(`${LOG_PREFIX} png export failed: toBlob returned null`);
        reject(new Error('canvas.toBlob returned null'));
      }
    }, 'image/png');
  });
}
