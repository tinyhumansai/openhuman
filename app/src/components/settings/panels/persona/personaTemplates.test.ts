import { describe, expect, it } from 'vitest';

import { parsePersonaFields } from './personaSections';
import { applyTemplate, PERSONA_TEMPLATES } from './personaTemplates';

const SOUL = `# OpenHuman

You are OpenHuman.

## Personality

- Old trait.

## Voice

- Old voice.

## When things go wrong

- Own it.
`;

describe('PERSONA_TEMPLATES', () => {
  it('covers the six requested roles with unique ids and content', () => {
    const ids = PERSONA_TEMPLATES.map(t => t.id);
    expect(ids).toEqual(['doctor', 'researcher', 'executive', 'teacher', 'student', 'family']);
    expect(new Set(ids).size).toBe(ids.length);
    for (const tpl of PERSONA_TEMPLATES) {
      expect(tpl.fields.personality.trim().length).toBeGreaterThan(0);
      expect(tpl.fields.voice.trim().length).toBeGreaterThan(0);
      expect(tpl.labelKey).toMatch(/^settings\.persona\.templates\./);
      expect(tpl.descriptionKey).toMatch(/^settings\.persona\.templates\./);
    }
  });
});

describe('applyTemplate', () => {
  const doctor = PERSONA_TEMPLATES[0];

  it('writes the template Personality and Voice into SOUL.md', () => {
    const next = applyTemplate(SOUL, doctor);
    const fields = parsePersonaFields(next);
    expect(fields.personality).toBe(doctor.fields.personality);
    expect(fields.voice).toBe(doctor.fields.voice);
  });

  it('preserves unmanaged sections and the user-owned About You section', () => {
    const withAbout = applyTemplate(SOUL, doctor).replace(
      /\n*$/,
      '\n\n## About You\n\nI am a nurse.\n'
    );
    const next = applyTemplate(withAbout, PERSONA_TEMPLATES[2]); // executive
    expect(next).toContain('## When things go wrong\n\n- Own it.');
    expect(parsePersonaFields(next).about).toBe('I am a nurse.');
    expect(parsePersonaFields(next).personality).toBe(PERSONA_TEMPLATES[2].fields.personality);
  });

  it('is idempotent when the same template is applied twice', () => {
    const once = applyTemplate(SOUL, doctor);
    expect(applyTemplate(once, doctor)).toBe(once);
  });
});
