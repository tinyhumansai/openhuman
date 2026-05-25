import pt1 from './chunks/pt-1';
import pt2 from './chunks/pt-2';
import pt3 from './chunks/pt-3';
import pt4 from './chunks/pt-4';
import pt5 from './chunks/pt-5';
import type { TranslationMap } from './types';

// Portuguese (Português) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const pt: TranslationMap = {
  ...pt1,
  ...pt2,
  ...pt3,
  ...pt4,
  ...pt5,
  'skills.composio.noApiKeyTitle': 'Nenhuma chave de API do Composio configurada',
  'skills.composio.noApiKeyDescription':
    'O modo local usa sua própria chave de API do Composio. Abra Configurações → Avançado → Composio para adicionar uma antes de conectar integrações aqui.',
  'skills.composio.noApiKeyCta': 'Abrir nas Configurações',
  'channels.localManagedUnavailable':
    'Canais gerenciados não estão disponíveis para usuários locais.',
  'rewards.localUnavailable':
    'O login local não rende recompensas, cupons nem crédito de indicação. Saia e continue entrando com uma conta OpenHuman se quiser que as recompensas contem.',
  'rewards.localUnavailableCta': 'Abrir configurações da conta',
  'settings.search.localManagedUnavailable':
    'A busca gerenciada pela OpenHuman não está disponível para usuários locais. Adicione sua própria chave de API do Parallel ou Brave para habilitar a busca na web.',
  'devices.comingSoonDescription':
    'O pareamento de dispositivos está chegando em breve. Esta página será o lugar para parear iPhones e gerenciar dispositivos conectados.',
  'welcome.continueLocallyExperimental': 'Continuar Localmente (Experimental)',
};

export default pt;
