import pt1 from './chunks/pt-1';
import pt2 from './chunks/pt-2';
import pt3 from './chunks/pt-3';
import pt4 from './chunks/pt-4';
import pt5 from './chunks/pt-5';
import type { TranslationMap } from './types';

// Portuguese (Português) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const pt: TranslationMap = { ...pt1, ...pt2, ...pt3, ...pt4, ...pt5 };

export default pt;
