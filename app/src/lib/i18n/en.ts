import type { TranslationMap } from './types';

const en: TranslationMap = {
  // Navigation
  'nav.home': 'Home',
  'nav.human': 'Human',
  'nav.chat': 'Chat',
  'nav.connections': 'Connections',
  'nav.memory': 'Intelligence',
  'nav.alerts': 'Alerts',
  'nav.rewards': 'Rewards',
  'nav.settings': 'Settings',

  // Common
  'common.cancel': 'Cancel',
  'common.save': 'Save',
  'common.confirm': 'Confirm',
  'common.delete': 'Delete',
  'common.edit': 'Edit',
  'common.create': 'Create',
  'common.search': 'Search',
  'common.loading': 'loading…',
  'common.error': 'Error',
  'common.success': 'Success',
  'common.back': 'Back',
  'common.next': 'Next',
  'common.finish': 'Finish',
  'common.close': 'Close',
  'common.enabled': 'Enabled',
  'common.disabled': 'Disabled',
  'common.on': 'On',
  'common.off': 'Off',
  'common.yes': 'Yes',
  'common.no': 'No',
  'common.ok': 'Got it',
  'common.retry': 'Retry',
  'common.copy': 'Copy',
  'common.copied': 'Copied!',
  'common.learnMore': 'Learn more',
  'common.seeAll': 'View',
  'common.dismiss': 'Dismiss',
  'common.clear': 'Clear',
  'common.reset': 'Reset',
  'common.refresh': 'Refresh',
  'common.export': 'Export',
  'common.import': 'Import',
  'common.upload': 'Upload',
  'common.download': 'Download',
  'common.add': 'Add',
  'common.remove': 'Remove',
  'common.showMore': 'Show more',
  'common.showLess': 'Show less',
  'common.submit': 'Submit',
  'common.continue': 'Continue',

  // Settings Home
  'settings.general': 'General',
  'settings.featuresAndAI': 'Features & AI',
  'settings.billingAndRewards': 'Billing & Rewards',
  'settings.support': 'Support',
  'settings.advanced': 'Advanced',
  'settings.dangerZone': 'Danger Zone',
  'settings.account': 'Account',
  'settings.accountDesc': 'Recovery phrase, team, connections, and privacy',
  'settings.notifications': 'Notifications',
  'settings.notificationsDesc': 'Do Not Disturb and per-account notification controls',
  'settings.features': 'Features',
  'settings.featuresDesc': 'Screen awareness, messaging, and tools',
  'settings.aiModels': 'AI & Models',
  'settings.aiModelsDesc': 'Local AI model setup, downloads, and LLM provider',
  'settings.ai': 'AI Configuration',
  'settings.aiDesc': 'Cloud providers, local Ollama models, and per-workload routing',
  'settings.billingUsage': 'Billing & Usage',
  'settings.billingUsageDesc': 'Subscription plan, credits, and payment methods',
  'settings.rewards': 'Rewards',
  'settings.rewardsDesc': 'Referrals, coupons, and earned credits',
  'settings.restartTour': 'Restart Tour',
  'settings.restartTourDesc': 'Replay the product walkthrough from the beginning',
  'settings.about': 'About',
  'settings.aboutDesc': 'App version and software updates',
  'settings.developerOptions': 'Advanced',
  'settings.developerOptionsDesc':
    'AI configuration, messaging channels, tools, diagnostics, and debug panels',
  'settings.clearAppData': 'Clear App Data',
  'settings.clearAppDataDesc': 'Sign out and permanently clear all local app data',
  'settings.logOut': 'Log out',
  'settings.logOutDesc': 'Sign out of your account',
  'settings.language': 'Language',
  'settings.languageDesc': 'Display language for the app interface',
  'settings.alerts': 'Alerts',
  'settings.alertsDesc': 'View recent alerts and activity in your inbox',

  // Settings: Account
  'settings.account.recoveryPhrase': 'Recovery Phrase',
  'settings.account.recoveryPhraseDesc': 'View and back up your account recovery phrase',
  'settings.account.team': 'Team',
  'settings.account.teamDesc': 'Manage team members and permissions',
  'settings.account.connections': 'Connections',
  'settings.account.connectionsDesc': 'Manage linked accounts and services',
  'settings.account.privacy': 'Privacy',
  'settings.account.privacyDesc': 'Control what data leaves your computer',

  // Settings: Notifications
  'settings.notifications.doNotDisturb': 'Do Not Disturb',
  'settings.notifications.doNotDisturbDesc': 'Pause all notifications for a set period',
  'settings.notifications.channelControls': 'Per-Channel Controls',
  'settings.notifications.channelControlsDesc':
    'Configure notification preferences for each channel',

  // Settings: Features
  'settings.features.screenAwareness': 'Screen Awareness',
  'settings.features.screenAwarenessDesc': 'Let the assistant see your active window',
  'settings.features.messaging': 'Messaging',
  'settings.features.messagingDesc': 'Channel and messaging integration settings',
  'settings.features.tools': 'Tools',
  'settings.features.toolsDesc': 'Manage connected tools and integrations',

  // Settings: AI & Models
  'settings.ai.localSetup': 'Local AI Setup',
  'settings.ai.localSetupDesc': 'Download and configure local AI models',
  'settings.ai.llmProvider': 'LLM Provider',
  'settings.ai.llmProviderDesc': 'Choose and configure your AI provider',

  // Clear App Data modal
  'clearData.title': 'Clear App Data',
  'clearData.warning': 'This will sign you out and permanently delete local app data including:',
  'clearData.bulletSettings': 'App settings and conversations',
  'clearData.bulletCache': 'All local integration cache data',
  'clearData.bulletWorkspace': 'Workspace data',
  'clearData.bulletOther': 'All other local data',
  'clearData.irreversible': 'This action cannot be undone.',
  'clearData.clearing': 'Clearing App Data...',
  'clearData.failed': 'Failed to clear data and logout. Please try again.',
  'clearData.failedLogout': 'Failed to log out. Please try again.',
  'clearData.failedPersist': 'Failed to clear persisted app state. Please try again.',

  // Welcome page
  'welcome.title': 'Welcome to OpenHuman',
  'welcome.subtitle':
    'Your personal AI super intelligence. Private, simple and extremely powerful.',
  'welcome.connectPrompt': 'Configure RPC URL (Advanced)',
  'welcome.selectRuntime': 'Select a Runtime',
  'welcome.urlPlaceholder': 'http://localhost:8089',
  'welcome.invalidUrl': 'Please enter a valid HTTP or HTTPS URL',
  'welcome.connecting': 'Testing',
  'welcome.connect': 'Test',

  // Home page
  'home.greeting': 'Good morning',
  'home.greetingAfternoon': 'Good afternoon',
  'home.greetingEvening': 'Good evening',
  'home.askAssistant': 'Ask your assistant anything...',
  'home.statusOk':
    'Your device is connected. Keep the app running to keep the connection alive. Message your agent with the button below.',
  'home.statusBackendOnly': 'Reconnecting to backend… your agent will be available again shortly.',
  'home.statusCoreUnreachable':
    "Local core sidecar isn't responding. The OpenHuman background process may have crashed or failed to start.",
  'home.statusInternetOffline':
    'Your device is offline right now. Check your network or restart the app to reconnect.',
  'home.restartCore': 'Restart Core',
  'home.restartingCore': 'Restarting core…',

  // Chat / Conversations
  'chat.newThread': 'New thread',
  'chat.typeMessage': 'Type a message...',
  'chat.send': 'Send message',
  'chat.thinking': 'Thinking...',
  'chat.noMessages': 'No messages yet',
  'chat.startConversation': 'Start a conversation',
  'chat.regenerate': 'Regenerate',
  'chat.copyResponse': 'Copy response',
  'chat.citations': 'Citations',
  'chat.toolUsed': 'Tool used',

  // Skills / Connections
  'scope.legacy': 'Legacy',
  'scope.user': 'User',
  'scope.project': 'Project',
  'skills.title': 'Connections',
  'skills.search': 'Search connections...',
  'skills.noResults': 'No connections found',
  'skills.connect': 'Connect',
  'skills.disconnect': 'Disconnect',
  'skills.configure': 'Manage',
  'skills.connected': 'Connected',
  'skills.available': 'Available',
  'skills.addAccount': 'Add Account',
  'skills.channels': 'Channels',
  'skills.integrations': 'Integrations',

  // Intelligence / Memory
  'memory.title': 'Memory',
  'memory.search': 'Search memories...',
  'memory.noResults': 'No memories found',
  'memory.empty': 'No memories yet. Memories are created automatically as you interact.',
  'memory.tab.memory': 'Memory',
  'memory.tab.subconscious': 'Subconscious',
  'memory.tab.dreams': 'Dreams',
  'memory.tab.calls': 'Calls',
  'memory.tab.settings': 'Settings',
  'memory.analyzeNow': 'Analyze Now',

  // Notifications / Alerts
  'alerts.title': 'Alerts',
  'alerts.empty': 'No alerts yet',
  'alerts.markAllRead': 'Mark all as read',
  'alerts.unread': 'unread',

  // Rewards
  'rewards.title': 'Rewards',
  'rewards.referrals': 'Referrals',
  'rewards.coupons': 'Redeem',
  'rewards.credits': 'Credits',
  'rewards.referralCode': 'Your referral code',
  'rewards.copyCode': 'Copy code',
  'rewards.share': 'Share',

  // Onboarding
  'onboarding.welcome': "Hi. I'm OpenHuman.",
  'onboarding.welcomeDesc':
    'Your super-intelligent AI assistant that runs on your computer. Private, simple, and extremely powerful.',
  'onboarding.context': 'Context Gathering',
  'onboarding.contextDesc': 'Connect the tools and services you use every day.',
  'onboarding.localAI': 'Local AI',
  'onboarding.localAIDesc': 'Set up a local AI model that runs on your machine.',
  'onboarding.chatProvider': 'Chat Provider',
  'onboarding.chatProviderDesc': 'Choose how you want to interact with your assistant.',
  'onboarding.referral': 'Referral',
  'onboarding.referralDesc': 'Apply a referral code if you have one.',
  'onboarding.finish': 'Finish Setup',
  'onboarding.finishDesc': "You're all set! Start using OpenHuman.",
  'onboarding.skip': 'Skip',
  'onboarding.getStarted': 'Get Started',

  // Onboarding: runtime-choice step (Cloud vs Custom)
  'onboarding.runtimeChoice.title': 'How would you like to run OpenHuman?',
  'onboarding.runtimeChoice.subtitle':
    'Pick the setup that fits you best. You can change this later in Settings.',
  'onboarding.runtimeChoice.cloud.title': 'Simple',
  'onboarding.runtimeChoice.cloud.tagline': 'Let OpenHuman manage everything for you.',
  'onboarding.runtimeChoice.cloud.f1': 'Built-in security',
  'onboarding.runtimeChoice.cloud.f2': 'Token compression to stretch your usage further',
  'onboarding.runtimeChoice.cloud.f3': 'One subscription, every model included',
  'onboarding.runtimeChoice.cloud.f4': 'No API keys to manage',
  'onboarding.runtimeChoice.cloud.f5': 'Simple to set up',
  'onboarding.runtimeChoice.custom.title': 'Run Custom',
  'onboarding.runtimeChoice.custom.tagline':
    "Bring your own keys. Full control of what you're using.",
  'onboarding.runtimeChoice.custom.f1': "You'll need API keys for almost everything",
  'onboarding.runtimeChoice.custom.f2': 'Reuses services you already pay for',
  'onboarding.runtimeChoice.custom.f3': 'Can be free if you run everything locally',
  'onboarding.runtimeChoice.custom.f4': 'More setup, more knobs',
  'onboarding.runtimeChoice.custom.f5': 'Best for power users and developers',
  'onboarding.runtimeChoice.cloud.creditHighlight': '$1 free credit to try it out',
  'onboarding.runtimeChoice.continueCloud': 'Continue with Simple',
  'onboarding.runtimeChoice.continueCustom': 'Continue with Custom',
  'onboarding.runtimeChoice.recommended': 'Recommended',

  // Onboarding: API keys step (only when Custom is picked)
  'onboarding.apiKeys.title': "Let's Add Your API Keys",
  'onboarding.apiKeys.subtitle':
    'You can paste them now or skip and add them later in Settings › AI. Keys are stored on this device, encrypted at rest.',
  'onboarding.apiKeys.openaiLabel': 'OpenAI API key',
  'onboarding.apiKeys.openaiPlaceholder': 'sk-...',
  'onboarding.apiKeys.anthropicLabel': 'Anthropic API key',
  'onboarding.apiKeys.anthropicPlaceholder': 'sk-ant-...',
  'onboarding.apiKeys.saveError': "Couldn't save that key. Please double-check it and try again.",
  'onboarding.apiKeys.skipForNow': 'Skip for now',
  'onboarding.apiKeys.continue': 'Save and continue',
  'onboarding.apiKeys.saving': 'Saving…',

  // Onboarding: Custom wizard (Inference / Voice / OAuth / Search / Memory)
  'onboarding.custom.stepperInference': 'Inference',
  'onboarding.custom.stepperVoice': 'Voice',
  'onboarding.custom.stepperOAuth': 'OAuth',
  'onboarding.custom.stepperSearch': 'Search',
  'onboarding.custom.stepperMemory': 'Memory',
  'onboarding.custom.stepCounter': 'Step {n} of {total}',
  'onboarding.custom.defaultTitle': 'Default',
  'onboarding.custom.defaultSubtitle': 'Let OpenHuman manage it for you.',
  'onboarding.custom.configureTitle': 'Configure',
  'onboarding.custom.configureSubtitle': "I'll pick what to use.",
  'onboarding.custom.progressAriaLabel': 'Onboarding progress',
  'onboarding.custom.continue': 'Continue',
  'onboarding.custom.back': 'Back',
  'onboarding.custom.finish': 'Finish Setup',
  'onboarding.custom.configureLater':
    "You can finish wiring this up after onboarding. We'll drop you on the matching Settings page once you're done.",
  'onboarding.custom.openSettings': 'Open in Settings',

  // Onboarding: Custom > Inference (text)
  'onboarding.custom.inference.title': 'Inference (Text)',
  'onboarding.custom.inference.subtitle':
    'Which language model should answer your questions and run your agents?',
  'onboarding.custom.inference.defaultDesc':
    'OpenHuman routes every workload to a sensible default model. No keys, no setup.',
  'onboarding.custom.inference.configureDesc':
    'Bring your own OpenAI or Anthropic key. We use it for every text-based workload.',

  // Onboarding: Custom > Voice
  'onboarding.custom.voice.title': 'Voice',
  'onboarding.custom.voice.subtitle': 'Speech-to-text and text-to-speech for voice mode.',
  'onboarding.custom.voice.defaultDesc':
    'OpenHuman ships with managed STT/TTS that just works. Nothing to wire up.',
  'onboarding.custom.voice.configureDesc':
    'Use your own ElevenLabs / OpenAI Whisper / etc. Configure in Settings › Voice.',

  // Onboarding: Custom > OAuth (Composio)
  'onboarding.custom.oauth.title': 'Connections (OAuth)',
  'onboarding.custom.oauth.subtitle':
    'Gmail, Slack, Notion, and other connected services that need OAuth.',
  'onboarding.custom.oauth.defaultDesc':
    'OpenHuman runs a managed Composio workspace. One click to connect each service later.',
  'onboarding.custom.oauth.configureDesc':
    'Bring your own Composio account / API key. Configure in Settings › Connections.',

  // Onboarding: Custom > Search
  'onboarding.custom.search.title': 'Web Search',
  'onboarding.custom.search.subtitle': 'How OpenHuman searches the web on your behalf.',
  'onboarding.custom.search.defaultDesc':
    'OpenHuman uses a managed search backend. No keys needed.',
  'onboarding.custom.search.configureDesc':
    'Bring your own search provider key (Tavily, Brave, etc.). Configure in Settings › Tools.',

  // Onboarding: Custom > Memory
  'onboarding.custom.memory.title': 'Memory',
  'onboarding.custom.memory.subtitle':
    'How OpenHuman remembers your context, preferences, and prior conversations.',
  'onboarding.custom.memory.defaultDesc':
    'OpenHuman manages memory storage and retrieval automatically. Nothing to set up.',
  'onboarding.custom.memory.configureDesc':
    'Inspect, export, or wipe memory yourself. Configure in Settings › Memory.',

  // Accounts
  'accounts.addAccount': 'Add Account',
  'accounts.manageAccounts': 'Manage Accounts',
  'accounts.noAccounts': 'No accounts connected',
  'accounts.connectAccount': 'Connect an account to get started',
  'accounts.agent': 'Agent',
  'accounts.respondQueue': 'Respond Queue',
  'accounts.disconnect': 'Disconnect',
  'accounts.disconnectConfirm': 'Are you sure you want to disconnect this account?',
  'accounts.searchAccounts': 'Search accounts...',

  // Channels
  'channels.title': 'Channels',
  'channels.configure': 'Configure Channel',
  'channels.setup': 'Setup',
  'channels.noChannels': 'No channels configured',
  'channels.addChannel': 'Add Channel',
  'channels.status.connected': 'Connected',
  'channels.status.disconnected': 'Disconnected',
  'channels.status.error': 'Error',
  'channels.status.configuring': 'Configuring',
  'channels.defaultMessaging': 'Default Messaging Channel',

  // Webhooks
  'webhooks.title': 'Webhooks',
  'webhooks.create': 'Create Webhook',
  'webhooks.noWebhooks': 'No webhooks configured',
  'webhooks.url': 'URL',
  'webhooks.secret': 'Secret',
  'webhooks.events': 'Events',
  'webhooks.archiveDirectory': 'Archive Directory',
  'webhooks.todayFile': "Today's File",

  // Invites
  'invites.title': 'Invites',
  'invites.create': 'Create Invite',
  'invites.noInvites': 'No pending invites',
  'invites.code': 'Invite Code',
  'invites.copyLink': 'Copy Link',

  // Developer Options
  'devOptions.title': 'Advanced',
  'devOptions.diagnostics': 'Diagnostics',
  'devOptions.diagnosticsDesc': 'System health, logs, and performance metrics',
  'devOptions.debugPanels': 'Debug Panels',
  'devOptions.debugPanelsDesc': 'Feature flags, state inspection, and debugging tools',
  'devOptions.webhooks': 'Webhooks',
  'devOptions.webhooksDesc': 'Configure and test webhook integrations',
  'devOptions.memoryInspection': 'Memory Inspection',
  'devOptions.memoryInspectionDesc': 'Browse, query, and manage memory entries',

  // Voice / Dictation
  'voice.pushToTalk': 'Push to Talk',
  'voice.recording': 'Recording...',
  'voice.processing': 'Processing...',
  'voice.languageHint': 'Language',

  // Misc
  'misc.rehydrating': 'Loading your data...',
  'misc.checkingServices': 'Checking services...',
  'misc.serviceUnavailable': 'Service Unavailable',
  'misc.somethingWentWrong': 'Something went wrong',
  'misc.tryAgainLater': 'Please try again later.',
  'misc.restartApp': 'Restart App',
  'misc.updateAvailable': 'Update Available',
  'misc.updateNow': 'Update Now',
  'misc.updateLater': 'Later',
  'misc.downloading': 'Downloading...',
  'misc.installing': 'Installing...',
  'misc.beta':
    'OpenHuman is in early beta. Feel free to share feedback or report any bugs you run into — every report helps us ship faster.',
  'misc.betaFeedback': 'Send feedback',

  // Mnemonic / Recovery
  'mnemonic.title': 'Recovery Phrase',
  'mnemonic.warning': 'Write down these words in order and store them somewhere safe.',
  'mnemonic.copyWarning':
    'Never share your recovery phrase. Anyone with these words can access your account.',
  'mnemonic.copied': 'Recovery phrase copied to clipboard',
  'mnemonic.reveal': 'Reveal phrase',
  'mnemonic.hidden': 'Recovery phrase is hidden',

  // What Leaves My Computer
  'privacy.title': 'Privacy & Security',
  'privacy.description': 'Transparency report of data sent to external services.',
  'privacy.empty': 'No external data transfers detected.',
  'privacy.whatLeavesComputer': 'What leaves your computer',
  'privacy.loading': 'Loading privacy details...',
  'privacy.loadError': 'Could not load the live privacy list. Analytics controls below still work.',
  'privacy.noCapabilities': 'No capabilities currently disclose data movement.',
  'privacy.sentTo': 'Sent to',
  'privacy.leavesDevice': 'Leaves device',
  'privacy.staysLocal': 'Stays local',
  'privacy.anonymizedAnalytics': 'Anonymized Analytics',
  'privacy.shareAnonymizedData': 'Share Anonymized Usage Data',
  'privacy.shareAnonymizedDataDesc':
    'Help improve OpenHuman by sharing anonymous crash reports and usage analytics. All data is fully anonymized — no personal data, messages, wallet keys, or session information is ever collected.',
  'privacy.meetingFollowUps': 'Meeting follow-ups',
  'privacy.autoHandoffMeet': 'Auto-handoff Google Meet transcripts to the orchestrator',
  'privacy.autoHandoffMeetDesc':
    "When a Google Meet call ends, OpenHuman's orchestrator can read the transcript and may take actions like drafting messages, scheduling follow-ups, or posting summaries to your connected Slack workspace. Off by default.",
  'privacy.analyticsDisclaimer':
    'All analytics and bug reports are fully anonymized. When enabled, we collect only crash information, device type, and the file location of errors. We never access your messages, session data, wallet keys, API keys, or any personally identifiable information. You can change this setting at any time.',

  // Settings: About
  'settings.about.version': 'Version',
  'settings.about.updateAvailable': 'is available',
  'settings.about.softwareUpdates': 'Software updates',
  'settings.about.lastChecked': 'Last checked',
  'settings.about.checking': 'Checking...',
  'settings.about.checkForUpdates': 'Check for updates',
  'settings.about.releases': 'Releases',
  'settings.about.releasesDesc': 'Browse release notes and earlier builds on GitHub.',
  'settings.about.openReleases': 'Open GitHub releases',

  // Settings: AI
  'settings.ai.overview': 'AI System Overview',
  'settings.ai.configStatus': 'Configuration Status',
  'settings.ai.fallbackMode': 'Fallback Mode',
  'settings.ai.loadedFromRuntime': 'Loaded from Runtime',
  'settings.ai.loadingDuration': 'Loading Duration',
  'settings.ai.localRuntime': 'Local Model Runtime',
  'settings.ai.openManager': 'Open Manager',
  'settings.ai.retryDownload': 'Retry Download',
  'settings.ai.state': 'State',
  'settings.ai.targetModel': 'Target Model',
  'settings.ai.download': 'Download',
  'settings.ai.localModelUnavailable': 'Local model status unavailable.',
  'settings.ai.soulConfig': 'SOUL Persona Configuration',
  'settings.ai.refreshing': 'Refreshing...',
  'settings.ai.refreshSoul': 'Refresh SOUL',
  'settings.ai.loadingSoul': 'Loading SOUL configuration...',
  'settings.ai.identity': 'Identity',
  'settings.ai.personality': 'Personality',
  'settings.ai.safetyRules': 'Safety Rules',
  'settings.ai.source': 'Source',
  'settings.ai.loaded': 'Loaded',
  'settings.ai.toolsConfig': 'TOOLS Configuration',
  'settings.ai.refreshTools': 'Refresh TOOLS',
  'settings.ai.toolsAvailable': 'Tools Available',
  'settings.ai.tools': 'tools',
  'settings.ai.activeSkills': 'Active Skills',
  'settings.ai.skills': 'skills',
  'settings.ai.skillsOverview': 'Skills Overview',
  'settings.ai.refreshingAll': 'Refreshing All...',
  'settings.ai.refreshAll': 'Refresh All AI Configuration',

  // Settings: Notifications
  'settings.notifications.suppressAll': 'Suppress all notifications',
  'settings.notifications.suppressAllDesc':
    'Block all OS notification toasts from embedded apps regardless of focus state.',
  'settings.notifications.toggleDnd': 'Toggle Do Not Disturb',
  'settings.notifications.categories': 'Categories',
  'settings.notifications.categoryFooter':
    'Disabling a category stops new notifications of that type from appearing in the notification center. Existing notifications remain until cleared.',

  // Settings: Billing
  'settings.billing.movedToWeb': 'Billing moved to the web',
  'settings.billing.openDashboard': 'Open billing dashboard',
  'settings.billing.movedToWebDesc':
    'Subscription changes, payment methods, credits, and invoices are now managed at TinyHumans on the web.',
  'settings.billing.backToSettings': 'Back to settings',
  'settings.billing.openingBrowser': 'Opening your browser...',
  'settings.billing.browserNotOpen': 'If your browser did not open, use the button above.',
  'settings.billing.browserOpenFailed':
    'The browser could not be opened automatically. Use the button above.',

  // Settings: Tools
  'settings.tools.chooseCapabilities':
    'Choose which capabilities OpenHuman can use on your behalf.',
  'settings.tools.saveChanges': 'Save Changes',
  'settings.tools.preferencesSaved': 'Preferences saved',
  'settings.tools.saveFailed': 'Failed to save preferences. Try again.',

  // Settings: Screen Awareness
  'settings.screenAwareness.mode': 'Mode',
  'settings.screenAwareness.allExceptBlacklist': 'All Except Blacklist',
  'settings.screenAwareness.whitelistOnly': 'Whitelist Only',
  'settings.screenAwareness.screenMonitoring': 'Screen Monitoring',
  'settings.screenAwareness.saveSettings': 'Save Settings',
  'settings.screenAwareness.session': 'Session',
  'settings.screenAwareness.status': 'Status',
  'settings.screenAwareness.active': 'Active',
  'settings.screenAwareness.stopped': 'Stopped',
  'settings.screenAwareness.remaining': 'Remaining',
  'settings.screenAwareness.startSession': 'Start Session',
  'settings.screenAwareness.stopSession': 'Stop Session',
  'settings.screenAwareness.analyzeNow': 'Analyze Now',
  'settings.screenAwareness.macosOnly':
    'Screen Awareness desktop capture and permission controls are currently supported on macOS only.',

  // Connections
  'connections.comingSoon': 'Coming soon',
  'connections.setUp': 'Set up',
  'connections.configured': 'Configured',
  'connections.unavailable': 'Unavailable',
  'connections.checking': 'Checking…',
  'connections.walletConfigured':
    'Local EVM, BTC, Solana, and Tron identities are configured from your recovery phrase.',
  'connections.walletReady':
    'Set up local EVM, BTC, Solana, and Tron identities from one recovery phrase.',
  'connections.walletError':
    'Could not check wallet status. Tap to retry from the Recovery Phrase panel.',
  'connections.walletChecking': 'Checking wallet status...',
  'connections.walletIdentities': 'Wallet identities',
  'connections.walletDerived':
    'Derived locally from your recovery phrase and stored as safe metadata only.',
  'connections.privacySecurity': 'Privacy & Security',
  'connections.privacySecurityDesc':
    'All data and credentials are stored locally with zero-data retention policy. Your information is encrypted and never shared with third parties.',

  // Channels
  'channels.status.connecting': 'Connecting',
  'channels.status.notConfigured': 'Not configured',
  'channels.noActiveRoute': 'No active route',
  'channels.activeRoute': 'Active route',
  'channels.loadingDefinitions': 'Loading channel definitions...',
  'channels.channelConnections': 'Channel Connections',
  'channels.configureAuthModes': 'Configure auth modes for each messaging channel.',
  'channels.configNotAvailable': 'Configuration for',
  'channels.channel': 'channel',

  // Dev Options
  'devOptions.coreModeNotSet': 'Core mode: not set',
  'devOptions.coreModeNotSetDesc':
    "The boot-check picker hasn't been confirmed yet. Use Switch mode on the picker to choose Local or Cloud.",
  'devOptions.local': 'Local',
  'devOptions.embeddedCoreSidecar': 'Embedded core sidecar',
  'devOptions.sidecarSpawned': 'Spawned in-process by the Tauri shell on app launch.',
  'devOptions.cloud': 'Cloud',
  'devOptions.remoteCoreRpc': 'Remote core RPC',
  'devOptions.token': 'Token',
  'devOptions.tokenNotSet': 'not set — RPC will 401',
  'devOptions.triggerSentryTest': 'Trigger Sentry Test (staging)',
  'devOptions.triggerSentryTestDesc':
    'Fires a tagged error to verify the Sentry pipeline. Issue #1072 — remove after verification.',
  'devOptions.sendTestEvent': 'Send test event',
  'devOptions.sending': 'Sending…',
  'devOptions.eventSent': 'Event sent',
  'devOptions.failed': 'Failed',
  'devOptions.appLogs': 'App logs',
  'devOptions.appLogsDesc':
    'Open the folder containing rolling daily log files. Attach the most recent file when reporting an issue.',
  'devOptions.openLogsFolder': 'Open logs folder',

  // Mnemonic
  'mnemonic.phraseSaved': 'Recovery phrase saved',
  'mnemonic.walletReady': 'Multi-chain wallet identities are ready. Returning to settings...',
  'mnemonic.writeDownWords': 'Write down these',
  'mnemonic.wordsInOrder':
    'words in order and store them somewhere safe. This phrase secures your local encryption key and your EVM, BTC, Solana, and Tron wallet identities.',
  'mnemonic.cannotRecover':
    'This phrase can never be recovered if lost and should stay fully local to your device.',
  'mnemonic.copyToClipboard': 'Copy to Clipboard',
  'mnemonic.alreadyHavePhrase': 'I already have a recovery phrase',
  'mnemonic.consentSaved': 'I saved this phrase and consent to using it for local wallet setup',
  'mnemonic.enterPhraseToRestore':
    'Enter your recovery phrase below to restore your local wallet identities, or paste the full phrase into any field (12 words for new backups; 24-word phrases from older versions still work).',
  'mnemonic.words': 'Words',
  'mnemonic.validPhrase': 'Valid recovery phrase',
  'mnemonic.generateNewPhrase': 'Generate a new recovery phrase instead',
  'mnemonic.securingData': 'Securing Your Data...',
  'mnemonic.saveRecoveryPhrase': 'Save Recovery Phrase',
  'mnemonic.userNotLoaded': 'User not loaded. Please sign in again or refresh the page.',
  'mnemonic.invalidPhrase': 'Invalid recovery phrase. Please check your words and try again.',
  'mnemonic.somethingWentWrong': 'Something went wrong. Please try again.',

  // Team
  'team.failedToCreate': 'Failed to create team',
  'team.invalidInviteCode': 'Invalid or expired invite code',
  'team.failedToSwitch': 'Failed to switch team',
  'team.failedToLeave': 'Failed to leave team',
  'team.role.owner': 'Owner',
  'team.role.admin': 'Admin',
  'team.role.billingManager': 'Billing Manager',
  'team.role.member': 'Member',
  'team.active': 'Active',
  'team.personalTeam': 'Personal team',
  'team.manageTeam': 'Manage Team',
  'team.switching': 'Switching...',
  'team.switch': 'Switch',
  'team.leaving': 'Leaving...',
  'team.leave': 'Leave',
  'team.yourTeams': 'Your Teams',
  'team.createNewTeam': 'Create New Team',
  'team.teamName': 'Team name',
  'team.creating': 'Creating...',
  'team.joinExistingTeam': 'Join Existing Team',
  'team.inviteCode': 'Invite code',
  'team.joining': 'Joining...',
  'team.join': 'Join',
  'team.leaveTeam': 'Leave Team',
  'team.confirmLeave': 'Are you sure you want to leave',
  'team.leaveWarning':
    "You will lose access to the team and all team resources. You'll need a new invite to rejoin.",
  'team.management': 'Team Management',
  'team.notFound': 'Team not found',
  'team.accessDenied': 'Access denied',
  'team.members': 'Members',

  // Voice
  'voice.title': 'Voice Dictation',
  'voice.settings': 'Voice Settings',
  'voice.settingsDesc': 'Hold the hotkey to dictate and insert text into the active field.',
  'voice.hotkey': 'Hotkey',
  'voice.activationMode': 'Activation Mode',
  'voice.tapToToggle': 'Tap to toggle',
  'voice.writingStyle': 'Writing Style',
  'voice.verbatimTranscription': 'Verbatim transcription',
  'voice.naturalCleanup': 'Natural cleanup',
  'voice.autoStart': 'Start voice server automatically with the core',
  'voice.customDictionary': 'Custom Dictionary',
  'voice.customDictionaryDesc':
    'Add names, technical terms, and domain words to improve recognition accuracy.',
  'voice.addWord': 'Add a word...',
  'voice.sttDisabled':
    'Voice dictation is disabled until the local STT model is downloaded and ready.',
  'voice.openLocalAiModel': 'Open Local AI Model',
  'voice.serverRestarted': 'Voice server restarted with the new settings.',
  'voice.settingsSaved': 'Voice settings saved.',
  'voice.serverStarted': 'Voice server started.',
  'voice.serverStopped': 'Voice server stopped.',
  'voice.saveVoiceSettings': 'Save Voice Settings',
  'voice.startVoiceServer': 'Start Voice Server',
  'voice.stopVoiceServer': 'Stop Voice Server',
  'voice.debugTitle': 'Voice Debug',

  // Autocomplete
  'autocomplete.title': 'Autocomplete',
  'autocomplete.settings': 'Settings',
  'autocomplete.acceptWithTab': 'Accept With Tab',
  'autocomplete.stylePreset': 'Style Preset',
  'autocomplete.style.balanced': 'Balanced',
  'autocomplete.style.concise': 'Concise',
  'autocomplete.style.formal': 'Formal',
  'autocomplete.style.casual': 'Casual',
  'autocomplete.style.custom': 'Custom',
  'autocomplete.disabledApps': 'Disabled Apps (one bundle/app token per line)',
  'autocomplete.saveSettings': 'Save Settings',
  'autocomplete.saving': 'Saving…',
  'autocomplete.runtime': 'Runtime',
  'autocomplete.running': 'Running',
  'autocomplete.start': 'Start',
  'autocomplete.stop': 'Stop',
  'autocomplete.settingsSaved': 'Autocomplete settings saved.',
  'autocomplete.started': 'Autocomplete started.',
  'autocomplete.didNotStart': 'Autocomplete did not start. Check if it is enabled.',
  'autocomplete.stopped': 'Autocomplete stopped.',
  'autocomplete.advancedSettings': 'Advanced settings',
  'autocomplete.debugTitle': 'Autocomplete Debug',

  // Chat
  'chat.agentChat': 'Agent Chat',
  'chat.overrides': 'Overrides',
  'chat.model': 'Model',
  'chat.temperature': 'Temperature',
  'chat.conversation': 'Conversation',
  'chat.startAgentConversation': 'Start a conversation with the agent.',
  'chat.you': 'You',
  'chat.agent': 'Agent',
  'chat.askAgent': 'Ask the agent anything...',
  'chat.sendMessage': 'Send Message',

  // Composio
  'composio.triageTitle': 'Integration Triggers',
  'composio.triageDesc':
    'When active, each incoming Composio trigger runs through an AI triage step that classifies the event and may kick off automated actions — one local LLM turn per trigger. Disable globally or per integration if you prefer manual review. If the environment variable',
  'composio.disableAllTriage': 'Disable AI triage for all triggers',
  'composio.triggersStillRecorded': 'Triggers are still recorded to history — no LLM turn is run.',
  'composio.disableSpecificIntegrations': 'Disable AI triage for specific integrations',
  'composio.settingsSaved': 'Settings saved',
  'composio.saveFailed': 'Failed to save. Try again.',

  // Cron
  'cron.title': 'Cron Jobs',
  'cron.scheduledJobs': 'Scheduled Jobs',
  'cron.manageCronJobs': 'Manage cron jobs from the core scheduler.',
  'cron.refreshCronJobs': 'Refresh Cron Jobs',

  // Local Model
  'localModel.modelStatus': 'Model Status',
  'localModel.downloadModels': 'Download Models',
  'localModel.usage': 'Usage',
  'localModel.usageDesc':
    'Choose which subsystems run on the local model. Anything off uses the cloud.',
  'localModel.enableRuntime': 'Enable local AI runtime',
  'localModel.enableRuntimeDesc':
    'Master switch. Off by default — Ollama stays idle. When on, the tree summarizer, screen intelligence, and autocomplete always use the local model.',
  'localModel.advancedSettings': 'Advanced settings',
  'localModel.debugTitle': 'Local Model Debug',

  // Screen Awareness
  'screenAwareness.debugTitle': 'Screen Awareness Debug',

  // Memory
  'memory.debugTitle': 'Memory Debug',

  // Webhooks
  'webhooks.debugTitle': 'Webhooks Debug',

  // Notifications
  'notifications.routingTitle': 'Notification Routing',

  // Common (additional)
  'common.reload': 'Reload',
  'common.skip': 'Skip',
  'common.disable': 'Disable',
  'common.enable': 'Enable',

  // Chat (additional)
  'chat.safetyTimeout':
    'No response from the agent after 2 minutes. Try again or check your connection.',
  'chat.filter.all': 'All',
  'chat.filter.work': 'Work',
  'chat.filter.briefing': 'Briefing',
  'chat.filter.notification': 'Notification',
  'chat.selectThread': 'Select a thread',
  'chat.threads': 'Threads',
  'chat.noThreads': 'No threads yet',
  'chat.noLabelThreads': 'No "{label}" threads',
  'chat.deleteThread': 'Delete thread',
  'chat.deleteThreadConfirm': 'Are you sure you want to delete "{title}"?',
  'chat.untitledThread': 'Untitled thread',
  'chat.hideSidebar': 'Hide sidebar',
  'chat.showSidebar': 'Show sidebar',
  'chat.newThreadShortcut': 'New thread (/new)',
  'chat.new': 'New',
  'chat.failedToLoadMessages': 'Failed to load messages',
  'chat.thinkingIteration': 'Thinking... ({n})',
  'chat.thinkingDots': 'Thinking...',
  'chat.approachingLimit': 'Approaching usage limit',
  'chat.approachingLimitMsg': 'You have used {pct}% of your available quota.',
  'chat.upgrade': 'Upgrade',
  'chat.weeklyLimitHit': "You've hit your weekly limit.",
  'chat.resets': 'Resets',
  'chat.topUpToContinue': 'Top up to continue.',
  'chat.budgetComplete': 'Your included budget is complete. Add credits or upgrade to continue.',
  'chat.rateLimitReached': '10-hour rate limit reached.',
  'chat.topUp': 'Top Up',
  'chat.fiveHourLimit': '5-hour limit',
  'chat.weeklyLimit': 'Weekly limit',
  'chat.left': 'left',
  'chat.setup': 'Set up',
  'chat.switchToText': 'Switch to text',
  'chat.transcribing': 'Transcribing...',
  'chat.stopAndSend': 'Stop and send',
  'chat.startTalking': 'Start talking',
  'chat.playingVoiceReply': 'Playing voice reply',
  'chat.voiceHint': 'Use the mic to speak',
  'chat.micUnavailable': 'Microphone unavailable',
  'chat.turn': 'turn',
  'chat.turns': 'turns',
  'chat.openWorkerThread': 'Open worker thread',

  // Memory (additional)
  'memory.searchAria': 'Search memory',
  'memory.searchPlaceholder': 'Search memory entries...',
  'memory.sourceFilter.all': 'All sources',
  'memory.sourceFilter.email': 'Email',
  'memory.sourceFilter.calendar': 'Calendar',
  'memory.sourceFilter.telegram': 'Telegram',
  'memory.sourceFilter.aiInsight': 'AI Insight',
  'memory.sourceFilter.system': 'System',
  'memory.sourceFilter.trading': 'Trading',
  'memory.sourceFilter.security': 'Security',
  'memory.ingestionActivity': 'Ingestion Activity',
  'memory.events': 'events',
  'memory.event': 'event',
  'memory.overTheLast': 'over the last',
  'memory.months': 'months',
  'memory.peak': 'Peak',
  'memory.perDay': '/day',
  'memory.less': 'Less',
  'memory.more': 'More',
  'memory.on': 'on',
  'memory.loading': 'Loading Memory',
  'memory.fetching': 'Fetching your memory entries...',
  'memory.analyzing': 'Analyzing Memory',
  'memory.analyzingHint': 'Processing your memories to extract insights...',
  'memory.noMatches': 'No Matches Found',
  'memory.noMatchesHint': 'Try changing your search terms or filters.',
  'memory.allCaughtUp': 'All Caught Up',
  'memory.allCaughtUpHint': 'No new memory entries to process.',
  'memory.noAnalysis': 'No Analysis Yet',
  'memory.noAnalysisHint': 'Run an analysis to discover patterns in your memories.',
  'memory.emptyHint': 'Start interacting to create your first memories.',

  // Mic
  'mic.unavailable': 'Microphone is not available',
  'mic.permissionDenied': 'Microphone permission denied',
  'mic.failedToStartRecorder': 'Failed to start recorder',
  'mic.transcribing': 'Transcribing...',
  'mic.tapToSend': 'Tap to send',
  'mic.waitingForAgent': 'Waiting for agent...',
  'mic.tapAndSpeak': 'Tap and speak',
  'mic.stopRecording': 'Stop recording and send',
  'mic.startRecording': 'Start recording',

  // Token
  'token.usageLimitReached': 'Usage limit reached',
  'token.approachingLimit': 'Approaching limit',
  'token.planClickForDetails': 'plan - click for details',
  'token.sessionTokens': 'In: {in} | Out: {out} | Turns: {turns}',
  'token.limit': 'Limit Reached',

  // Catalog
  'catalog.noCapabilityBinding': 'No capability binding',
  'catalog.downloadFailed': 'Download failed',
  'catalog.active': 'Active',
  'catalog.installed': 'Installed',
  'catalog.notDownloaded': 'Not downloaded',
  'catalog.inUse': 'In Use',
  'catalog.use': 'Use',
  'catalog.deleteModel': 'Delete model',
  'catalog.download': 'Download',

  // Navigator
  'navigator.recent': 'Recent',
  'navigator.today': 'Today',
  'navigator.thisWeek': 'This Week',
  'navigator.sources': 'Sources',
  'navigator.email': 'Email',
  'navigator.slack': 'Slack',
  'navigator.chat': 'Chat',
  'navigator.documents': 'Documents',
  'navigator.people': 'People',
  'navigator.topics': 'Topics',

  // Dreams
  'dreams.description':
    'Dreams are AI-generated reflections that synthesize patterns from your memories.',
  'dreams.comingSoon': 'Coming soon',

  // Assignment
  'assignment.memoryLlm': 'Memory LLM',
  'assignment.memoryLlmAria': 'Memory LLM selection',
  'assignment.embedder': 'Embedder',
  'assignment.loaded': 'Loaded',
  'assignment.notDownloaded': 'Not downloaded',
  'assignment.usedForExtractSummarise': 'Used for extraction and summarization',

  // Insights
  'insights.knownFacts': 'Known Facts',
  'insights.preferences': 'Preferences',
  'insights.relationships': 'Relationships',
  'insights.skills': 'Skills',
  'insights.opinions': 'Opinions',
  'insights.other': 'Other',
  'insights.title': 'Insights',
  'insights.empty': 'No insights yet. Insights are generated as your memory grows.',
  'insights.description': 'Based on {count} relations in your memory graph.',
  'insights.items': 'items',
  'insights.more': 'more',

  // Calls
  'calls.joiningCall': 'Joining call',
  'calls.meetWindowOpening': 'The Meet window is opening...',
  'calls.failedToStart': 'Failed to start Meet call',
  'calls.couldNotStart': 'Could not start call',
  'calls.failedToClose': 'Failed to close call',
  'calls.couldNotClose': 'Could not close call',
  'calls.joinMeet': 'Join a Google Meet call',
  'calls.joinMeetDescription': 'Enter a Google Meet link to join.',
  'calls.meetLink': 'Meet Link',
  'calls.displayName': 'Display Name',
  'calls.openingMeet': 'Opening Meet...',
  'calls.joinCall': 'Join Call',
  'calls.activeCalls': 'Active Calls',
  'calls.leave': 'Leave',

  // Workspace
  'workspace.wipeConfirm': 'Are you sure you want to wipe all memory? This cannot be undone.',
  'workspace.resetTreeConfirm': 'Are you sure you want to rebuild the memory tree?',
  'workspace.wipeTitle': 'Wipe Memory',
  'workspace.resetting': 'Resetting...',
  'workspace.resetMemory': 'Reset Memory',
  'workspace.resetTreeTitle': 'Rebuild Memory Tree',
  'workspace.rebuilding': 'Rebuilding...',
  'workspace.resetMemoryTree': 'Reset Memory Tree',
  'workspace.building': 'Building...',
  'workspace.buildSummaryTrees': 'Build Summary Trees',
  'workspace.viewVault': 'View Vault',
  'workspace.graphLoadFailed': 'Failed to load memory graph',
  'workspace.loadingGraph': 'Loading memory graph...',
  'workspace.graphViewMode': 'Memory graph view mode',
  'workspace.trees': 'Trees',
  'workspace.contacts': 'Contacts',

  // Graph
  'graph.noContactMentions': 'No contact mentions',
  'graph.noMemory': 'No memory',
  'graph.source': 'Source',
  'graph.topic': 'Topic',
  'graph.global': 'Global',
  'graph.document': 'Document',
  'graph.contact': 'Contact',
  'graph.nodes': 'nodes',
  'graph.parentChild': 'parent-child',
  'graph.documentContact': 'document-contact',
  'graph.link': 'link',
  'graph.links': 'links',
  'graph.children': 'children',
  'graph.clickToOpenObsidian': 'Click to open in Obsidian',
  'graph.person': 'Person',

  // Modal
  'modal.dontShowAgain': "Don't show similar suggestions",

  // Reflections
  'reflections.loading': 'Loading reflections...',
  'reflections.empty': 'No reflections yet',
  'reflections.title': 'Reflections',
  'reflections.proposedAction': 'Proposed Action',
  'reflections.act': 'Act',
  'reflections.dismiss': 'Dismiss',

  // WhatsApp
  'whatsapp.chatsSynced': 'chats synced',
  'whatsapp.chatSynced': 'chat synced',

  // Sync
  'sync.active': 'Active',
  'sync.recent': 'Recent',
  'sync.idle': 'Idle',
  'sync.memorySources': 'Memory Sources',
  'sync.noConnectedSources': 'No connected sources',
  'sync.chunks': 'chunks',
  'sync.lastChunk': 'Last chunk:',
  'sync.pending': 'pending',
  'sync.processed': 'processed',
  'sync.syncing': 'Syncing…',
  'sync.sync': 'Sync',
  'sync.failedToLoad': 'Failed to load sync status',
  'sync.noContent': 'No content has been synced into memory yet. Connect an integration to start.',

  // Backend
  'backend.aiBackend': 'AI Backend',
  'backend.cloud': 'Cloud',
  'backend.recommended': 'Recommended',
  'backend.cloudDescription':
    'Fast, powerful models hosted on our servers. Ready to use immediately.',
  'backend.privacyNote': 'No personal data, messages, or keys are ever sent to our servers.',
  'backend.local': 'Local',
  'backend.advanced': 'Advanced',
  'backend.localDescription':
    'Run models on your own machine using Ollama. Full privacy, requires setup.',
  'backend.ramRecommended': '16GB+ RAM recommended',

  // Subconscious
  'subconscious.tasks': 'tasks',
  'subconscious.ticks': 'ticks',
  'subconscious.last': 'Last',
  'subconscious.failed': 'failed',
  'subconscious.tickInterval': 'Tick Interval',
  'subconscious.runNow': 'Run Now',
  'subconscious.approvalNeeded': 'Approval Needed',
  'subconscious.requiresApproval': 'Requires approval',
  'subconscious.fixInConnections': 'Fix in Connections',
  'subconscious.goAhead': 'Go Ahead',
  'subconscious.activeTasks': 'Active Tasks',
  'subconscious.noActiveTasks': 'No active tasks',
  'subconscious.default': 'Default',
  'subconscious.addTaskPlaceholder': 'Add a new task...',
  'subconscious.activityLog': 'Activity Log',
  'subconscious.noActivity': 'No activity yet',
  'subconscious.decision.nothingNew': 'Nothing new',
  'subconscious.decision.completed': 'Completed',
  'subconscious.decision.evaluating': 'Evaluating',
  'subconscious.decision.waitingApproval': 'Waiting for approval',
  'subconscious.decision.failed': 'Failed',
  'subconscious.decision.cancelled': 'Cancelled',
  'subconscious.decision.skipped': 'Skipped',

  // Actionable
  'actionable.complete': 'Complete',
  'actionable.dismiss': 'Dismiss',
  'actionable.snooze': 'Snooze',
  'actionable.new': 'New',

  // Stats
  'stats.storage': 'Storage',
  'stats.files': 'files',
  'stats.documents': 'Documents',
  'stats.today': 'today',
  'stats.namespaces': 'Namespaces',
  'stats.relations': 'Relations',
  'stats.firstMemory': 'First Memory',
  'stats.latest': 'Latest',
  'stats.sessions': 'Sessions',
  'stats.tokens': 'tokens',

  // Boot Check Gate
  'bootCheck.invalidUrl': 'Please enter a runtime URL.',
  'bootCheck.urlMustStartWith': 'The URL needs to start with http:// or https://',
  'bootCheck.validUrlRequired':
    "That doesn't look like a valid URL (try https://core.example.com/rpc)",
  'bootCheck.tokenRequired': "We'll need an auth token to connect.",
  'bootCheck.chooseCoreMode': 'Select a Runtime',
  'bootCheck.connectToCore': 'Connect to Your Runtime',
  'bootCheck.desktopDescription': 'OpenHuman needs a runtime to think. Pick where it should live.',
  'bootCheck.webDescription':
    'On the web, OpenHuman connects to a runtime you control. Drop in its URL and auth token below, or grab the desktop app to run one right on your machine.',
  'bootCheck.preferDesktop': 'Rather keep everything on your own device?',
  'bootCheck.downloadDesktop': 'Get the Desktop App',
  'bootCheck.localRecommended': 'Run Locally (Recommended)',
  'bootCheck.localDescription':
    'Runs right here on your computer. Fastest, fully private, nothing to set up.',
  'bootCheck.cloudMode': 'Run on the Cloud (Complex)',
  'bootCheck.cloudDescription':
    "Connect to a runtime you're hosting elsewhere. Stays online 24×7 so you don't need to keep this device running.",
  'bootCheck.coreRpcUrl': 'Runtime URL',
  'bootCheck.rpcUrlPlaceholder': 'https://core.example.com/rpc',
  'bootCheck.authToken': 'Auth Token',
  'bootCheck.bearerTokenPlaceholder': 'The bearer token from your remote runtime',
  'bootCheck.storedLocally': 'Kept on this device only. Sent as ',
  'bootCheck.testing': 'Testing…',
  'bootCheck.testConnection': 'Test Connection',
  'bootCheck.connectedOk': "Connected. You're good to go.",
  'bootCheck.authFailed': "That token didn't work. Double-check it and try again.",
  'bootCheck.unreachablePrefix': "Couldn't reach it:",
  'bootCheck.checkingCore': 'Waking up your runtime…',
  'bootCheck.cannotReach': "Can't Reach the Runtime",
  'bootCheck.cannotReachDesc': "We couldn't connect to your runtime. Want to try a different one?",
  'bootCheck.switchMode': 'Pick a Different Runtime',
  'bootCheck.quit': 'Quit',
  'bootCheck.legacyDetected': 'Legacy Background Runtime Detected',
  'bootCheck.legacyDescription':
    'A separately-installed OpenHuman daemon is already running on this device. We need to clear it out before the built-in runtime can take over.',
  'bootCheck.removing': 'Removing…',
  'bootCheck.removeContinue': 'Remove and Continue',
  'bootCheck.localNeedsRestart': 'Local Runtime Needs a Restart',
  'bootCheck.localNeedsRestartDesc':
    'Your local runtime is on a different version than this app. A quick restart will get them back in sync.',
  'bootCheck.restarting': 'Restarting…',
  'bootCheck.restartCore': 'Restart Runtime',
  'bootCheck.cloudNeedsUpdate': 'Cloud Runtime Needs an Update',
  'bootCheck.cloudNeedsUpdateDesc':
    'Your cloud runtime is on a different version than this app. Run the updater to bring them back in sync.',
  'bootCheck.updating': 'Updating…',
  'bootCheck.updateCloudCore': 'Update Cloud Runtime',
  'bootCheck.versionCheckFailed': 'Runtime Version Check Failed',
  'bootCheck.versionCheckFailedDesc':
    "Your runtime is up but isn't reporting its version. It may be outdated. Restart or update it to continue.",
  'bootCheck.working': 'Working…',
  'bootCheck.restartUpdateCore': 'Restart / Update Runtime',
  'bootCheck.unexpectedError': 'Unexpected Boot-Check Error',
  'bootCheck.actionFailed': 'Something went wrong. Please try again.',

  // Notifications: category labels & timestamps
  'notifications.justNow': 'just now',
  'notifications.minAgo': '{n}m ago',
  'notifications.hrAgo': '{n}h ago',
  'notifications.dayAgo': '{n}d ago',
  'notifications.category.messages': 'Messages',
  'notifications.category.agents': 'Agents',
  'notifications.category.skills': 'Skills',
  'notifications.category.system': 'System',
  'notifications.category.meetings': 'Meetings',
  'notifications.category.reminders': 'Reminders',
  'notifications.category.important': 'Important',

  // About / Updates: status summary phrases
  'about.update.status.checking': 'Checking...',
  'about.update.status.available': 'v{version} available',
  'about.update.status.availableNoVersion': 'Update available',
  'about.update.status.downloading': 'Downloading...',
  'about.update.status.readyToInstall': 'v{version} ready to install',
  'about.update.status.readyToInstallNoVersion':
    'A new version is downloaded and ready. Restart to apply.',
  'about.update.status.installing': 'Installing...',
  'about.update.status.restarting': 'Restarting...',
  'about.update.status.upToDate': 'You are running the latest version.',
  'about.update.status.error': 'Update check failed',
  'about.update.status.default': 'Check for updates',

  // Welcome: connection error messages
  'welcome.connectionFailed': 'Connection failed: {status} {statusText}',
  'welcome.connectionFailedMsg': 'Connection failed: {message}',

  // Chat: Agent chat panel description
  'chat.agentChatDesc': 'Open a direct chat session with the agent.',

  // Channels: active route interpolated value
  'channels.activeRouteValue': '{channel} via {authMode}',

  // Privacy: data kind labels for What Leaves My Computer
  'privacy.dataKind.messages': 'Messages',
  'privacy.dataKind.agents': 'Agents',
  'privacy.dataKind.skills': 'Skills',
  'privacy.dataKind.system': 'System',
  'privacy.dataKind.meetings': 'Meetings',
  'privacy.dataKind.reminders': 'Reminders',
  'privacy.dataKind.important': 'Important',

  // Onboarding: supplementary keys
  'onboarding.enableLocalAI': 'Enable Local AI',
  'onboarding.skills.status.available': 'Available',
  'onboarding.skills.status.connected': 'Connected',
  'onboarding.skills.status.connecting': 'Connecting',
  'onboarding.skills.status.error': 'Error',
  'onboarding.skills.status.unavailable': 'Unavailable',

  // Composio: miscellaneous
  'composio.statusUnavailable': 'Status unavailable',
  'composio.envVarOverrides': 'is set, it overrides this setting.',

  // Memory: day-of-week labels for heatmap
  'memory.day.sun': 'Sun',
  'memory.day.mon': 'Mon',
  'memory.day.tue': 'Tue',
  'memory.day.wed': 'Wed',
  'memory.day.thu': 'Thu',
  'memory.day.fri': 'Fri',
  'memory.day.sat': 'Sat',

  // Memory: ingestion status labels
  'memory.ingesting': 'Ingesting',
  'memory.ingestionQueued': 'Queued',
  'memory.ingestingTitle': 'Ingesting {title}',

  // Mic: error messages
  'mic.noAudioCaptured': 'No audio captured',
  'mic.noSpeechDetected': 'No speech detected',
  'mic.failedToStopRecording': 'Failed to stop recording: {message}',
  'mic.transcriptionFailed': 'Transcription failed: {message}',

  // Reflections: kind labels
  'reflections.kind.retrospective': 'Retrospective',
  'reflections.kind.derivedFact': 'Derived Fact',
  'reflections.kind.moodInsight': 'Mood Insight',
  'reflections.kind.relationshipInsight': 'Relationship Insight',

  // Graph: tooltip keys
  'graph.tooltip.summary': 'Summary',
  'graph.tooltip.contact': 'Contact',

  // Local Model: usage labels
  'localModel.usage.never': 'Never',
  'localModel.usage.mediumLoad': 'Medium load',
  'localModel.usage.lowLoad': 'Low load',
  'localModel.usage.idleMode': 'Idle mode',
  'localModel.rebootstrapComplete': 'Model re-bootstrap complete.',
  'localModel.modelsVerified': 'Local models verified.',
};

export default en;
