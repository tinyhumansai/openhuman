import ar1 from './chunks/ar-1';
import ar2 from './chunks/ar-2';
import ar3 from './chunks/ar-3';
import ar4 from './chunks/ar-4';
import ar5 from './chunks/ar-5';
import type { TranslationMap } from './types';

// Arabic (العربية) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const ar: TranslationMap = {
  ...ar1,
  ...ar2,
  ...ar3,
  ...ar4,
  ...ar5,
  'skills.composio.noApiKeyTitle': 'لم يتم إعداد مفتاح Composio API',
  'skills.composio.noApiKeyDescription':
    'يستخدم الوضع المحلي مفتاح Composio API الخاص بك. افتح الإعدادات ← الخيارات المتقدمة ← Composio لإضافته قبل توصيل التكاملات هنا.',
  'skills.composio.noApiKeyCta': 'افتح في الإعدادات',
  'channels.localManagedUnavailable': 'القنوات المُدارة غير متاحة للمستخدمين المحليين.',
  'rewards.localUnavailable':
    'تسجيل الدخول المحلي لا يمنح مكافآت أو قسائم أو رصيد إحالة. لكسب المكافآت، سجّل الخروج ثم تابِع بتسجيل الدخول باستخدام حساب OpenHuman.',
  'rewards.localUnavailableCta': 'افتح إعدادات الحساب',
  'settings.search.localManagedUnavailable':
    'بحث OpenHuman المُدار غير متاح للمستخدمين المحليين. أضف مفتاح Parallel أو Brave الخاص بك لتفعيل البحث على الويب.',
  'devices.comingSoonDescription':
    'إقران الأجهزة قريبًا. ستكون هذه الصفحة مخصصة لإقران أجهزة iPhone وإدارة الأجهزة المتصلة.',
  'welcome.continueLocallyExperimental': 'المتابعة محليًا (تجريبي)',
};

export default ar;
