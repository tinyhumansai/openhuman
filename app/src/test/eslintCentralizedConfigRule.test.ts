import tsparser from '@typescript-eslint/parser';
import { Linter } from 'eslint';
import { describe, expect, it } from 'vitest';

// eslint.config.js is plain JS and ships no declarations; importing the real
// file is the point of this test, so the shape is asserted below at runtime.
// @ts-expect-error -- untyped JS module
import eslintConfig from '../../eslint.config.js';

/**
 * Guards the `no-restricted-syntax` selector that keeps frontend config reads
 * funnelled through `src/utils/config.ts`. The selector is easy to get subtly
 * wrong in both directions -- `new.target` is a MetaProperty just like
 * `import.meta`, and computed access stores the key on `property.value` rather
 * than `property.name` -- so the exact boundary is pinned here.
 *
 * The selector and the ignore list are read out of the real `eslint.config.js`
 * rather than restated, so these assertions cannot drift away from what ships.
 * Linting runs through a bare `Linter` because the repo config is type-aware
 * and `parserOptions.project` rejects the synthetic file used for the probes.
 */

const configBlock = (eslintConfig as Linter.Config[]).find(
  block => block.rules?.['no-restricted-syntax']
);

const [, restriction] = (configBlock?.rules?.['no-restricted-syntax'] ?? []) as [
  string,
  { selector: string; message: string },
];

const linter = new Linter();

function lint(code: string) {
  const messages = linter.verify(code, {
    languageOptions: { parser: tsparser, ecmaVersion: 'latest', sourceType: 'module' },
    rules: { 'no-restricted-syntax': ['error', restriction] },
  });
  // A parse failure would otherwise read as "the rule did not fire".
  expect(messages.filter(message => message.fatal)).toEqual([]);
  return messages;
}

describe('centralized-frontend-config lint rule', () => {
  it('is wired into the shipped config', () => {
    expect(restriction?.selector).toBeTypeOf('string');
    expect(configBlock?.ignores).toContain('src/utils/config.ts');
    expect(configBlock?.ignores).toContain('src/test/**');
  });

  it('rejects dotted import.meta.env access', () => {
    expect(lint('export const a = import.meta.env.DEV;')).toHaveLength(1);
  });

  it('rejects computed import.meta["env"] access', () => {
    expect(lint("export const a = import.meta['env'].DEV;")).toHaveLength(1);
  });

  it('allows other import.meta properties', () => {
    expect(lint('export const a = import.meta.url;')).toHaveLength(0);
  });

  it('allows new.target.env, which is a different meta-property', () => {
    expect(lint('export function G() { return new.target.env; }')).toHaveLength(0);
  });

  it('allows a plain object property named env', () => {
    expect(lint('const o = { env: 1 }; export const a = o.env;')).toHaveLength(0);
  });
});
