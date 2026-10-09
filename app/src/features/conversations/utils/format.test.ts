import { describe, expect, it } from 'vitest';

import { classifyHref, isAllowedExternalHref, transformChatUrl } from './format';

describe('classifyHref', () => {
  it.each([
    ['https://pay.example.com/o/1', 'external'],
    ['http://example.com', 'external'],
    ['mailto:me@example.com', 'external'],
    ['phonepe://pay?id=1', 'handoff'],
    ['upi://pay?pa=shop@bank&am=10', 'handoff'],
    ['tez://upi/pay', 'handoff'],
    ['workspace:docs/a.md', 'workspace'],
    ['#citation-1', 'relative'],
    ['javascript:alert(1)', 'blocked'],
    ['JavaScript://%0Aalert(1)', 'blocked'],
    ['data:text/html,hi', 'blocked'],
    ['file:///etc/passwd', 'blocked'],
    ['tauri://localhost', 'blocked'],
    ['ohwidget://localhost/proxy', 'blocked'],
    ['about:blank', 'blocked'],
    ['upi:pay', 'blocked'],
    ['https://', 'blocked'],
    ['https://a.com/\nx', 'blocked'],
    ['', 'blocked'],
  ])('%s is %s', (href, expected) => {
    expect(classifyHref(href)).toBe(expected);
  });

  it('keeps handoff links in markdown and blanks dangerous ones', () => {
    expect(transformChatUrl('phonepe://pay?id=1')).toBe('phonepe://pay?id=1');
    expect(transformChatUrl('javascript:alert(1)')).toBe('');
    expect(transformChatUrl('#citation-2')).toBe('#citation-2');
  });

  it('only treats http(s) and mailto as openable', () => {
    expect(isAllowedExternalHref('https://a.com')).toBe(true);
    expect(isAllowedExternalHref('phonepe://pay')).toBe(false);
  });
});
