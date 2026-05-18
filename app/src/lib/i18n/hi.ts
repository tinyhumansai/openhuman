import hi1 from './chunks/hi-1';
import hi2 from './chunks/hi-2';
import hi3 from './chunks/hi-3';
import hi4 from './chunks/hi-4';
import hi5 from './chunks/hi-5';
import type { TranslationMap } from './types';

// Hindi (हिन्दी) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const hi: TranslationMap = { ...hi1, ...hi2, ...hi3, ...hi4, ...hi5 };

export default hi;
