import { describe, expect, it } from 'vitest';

import { sanitizeSvg } from './sanitizeSvg';

describe('sanitizeSvg', () => {
  it('returns empty string for empty input', () => {
    expect(sanitizeSvg('')).toBe('');
  });

  it('preserves benign SVG markup', () => {
    const svg = '<g id="m-bob"><path d="M0 0" fill="red"/></g>';
    expect(sanitizeSvg(svg)).toBe(svg);
  });

  it('strips <script> elements and their content', () => {
    const out = sanitizeSvg('<g><script>alert(1)</script><path d="M0 0"/></g>');
    expect(out).not.toContain('script');
    expect(out).toContain('<path d="M0 0"/>');
  });

  it('strips <foreignObject> elements (the innerHTML execution sink)', () => {
    const out = sanitizeSvg(
      '<foreignObject><img src="x" onerror="alert(1)"></foreignObject><path/>'
    );
    expect(out.toLowerCase()).not.toContain('foreignobject');
    expect(out).not.toContain('onerror');
    expect(out).toContain('<path/>');
  });

  it('strips inline event-handler attributes', () => {
    const out = sanitizeSvg('<circle onload="steal()" onclick=\'x\' r="5"/>');
    expect(out).not.toContain('onload');
    expect(out).not.toContain('onclick');
    expect(out).toContain('r="5"');
  });

  it('strips javascript: and data:text/html URLs in href/xlink:href', () => {
    const out = sanitizeSvg(
      '<a href="javascript:alert(1)"><use xlink:href="data:text/html,<x>"/></a>'
    );
    expect(out.toLowerCase()).not.toContain('javascript:');
    expect(out.toLowerCase()).not.toContain('data:text/html');
  });

  it('leaves a fragment-only xlink:href intact', () => {
    const svg = '<use xlink:href="#m-mouth"/>';
    expect(sanitizeSvg(svg)).toBe(svg);
  });
});
