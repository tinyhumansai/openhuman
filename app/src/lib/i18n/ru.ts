import ru1 from './chunks/ru-1';
import ru2 from './chunks/ru-2';
import ru3 from './chunks/ru-3';
import ru4 from './chunks/ru-4';
import ru5 from './chunks/ru-5';
import type { TranslationMap } from './types';

// Russian (Русский) translations. Each chunk maps to chunks/en-N.ts.
// Missing keys fall back to English via I18nContext.resolveEn().
const ru: TranslationMap = {
  ...ru1,
  ...ru2,
  ...ru3,
  ...ru4,
  ...ru5,
  'skills.composio.noApiKeyTitle': 'Ключ API Composio не настроен',
  'skills.composio.noApiKeyDescription':
    'Локальный режим использует ваш собственный ключ API Composio. Откройте Настройки → Дополнительно → Composio, чтобы добавить ключ перед подключением интеграций здесь.',
  'skills.composio.noApiKeyCta': 'Открыть в настройках',
  'channels.localManagedUnavailable': 'Управляемые каналы недоступны для локальных пользователей.',
  'rewards.localUnavailable':
    'Локальный вход не приносит награды, купоны или реферальный кредит. Выйдите и войдите с аккаунтом OpenHuman, если хотите, чтобы награды начислялись.',
  'rewards.localUnavailableCta': 'Открыть настройки аккаунта',
  'settings.search.localManagedUnavailable':
    'Поиск OpenHuman Managed недоступен для локальных пользователей. Добавьте свой ключ API Parallel или Brave, чтобы включить веб-поиск.',
  'devices.comingSoonDescription':
    'Сопряжение устройств скоро появится. Эта страница будет местом для подключения iPhone и управления подключёнными устройствами.',
  'welcome.continueLocallyExperimental': 'Продолжить локально (Экспериментально)',
};

export default ru;
