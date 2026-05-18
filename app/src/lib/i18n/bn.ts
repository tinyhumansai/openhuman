import bn1 from './chunks/bn-1';
import bn2 from './chunks/bn-2';
import bn3 from './chunks/bn-3';
import bn4 from './chunks/bn-4';
import bn5 from './chunks/bn-5';
import type { TranslationMap } from './types';

// Bengali (বাংলা) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const bn: TranslationMap = { ...bn1, ...bn2, ...bn3, ...bn4, ...bn5 };

export default bn;
