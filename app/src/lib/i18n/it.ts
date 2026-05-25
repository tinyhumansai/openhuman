import it1 from './chunks/it-1';
import it2 from './chunks/it-2';
import it3 from './chunks/it-3';
import it4 from './chunks/it-4';
import it5 from './chunks/it-5';
import type { TranslationMap } from './types';

// Italian (Italiano) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const it: TranslationMap = {
  ...it1,
  ...it2,
  ...it3,
  ...it4,
  ...it5,
  'skills.composio.noApiKeyTitle': 'Nessuna chiave API Composio configurata',
  'skills.composio.noApiKeyDescription':
    'La modalità locale usa la tua chiave API Composio. Apri Impostazioni → Avanzate → Composio per aggiungerne una prima di collegare le integrazioni qui.',
  'skills.composio.noApiKeyCta': 'Apri nelle impostazioni',
  'channels.localManagedUnavailable':
    'I canali gestiti non sono disponibili per gli utenti locali.',
  'rewards.localUnavailable':
    "L'accesso locale non guadagna ricompense, coupon o credito referral. Esci e continua accedendo con un account OpenHuman se vuoi che le ricompense vengano conteggiate.",
  'rewards.localUnavailableCta': 'Apri le impostazioni account',
  'settings.search.localManagedUnavailable':
    'La ricerca gestita da OpenHuman non è disponibile per gli utenti locali. Aggiungi la tua chiave API Parallel o Brave per abilitare la ricerca web.',
  'devices.comingSoonDescription':
    "L'abbinamento dei dispositivi arriverà presto. Questa pagina sarà il punto centrale per abbinare gli iPhone e gestire i dispositivi connessi.",
  'welcome.continueLocallyExperimental': 'Continua Localmente (Sperimentale)',
};

export default it;
