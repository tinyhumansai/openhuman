import it1 from './chunks/it-1';
import it2 from './chunks/it-2';
import it3 from './chunks/it-3';
import it4 from './chunks/it-4';
import it5 from './chunks/it-5';
import type { TranslationMap } from './types';

// Italian (Italiano) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const it: TranslationMap = { ...it1, ...it2, ...it3, ...it4, ...it5 };

export default it;
