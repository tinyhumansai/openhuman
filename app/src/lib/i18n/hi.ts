import hi1 from './chunks/hi-1';
import hi2 from './chunks/hi-2';
import hi3 from './chunks/hi-3';
import hi4 from './chunks/hi-4';
import hi5 from './chunks/hi-5';
import type { TranslationMap } from './types';

// Hindi (हिन्दी) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const hi: TranslationMap = {
  ...hi1,
  ...hi2,
  ...hi3,
  ...hi4,
  ...hi5,
  'skills.composio.noApiKeyTitle': 'कोई Composio API key कॉन्फ़िगर नहीं है',
  'skills.composio.noApiKeyDescription':
    'लोकल मोड आपकी अपनी Composio API key का उपयोग करता है। यहाँ integrations जोड़ने से पहले Settings → Advanced → Composio खोलकर key जोड़ें।',
  'skills.composio.noApiKeyCta': 'Settings में खोलें',
  'channels.localManagedUnavailable': 'लोकल उपयोगकर्ताओं के लिए managed channels उपलब्ध नहीं हैं।',
  'rewards.localUnavailable':
    'लोकल लॉगिन पर rewards, coupons या referral credit नहीं मिलते। rewards पाने के लिए लॉग आउट करें और OpenHuman खाते से साइन इन करें।',
  'rewards.localUnavailableCta': 'Account Settings खोलें',
  'settings.search.localManagedUnavailable':
    'लोकल उपयोगकर्ताओं के लिए OpenHuman Managed search उपलब्ध नहीं है। वेब सर्च चालू करने के लिए अपनी Parallel या Brave API key जोड़ें।',
  'devices.comingSoonDescription':
    'डिवाइस पेयरिंग जल्द आ रही है। यह पेज iPhone पेयर करने और जुड़े हुए डिवाइस प्रबंधित करने का स्थान होगा।',
  'welcome.continueLocallyExperimental': 'लोकल रूप से जारी रखें (प्रायोगिक)',
};

export default hi;
