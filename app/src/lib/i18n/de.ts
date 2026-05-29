import de1 from './chunks/de-1';
import de2 from './chunks/de-2';
import de3 from './chunks/de-3';
import de4 from './chunks/de-4';
import de5 from './chunks/de-5';
import type { TranslationMap } from './types';

// German (Deutsch) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const de: TranslationMap = {
  ...de1,
  ...de2,
  ...de3,
  ...de4,
  ...de5,
  'skills.composio.noApiKeyTitle': 'Kein Composio-API-Schlüssel konfiguriert',
  'skills.composio.noApiKeyDescription':
    'Im lokalen Modus wird dein eigener Composio-API-Schlüssel verwendet. Öffne Einstellungen → Erweitert → Composio, um einen Schlüssel hinzuzufügen, bevor du hier Integrationen verbindest.',
  'skills.composio.noApiKeyCta': 'In den Einstellungen öffnen',
  'channels.localManagedUnavailable': 'Verwaltete Kanäle sind für lokale Benutzer nicht verfügbar.',
  'rewards.localUnavailable':
    'Ein lokaler Login sammelt keine Belohnungen, Gutscheine oder Empfehlungsguthaben. Melde dich ab und anschließend mit einem OpenHuman-Konto an, wenn Belohnungen zählen sollen.',
  'rewards.localUnavailableCta': 'Kontoeinstellungen öffnen',
  'settings.search.localManagedUnavailable':
    'Die von OpenHuman verwaltete Suche ist für lokale Benutzer nicht verfügbar. Füge deinen eigenen Parallel- oder Brave-API-Schlüssel hinzu, um die Websuche zu aktivieren.',
  'devices.comingSoonDescription':
    'Gerätekopplung kommt bald. Diese Seite wird für das Koppeln von iPhones und die Verwaltung verbundener Geräte zuständig sein.',
  'welcome.continueLocallyExperimental': 'Lokal fortfahren (Experimentell)',
};

export default de;
