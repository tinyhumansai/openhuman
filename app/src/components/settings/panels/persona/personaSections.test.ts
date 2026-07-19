import { describe, expect, it } from 'vitest';

import { applyPersonaField, applyPersonaFields, parsePersonaFields } from './personaSections';

const SOUL = `# OpenHuman

You are OpenHuman.

## Personality

- Warm
- Direct

## Voice

- Lead with the answer.

## When things go wrong

- Own it.
`;

describe('parsePersonaFields', () => {
  it('reads the managed sections and leaves About empty when absent', () => {
    const fields = parsePersonaFields(SOUL);
    expect(fields.personality).toBe('- Warm\n- Direct');
    expect(fields.voice).toBe('- Lead with the answer.');
    expect(fields.about).toBe('');
  });

  it('does not match a deeper or differently-named heading', () => {
    const text = '## Personality Traits\n\nfoo\n\n### Personality\n\nbar\n';
    expect(parsePersonaFields(text).personality).toBe('');
  });

  it('includes nested h3 content but stops at the next h2', () => {
    const text = '## Personality\n\n- a\n### sub\n- b\n\n## Voice\n\nx\n';
    expect(parsePersonaFields(text).personality).toBe('- a\n### sub\n- b');
    expect(parsePersonaFields(text).voice).toBe('x');
  });
});

describe('applyPersonaField', () => {
  it('is a no-op (identical string) when the value is unchanged', () => {
    expect(applyPersonaField(SOUL, 'personality', '- Warm\n- Direct')).toBe(SOUL);
    // trimming differences also count as unchanged
    expect(applyPersonaField(SOUL, 'voice', '  - Lead with the answer.  ')).toBe(SOUL);
  });

  it('replaces only the target section and preserves every other byte', () => {
    const next = applyPersonaField(SOUL, 'voice', 'Be terse.');
    expect(parsePersonaFields(next).voice).toBe('Be terse.');
    // untouched sections are byte-identical
    expect(next).toContain('## Personality\n\n- Warm\n- Direct');
    expect(next).toContain('## When things go wrong\n\n- Own it.');
    // and re-applying the original value restores the exact original document
    expect(applyPersonaField(next, 'voice', '- Lead with the answer.')).toBe(SOUL);
  });

  it('appends a new section when the managed heading is absent', () => {
    const next = applyPersonaField(SOUL, 'about', 'I design things.');
    expect(next.startsWith(SOUL.replace(/\n*$/, '\n'))).toBe(true);
    expect(next).toContain('## About You\n\nI design things.\n');
    expect(parsePersonaFields(next).about).toBe('I design things.');
  });

  it('empties the body but keeps the heading when cleared', () => {
    const next = applyPersonaField(SOUL, 'voice', '');
    expect(parsePersonaFields(next).voice).toBe('');
    expect(next).toContain('## Voice');
    expect(next).toContain('## When things go wrong');
  });
});

describe('applyPersonaFields round-trip', () => {
  it('is idempotent when nothing changed', () => {
    expect(applyPersonaFields(SOUL, parsePersonaFields(SOUL))).toBe(SOUL);
  });

  it('round-trips edited fields through parse → apply', () => {
    const edited = { personality: 'Calm.', voice: 'Brief.', about: 'A designer.' };
    const next = applyPersonaFields(SOUL, edited);
    expect(parsePersonaFields(next)).toEqual(edited);
    // applying the parsed fields again changes nothing further
    expect(applyPersonaFields(next, parsePersonaFields(next))).toBe(next);
  });
});
