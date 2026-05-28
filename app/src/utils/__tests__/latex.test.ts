import { describe, expect, it } from 'vitest';

import { hasLatexContent, normalizeLatexDelimiters } from '../latex';

describe('normalizeLatexDelimiters', () => {
  it('converts \\[ ... \\] to $$ ... $$', () => {
    expect(normalizeLatexDelimiters('\\[ x^2 + y^2 = z^2 \\]')).toContain('$$ x^2 + y^2 = z^2 $$');
  });

  it('converts \\( ... \\) to $ ... $', () => {
    expect(normalizeLatexDelimiters('inline \\(a+b\\) here')).toBe('inline $a+b$ here');
  });

  it('converts bare bracketed LaTeX-only block to $$', () => {
    const input =
      '直接套公式:\n\n[ V_3 = (x_2 - x_1)(x_3 - x_1)(x_3 - x_2) = 1 \\times 3 \\times 2 = 6 ]';
    const out = normalizeLatexDelimiters(input);
    expect(out).toContain('$$');
    expect(out).toContain('\\times');
    expect(out).not.toMatch(/\[ V_3/);
  });

  it('preserves markdown link syntax', () => {
    const input = 'see [link](https://example.com) and more';
    expect(normalizeLatexDelimiters(input)).toBe(input);
  });

  it('returns input unchanged when no LaTeX present', () => {
    const input = 'plain text with no math';
    expect(normalizeLatexDelimiters(input)).toBe(input);
  });

  it('handles vmatrix blocks', () => {
    const input = '[ \\begin{vmatrix} 1 & 2 \\\\ 3 & 4 \\end{vmatrix} = -2 ]';
    const out = normalizeLatexDelimiters(input);
    expect(out).toContain('$$');
    expect(out).toContain('\\begin{vmatrix}');
  });

  it('preserves \\[...\\] inside inline code spans', () => {
    const input = 'use `\\[x^2\\]` for display math and \\[a+b\\] renders';
    const out = normalizeLatexDelimiters(input);
    expect(out).toContain('`\\[x^2\\]`');
    expect(out).toContain('$$a+b$$');
  });

  it('preserves \\(...\\) inside inline code spans', () => {
    const input = 'use `\\(x\\)` for inline math and \\(a+b\\) renders';
    const out = normalizeLatexDelimiters(input);
    expect(out).toContain('`\\(x\\)`');
    expect(out).toContain('$a+b$');
  });

  it('preserves LaTeX delimiters inside fenced code blocks', () => {
    const input = '```\n\\[x^2\\]\n\\(y\\)\n```\n\nthen \\(a+b\\) here';
    const out = normalizeLatexDelimiters(input);
    expect(out).toContain('```\n\\[x^2\\]\n\\(y\\)\n```');
    expect(out).toContain('$a+b$');
  });

  it('does not corrupt math bodies containing digits when code blocks present', () => {
    const input = 'use `x` and \\[a^2 + 7\\] and `y` and \\(b_3\\)';
    const out = normalizeLatexDelimiters(input);
    expect(out).toContain('`x`');
    expect(out).toContain('`y`');
    expect(out).toContain('$$a^2 + 7$$');
    expect(out).toContain('$b_3$');
    expect(out).not.toContain('undefined');
  });
});

describe('hasLatexContent', () => {
  it('detects backslash math commands', () => {
    expect(hasLatexContent('use \\frac{1}{2} here')).toBe(true);
    expect(hasLatexContent('\\begin{vmatrix} 1 \\end{vmatrix}')).toBe(true);
    expect(hasLatexContent('\\[ a \\]')).toBe(true);
    expect(hasLatexContent('\\(a\\)')).toBe(true);
    expect(hasLatexContent('$$x$$')).toBe(true);
  });

  it('rejects currency mentions', () => {
    expect(hasLatexContent('total is $10 and $20')).toBe(false);
    expect(hasLatexContent('$100')).toBe(false);
  });

  it('rejects plain text and markdown', () => {
    expect(hasLatexContent('see [link](https://example.com)')).toBe(false);
    expect(hasLatexContent('hello world')).toBe(false);
    expect(hasLatexContent('')).toBe(false);
  });
});
