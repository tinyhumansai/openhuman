import bn1 from './chunks/bn-1';
import bn2 from './chunks/bn-2';
import bn3 from './chunks/bn-3';
import bn4 from './chunks/bn-4';
import bn5 from './chunks/bn-5';
import type { TranslationMap } from './types';

// Bengali (বাংলা) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const bn: TranslationMap = {
  ...bn1,
  ...bn2,
  ...bn3,
  ...bn4,
  ...bn5,
  'skills.composio.noApiKeyTitle': 'কোনো Composio API Key কনফিগার করা নেই',
  'skills.composio.noApiKeyDescription':
    'লোকাল মোডে আপনার নিজের Composio API key ব্যবহার হয়। এখানে ইন্টিগ্রেশন যুক্ত করার আগে Settings → Advanced → Composio খুলে একটি key যোগ করুন।',
  'skills.composio.noApiKeyCta': 'সেটিংসে খুলুন',
  'channels.localManagedUnavailable': 'লোকাল ব্যবহারকারীদের জন্য ম্যানেজড চ্যানেল উপলভ্য নয়।',
  'rewards.localUnavailable':
    'লোকাল লগইনে কোনো রিওয়ার্ড, কুপন বা রেফারেল ক্রেডিট মেলে না। রিওয়ার্ড পেতে লগ আউট করে একটি OpenHuman অ্যাকাউন্ট দিয়ে সাইন ইন করুন।',
  'rewards.localUnavailableCta': 'অ্যাকাউন্ট সেটিংস খুলুন',
  'settings.search.localManagedUnavailable':
    'লোকাল ব্যবহারকারীদের জন্য OpenHuman Managed সার্চ উপলভ্য নয়। ওয়েব সার্চ চালু করতে আপনার নিজের Parallel বা Brave API key যোগ করুন।',
  'devices.comingSoonDescription':
    'ডিভাইস পেয়ারিং শীঘ্রই আসছে। এই পেজে iPhone পেয়ারিং এবং সংযুক্ত ডিভাইস ম্যানেজ করা যাবে।',
  'welcome.continueLocallyExperimental': 'লোকালি চালিয়ে যান (প্রায়োগিক)',
};

export default bn;
