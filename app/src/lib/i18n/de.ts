import de1 from './chunks/de-1';
import de2 from './chunks/de-2';
import de3 from './chunks/de-3';
import de4 from './chunks/de-4';
import de5 from './chunks/de-5';
import type { TranslationMap } from './types';

// German (Deutsch) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const de: TranslationMap = { ...de1, ...de2, ...de3, ...de4, ...de5 };

export default de;
