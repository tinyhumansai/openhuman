import { describe, expect, it } from 'vitest';

import { extensionFor } from './artifactExtension';

describe('extensionFor', () => {
  it('maps document artifacts to docx (GH #4847, was pdf)', () => {
    // The regression this guards: a `document` artifact carrying no
    // explicit extension in its title must export as `.docx` — the
    // format `generate_document` actually emits — not the stale `pdf`.
    expect(extensionFor('document', 'Quarterly Report')).toBe('docx');
  });

  it('maps the other known kinds to their default extensions', () => {
    expect(extensionFor('presentation', 'Q3 Deck')).toBe('pptx');
    expect(extensionFor('image', 'Chart')).toBe('png');
    expect(extensionFor('other', 'Blob')).toBe('bin');
  });

  it('prefers an explicit extension already present on the title', () => {
    // The title-carried extension wins over the per-kind default, so a
    // legacy pdf document still round-trips as pdf.
    expect(extensionFor('document', 'legacy.pdf')).toBe('pdf');
    expect(extensionFor('document', 'notes.DOCX')).toBe('docx');
    expect(extensionFor('image', 'photo.jpeg')).toBe('jpeg');
  });

  it('ignores a leading or trailing dot that is not a real extension', () => {
    // No extension segment → fall back to the kind default.
    expect(extensionFor('document', '.hidden')).toBe('docx');
    expect(extensionFor('document', 'trailing.')).toBe('docx');
  });
});
