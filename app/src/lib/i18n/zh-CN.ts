import zhCN1 from './chunks/zh-CN-1';
import zhCN2 from './chunks/zh-CN-2';
import zhCN3 from './chunks/zh-CN-3';
import zhCN4 from './chunks/zh-CN-4';
import zhCN5 from './chunks/zh-CN-5';
import type { TranslationMap } from './types';

// Simplified Chinese (简体中文) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const zhCN: TranslationMap = { ...zhCN1, ...zhCN2, ...zhCN3, ...zhCN4, ...zhCN5 };

export default zhCN;
