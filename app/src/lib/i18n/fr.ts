import fr1 from './chunks/fr-1';
import fr2 from './chunks/fr-2';
import fr3 from './chunks/fr-3';
import fr4 from './chunks/fr-4';
import fr5 from './chunks/fr-5';
import type { TranslationMap } from './types';

// French (Français) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const fr: TranslationMap = {
  ...fr1,
  ...fr2,
  ...fr3,
  ...fr4,
  ...fr5,
  'skills.composio.noApiKeyTitle': 'Aucune clé API Composio configurée',
  'skills.composio.noApiKeyDescription':
    'Le mode local utilise votre propre clé API Composio. Ouvrez Paramètres → Avancé → Composio pour en ajouter une avant de connecter des intégrations ici.',
  'skills.composio.noApiKeyCta': 'Ouvrir dans les paramètres',
  'channels.localManagedUnavailable':
    'Les canaux gérés ne sont pas disponibles pour les utilisateurs locaux.',
  'rewards.localUnavailable':
    'La connexion locale ne permet pas de gagner des récompenses, des coupons ou du crédit de parrainage. Déconnecte-toi puis connecte-toi avec un compte OpenHuman si tu veux que les récompenses comptent.',
  'rewards.localUnavailableCta': 'Ouvrir les paramètres du compte',
  'settings.search.localManagedUnavailable':
    'La recherche gérée par OpenHuman n’est pas disponible pour les utilisateurs locaux. Ajoutez votre propre clé API Parallel ou Brave pour activer la recherche web.',
  'devices.comingSoonDescription':
    'L’appairage des appareils arrive bientôt. Cette page servira à appairer des iPhone et à gérer les appareils connectés.',
  'welcome.continueLocallyExperimental': 'Continuer en local (Expérimental)',
};

export default fr;
