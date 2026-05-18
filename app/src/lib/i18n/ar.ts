import ar1 from './chunks/ar-1';
import ar2 from './chunks/ar-2';
import ar3 from './chunks/ar-3';
import ar4 from './chunks/ar-4';
import ar5 from './chunks/ar-5';
import type { TranslationMap } from './types';

// Arabic (العربية) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const ar: TranslationMap = { ...ar1, ...ar2, ...ar3, ...ar4, ...ar5 };

export default ar;
