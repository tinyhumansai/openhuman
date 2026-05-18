import id1 from './chunks/id-1';
import id2 from './chunks/id-2';
import id3 from './chunks/id-3';
import id4 from './chunks/id-4';
import id5 from './chunks/id-5';
import type { TranslationMap } from './types';

// Bahasa Indonesia translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const id: TranslationMap = { ...id1, ...id2, ...id3, ...id4, ...id5 };

export default id;
