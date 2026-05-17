import type { TranslationMap } from './types';

const ru: TranslationMap = {
  // Navigation
  'nav.home': 'Главная',
  'nav.human': 'Профиль',
  'nav.chat': 'Чат',
  'nav.connections': 'Подключения',
  'nav.memory': 'Интеллект',
  'nav.alerts': 'Уведомления',
  'nav.rewards': 'Бонусы',
  'nav.settings': 'Настройки',

  // Common
  'common.cancel': 'Отмена',
  'common.save': 'Сохранить',
  'common.confirm': 'Подтвердить',
  'common.delete': 'Удалить',
  'common.edit': 'Изменить',
  'common.create': 'Создать',
  'common.search': 'Поиск',
  'common.loading': 'загрузка…',
  'common.error': 'Ошибка',
  'common.success': 'Готово',
  'common.back': 'Назад',
  'common.next': 'Далее',
  'common.finish': 'Завершить',
  'common.close': 'Закрыть',
  'common.enabled': 'Включено',
  'common.disabled': 'Отключено',
  'common.on': 'Вкл',
  'common.off': 'Выкл',
  'common.yes': 'Да',
  'common.no': 'Нет',
  'common.ok': 'Понятно',
  'common.retry': 'Повторить',
  'common.copy': 'Скопировать',
  'common.copied': 'Скопировано!',
  'common.learnMore': 'Подробнее',
  'common.seeAll': 'Открыть',
  'common.dismiss': 'Скрыть',
  'common.clear': 'Очистить',
  'common.reset': 'Сбросить',
  'common.refresh': 'Обновить',
  'common.export': 'Экспорт',
  'common.import': 'Импорт',
  'common.upload': 'Загрузить',
  'common.download': 'Скачать',
  'common.add': 'Добавить',
  'common.remove': 'Убрать',
  'common.showMore': 'Показать больше',
  'common.showLess': 'Свернуть',
  'common.submit': 'Отправить',
  'common.continue': 'Продолжить',

  // Settings Home
  'settings.general': 'Общие',
  'settings.featuresAndAI': 'Функции и ИИ',
  'settings.billingAndRewards': 'Оплата и бонусы',
  'settings.support': 'Поддержка',
  'settings.advanced': 'Дополнительно',
  'settings.dangerZone': 'Опасная зона',
  'settings.account': 'Аккаунт',
  'settings.accountDesc': 'Фраза восстановления, команда, подключения и приватность',
  'settings.notifications': 'Уведомления',
  'settings.notificationsDesc': 'Режим «Не беспокоить» и уведомления по каждому аккаунту',
  'settings.features': 'Функции',
  'settings.featuresDesc': 'Анализ экрана, мессенджеры и инструменты',
  'settings.aiModels': 'ИИ и модели',
  'settings.aiModelsDesc': 'Настройка локального ИИ, загрузки и провайдер LLM',
  'settings.ai': 'Настройка ИИ',
  'settings.aiDesc': 'Облачные провайдеры, локальные модели Ollama и маршрутизация по задачам',
  'settings.billingUsage': 'Оплата и использование',
  'settings.billingUsageDesc': 'Тариф, кредиты и способы оплаты',
  'settings.rewards': 'Бонусы',
  'settings.rewardsDesc': 'Рефералы, купоны и накопленные кредиты',
  'settings.restartTour': 'Запустить тур заново',
  'settings.restartTourDesc': 'Повторить знакомство с продуктом с самого начала',
  'settings.about': 'О приложении',
  'settings.aboutDesc': 'Версия приложения и обновления',
  'settings.developerOptions': 'Дополнительно',
  'settings.developerOptionsDesc':
    'Настройки ИИ, мессенджеры, инструменты, диагностика и панели отладки',
  'settings.clearAppData': 'Очистить данные приложения',
  'settings.clearAppDataDesc': 'Выйти и безвозвратно удалить все локальные данные',
  'settings.logOut': 'Выйти',
  'settings.logOutDesc': 'Выйти из аккаунта',
  'settings.language': 'Язык',
  'settings.languageDesc': 'Язык интерфейса приложения',
  'settings.alerts': 'Уведомления',
  'settings.alertsDesc': 'Просмотр последних уведомлений и активности в инбоксе',

  // Settings: Account
  'settings.account.recoveryPhrase': 'Фраза восстановления',
  'settings.account.recoveryPhraseDesc': 'Просмотр и резервная копия фразы восстановления',
  'settings.account.team': 'Команда',
  'settings.account.teamDesc': 'Управление участниками и правами',
  'settings.account.connections': 'Подключения',
  'settings.account.connectionsDesc': 'Управление привязанными аккаунтами и сервисами',
  'settings.account.privacy': 'Приватность',
  'settings.account.privacyDesc': 'Контроль над данными, покидающими ваше устройство',

  // Settings: Notifications
  'settings.notifications.doNotDisturb': 'Не беспокоить',
  'settings.notifications.doNotDisturbDesc': 'Приостановить все уведомления на заданный срок',
  'settings.notifications.channelControls': 'Настройка по каналам',
  'settings.notifications.channelControlsDesc':
    'Параметры уведомлений отдельно для каждого канала',

  // Settings: Features
  'settings.features.screenAwareness': 'Анализ экрана',
  'settings.features.screenAwarenessDesc': 'Разрешить ассистенту видеть активное окно',
  'settings.features.messaging': 'Мессенджеры',
  'settings.features.messagingDesc': 'Настройки интеграции с каналами и мессенджерами',
  'settings.features.tools': 'Инструменты',
  'settings.features.toolsDesc': 'Управление подключёнными инструментами и интеграциями',

  // Settings: AI & Models
  'settings.ai.localSetup': 'Локальный ИИ',
  'settings.ai.localSetupDesc': 'Скачивание и настройка локальных моделей ИИ',
  'settings.ai.llmProvider': 'Провайдер LLM',
  'settings.ai.llmProviderDesc': 'Выбор и настройка провайдера ИИ',

  // Clear App Data modal
  'clearData.title': 'Очистка данных приложения',
  'clearData.warning': 'Вы выйдете из аккаунта, и будут безвозвратно удалены локальные данные, в том числе:',
  'clearData.bulletSettings': 'Настройки приложения и переписки',
  'clearData.bulletCache': 'Весь локальный кэш интеграций',
  'clearData.bulletWorkspace': 'Данные рабочей области',
  'clearData.bulletOther': 'Все остальные локальные данные',
  'clearData.irreversible': 'Это действие нельзя отменить.',
  'clearData.clearing': 'Очистка данных приложения…',
  'clearData.failed': 'Не удалось очистить данные и выйти. Попробуйте ещё раз.',
  'clearData.failedLogout': 'Не удалось выйти. Попробуйте ещё раз.',
  'clearData.failedPersist': 'Не удалось очистить сохранённое состояние. Попробуйте ещё раз.',

  // Welcome page
  'welcome.title': 'Добро пожаловать в OpenHuman',
  'welcome.subtitle':
    'Ваш персональный сверхинтеллект на базе ИИ. Приватный, простой и невероятно мощный.',
  'welcome.connectPrompt': 'Указать RPC URL (для опытных пользователей)',
  'welcome.selectRuntime': 'Выберите среду выполнения',
  'welcome.urlPlaceholder': 'http://localhost:8089',
  'welcome.invalidUrl': 'Введите корректный URL с http или https',
  'welcome.connecting': 'Проверка',
  'welcome.connect': 'Проверить',

  // Home page
  'home.greeting': 'Доброе утро',
  'home.greetingAfternoon': 'Добрый день',
  'home.greetingEvening': 'Добрый вечер',
  'home.askAssistant': 'Спросите ассистента о чём угодно…',
  'home.statusOk':
    'Устройство подключено. Оставьте приложение запущенным, чтобы соединение не разрывалось. Напишите агенту кнопкой ниже.',
  'home.statusBackendOnly': 'Переподключение к бэкенду… агент скоро снова будет доступен.',
  'home.statusCoreUnreachable':
    'Локальная служба core не отвечает. Фоновый процесс OpenHuman мог упасть или не запуститься.',
  'home.statusInternetOffline':
    'Сейчас устройство не в сети. Проверьте подключение или перезапустите приложение.',
  'home.restartCore': 'Перезапустить core',
  'home.restartingCore': 'Перезапуск core…',

  // Chat / Conversations
  'chat.newThread': 'Новый тред',
  'chat.typeMessage': 'Введите сообщение…',
  'chat.send': 'Отправить сообщение',
  'chat.thinking': 'Думаю…',
  'chat.noMessages': 'Сообщений пока нет',
  'chat.startConversation': 'Начните разговор',
  'chat.regenerate': 'Сгенерировать заново',
  'chat.copyResponse': 'Скопировать ответ',
  'chat.citations': 'Источники',
  'chat.toolUsed': 'Использован инструмент',

  // Skills / Connections
  'scope.legacy': 'Устаревшие',
  'scope.user': 'Пользователь',
  'scope.project': 'Проект',
  'skills.title': 'Подключения',
  'skills.search': 'Поиск подключений…',
  'skills.noResults': 'Подключений не найдено',
  'skills.connect': 'Подключить',
  'skills.disconnect': 'Отключить',
  'skills.configure': 'Управление',
  'skills.connected': 'Подключено',
  'skills.available': 'Доступно',
  'skills.addAccount': 'Добавить аккаунт',
  'skills.channels': 'Каналы',
  'skills.integrations': 'Интеграции',

  // Intelligence / Memory
  'memory.title': 'Память',
  'memory.search': 'Поиск по памяти…',
  'memory.noResults': 'Записей не найдено',
  'memory.empty': 'Память пока пуста. Записи создаются автоматически по мере вашего взаимодействия.',
  'memory.tab.memory': 'Память',
  'memory.tab.subconscious': 'Подсознание',
  'memory.tab.dreams': 'Сны',
  'memory.tab.calls': 'Звонки',
  'memory.tab.settings': 'Настройки',
  'memory.analyzeNow': 'Анализировать сейчас',

  // Notifications / Alerts
  'alerts.title': 'Уведомления',
  'alerts.empty': 'Уведомлений пока нет',
  'alerts.markAllRead': 'Отметить все прочитанными',
  'alerts.unread': 'непрочитанных',

  // Rewards
  'rewards.title': 'Бонусы',
  'rewards.referrals': 'Рефералы',
  'rewards.coupons': 'Купоны',
  'rewards.credits': 'Кредиты',
  'rewards.referralCode': 'Ваш реферальный код',
  'rewards.copyCode': 'Скопировать код',
  'rewards.share': 'Поделиться',

  // Onboarding
  'onboarding.welcome': 'Привет. Я OpenHuman.',
  'onboarding.welcomeDesc':
    'Сверхинтеллектуальный ИИ-ассистент, работающий на вашем компьютере. Приватный, простой и невероятно мощный.',
  'onboarding.context': 'Сбор контекста',
  'onboarding.contextDesc': 'Подключите инструменты и сервисы, которыми пользуетесь каждый день.',
  'onboarding.localAI': 'Локальный ИИ',
  'onboarding.localAIDesc': 'Настроить локальную модель ИИ на вашей машине.',
  'onboarding.chatProvider': 'Провайдер чата',
  'onboarding.chatProviderDesc': 'Выберите, как взаимодействовать с ассистентом.',
  'onboarding.referral': 'Реферал',
  'onboarding.referralDesc': 'Введите реферальный код, если у вас есть.',
  'onboarding.finish': 'Завершить настройку',
  'onboarding.finishDesc': 'Готово! Начните пользоваться OpenHuman.',
  'onboarding.skip': 'Пропустить',
  'onboarding.getStarted': 'Начать',

  // Onboarding: runtime-choice step (Cloud vs Custom)
  'onboarding.runtimeChoice.title': 'Как вы хотите запустить OpenHuman?',
  'onboarding.runtimeChoice.subtitle':
    'Выберите подходящий вариант. Это можно поменять позже в настройках.',
  'onboarding.runtimeChoice.cloud.title': 'Просто',
  'onboarding.runtimeChoice.cloud.tagline': 'OpenHuman берёт всё на себя.',
  'onboarding.runtimeChoice.cloud.f1': 'Встроенная безопасность',
  'onboarding.runtimeChoice.cloud.f2': 'Сжатие токенов экономит лимит',
  'onboarding.runtimeChoice.cloud.f3': 'Одна подписка, все модели включены',
  'onboarding.runtimeChoice.cloud.f4': 'Не нужны API-ключи',
  'onboarding.runtimeChoice.cloud.f5': 'Простая настройка',
  'onboarding.runtimeChoice.custom.title': 'Своя сборка',
  'onboarding.runtimeChoice.custom.tagline':
    'Свои ключи. Полный контроль над тем, что используется.',
  'onboarding.runtimeChoice.custom.f1': 'API-ключи понадобятся практически для всего',
  'onboarding.runtimeChoice.custom.f2': 'Переиспользует сервисы, за которые вы уже платите',
  'onboarding.runtimeChoice.custom.f3': 'Может быть бесплатным, если запускать всё локально',
  'onboarding.runtimeChoice.custom.f4': 'Больше настроек, больше тонкостей',
  'onboarding.runtimeChoice.custom.f5': 'Подходит опытным пользователям и разработчикам',
  'onboarding.runtimeChoice.cloud.creditHighlight': '$1 бесплатных кредитов для пробы',
  'onboarding.runtimeChoice.continueCloud': 'Продолжить с «Просто»',
  'onboarding.runtimeChoice.continueCustom': 'Продолжить со своей сборкой',
  'onboarding.runtimeChoice.recommended': 'Рекомендуем',

  // Onboarding: API keys step (only when Custom is picked)
  'onboarding.apiKeys.title': 'Добавим ваши API-ключи',
  'onboarding.apiKeys.subtitle':
    'Можно вставить их сейчас или пропустить и добавить позже в Настройках → ИИ. Ключи хранятся на этом устройстве, в зашифрованном виде.',
  'onboarding.apiKeys.openaiLabel': 'API-ключ OpenAI',
  'onboarding.apiKeys.openaiPlaceholder': 'sk-…',
  'onboarding.apiKeys.anthropicLabel': 'API-ключ Anthropic',
  'onboarding.apiKeys.anthropicPlaceholder': 'sk-ant-…',
  'onboarding.apiKeys.saveError': 'Не удалось сохранить ключ. Проверьте его и попробуйте ещё раз.',
  'onboarding.apiKeys.skipForNow': 'Пропустить пока',
  'onboarding.apiKeys.continue': 'Сохранить и продолжить',
  'onboarding.apiKeys.saving': 'Сохранение…',

  // Onboarding: Custom wizard (Inference / Voice / OAuth / Search / Memory)
  'onboarding.custom.stepperInference': 'Модель',
  'onboarding.custom.stepperVoice': 'Голос',
  'onboarding.custom.stepperOAuth': 'OAuth',
  'onboarding.custom.stepperSearch': 'Поиск',
  'onboarding.custom.stepperMemory': 'Память',
  'onboarding.custom.stepCounter': 'Шаг {n} из {total}',
  'onboarding.custom.defaultTitle': 'По умолчанию',
  'onboarding.custom.defaultSubtitle': 'OpenHuman берёт это на себя.',
  'onboarding.custom.configureTitle': 'Настроить',
  'onboarding.custom.configureSubtitle': 'Я выберу сам.',
  'onboarding.custom.progressAriaLabel': 'Прогресс знакомства',
  'onboarding.custom.continue': 'Продолжить',
  'onboarding.custom.back': 'Назад',
  'onboarding.custom.finish': 'Завершить настройку',
  'onboarding.custom.configureLater':
    'Эту настройку можно закончить позже. После завершения мы откроем соответствующую страницу в настройках.',
  'onboarding.custom.openSettings': 'Открыть настройки',

  // Onboarding: Custom > Inference (text)
  'onboarding.custom.inference.title': 'Модель (текст)',
  'onboarding.custom.inference.subtitle':
    'Какая языковая модель будет отвечать на ваши вопросы и обслуживать агентов?',
  'onboarding.custom.inference.defaultDesc':
    'OpenHuman направляет каждую задачу на разумную модель по умолчанию. Без ключей и настроек.',
  'onboarding.custom.inference.configureDesc':
    'Свой ключ OpenAI или Anthropic. Будем использовать его для всех текстовых задач.',

  // Onboarding: Custom > Voice
  'onboarding.custom.voice.title': 'Голос',
  'onboarding.custom.voice.subtitle': 'Распознавание и синтез речи для голосового режима.',
  'onboarding.custom.voice.defaultDesc':
    'OpenHuman поставляется со встроенным STT/TTS, который сразу работает. Настраивать ничего не нужно.',
  'onboarding.custom.voice.configureDesc':
    'Свои ElevenLabs / OpenAI Whisper / прочее. Настройка в Настройках → Голос.',

  // Onboarding: Custom > OAuth (Composio)
  'onboarding.custom.oauth.title': 'Подключения (OAuth)',
  'onboarding.custom.oauth.subtitle':
    'Gmail, Slack, Notion и другие сервисы, которым нужен OAuth.',
  'onboarding.custom.oauth.defaultDesc':
    'OpenHuman использует управляемый рабочий пространство Composio. Подключение каждого сервиса — в один клик.',
  'onboarding.custom.oauth.configureDesc':
    'Свой аккаунт / API-ключ Composio. Настройка в Настройках → Подключения.',

  // Onboarding: Custom > Search
  'onboarding.custom.search.title': 'Веб-поиск',
  'onboarding.custom.search.subtitle': 'Как OpenHuman ищет в интернете от вашего имени.',
  'onboarding.custom.search.defaultDesc':
    'OpenHuman использует управляемый поисковый бэкенд. Ключи не нужны.',
  'onboarding.custom.search.configureDesc':
    'Свой ключ провайдера поиска (Tavily, Brave и т. п.). Настройка в Настройках → Инструменты.',

  // Onboarding: Custom > Memory
  'onboarding.custom.memory.title': 'Память',
  'onboarding.custom.memory.subtitle':
    'Как OpenHuman запоминает ваш контекст, предпочтения и предыдущие разговоры.',
  'onboarding.custom.memory.defaultDesc':
    'OpenHuman управляет хранением и извлечением памяти автоматически. Настраивать ничего не нужно.',
  'onboarding.custom.memory.configureDesc':
    'Просмотр, экспорт или очистка памяти вручную. Настройка в Настройках → Память.',

  // Accounts
  'accounts.addAccount': 'Добавить аккаунт',
  'accounts.manageAccounts': 'Управление аккаунтами',
  'accounts.noAccounts': 'Аккаунты не подключены',
  'accounts.connectAccount': 'Подключите аккаунт, чтобы начать',
  'accounts.agent': 'Агент',
  'accounts.respondQueue': 'Очередь ответов',
  'accounts.disconnect': 'Отключить',
  'accounts.disconnectConfirm': 'Точно отключить этот аккаунт?',
  'accounts.searchAccounts': 'Поиск аккаунтов…',

  // Channels
  'channels.title': 'Каналы',
  'channels.configure': 'Настроить канал',
  'channels.setup': 'Настроить',
  'channels.noChannels': 'Каналы не настроены',
  'channels.addChannel': 'Добавить канал',
  'channels.status.connected': 'Подключено',
  'channels.status.disconnected': 'Отключено',
  'channels.status.error': 'Ошибка',
  'channels.status.configuring': 'Настройка',
  'channels.defaultMessaging': 'Канал по умолчанию',

  // Webhooks
  'webhooks.title': 'Вебхуки',
  'webhooks.create': 'Создать вебхук',
  'webhooks.noWebhooks': 'Вебхуки не настроены',
  'webhooks.url': 'URL',
  'webhooks.secret': 'Секрет',
  'webhooks.events': 'События',
  'webhooks.archiveDirectory': 'Каталог архива',
  'webhooks.todayFile': 'Файл за сегодня',

  // Invites
  'invites.title': 'Приглашения',
  'invites.create': 'Создать приглашение',
  'invites.noInvites': 'Нет ожидающих приглашений',
  'invites.code': 'Код приглашения',
  'invites.copyLink': 'Скопировать ссылку',

  // Developer Options
  'devOptions.title': 'Дополнительно',
  'devOptions.diagnostics': 'Диагностика',
  'devOptions.diagnosticsDesc': 'Состояние системы, логи и метрики производительности',
  'devOptions.debugPanels': 'Панели отладки',
  'devOptions.debugPanelsDesc': 'Флаги функций, инспекция состояния и отладочные инструменты',
  'devOptions.webhooks': 'Вебхуки',
  'devOptions.webhooksDesc': 'Настройка и тестирование интеграций через вебхуки',
  'devOptions.memoryInspection': 'Инспекция памяти',
  'devOptions.memoryInspectionDesc': 'Просмотр, запросы и управление записями памяти',

  // Voice / Dictation
  'voice.pushToTalk': 'Push-to-talk',
  'voice.recording': 'Запись…',
  'voice.processing': 'Обработка…',
  'voice.languageHint': 'Язык',

  // Misc
  'misc.rehydrating': 'Загрузка ваших данных…',
  'misc.checkingServices': 'Проверка сервисов…',
  'misc.serviceUnavailable': 'Сервис недоступен',
  'misc.somethingWentWrong': 'Что-то пошло не так',
  'misc.tryAgainLater': 'Попробуйте позже.',
  'misc.restartApp': 'Перезапустить приложение',
  'misc.updateAvailable': 'Доступно обновление',
  'misc.updateNow': 'Обновить сейчас',
  'misc.updateLater': 'Позже',
  'misc.downloading': 'Скачивание…',
  'misc.installing': 'Установка…',
  'misc.beta':
    'OpenHuman находится на ранней стадии бета-тестирования. Делитесь обратной связью или сообщайте об ошибках — каждое сообщение помогает нам двигаться быстрее.',
  'misc.betaFeedback': 'Отправить отзыв',

  // Mnemonic / Recovery
  'mnemonic.title': 'Фраза восстановления',
  'mnemonic.warning': 'Запишите эти слова по порядку и храните в надёжном месте.',
  'mnemonic.copyWarning':
    'Никогда не передавайте фразу восстановления. Тот, у кого она есть, получит доступ к вашему аккаунту.',
  'mnemonic.copied': 'Фраза восстановления скопирована в буфер обмена',
  'mnemonic.reveal': 'Показать фразу',
  'mnemonic.hidden': 'Фраза восстановления скрыта',

  // What Leaves My Computer
  'privacy.title': 'Приватность и безопасность',
  'privacy.description': 'Отчёт о прозрачности: какие данные уходят во внешние сервисы.',
  'privacy.empty': 'Внешних передач данных не обнаружено.',
  'privacy.whatLeavesComputer': 'Что покидает ваш компьютер',
  'privacy.loading': 'Загрузка данных о приватности…',
  'privacy.loadError': 'Не удалось загрузить актуальный список приватности. Настройки аналитики ниже всё равно работают.',
  'privacy.noCapabilities': 'Сейчас ни одна возможность не раскрывает движение данных.',
  'privacy.sentTo': 'Отправляется',
  'privacy.leavesDevice': 'Покидает устройство',
  'privacy.staysLocal': 'Остаётся локально',
  'privacy.anonymizedAnalytics': 'Анонимная аналитика',
  'privacy.shareAnonymizedData': 'Делиться анонимной статистикой использования',
  'privacy.shareAnonymizedDataDesc':
    'Помогите улучшить OpenHuman, отправляя анонимные отчёты о сбоях и статистику использования. Все данные полностью обезличены — личная информация, сообщения, ключи кошелька и данные сессий никогда не собираются.',
  'privacy.meetingFollowUps': 'Действия после встреч',
  'privacy.autoHandoffMeet': 'Автоматически передавать стенограммы Google Meet оркестратору',
  'privacy.autoHandoffMeetDesc':
    'Когда звонок в Google Meet заканчивается, оркестратор OpenHuman может прочитать стенограмму и выполнить действия: написать сообщения, запланировать встречи или опубликовать резюме в подключённом Slack. По умолчанию выключено.',
  'privacy.analyticsDisclaimer':
    'Вся аналитика и отчёты об ошибках полностью обезличены. При включении мы собираем только информацию о сбоях, тип устройства и местоположение ошибок в коде. Мы никогда не получаем доступ к вашим сообщениям, данным сессий, ключам кошелька, API-ключам или любой персональной информации. Эту настройку можно изменить в любой момент.',

  // Settings: About
  'settings.about.version': 'Версия',
  'settings.about.updateAvailable': 'доступно',
  'settings.about.softwareUpdates': 'Обновления ПО',
  'settings.about.lastChecked': 'Последняя проверка',
  'settings.about.checking': 'Проверка…',
  'settings.about.checkForUpdates': 'Проверить обновления',
  'settings.about.releases': 'Релизы',
  'settings.about.releasesDesc': 'Список релизов и более ранние сборки на GitHub.',
  'settings.about.openReleases': 'Открыть релизы на GitHub',

  // Settings: AI
  'settings.ai.overview': 'Обзор системы ИИ',
  'settings.ai.configStatus': 'Состояние конфигурации',
  'settings.ai.fallbackMode': 'Режим запасного варианта',
  'settings.ai.loadedFromRuntime': 'Загружено из среды выполнения',
  'settings.ai.loadingDuration': 'Время загрузки',
  'settings.ai.localRuntime': 'Среда выполнения локальной модели',
  'settings.ai.openManager': 'Открыть менеджер',
  'settings.ai.retryDownload': 'Повторить загрузку',
  'settings.ai.state': 'Состояние',
  'settings.ai.targetModel': 'Целевая модель',
  'settings.ai.download': 'Скачать',
  'settings.ai.localModelUnavailable': 'Статус локальной модели недоступен.',
  'settings.ai.soulConfig': 'Конфигурация персоны SOUL',
  'settings.ai.refreshing': 'Обновление…',
  'settings.ai.refreshSoul': 'Обновить SOUL',
  'settings.ai.loadingSoul': 'Загрузка конфигурации SOUL…',
  'settings.ai.identity': 'Идентичность',
  'settings.ai.personality': 'Личность',
  'settings.ai.safetyRules': 'Правила безопасности',
  'settings.ai.source': 'Источник',
  'settings.ai.loaded': 'Загружено',
  'settings.ai.toolsConfig': 'Конфигурация TOOLS',
  'settings.ai.refreshTools': 'Обновить TOOLS',
  'settings.ai.toolsAvailable': 'Доступно инструментов',
  'settings.ai.tools': 'инструментов',
  'settings.ai.activeSkills': 'Активные навыки',
  'settings.ai.skills': 'навыков',
  'settings.ai.skillsOverview': 'Обзор навыков',
  'settings.ai.refreshingAll': 'Обновление всего…',
  'settings.ai.refreshAll': 'Обновить всю конфигурацию ИИ',

  // Settings: Notifications
  'settings.notifications.suppressAll': 'Подавлять все уведомления',
  'settings.notifications.suppressAllDesc':
    'Блокировать все системные уведомления от встроенных приложений независимо от фокуса.',
  'settings.notifications.toggleDnd': 'Переключить «Не беспокоить»',
  'settings.notifications.categories': 'Категории',
  'settings.notifications.categoryFooter':
    'Отключение категории останавливает появление новых уведомлений этого типа в центре уведомлений. Существующие остаются, пока не будут удалены.',

  // Settings: Billing
  'settings.billing.movedToWeb': 'Оплата переехала в веб',
  'settings.billing.openDashboard': 'Открыть кабинет оплаты',
  'settings.billing.movedToWebDesc':
    'Изменение подписки, способы оплаты, кредиты и счета теперь управляются в TinyHumans в вебе.',
  'settings.billing.backToSettings': 'Назад к настройкам',
  'settings.billing.openingBrowser': 'Открываем браузер…',
  'settings.billing.browserNotOpen': 'Если браузер не открылся, нажмите кнопку выше.',
  'settings.billing.browserOpenFailed':
    'Не удалось открыть браузер автоматически. Используйте кнопку выше.',

  // Settings: Tools
  'settings.tools.chooseCapabilities':
    'Выберите, какие возможности OpenHuman может использовать от вашего имени.',
  'settings.tools.saveChanges': 'Сохранить изменения',
  'settings.tools.preferencesSaved': 'Настройки сохранены',
  'settings.tools.saveFailed': 'Не удалось сохранить настройки. Попробуйте ещё раз.',

  // Settings: Screen Awareness
  'settings.screenAwareness.mode': 'Режим',
  'settings.screenAwareness.allExceptBlacklist': 'Все, кроме чёрного списка',
  'settings.screenAwareness.whitelistOnly': 'Только белый список',
  'settings.screenAwareness.screenMonitoring': 'Мониторинг экрана',
  'settings.screenAwareness.saveSettings': 'Сохранить настройки',
  'settings.screenAwareness.session': 'Сессия',
  'settings.screenAwareness.status': 'Статус',
  'settings.screenAwareness.active': 'Активна',
  'settings.screenAwareness.stopped': 'Остановлена',
  'settings.screenAwareness.remaining': 'Осталось',
  'settings.screenAwareness.startSession': 'Запустить сессию',
  'settings.screenAwareness.stopSession': 'Остановить сессию',
  'settings.screenAwareness.analyzeNow': 'Анализировать сейчас',
  'settings.screenAwareness.macosOnly':
    'Захват рабочего стола и управление разрешениями для анализа экрана пока поддерживаются только на macOS.',

  // Connections
  'connections.comingSoon': 'Скоро',
  'connections.setUp': 'Настроить',
  'connections.configured': 'Настроено',
  'connections.unavailable': 'Недоступно',
  'connections.checking': 'Проверка…',
  'connections.walletConfigured':
    'Локальные идентичности EVM, BTC, Solana и Tron сформированы из вашей фразы восстановления.',
  'connections.walletReady':
    'Настройте локальные идентичности EVM, BTC, Solana и Tron из одной фразы восстановления.',
  'connections.walletError':
    'Не удалось проверить статус кошелька. Нажмите, чтобы повторить из панели фразы восстановления.',
  'connections.walletChecking': 'Проверка статуса кошелька…',
  'connections.walletIdentities': 'Идентичности кошелька',
  'connections.walletDerived':
    'Сгенерированы локально из вашей фразы восстановления и хранятся только как безопасные метаданные.',
  'connections.privacySecurity': 'Приватность и безопасность',
  'connections.privacySecurityDesc':
    'Все данные и учётные данные хранятся локально с политикой нулевого удержания. Информация шифруется и никогда не передаётся третьим лицам.',

  // Channels
  'channels.status.connecting': 'Подключение',
  'channels.status.notConfigured': 'Не настроено',
  'channels.noActiveRoute': 'Активный маршрут не задан',
  'channels.activeRoute': 'Активный маршрут',
  'channels.loadingDefinitions': 'Загрузка описаний каналов…',
  'channels.channelConnections': 'Подключения каналов',
  'channels.configureAuthModes': 'Настройка режимов авторизации для каждого канала.',
  'channels.configNotAvailable': 'Конфигурация для',
  'channels.channel': 'канала',

  // Dev Options
  'devOptions.coreModeNotSet': 'Режим core: не задан',
  'devOptions.coreModeNotSetDesc':
    'Выбор в окне проверки запуска ещё не подтверждён. Используйте «Сменить режим», чтобы выбрать Local или Cloud.',
  'devOptions.local': 'Локально',
  'devOptions.embeddedCoreSidecar': 'Встроенный sidecar core',
  'devOptions.sidecarSpawned': 'Запущен в процессе Tauri при старте приложения.',
  'devOptions.cloud': 'Облако',
  'devOptions.remoteCoreRpc': 'Удалённый RPC core',
  'devOptions.token': 'Токен',
  'devOptions.tokenNotSet': 'не задан — RPC вернёт 401',
  'devOptions.triggerSentryTest': 'Запустить тест Sentry (staging)',
  'devOptions.triggerSentryTestDesc':
    'Отправляет помеченную ошибку для проверки конвейера Sentry. Issue #1072 — удалить после проверки.',
  'devOptions.sendTestEvent': 'Отправить тестовое событие',
  'devOptions.sending': 'Отправка…',
  'devOptions.eventSent': 'Событие отправлено',
  'devOptions.failed': 'Ошибка',
  'devOptions.appLogs': 'Логи приложения',
  'devOptions.appLogsDesc':
    'Открыть папку с ежедневными лог-файлами. При сообщении об ошибке прикладывайте самый свежий файл.',
  'devOptions.openLogsFolder': 'Открыть папку с логами',

  // Mnemonic
  'mnemonic.phraseSaved': 'Фраза восстановления сохранена',
  'mnemonic.walletReady': 'Идентичности мультичейн-кошелька готовы. Возвращаемся в настройки…',
  'mnemonic.writeDownWords': 'Запишите эти',
  'mnemonic.wordsInOrder':
    'слов по порядку и храните в надёжном месте. Эта фраза защищает ваш локальный ключ шифрования и идентичности кошельков EVM, BTC, Solana и Tron.',
  'mnemonic.cannotRecover':
    'Эту фразу нельзя восстановить, если потеряете, и она должна оставаться полностью локальной.',
  'mnemonic.copyToClipboard': 'Скопировать в буфер обмена',
  'mnemonic.alreadyHavePhrase': 'У меня уже есть фраза восстановления',
  'mnemonic.consentSaved': 'Я сохранил эту фразу и соглашаюсь использовать её для настройки локального кошелька',
  'mnemonic.enterPhraseToRestore':
    'Введите фразу восстановления, чтобы восстановить локальные идентичности кошельков, или вставьте полную фразу в любое поле (12 слов для новых резервных копий; 24 слова из старых версий тоже подойдут).',
  'mnemonic.words': 'Слова',
  'mnemonic.validPhrase': 'Корректная фраза восстановления',
  'mnemonic.generateNewPhrase': 'Сгенерировать новую фразу',
  'mnemonic.securingData': 'Защищаем ваши данные…',
  'mnemonic.saveRecoveryPhrase': 'Сохранить фразу восстановления',
  'mnemonic.userNotLoaded': 'Пользователь не загружен. Войдите ещё раз или обновите страницу.',
  'mnemonic.invalidPhrase': 'Некорректная фраза восстановления. Проверьте слова и попробуйте ещё раз.',
  'mnemonic.somethingWentWrong': 'Что-то пошло не так. Попробуйте ещё раз.',

  // Team
  'team.failedToCreate': 'Не удалось создать команду',
  'team.invalidInviteCode': 'Недействительный или просроченный код приглашения',
  'team.failedToSwitch': 'Не удалось переключить команду',
  'team.failedToLeave': 'Не удалось покинуть команду',
  'team.role.owner': 'Владелец',
  'team.role.admin': 'Администратор',
  'team.role.billingManager': 'Менеджер биллинга',
  'team.role.member': 'Участник',
  'team.active': 'Активна',
  'team.personalTeam': 'Личная команда',
  'team.manageTeam': 'Управление командой',
  'team.switching': 'Переключение…',
  'team.switch': 'Переключить',
  'team.leaving': 'Выход…',
  'team.leave': 'Покинуть',
  'team.yourTeams': 'Ваши команды',
  'team.createNewTeam': 'Создать новую команду',
  'team.teamName': 'Название команды',
  'team.creating': 'Создание…',
  'team.joinExistingTeam': 'Присоединиться к существующей команде',
  'team.inviteCode': 'Код приглашения',
  'team.joining': 'Подключение…',
  'team.join': 'Присоединиться',
  'team.leaveTeam': 'Покинуть команду',
  'team.confirmLeave': 'Точно покинуть',
  'team.leaveWarning':
    'Вы потеряете доступ к команде и всем её ресурсам. Чтобы вернуться, понадобится новое приглашение.',
  'team.management': 'Управление командой',
  'team.notFound': 'Команда не найдена',
  'team.accessDenied': 'Доступ запрещён',
  'team.members': 'Участники',

  // Voice
  'voice.title': 'Голосовой ввод',
  'voice.settings': 'Настройки голоса',
  'voice.settingsDesc': 'Удерживайте горячую клавишу, чтобы диктовать и вставлять текст в активное поле.',
  'voice.hotkey': 'Горячая клавиша',
  'voice.activationMode': 'Режим активации',
  'voice.tapToToggle': 'Нажмите для переключения',
  'voice.writingStyle': 'Стиль письма',
  'voice.verbatimTranscription': 'Дословная транскрипция',
  'voice.naturalCleanup': 'Естественная очистка',
  'voice.autoStart': 'Запускать голосовой сервер автоматически вместе с core',
  'voice.customDictionary': 'Пользовательский словарь',
  'voice.customDictionaryDesc':
    'Добавляйте имена, технические термины и специфичные слова, чтобы повысить точность распознавания.',
  'voice.addWord': 'Добавить слово…',
  'voice.sttDisabled':
    'Голосовой ввод отключён, пока не загружена и не готова локальная модель STT.',
  'voice.openLocalAiModel': 'Открыть локальную модель ИИ',
  'voice.serverRestarted': 'Голосовой сервер перезапущен с новыми настройками.',
  'voice.settingsSaved': 'Настройки голоса сохранены.',
  'voice.serverStarted': 'Голосовой сервер запущен.',
  'voice.serverStopped': 'Голосовой сервер остановлен.',
  'voice.saveVoiceSettings': 'Сохранить настройки голоса',
  'voice.startVoiceServer': 'Запустить голосовой сервер',
  'voice.stopVoiceServer': 'Остановить голосовой сервер',
  'voice.debugTitle': 'Отладка голоса',

  // Autocomplete
  'autocomplete.title': 'Автодополнение',
  'autocomplete.settings': 'Настройки',
  'autocomplete.acceptWithTab': 'Принимать клавишей Tab',
  'autocomplete.stylePreset': 'Шаблон стиля',
  'autocomplete.style.balanced': 'Сбалансированный',
  'autocomplete.style.concise': 'Лаконичный',
  'autocomplete.style.formal': 'Формальный',
  'autocomplete.style.casual': 'Разговорный',
  'autocomplete.style.custom': 'Свой',
  'autocomplete.disabledApps': 'Отключённые приложения (по одному bundle/app-токену в строке)',
  'autocomplete.saveSettings': 'Сохранить настройки',
  'autocomplete.saving': 'Сохранение…',
  'autocomplete.runtime': 'Среда выполнения',
  'autocomplete.running': 'Работает',
  'autocomplete.start': 'Запустить',
  'autocomplete.stop': 'Остановить',
  'autocomplete.settingsSaved': 'Настройки автодополнения сохранены.',
  'autocomplete.started': 'Автодополнение запущено.',
  'autocomplete.didNotStart': 'Автодополнение не запустилось. Проверьте, что оно включено.',
  'autocomplete.stopped': 'Автодополнение остановлено.',
  'autocomplete.advancedSettings': 'Расширенные настройки',
  'autocomplete.debugTitle': 'Отладка автодополнения',

  // Chat
  'chat.agentChat': 'Чат с агентом',
  'chat.overrides': 'Переопределения',
  'chat.model': 'Модель',
  'chat.temperature': 'Температура',
  'chat.conversation': 'Разговор',
  'chat.startAgentConversation': 'Начните разговор с агентом.',
  'chat.you': 'Вы',
  'chat.agent': 'Агент',
  'chat.askAgent': 'Спросите агента о чём угодно…',
  'chat.sendMessage': 'Отправить сообщение',

  // Composio
  'composio.triageTitle': 'Триггеры интеграций',
  'composio.triageDesc':
    'Когда включено, каждый входящий триггер Composio проходит шаг ИИ-сортировки, который классифицирует событие и может запустить автоматические действия — один локальный LLM-ход на триггер. Отключите глобально или для отдельных интеграций, если предпочитаете ручную проверку. Если переменная окружения',
  'composio.disableAllTriage': 'Отключить ИИ-сортировку для всех триггеров',
  'composio.triggersStillRecorded': 'Триггеры всё равно записываются в историю — LLM-ход не выполняется.',
  'composio.disableSpecificIntegrations': 'Отключить ИИ-сортировку для отдельных интеграций',
  'composio.settingsSaved': 'Настройки сохранены',
  'composio.saveFailed': 'Не удалось сохранить. Попробуйте ещё раз.',

  // Cron
  'cron.title': 'Cron-задачи',
  'cron.scheduledJobs': 'Запланированные задачи',
  'cron.manageCronJobs': 'Управление cron-задачами из планировщика core.',
  'cron.refreshCronJobs': 'Обновить cron-задачи',

  // Local Model
  'localModel.modelStatus': 'Статус модели',
  'localModel.downloadModels': 'Скачать модели',
  'localModel.usage': 'Использование',
  'localModel.usageDesc':
    'Выберите, какие подсистемы используют локальную модель. Всё, что отключено, уходит в облако.',
  'localModel.enableRuntime': 'Включить локальную среду ИИ',
  'localModel.enableRuntimeDesc':
    'Главный переключатель. По умолчанию выключен — Ollama бездействует. Если включён, дерево саммари, анализ экрана и автодополнение всегда используют локальную модель.',
  'localModel.advancedSettings': 'Расширенные настройки',
  'localModel.debugTitle': 'Отладка локальной модели',

  // Screen Awareness
  'screenAwareness.debugTitle': 'Отладка анализа экрана',

  // Memory
  'memory.debugTitle': 'Отладка памяти',

  // Webhooks
  'webhooks.debugTitle': 'Отладка вебхуков',

  // Notifications
  'notifications.routingTitle': 'Маршрутизация уведомлений',

  // Common (additional)
  'common.reload': 'Перезагрузить',
  'common.skip': 'Пропустить',
  'common.disable': 'Отключить',
  'common.enable': 'Включить',

  // Chat (additional)
  'chat.safetyTimeout':
    'От агента нет ответа уже 2 минуты. Попробуйте ещё раз или проверьте соединение.',
  'chat.filter.all': 'Все',
  'chat.filter.work': 'Работа',
  'chat.filter.briefing': 'Брифинг',
  'chat.filter.notification': 'Уведомление',
  'chat.filter.workers': 'Воркеры',
  'chat.selectThread': 'Выберите тред',
  'chat.threads': 'Треды',
  'chat.noThreads': 'Тредов пока нет',
  'chat.noLabelThreads': 'Тредов «{label}» нет',
  'chat.noWorkerThreads': 'Воркер-тредов пока нет',
  'chat.deleteThread': 'Удалить тред',
  'chat.deleteThreadConfirm': 'Точно удалить «{title}»?',
  'chat.untitledThread': 'Тред без названия',
  'chat.hideSidebar': 'Скрыть боковую панель',
  'chat.showSidebar': 'Показать боковую панель',
  'chat.newThreadShortcut': 'Новый тред (/new)',
  'chat.new': 'Новый',
  'chat.failedToLoadMessages': 'Не удалось загрузить сообщения',
  'chat.thinkingIteration': 'Думаю… ({n})',
  'chat.thinkingDots': 'Думаю…',
  'chat.approachingLimit': 'Приближаемся к лимиту использования',
  'chat.approachingLimitMsg': 'Использовано {pct}% доступной квоты.',
  'chat.upgrade': 'Повысить тариф',
  'chat.weeklyLimitHit': 'Вы исчерпали недельный лимит.',
  'chat.resets': 'Сбрасывается',
  'chat.topUpToContinue': 'Пополните, чтобы продолжить.',
  'chat.budgetComplete': 'Включённый бюджет исчерпан. Добавьте кредиты или повысьте тариф, чтобы продолжить.',
  'chat.rateLimitReached': 'Достигнут 10-часовой лимит запросов.',
  'chat.topUp': 'Пополнить',
  'chat.fiveHourLimit': '5-часовой лимит',
  'chat.weeklyLimit': 'Недельный лимит',
  'chat.left': 'осталось',
  'chat.setup': 'Настроить',
  'chat.switchToText': 'Перейти к тексту',
  'chat.transcribing': 'Транскрибирую…',
  'chat.stopAndSend': 'Остановить и отправить',
  'chat.startTalking': 'Начать говорить',
  'chat.playingVoiceReply': 'Проигрываю голосовой ответ',
  'chat.voiceHint': 'Используйте микрофон, чтобы говорить',
  'chat.micUnavailable': 'Микрофон недоступен',
  'chat.turn': 'ход',
  'chat.turns': 'ходов',
  'chat.openWorkerThread': 'Открыть воркер-тред',

  // Memory (additional)
  'memory.searchAria': 'Поиск по памяти',
  'memory.searchPlaceholder': 'Поиск записей памяти…',
  'memory.sourceFilter.all': 'Все источники',
  'memory.sourceFilter.email': 'Почта',
  'memory.sourceFilter.calendar': 'Календарь',
  'memory.sourceFilter.telegram': 'Telegram',
  'memory.sourceFilter.aiInsight': 'Инсайт ИИ',
  'memory.sourceFilter.system': 'Система',
  'memory.sourceFilter.trading': 'Трейдинг',
  'memory.sourceFilter.security': 'Безопасность',
  'memory.ingestionActivity': 'Активность загрузки',
  'memory.events': 'событий',
  'memory.event': 'событие',
  'memory.overTheLast': 'за последние',
  'memory.months': 'месяцев',
  'memory.peak': 'Пик',
  'memory.perDay': '/день',
  'memory.less': 'Меньше',
  'memory.more': 'Больше',
  'memory.on': '—',
  'memory.loading': 'Загрузка памяти',
  'memory.fetching': 'Получаем записи памяти…',
  'memory.analyzing': 'Анализ памяти',
  'memory.analyzingHint': 'Обрабатываем ваши записи, чтобы извлечь инсайты…',
  'memory.noMatches': 'Совпадений не найдено',
  'memory.noMatchesHint': 'Попробуйте изменить поисковый запрос или фильтры.',
  'memory.allCaughtUp': 'Всё обработано',
  'memory.allCaughtUpHint': 'Новых записей для обработки нет.',
  'memory.noAnalysis': 'Анализа пока нет',
  'memory.noAnalysisHint': 'Запустите анализ, чтобы найти закономерности в воспоминаниях.',
  'memory.emptyHint': 'Начните взаимодействие, чтобы создать первые записи.',

  // Mic
  'mic.unavailable': 'Микрофон недоступен',
  'mic.permissionDenied': 'В доступе к микрофону отказано',
  'mic.failedToStartRecorder': 'Не удалось запустить запись',
  'mic.transcribing': 'Транскрибирую…',
  'mic.tapToSend': 'Нажмите, чтобы отправить',
  'mic.waitingForAgent': 'Ожидаем агента…',
  'mic.tapAndSpeak': 'Нажмите и говорите',
  'mic.stopRecording': 'Остановить запись и отправить',
  'mic.startRecording': 'Начать запись',

  // Token
  'token.usageLimitReached': 'Лимит использования достигнут',
  'token.approachingLimit': 'Приближаемся к лимиту',
  'token.planClickForDetails': 'тариф — нажмите для подробностей',
  'token.sessionTokens': 'Вход: {in} | Выход: {out} | Ходов: {turns}',
  'token.limit': 'Достигнут лимит',

  // Catalog
  'catalog.noCapabilityBinding': 'Привязки возможности нет',
  'catalog.downloadFailed': 'Загрузка не удалась',
  'catalog.active': 'Активна',
  'catalog.installed': 'Установлена',
  'catalog.notDownloaded': 'Не скачана',
  'catalog.inUse': 'Используется',
  'catalog.use': 'Использовать',
  'catalog.deleteModel': 'Удалить модель',
  'catalog.download': 'Скачать',

  // Navigator
  'navigator.recent': 'Недавнее',
  'navigator.today': 'Сегодня',
  'navigator.thisWeek': 'На этой неделе',
  'navigator.sources': 'Источники',
  'navigator.email': 'Почта',
  'navigator.slack': 'Slack',
  'navigator.chat': 'Чат',
  'navigator.documents': 'Документы',
  'navigator.people': 'Люди',
  'navigator.topics': 'Темы',

  // Dreams
  'dreams.description':
    'Сны — это сгенерированные ИИ размышления, которые объединяют закономерности из ваших воспоминаний.',
  'dreams.comingSoon': 'Скоро',

  // Assignment
  'assignment.memoryLlm': 'LLM для памяти',
  'assignment.memoryLlmAria': 'Выбор LLM для памяти',
  'assignment.embedder': 'Эмбеддер',
  'assignment.loaded': 'Загружено',
  'assignment.notDownloaded': 'Не скачано',
  'assignment.usedForExtractSummarise': 'Используется для извлечения и резюмирования',

  // Insights
  'insights.knownFacts': 'Известные факты',
  'insights.preferences': 'Предпочтения',
  'insights.relationships': 'Связи',
  'insights.skills': 'Навыки',
  'insights.opinions': 'Мнения',
  'insights.other': 'Другое',
  'insights.title': 'Инсайты',
  'insights.empty': 'Инсайтов пока нет. Они появляются по мере роста памяти.',
  'insights.description': 'По данным {count} связей в графе памяти.',
  'insights.items': 'элементов',
  'insights.more': 'ещё',

  // Calls
  'calls.joiningCall': 'Подключаемся к звонку',
  'calls.meetWindowOpening': 'Окно Meet открывается…',
  'calls.failedToStart': 'Не удалось запустить звонок Meet',
  'calls.couldNotStart': 'Не удалось начать звонок',
  'calls.failedToClose': 'Не удалось закрыть звонок',
  'calls.couldNotClose': 'Не удалось закрыть звонок',
  'calls.joinMeet': 'Подключиться к звонку Google Meet',
  'calls.joinMeetDescription': 'Введите ссылку на Google Meet, чтобы подключиться.',
  'calls.meetLink': 'Ссылка Meet',
  'calls.displayName': 'Отображаемое имя',
  'calls.openingMeet': 'Открываем Meet…',
  'calls.joinCall': 'Подключиться к звонку',
  'calls.activeCalls': 'Активные звонки',
  'calls.leave': 'Выйти',

  // Workspace
  'workspace.wipeConfirm': 'Точно стереть всю память? Это нельзя отменить.',
  'workspace.resetTreeConfirm': 'Точно пересобрать дерево памяти?',
  'workspace.wipeTitle': 'Стереть память',
  'workspace.resetting': 'Сброс…',
  'workspace.resetMemory': 'Сбросить память',
  'workspace.resetTreeTitle': 'Пересобрать дерево памяти',
  'workspace.rebuilding': 'Пересборка…',
  'workspace.resetMemoryTree': 'Сбросить дерево памяти',
  'workspace.building': 'Сборка…',
  'workspace.buildSummaryTrees': 'Построить деревья саммари',
  'workspace.viewVault': 'Открыть хранилище',
  'workspace.graphLoadFailed': 'Не удалось загрузить граф памяти',
  'workspace.loadingGraph': 'Загрузка графа памяти…',
  'workspace.graphViewMode': 'Режим отображения графа памяти',
  'workspace.trees': 'Деревья',
  'workspace.contacts': 'Контакты',

  // Graph
  'graph.noContactMentions': 'Упоминаний контактов нет',
  'graph.noMemory': 'Памяти нет',
  'graph.source': 'Источник',
  'graph.topic': 'Тема',
  'graph.global': 'Глобально',
  'graph.document': 'Документ',
  'graph.contact': 'Контакт',
  'graph.nodes': 'узлов',
  'graph.parentChild': 'родитель-потомок',
  'graph.documentContact': 'документ-контакт',
  'graph.link': 'связь',
  'graph.links': 'связей',
  'graph.children': 'потомков',
  'graph.clickToOpenObsidian': 'Нажмите, чтобы открыть в Obsidian',
  'graph.person': 'Человек',

  // Modal
  'modal.dontShowAgain': 'Не показывать похожие предложения',

  // Reflections
  'reflections.loading': 'Загрузка размышлений…',
  'reflections.empty': 'Размышлений пока нет',
  'reflections.title': 'Размышления',
  'reflections.proposedAction': 'Предложенное действие',
  'reflections.act': 'Выполнить',
  'reflections.dismiss': 'Скрыть',

  // WhatsApp
  'whatsapp.chatsSynced': 'чатов синхронизировано',
  'whatsapp.chatSynced': 'чат синхронизирован',

  // Sync
  'sync.active': 'Активна',
  'sync.recent': 'Недавно',
  'sync.idle': 'Простой',
  'sync.memorySources': 'Источники памяти',
  'sync.noConnectedSources': 'Подключённых источников нет',
  'sync.chunks': 'фрагментов',
  'sync.lastChunk': 'Последний фрагмент:',
  'sync.pending': 'в ожидании',
  'sync.processed': 'обработано',
  'sync.syncing': 'Синхронизация…',
  'sync.sync': 'Синхронизировать',
  'sync.failedToLoad': 'Не удалось загрузить статус синхронизации',
  'sync.noContent': 'В память пока ничего не синхронизировано. Подключите интеграцию, чтобы начать.',

  // Backend
  'backend.aiBackend': 'Бэкенд ИИ',
  'backend.cloud': 'Облако',
  'backend.recommended': 'Рекомендуем',
  'backend.cloudDescription':
    'Быстрые мощные модели на наших серверах. Готовы к работе сразу.',
  'backend.privacyNote': 'Личные данные, сообщения и ключи никогда не отправляются на наши серверы.',
  'backend.local': 'Локально',
  'backend.advanced': 'Дополнительно',
  'backend.localDescription':
    'Запускайте модели на своей машине через Ollama. Полная приватность, требуется настройка.',
  'backend.ramRecommended': 'Рекомендуется 16 ГБ ОЗУ и больше',

  // Subconscious
  'subconscious.tasks': 'задач',
  'subconscious.ticks': 'тиков',
  'subconscious.last': 'Последний',
  'subconscious.failed': 'не выполнено',
  'subconscious.tickInterval': 'Интервал тика',
  'subconscious.runNow': 'Запустить сейчас',
  'subconscious.approvalNeeded': 'Нужно одобрение',
  'subconscious.requiresApproval': 'Требуется одобрение',
  'subconscious.fixInConnections': 'Исправить в подключениях',
  'subconscious.goAhead': 'Действуй',
  'subconscious.activeTasks': 'Активные задачи',
  'subconscious.noActiveTasks': 'Активных задач нет',
  'subconscious.default': 'По умолчанию',
  'subconscious.addTaskPlaceholder': 'Добавить новую задачу…',
  'subconscious.activityLog': 'Журнал активности',
  'subconscious.noActivity': 'Активности пока нет',
  'subconscious.decision.nothingNew': 'Ничего нового',
  'subconscious.decision.completed': 'Завершено',
  'subconscious.decision.evaluating': 'Оценка',
  'subconscious.decision.waitingApproval': 'Ожидание одобрения',
  'subconscious.decision.failed': 'Не выполнено',
  'subconscious.decision.cancelled': 'Отменено',
  'subconscious.decision.skipped': 'Пропущено',

  // Actionable
  'actionable.complete': 'Завершить',
  'actionable.dismiss': 'Скрыть',
  'actionable.snooze': 'Отложить',
  'actionable.new': 'Новое',

  // Stats
  'stats.storage': 'Хранилище',
  'stats.files': 'файлов',
  'stats.documents': 'Документы',
  'stats.today': 'сегодня',
  'stats.namespaces': 'Пространства имён',
  'stats.relations': 'Связи',
  'stats.firstMemory': 'Первая запись',
  'stats.latest': 'Последняя',
  'stats.sessions': 'Сессии',
  'stats.tokens': 'токенов',

  // Boot Check Gate
  'bootCheck.invalidUrl': 'Введите URL среды выполнения.',
  'bootCheck.urlMustStartWith': 'URL должен начинаться с http:// или https://',
  'bootCheck.validUrlRequired':
    'Это не похоже на корректный URL (попробуйте https://core.example.com/rpc)',
  'bootCheck.tokenRequired': 'Чтобы подключиться, нужен токен авторизации.',
  'bootCheck.chooseCoreMode': 'Выберите среду выполнения',
  'bootCheck.connectToCore': 'Подключение к среде выполнения',
  'bootCheck.desktopDescription': 'OpenHuman нужна среда выполнения, чтобы думать. Выберите, где она будет жить.',
  'bootCheck.webDescription':
    'В вебе OpenHuman подключается к среде выполнения, которой управляете вы. Укажите её URL и токен ниже, или скачайте десктоп-приложение, чтобы запустить среду прямо на своей машине.',
  'bootCheck.preferDesktop': 'Хотите оставить всё на своём устройстве?',
  'bootCheck.downloadDesktop': 'Скачать десктоп-приложение',
  'bootCheck.localRecommended': 'Запустить локально (рекомендуем)',
  'bootCheck.localDescription':
    'Работает прямо на вашем компьютере. Самый быстрый и приватный вариант, настраивать ничего не нужно.',
  'bootCheck.cloudMode': 'Запустить в облаке (сложнее)',
  'bootCheck.cloudDescription':
    'Подключение к среде, которую вы развернули в другом месте. Она работает 24×7, и держать это устройство включённым не обязательно.',
  'bootCheck.coreRpcUrl': 'URL среды выполнения',
  'bootCheck.rpcUrlPlaceholder': 'https://core.example.com/rpc',
  'bootCheck.authToken': 'Токен авторизации',
  'bootCheck.bearerTokenPlaceholder': 'Bearer-токен от вашей удалённой среды',
  'bootCheck.storedLocally': 'Хранится только на этом устройстве. Отправляется как ',
  'bootCheck.testing': 'Проверка…',
  'bootCheck.testConnection': 'Проверить соединение',
  'bootCheck.connectedOk': 'Подключено. Можно работать.',
  'bootCheck.authFailed': 'Этот токен не подошёл. Проверьте его и попробуйте ещё раз.',
  'bootCheck.unreachablePrefix': 'Не удалось достучаться:',
  'bootCheck.checkingCore': 'Будим вашу среду выполнения…',
  'bootCheck.cannotReach': 'Не удаётся достучаться до среды',
  'bootCheck.cannotReachDesc': 'Не получилось подключиться к среде выполнения. Попробовать другую?',
  'bootCheck.switchMode': 'Выбрать другую среду',
  'bootCheck.quit': 'Выйти',
  'bootCheck.legacyDetected': 'Обнаружена устаревшая фоновая среда',
  'bootCheck.legacyDescription':
    'На этом устройстве уже запущен отдельно установленный демон OpenHuman. Его нужно убрать, чтобы встроенная среда могла взять управление.',
  'bootCheck.removing': 'Удаление…',
  'bootCheck.removeContinue': 'Удалить и продолжить',
  'bootCheck.localNeedsRestart': 'Локальной среде нужен перезапуск',
  'bootCheck.localNeedsRestartDesc':
    'Версия локальной среды отличается от версии этого приложения. Быстрый перезапуск приведёт их в соответствие.',
  'bootCheck.restarting': 'Перезапуск…',
  'bootCheck.restartCore': 'Перезапустить среду',
  'bootCheck.cloudNeedsUpdate': 'Облачной среде нужно обновление',
  'bootCheck.cloudNeedsUpdateDesc':
    'Версия облачной среды отличается от версии этого приложения. Запустите обновление, чтобы привести их в соответствие.',
  'bootCheck.updating': 'Обновление…',
  'bootCheck.updateCloudCore': 'Обновить облачную среду',
  'bootCheck.versionCheckFailed': 'Проверка версии среды не удалась',
  'bootCheck.versionCheckFailedDesc':
    'Среда выполнения работает, но не сообщает свою версию. Возможно, она устарела. Перезапустите или обновите, чтобы продолжить.',
  'bootCheck.working': 'Работаем…',
  'bootCheck.restartUpdateCore': 'Перезапустить / обновить среду',
  'bootCheck.unexpectedError': 'Неожиданная ошибка проверки запуска',
  'bootCheck.actionFailed': 'Что-то пошло не так. Попробуйте ещё раз.',

  // Notifications: category labels & timestamps
  'notifications.justNow': 'только что',
  'notifications.minAgo': '{n} мин назад',
  'notifications.hrAgo': '{n} ч назад',
  'notifications.dayAgo': '{n} дн назад',
  'notifications.category.messages': 'Сообщения',
  'notifications.category.agents': 'Агенты',
  'notifications.category.skills': 'Навыки',
  'notifications.category.system': 'Система',
  'notifications.category.meetings': 'Встречи',
  'notifications.category.reminders': 'Напоминания',
  'notifications.category.important': 'Важное',

  // About / Updates: status summary phrases
  'about.update.status.checking': 'Проверка…',
  'about.update.status.available': 'Доступна v{version}',
  'about.update.status.availableNoVersion': 'Доступно обновление',
  'about.update.status.downloading': 'Скачивание…',
  'about.update.status.readyToInstall': 'v{version} готова к установке',
  'about.update.status.readyToInstallNoVersion':
    'Новая версия скачана и готова. Перезапустите, чтобы применить.',
  'about.update.status.installing': 'Установка…',
  'about.update.status.restarting': 'Перезапуск…',
  'about.update.status.upToDate': 'Установлена последняя версия.',
  'about.update.status.error': 'Проверка обновлений не удалась',
  'about.update.status.default': 'Проверить обновления',

  // Welcome: connection error messages
  'welcome.connectionFailed': 'Не удалось подключиться: {status} {statusText}',
  'welcome.connectionFailedMsg': 'Не удалось подключиться: {message}',

  // Chat: Agent chat panel description
  'chat.agentChatDesc': 'Откройте прямой чат с агентом.',

  // Channels: active route interpolated value
  'channels.activeRouteValue': '{channel} через {authMode}',

  // Privacy: data kind labels for What Leaves My Computer
  'privacy.dataKind.messages': 'Сообщения',
  'privacy.dataKind.agents': 'Агенты',
  'privacy.dataKind.skills': 'Навыки',
  'privacy.dataKind.system': 'Система',
  'privacy.dataKind.meetings': 'Встречи',
  'privacy.dataKind.reminders': 'Напоминания',
  'privacy.dataKind.important': 'Важное',

  // Onboarding: supplementary keys
  'onboarding.enableLocalAI': 'Включить локальный ИИ',
  'onboarding.skills.status.available': 'Доступно',
  'onboarding.skills.status.connected': 'Подключено',
  'onboarding.skills.status.connecting': 'Подключение',
  'onboarding.skills.status.error': 'Ошибка',
  'onboarding.skills.status.unavailable': 'Недоступно',

  // Composio: miscellaneous
  'composio.statusUnavailable': 'Статус недоступен',
  'composio.authExpired': 'Авторизация истекла',
  'composio.reconnect': 'Переподключить',
  'composio.envVarOverrides': 'задана, она переопределяет эту настройку.',

  // Memory: day-of-week labels for heatmap
  'memory.day.sun': 'Вс',
  'memory.day.mon': 'Пн',
  'memory.day.tue': 'Вт',
  'memory.day.wed': 'Ср',
  'memory.day.thu': 'Чт',
  'memory.day.fri': 'Пт',
  'memory.day.sat': 'Сб',

  // Memory: ingestion status labels
  'memory.ingesting': 'Загружается',
  'memory.ingestionQueued': 'В очереди',
  'memory.ingestingTitle': 'Загружается {title}',

  // Mic: error messages
  'mic.noAudioCaptured': 'Звук не записан',
  'mic.noSpeechDetected': 'Речь не обнаружена',
  'mic.failedToStopRecording': 'Не удалось остановить запись: {message}',
  'mic.transcriptionFailed': 'Транскрипция не удалась: {message}',

  // Reflections: kind labels
  'reflections.kind.retrospective': 'Ретроспектива',
  'reflections.kind.derivedFact': 'Производный факт',
  'reflections.kind.moodInsight': 'Инсайт о настроении',
  'reflections.kind.relationshipInsight': 'Инсайт о связях',

  // Graph: tooltip keys
  'graph.tooltip.summary': 'Саммари',
  'graph.tooltip.contact': 'Контакт',

  // Local Model: usage labels
  'localModel.usage.never': 'Никогда',
  'localModel.usage.mediumLoad': 'Средняя нагрузка',
  'localModel.usage.lowLoad': 'Низкая нагрузка',
  'localModel.usage.idleMode': 'Режим простоя',
  'localModel.rebootstrapComplete': 'Повторная инициализация модели завершена.',
  'localModel.modelsVerified': 'Локальные модели проверены.',
};

export default ru;
