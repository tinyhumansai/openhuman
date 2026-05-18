import es1 from './chunks/es-1';
import es2 from './chunks/es-2';
import es3 from './chunks/es-3';
import es4 from './chunks/es-4';
import es5 from './chunks/es-5';
import type { TranslationMap } from './types';

// Spanish (Español) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const es: TranslationMap = { ...es1, ...es2, ...es3, ...es4, ...es5 };

export default es;
