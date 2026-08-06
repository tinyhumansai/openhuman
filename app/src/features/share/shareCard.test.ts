import { describe, expect, test, vi } from 'vitest';

import {
  CARD_HEIGHT,
  CARD_WIDTH,
  type CardPaintContext,
  computeCardModel,
  paintShareCard,
  wrapLines,
} from './shareCard';

describe('wrapLines', () => {
  test('wraps on word boundaries within the char budget', () => {
    const lines = wrapLines('the quick brown fox jumps', 10, 5);
    expect(lines.every(l => l.length <= 10)).toBe(true);
    expect(lines.join(' ')).toBe('the quick brown fox jumps');
  });

  test('hard-splits an over-long token', () => {
    const lines = wrapLines('supercalifragilisticexpialidocious', 8, 6);
    expect(lines[0].length).toBe(8);
  });

  test('caps at maxLines and ellipsises the overflow', () => {
    const lines = wrapLines('a b c d e f g h i j k', 3, 2);
    expect(lines.length).toBe(2);
    expect(lines[1].endsWith('…')).toBe(true);
  });
});

describe('computeCardModel', () => {
  test('wraps the headline and normalises fields', () => {
    const model = computeCardModel({
      headline: 'My agent summarised three months of emails in twelve seconds',
      agentName: 'Tiny',
      stat: '12s',
      brandUrl: 'tinyhumans.ai',
    });
    expect(model.headlineLines.length).toBeGreaterThan(0);
    expect(model.agentName).toBe('Tiny');
    expect(model.stat).toBe('12s');
  });

  test('supplies defaults for empty headline / agent / stat', () => {
    const model = computeCardModel({ headline: '  ', agentName: '', brandUrl: '' });
    expect(model.headlineLines.join(' ')).toContain('OpenHuman');
    expect(model.agentName).toBe('My agent');
    expect(model.stat).toBeNull();
  });

  test('uses caller-supplied localized fallbacks instead of English', () => {
    const model = computeCardModel(
      { headline: '  ', agentName: '', brandUrl: '' },
      { headline: 'Mira lo que hizo mi agente', agentName: 'Mi agente' }
    );
    expect(model.headlineLines.join(' ')).toContain('Mira lo que hizo mi agente');
    expect(model.agentName).toBe('Mi agente');
  });
});

function makeMockCtx(): CardPaintContext & { texts: string[] } {
  const texts: string[] = [];
  return {
    texts,
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 0,
    font: '',
    textBaseline: '',
    textAlign: '',
    globalAlpha: 1,
    fillRect: vi.fn(),
    fillText: (t: string) => {
      texts.push(t);
    },
    beginPath: vi.fn(),
    arc: vi.fn(),
    fill: vi.fn(),
    rect: vi.fn(),
    roundRect: vi.fn(),
    createLinearGradient: () => ({ addColorStop: vi.fn() }) as unknown as CanvasGradient,
  };
}

describe('paintShareCard', () => {
  test('draws the brand, headline, stat, and footer', () => {
    const ctx = makeMockCtx();
    paintShareCard(ctx, {
      headline: 'Cleared my inbox',
      agentName: 'Tiny',
      stat: '12s',
      brandUrl: 'tinyhumans.ai',
    });
    const joined = ctx.texts.join('\n');
    expect(joined).toContain('OpenHuman');
    expect(joined).toContain('Cleared my inbox');
    expect(joined).toContain('12s');
    expect(joined).toContain('Tiny');
    expect(joined).toContain('tinyhumans.ai');
  });

  test('omits the stat chip when no stat given', () => {
    const ctx = makeMockCtx();
    paintShareCard(ctx, { headline: 'Did a thing', agentName: 'Tiny', brandUrl: 'tinyhumans.ai' });
    // With no roundRect stat chip, roundRect is never called.
    expect(ctx.roundRect).not.toHaveBeenCalled();
  });

  test('card dimensions are 16:9', () => {
    expect(CARD_WIDTH / CARD_HEIGHT).toBeCloseTo(16 / 9, 1);
  });
});
