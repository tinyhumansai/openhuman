import es1 from './chunks/es-1';
import es2 from './chunks/es-2';
import es3 from './chunks/es-3';
import es4 from './chunks/es-4';
import es5 from './chunks/es-5';
import type { TranslationMap } from './types';

// Spanish (Español) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const es: TranslationMap = {
  ...es1,
  ...es2,
  ...es3,
  ...es4,
  ...es5,
  'skills.composio.noApiKeyTitle': 'No hay una API key de Composio configurada',
  'skills.composio.noApiKeyDescription':
    'El modo local usa tu propia API key de Composio. Abre Ajustes → Avanzado → Composio para añadir una antes de conectar integraciones aquí.',
  'skills.composio.noApiKeyCta': 'Abrir en Ajustes',
  'channels.localManagedUnavailable':
    'Los canales gestionados no están disponibles para usuarios locales.',
  'rewards.localUnavailable':
    'El acceso local no obtiene recompensas, cupones ni crédito por referidos. Cierra sesión y continúa iniciando sesión con una cuenta de OpenHuman si quieres que las recompensas cuenten.',
  'rewards.localUnavailableCta': 'Abrir ajustes de la cuenta',
  'settings.search.localManagedUnavailable':
    'La búsqueda gestionada por OpenHuman no está disponible para usuarios locales. Añade tu propia API key de Parallel o Brave para habilitar la búsqueda web.',
  'devices.comingSoonDescription':
    'El emparejamiento de dispositivos llegará pronto. Esta página será el lugar para emparejar iPhones y gestionar dispositivos conectados.',
  'welcome.continueLocallyExperimental': 'Continuar localmente (Experimental)',
};

export default es;
