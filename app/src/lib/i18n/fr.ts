import fr1 from './chunks/fr-1';
import fr2 from './chunks/fr-2';
import fr3 from './chunks/fr-3';
import fr4 from './chunks/fr-4';
import fr5 from './chunks/fr-5';
import type { TranslationMap } from './types';

// French (Français) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const fr: TranslationMap = { ...fr1, ...fr2, ...fr3, ...fr4, ...fr5 };

export default fr;
