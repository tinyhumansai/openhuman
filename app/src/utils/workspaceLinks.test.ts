import { describe, expect, test } from 'vitest';

import { isWorkspaceHref, parseWorkspaceHref } from './workspaceLinks';

describe('workspaceLinks', () => {
  test('parses workspace: links into normalized workspace-relative paths', () => {
    expect(parseWorkspaceHref('workspace:memory_tree/content/Daily%20Note.md')).toEqual({
      path: 'memory_tree/content/Daily Note.md',
    });
    expect(parseWorkspaceHref('workspace://memory_tree/content/summary.md')).toEqual({
      path: 'memory_tree/content/summary.md',
    });
    expect(parseWorkspaceHref('openhuman-workspace:/docs/readme.md')).toEqual({
      path: 'docs/readme.md',
    });
  });

  test('rejects non-workspace links and traversal payloads', () => {
    expect(parseWorkspaceHref('https://example.com/docs')).toBeNull();
    expect(parseWorkspaceHref('file:///etc/passwd')).toBeNull();
    expect(parseWorkspaceHref('workspace:../secret.txt')).toBeNull();
    expect(parseWorkspaceHref('workspace:docs/%2e%2e/secret.txt')).toBeNull();
    expect(parseWorkspaceHref('workspace:docs/%00secret.txt')).toBeNull();
    expect(parseWorkspaceHref('workspace:C:/Users/me/secret.txt')).toBeNull();
  });

  test('identifies workspace links without allowing unsafe paths', () => {
    expect(isWorkspaceHref('workspace:docs/plan.md')).toBe(true);
    expect(isWorkspaceHref('workspace:../plan.md')).toBe(false);
    expect(isWorkspaceHref('mailto:support@example.com')).toBe(false);
  });
});
