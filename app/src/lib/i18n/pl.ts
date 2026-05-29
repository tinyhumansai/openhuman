import pl1 from './chunks/pl-1';
import pl2 from './chunks/pl-2';
import pl3 from './chunks/pl-3';
import pl4 from './chunks/pl-4';
import pl5 from './chunks/pl-5';
import type { TranslationMap } from './types';

// Polish (Polski) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const pl: TranslationMap = {
  ...pl1,
  ...pl2,
  ...pl3,
  ...pl4,
  ...pl5,
  'skills.composio.noApiKeyTitle': 'Brak skonfigurowanego klucza API Composio',
  'skills.composio.noApiKeyDescription':
    'W trybie lokalnym używany jest Twój własny klucz API Composio. Otwórz Ustawienia → Zaawansowane → Composio, aby dodać klucz, zanim podłączysz tutaj integracje.',
  'skills.composio.noApiKeyCta': 'Otwórz ustawienia',
  'channels.localManagedUnavailable':
    'Kanały zarządzane są niedostępne dla użytkowników lokalnych.',
  'rewards.localUnavailable':
    'Logowanie lokalne nie zbiera nagród, voucherów ani środków z poleceń. Wyloguj się i zaloguj kontem OpenHuman, jeśli zależy Ci na nagrodach.',
  'rewards.localUnavailableCta': 'Otwórz ustawienia konta',
  'settings.search.localManagedUnavailable':
    'Wyszukiwarka zarządzana przez OpenHuman jest niedostępna dla użytkowników lokalnych. Dodaj własny klucz API Parallel lub Brave, aby włączyć wyszukiwanie w sieci.',
  'devices.comingSoonDescription':
    'Parowanie urządzeń pojawi się wkrótce. Ta strona będzie służyć do parowania iPhone’ów i zarządzania połączonymi urządzeniami.',
  'welcome.continueLocallyExperimental': 'Kontynuuj lokalnie (Eksperymentalne)',
};

export default pl;
