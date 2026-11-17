/**
 * WebKit before Safari 16.4 cannot parse a regex lookbehind: the literal throws
 * `SyntaxError: Invalid regular expression: invalid group specifier name` as
 * soon as the module that holds it is evaluated, which took the chat's markdown
 * renderer down for those users (Sentry TAURI-REACT-A7/AB/AR/AQ/AZ).
 *
 * The culprit is `mdast-util-gfm-autolink-literal` (pulled in by `remark-gfm`),
 * whose email autolink regex opens with `(?<=^|\s|\p{P}|\p{S})`. We patch it out
 * (`app/patches/mdast-util-gfm-autolink-literal@2.0.1.patch`); `findEmail`
 * already enforces the same boundary through `previous(match, true)`.
 *
 * jsdom runs on V8, which supports lookbehind, so the regression is asserted on
 * the shipped source itself; the rendering cases prove the patch kept GFM's
 * email autolink behaviour.
 */
import { TextMessagePartProvider } from '@assistant-ui/react';
import { render, waitFor } from '@testing-library/react';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { MarkdownText } from '../markdown-text';

/** Resolve `chain[last]` the way the bundler does: each from its dependent. */
function resolvePackageDir(chain: string[]): string {
  let from = import.meta.url;
  let resolved = '';
  for (const name of chain) {
    resolved = createRequire(from).resolve(name);
    from = resolved;
  }
  let dir = dirname(resolved);
  for (;;) {
    try {
      const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8')) as { name?: string };
      if (pkg.name === chain[chain.length - 1]) return dir;
    } catch {
      // keep walking up
    }
    const up = dirname(dir);
    if (up === dir) throw new Error(`package dir not found for ${chain.join(' > ')}`);
    dir = up;
  }
}

function jsSources(dir: string): string[] {
  return readdirSync(dir).flatMap(entry => {
    const path = join(dir, entry);
    if (entry === 'node_modules') return [];
    if (statSync(path).isDirectory()) return jsSources(path);
    return /\.(m?js|cjs)$/.test(entry) ? [path] : [];
  });
}

async function renderMarkdown(text: string) {
  const { container } = render(
    <TextMessagePartProvider text={text} isRunning={false}>
      <MarkdownText />
    </TextMessagePartProvider>
  );
  await waitFor(() => expect(container.querySelector('.aui-md')?.innerHTML).toBeTruthy());
  return container;
}

describe('MarkdownText — no regex lookbehind for WebKit < 16.4', () => {
  it('ships mdast-util-gfm-autolink-literal without a lookbehind', () => {
    const dir = resolvePackageDir([
      'remark-gfm',
      'mdast-util-gfm',
      'mdast-util-gfm-autolink-literal',
    ]);
    const sources = jsSources(dir);
    expect(sources.length).toBeGreaterThan(0);

    const offenders = sources.filter(file => /\(\?<[=!]/.test(readFileSync(file, 'utf8')));
    expect(offenders).toEqual([]);
  });

  it('still autolinks a bare email address', async () => {
    const container = await renderMarkdown('mail a.b@c.com');

    const link = container.querySelector('a[href="mailto:a.b@c.com"]');
    expect(link?.textContent).toBe('a.b@c.com');
  });

  it('does not autolink an email that follows a slash', async () => {
    const container = await renderMarkdown('x/a@b.com');

    expect(container.querySelector('a[href^="mailto:"]')).toBeNull();
    expect(container.textContent).toContain('x/a@b.com');
  });
});
