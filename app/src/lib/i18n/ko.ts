import ko1 from './chunks/ko-1';
import ko2 from './chunks/ko-2';
import ko3 from './chunks/ko-3';
import ko4 from './chunks/ko-4';
import ko5 from './chunks/ko-5';
import type { TranslationMap } from './types';

// Korean translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const ko: TranslationMap = { ...ko1, ...ko2, ...ko3, ...ko4, ...ko5 };

export default ko;
