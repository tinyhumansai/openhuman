import { describe, expect, it } from 'vitest';
// @ts-expect-error — pure ESM module under repo-root scripts/, no .d.ts
import { parseMatrix, validateAgainstCatalog } from '../../../scripts/lib/coverage-matrix-parser.mjs';

describe('parseMatrix', () => {
  it('parses three valid rows including a 4-component ID', () => {
    const md = `Some intro text.

| 0.1.1 | Auth login | RU | src/auth/login.rs | ✅ | covered |
| 3.3.1.1 | Voice hotkey | WD | app/test/e2e/specs/voice.spec.ts | 🟡 | partial |
| 13.5.3 | Release smoke | MS | docs/RELEASE-MANUAL-SMOKE.md | 🚫 | manual only |

Trailing prose.`;
    const result = parseMatrix(md);
    expect(result.errors).toEqual([]);
    expect(result.rows).toHaveLength(3);
    expect(result.rows[0].id).toBe('0.1.1');
    expect(result.rows[1].id).toBe('3.3.1.1');
    expect(result.rows[2].status).toBe('🚫');
  });

  it('flags duplicate IDs and invalid statuses', () => {
    const md = `| 1.1.1 | First | RU | a.rs | ✅ | ok |
| 1.1.1 | Duplicate | VU | b.ts | ✅ | second copy |
| 2.2.2 | Bad status | WD | c.spec.ts | ⚠️ | not a real legend |`;
    const parsed = parseMatrix(md);
    expect(parsed.errors.some((e) => e.includes('invalid status'))).toBe(true);
    const validation = validateAgainstCatalog(parsed.rows, ['1.1.1', '2.2.2', '9.9.9']);
    expect(validation.duplicates).toEqual(['1.1.1']);
    expect(validation.missingFromMatrix).toEqual(['9.9.9']);
  });
});
