import ru1 from './chunks/ru-1';
import ru2 from './chunks/ru-2';
import ru3 from './chunks/ru-3';
import ru4 from './chunks/ru-4';
import ru5 from './chunks/ru-5';
import type { TranslationMap } from './types';

// Russian (Русский) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const ru: TranslationMap = { ...ru1, ...ru2, ...ru3, ...ru4, ...ru5 };

export default ru;
