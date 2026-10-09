/**
 * How each core tool is presented: icon, phrase, category, target chip and
 * the rich body its detail panel renders.
 *
 * Resolution order lives in `toolPresentation.ts`; this file is only data.
 * Three layers:
 *
 * - {@link EXACT_TOOL_SPECS}: one entry per registered tool name.
 * - {@link ACTION_TOOL_SPECS}: collapsed tools that switch on an argument
 *   (`memory { action: "recall" }`, `browser { action: "click" }`).
 * - {@link FAMILY_TOOL_SPECS}: prefix rules for tool families whose members
 *   share a meaning (`hosting_*`, `wallet_*`), so a new member of a known
 *   family is labelled without an edit here.
 *
 * `toolPresentation.catalog.test.ts` walks every name the core registers
 * (`__fixtures__/coreToolNames.json`) and fails if any falls through to the
 * generic fallback, so a new core tool cannot ship unlabelled.
 */
import {
  AppWindowIcon,
  ArchiveRestoreIcon,
  ArrowLeftRightIcon,
  BellIcon,
  BlocksIcon,
  BookOpenIcon,
  BotIcon,
  BrainCircuitIcon,
  BrainIcon,
  CalendarClockIcon,
  CameraIcon,
  ChartBarIcon,
  ClapperboardIcon,
  ClipboardCheckIcon,
  ClockIcon,
  CodeIcon,
  CoinsIcon,
  DatabaseIcon,
  DownloadIcon,
  EraserIcon,
  FilePenIcon,
  FilePlusIcon,
  FileSpreadsheetIcon,
  FileTextIcon,
  FlagIcon,
  FolderOpenIcon,
  FolderSearchIcon,
  GitBranchIcon,
  GitCompareIcon,
  GlobeIcon,
  GraduationCapIcon,
  HardDriveIcon,
  HeartIcon,
  HourglassIcon,
  ImageIcon,
  ImagePlusIcon,
  KeyboardIcon,
  LayersIcon,
  Link2Icon,
  LinkIcon,
  ListChecksIcon,
  ListTodoIcon,
  type LucideIcon,
  MailXIcon,
  MapPinIcon,
  MessageCircleQuestionIcon,
  MessageSquareReplyIcon,
  MousePointerClickIcon,
  NetworkIcon,
  NewspaperIcon,
  PackageIcon,
  PackagePlusIcon,
  PackageSearchIcon,
  PhoneIcon,
  PlugIcon,
  PodcastIcon,
  PowerIcon,
  PresentationIcon,
  ReceiptIcon,
  RocketIcon,
  SaveIcon,
  ScanEyeIcon,
  ScanSearchIcon,
  ScrollTextIcon,
  ServerIcon,
  SettingsIcon,
  ShieldIcon,
  SparklesIcon,
  SquareTerminalIcon,
  StethoscopeIcon,
  TargetIcon,
  TelescopeIcon,
  TextSearchIcon,
  TrendingUpIcon,
  UserRoundIcon,
  UsersIcon,
  VideoIcon,
  WalletIcon,
  WorkflowIcon,
  WrenchIcon,
} from 'lucide-react';

import { chip, type ChipRule } from './toolChips';
import type { ToolPhraseId } from './toolPhrases';

/** Broad activity category, used for grouping summaries and the timeline. */
export type ToolCategory =
  | 'file'
  | 'code'
  | 'shell'
  | 'web'
  | 'browser'
  | 'media'
  | 'memory'
  | 'agent'
  | 'plan'
  | 'schedule'
  | 'app'
  | 'mcp'
  | 'storage'
  | 'wallet'
  | 'skill'
  | 'system'
  | 'other';

/** Which rich body the expanded row renders. `generic` is the Input/Output view. */
export type ToolBodyKind = 'webSearch' | 'webFetch' | 'shell' | 'file' | 'mcp' | 'generic';

export interface ToolSpec {
  phrase: ToolPhraseId;
  icon: LucideIcon;
  category: ToolCategory;
  chip?: ChipRule;
  body?: ToolBodyKind;
}

const spec = (
  phrase: ToolPhraseId,
  icon: LucideIcon,
  category: ToolCategory,
  extra: Partial<Pick<ToolSpec, 'chip' | 'body'>> = {}
): ToolSpec => ({ phrase, icon, category, ...extra });

const webSearch = (phrase: ToolPhraseId, icon: LucideIcon = GlobeIcon) =>
  spec(phrase, icon, 'web', { chip: chip.query(), body: 'webSearch' });
const readPages = spec('readPages', LinkIcon, 'web', { chip: chip.url(), body: 'webFetch' });

export const FALLBACK_ICON = WrenchIcon;
export const INTEGRATION_ICON = PlugIcon;

export const EXACT_TOOL_SPECS: Record<string, ToolSpec> = {
  // ── Files and code ──────────────────────────────────────────────────────
  file_read: spec('readFile', FileTextIcon, 'file', { chip: chip.path(), body: 'file' }),
  file_write: spec('writeFile', FilePlusIcon, 'file', { chip: chip.path(), body: 'file' }),
  edit: spec('editFile', FilePenIcon, 'file', { chip: chip.path(), body: 'file' }),
  apply_patch: spec('applyEdits', FilePenIcon, 'file', { chip: chip.editsPath(), body: 'file' }),
  vault_write_markdown: spec('writeFile', FilePlusIcon, 'file', { chip: chip.path() }),
  // Older and foreign spellings of the file tools, seen in persisted
  // transcripts and other harnesses' tool names.
  read_file: spec('readFile', FileTextIcon, 'file', { chip: chip.path(), body: 'file' }),
  write_file: spec('writeFile', FilePlusIcon, 'file', { chip: chip.path(), body: 'file' }),
  grep: spec('searchCode', TextSearchIcon, 'code', { chip: chip.text('pattern') }),
  glob: spec('findFiles', FolderSearchIcon, 'file', { chip: chip.text('pattern') }),
  list: spec('listFolder', FolderOpenIcon, 'file', { chip: chip.path() }),
  csv_export: spec('exportCsv', FileSpreadsheetIcon, 'file', { chip: chip.text('filename') }),
  update_memory_md: spec('updateMemoryNotes', ScrollTextIcon, 'memory', {
    chip: chip.text('file'),
  }),
  git_operations: spec('runGit', GitBranchIcon, 'code', {
    chip: chip.text('operation', 'command'),
  }),
  read_diff: spec('readChanges', GitCompareIcon, 'code', { chip: chip.path() }),
  run_linter: spec('runLinter', ListChecksIcon, 'code'),
  run_tests: spec('runTests', ListChecksIcon, 'code'),
  lsp: spec('analyzeCode', CodeIcon, 'code', { chip: chip.path() }),

  // ── Shell and system ────────────────────────────────────────────────────
  shell: spec('runCommand', SquareTerminalIcon, 'shell', { chip: chip.command(), body: 'shell' }),
  node_exec: spec('runCode', SquareTerminalIcon, 'shell', {
    chip: chip.command('script_path', 'inline_code'),
    body: 'shell',
  }),
  python_exec: spec('runCode', SquareTerminalIcon, 'shell', {
    chip: chip.command('script_path', 'inline_code'),
    body: 'shell',
  }),
  npm_exec: spec('runPackageManager', SquareTerminalIcon, 'shell', {
    chip: chip.command('subcommand'),
    body: 'shell',
  }),
  detect_tools: spec('checkInstalledTools', ScanSearchIcon, 'system'),
  install_tool: spec('installTool', PackagePlusIcon, 'system', {
    chip: chip.text('package', 'tool_name'),
  }),
  current_time: spec('checkTime', ClockIcon, 'system'),
  resolve_time: spec('resolveDate', ClockIcon, 'system', { chip: chip.text('expr') }),
  retrieve_tool_output: spec('retrieveOutput', ArchiveRestoreIcon, 'system'),
  juice_retrieve: spec('retrieveOutput', ArchiveRestoreIcon, 'system'),
  // REPL tools over a stored (handle) tool result; they reuse existing phrases.
  juice_find: spec('searchCode', TextSearchIcon, 'system', { chip: chip.text('query') }),
  juice_extract: spec('extractDetails', LayersIcon, 'system'),
  juice_summarize: spec('extractDetails', LayersIcon, 'system'),
  read_workspace_state: spec('reviewWorkspace', FolderOpenIcon, 'system'),
  proxy_config: spec('configureProxy', SettingsIcon, 'system', { chip: chip.text('action') }),
  update_check: spec('checkUpdates', DownloadIcon, 'system'),
  update_apply: spec('installUpdate', DownloadIcon, 'system'),
  pushover: spec('sendNotification', BellIcon, 'system', { chip: chip.text('title', 'message') }),
  tool_stats: spec('reviewToolUsage', ChartBarIcon, 'system'),
  keyboard: spec('typeKeys', KeyboardIcon, 'browser', { chip: chip.text('text', 'key') }),
  mouse: spec('click', MousePointerClickIcon, 'browser'),

  // ── Web ─────────────────────────────────────────────────────────────────
  web_search: webSearch('searchWeb'),
  web_search_tool: webSearch('searchWeb'),
  // Role tools: `web_answer_tool` returns a grounded answer with citations
  // (and runs deep research on `depth: "deep"`); `web_contents_tool` reads
  // the given URLs.
  web_answer_tool: webSearch('askTheWeb'),
  web_contents_tool: readPages,
  gemini_agentic_search: webSearch('askTheWeb'),
  gemini_deep_research: webSearch('research', TelescopeIcon),
  exa_answer: webSearch('askTheWeb'),
  exa_search: webSearch('searchWeb'),
  tavily_search: webSearch('searchWeb'),
  querit_search: webSearch('searchWeb'),
  parallel_search: webSearch('searchWeb'),
  tinyfish_search: webSearch('searchWeb'),
  searxng_search: webSearch('searchWeb'),
  seltz_search: webSearch('searchWeb'),
  brave_news_search: webSearch('searchNews', NewspaperIcon),
  brave_image_search: webSearch('searchImages', ImageIcon),
  brave_video_search: webSearch('searchVideos', VideoIcon),
  exa_find_similar: spec('findSimilarPages', GlobeIcon, 'web', {
    chip: chip.url(),
    body: 'webSearch',
  }),
  exa_get_contents: readPages,
  tavily_extract: readPages,
  parallel_extract: readPages,
  tinyfish_fetch: readPages,
  // Parallel was removed from the core; these stay so old transcripts still
  // render with a meaningful label.
  parallel_research: spec('research', TelescopeIcon, 'web', { chip: chip.query() }),
  parallel_chat: spec('askTheWeb', GlobeIcon, 'web', { chip: chip.query() }),
  parallel_enrich: spec('enrichData', SparklesIcon, 'web', { chip: chip.query() }),
  parallel_dataset: spec('buildDataset', DatabaseIcon, 'web', { chip: chip.query() }),
  tinyfish_agent_run: spec('browseForYou', MousePointerClickIcon, 'web', {
    chip: chip.text('goal', 'url'),
  }),
  web_fetch: spec('readWebpage', LinkIcon, 'web', { chip: chip.url(), body: 'webFetch' }),
  http_request: spec('callApi', ArrowLeftRightIcon, 'web', { chip: chip.url(), body: 'webFetch' }),
  curl: spec('downloadFile', DownloadIcon, 'web', { chip: chip.url() }),
  x402_request: spec('makePaidRequest', CoinsIcon, 'web', { chip: chip.url() }),
  gitbooks_search: spec('searchDocs', BookOpenIcon, 'web', { chip: chip.query() }),
  gitbooks_get_page: spec('readDocs', BookOpenIcon, 'web', { chip: chip.url() }),

  // ── Browser ─────────────────────────────────────────────────────────────
  browser: spec('useBrowser', AppWindowIcon, 'browser', { chip: chip.url() }),
  browser_open: spec('openPage', AppWindowIcon, 'browser', { chip: chip.url() }),

  // ── Native desktop ───────────────────────────────────────────────────────
  desktop_list_apps: spec('inspectDesktop', AppWindowIcon, 'app'),
  desktop_list_windows: spec('inspectDesktop', AppWindowIcon, 'app', { chip: chip.text('app') }),
  desktop_launch: spec('controlDesktop', AppWindowIcon, 'app', { chip: chip.text('app') }),
  desktop_snapshot: spec('inspectDesktop', ScanEyeIcon, 'app', { chip: chip.text('app') }),
  desktop_find: spec('inspectDesktop', ScanSearchIcon, 'app', { chip: chip.text('app') }),
  desktop_goal: spec('controlDesktop', MousePointerClickIcon, 'app', { chip: chip.text('app') }),
  desktop_continue_goal: spec('controlDesktop', MousePointerClickIcon, 'app'),

  // ── Media and documents ────────────────────────────────────────────────
  image_info: spec('analyzeImage', ScanEyeIcon, 'media', { chip: chip.path() }),
  media_generate_image: spec('generateImage', ImagePlusIcon, 'media', {
    chip: chip.text('prompt'),
  }),
  media_generate_video: spec('generateVideo', ClapperboardIcon, 'media', {
    chip: chip.text('prompt'),
  }),
  media_list_models: spec('checkMediaModels', ImageIcon, 'media'),
  generate_document: spec('createDocument', FileTextIcon, 'media', { chip: chip.text('title') }),
  show_ui: spec('showView', AppWindowIcon, 'skill'),
  generate_presentation: spec('createPresentation', PresentationIcon, 'media', {
    chip: chip.text('title'),
  }),
  audio_generate_podcast: spec('generatePodcast', PodcastIcon, 'media', {
    chip: chip.text('title', 'topic'),
  }),
  audio_email_podcast: spec('emailPodcast', PodcastIcon, 'media', { chip: chip.text('to') }),
  audio_generate_and_email_podcast: spec('createAndEmailPodcast', PodcastIcon, 'media', {
    chip: chip.text('title', 'topic'),
  }),

  // ── Memory ──────────────────────────────────────────────────────────────
  // The single memory tool: recall asks a `question`, fetch a `query`, learn stores `text`.
  memory: spec('searchMemory', BrainIcon, 'memory', {
    chip: chip.text('question', 'query', 'text'),
  }),
  memory_store: spec('saveToMemory', SaveIcon, 'memory', { chip: chip.text('key', 'content') }),
  memory_recall: spec('recallMemories', BrainCircuitIcon, 'memory', { chip: chip.query() }),
  memory_forget: spec('forgetMemory', EraserIcon, 'memory', { chip: chip.text('key') }),
  memory_hybrid_search: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),
  memory_vector_search: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),
  memory_chunk_context: spec('inspectMemory', BrainIcon, 'memory'),
  memory_store_raw_search: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),
  memory_store_raw_chunks: spec('inspectMemory', BrainIcon, 'memory'),
  memory_store_kinds: spec('inspectMemory', BrainIcon, 'memory'),
  memory_doctor: spec('inspectMemory', StethoscopeIcon, 'memory'),
  memory_flavour: spec('inspectMemory', BrainIcon, 'memory'),
  memory_tree: spec('exploreMemory', NetworkIcon, 'memory', { chip: chip.query() }),
  goals: spec('reviewGoals', TargetIcon, 'memory'),
  remember_preference: spec('savePreference', HeartIcon, 'memory', {
    chip: chip.text('preference', 'key'),
  }),
  save_preference: spec('savePreference', HeartIcon, 'memory', {
    chip: chip.text('preference', 'key'),
  }),
  flow_memory_recall: spec('recallMemories', BrainCircuitIcon, 'memory', { chip: chip.query() }),
  flow_memory_remember: spec('saveToMemory', SaveIcon, 'memory', { chip: chip.text('key') }),
  memory_tools_list: spec('inspectMemory', BrainIcon, 'memory'),
  memory_tools_put: spec('saveToMemory', SaveIcon, 'memory'),
  call_memory_agent: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),

  // ── Agents and delegation ──────────────────────────────────────────────
  spawn_subagent: spec('delegateTask', BotIcon, 'agent', { chip: chip.text('agent_id') }),
  spawn_async_subagent: spec('delegateTask', BotIcon, 'agent', { chip: chip.text('agent_id') }),
  spawn_worker_thread: spec('delegateTask', BotIcon, 'agent', { chip: chip.text('agent_id') }),
  delegate_graph: spec('delegateTask', BotIcon, 'agent', { chip: chip.text('agent_id') }),
  delegate: spec('delegateTask', BotIcon, 'agent', { chip: chip.text('agent') }),
  delegate_to: spec('delegateTask', BotIcon, 'agent', { chip: chip.text('agent', 'agent_id') }),
  spawn_parallel_agents: spec('runAgentsInParallel', UsersIcon, 'agent'),
  continue_subagent: spec('messageAgent', MessageSquareReplyIcon, 'agent', {
    chip: chip.text('agent_id'),
  }),
  steer_subagent: spec('messageAgent', MessageSquareReplyIcon, 'agent', {
    chip: chip.text('agent_id'),
  }),
  wait_subagent: spec('waitForAgent', HourglassIcon, 'agent'),
  close_subagent: spec('closeAgent', BotIcon, 'agent'),
  list_subagents: spec('checkAgents', BotIcon, 'agent'),
  wait: spec('wait', HourglassIcon, 'agent'),
  wait_loop: spec('wait', HourglassIcon, 'agent'),
  ask_user_clarification: spec('askQuestion', MessageCircleQuestionIcon, 'agent', {
    chip: chip.text('question'),
  }),
  extract_from_result: spec('extractDetails', LayersIcon, 'agent'),

  // ── Planning ────────────────────────────────────────────────────────────
  todo: spec('updateTodos', ListTodoIcon, 'plan'),
  request_plan_review: spec('requestPlanReview', ClipboardCheckIcon, 'plan'),
  plan_exit: spec('finishPlan', ClipboardCheckIcon, 'plan'),
  goal_set: spec('setGoal', FlagIcon, 'plan', { chip: chip.text('objective') }),
  goal_get: spec('checkGoal', FlagIcon, 'plan'),
  goal_complete: spec('completeGoal', FlagIcon, 'plan'),

  // ── Scheduling ──────────────────────────────────────────────────────────
  cron: spec('checkSchedules', CalendarClockIcon, 'schedule'),
  cron_add: spec('scheduleTask', CalendarClockIcon, 'schedule', { chip: chip.text('name') }),
  cron_list: spec('checkSchedules', CalendarClockIcon, 'schedule'),
  cron_update: spec('updateSchedule', CalendarClockIcon, 'schedule', { chip: chip.text('name') }),
  cron_remove: spec('removeSchedule', CalendarClockIcon, 'schedule'),
  cron_run: spec('runScheduledTask', CalendarClockIcon, 'schedule'),
  cron_runs: spec('checkRunHistory', CalendarClockIcon, 'schedule'),
  schedule: spec('scheduleTask', CalendarClockIcon, 'schedule'),

  // ── Connected apps ─────────────────────────────────────────────────────
  composio_list_toolkits: spec('checkAvailableApps', PlugIcon, 'app'),
  composio_list_connections: spec('checkConnections', PlugIcon, 'app'),
  composio_connect: spec('connectApp', Link2Icon, 'app', { chip: chip.text('toolkit') }),
  composio_authorize: spec('authorizeApp', Link2Icon, 'app', { chip: chip.text('toolkit') }),
  composio_list_tools: spec('findAppActions', PlugIcon, 'app', {
    chip: chip.text('toolkits', 'toolkit'),
  }),
  composio_execute: spec('runAppAction', PlugIcon, 'app', { chip: chip.text('tool') }),
  tool_search: spec('findTools', PackageSearchIcon, 'system', { chip: chip.query() }),
  // The deferred-tool bridge is described as the tool it calls
  // (`toolPresentation.ts`); this entry covers it before its args arrive.
  tool_call: spec('useTools', WrenchIcon, 'system', { chip: chip.text('name') }),
  search_tool_catalog: spec('findTools', PackageSearchIcon, 'system', { chip: chip.query() }),
  gmail_unsubscribe: spec('unsubscribe', MailXIcon, 'app', { chip: chip.text('sender', 'email') }),
  google_places_search: spec('searchPlaces', MapPinIcon, 'app', { chip: chip.query() }),
  google_places_details: spec('lookUpPlace', MapPinIcon, 'app', { chip: chip.text('place_id') }),
  twilio_call: spec('placeCall', PhoneIcon, 'app', { chip: chip.text('to') }),

  // ── MCP ─────────────────────────────────────────────────────────────────
  mcp_list_servers: spec('checkMcpServers', ServerIcon, 'mcp'),
  mcp_list_tools: spec('checkMcpTools', BlocksIcon, 'mcp', { chip: chip.text('server') }),
  mcp_call_tool: spec('callMcpTool', BlocksIcon, 'mcp', { chip: chip.text('server'), body: 'mcp' }),
  mcp_registry_tool_call: spec('callMcpTool', BlocksIcon, 'mcp', {
    chip: chip.text('server_id'),
    body: 'mcp',
  }),
  mcp_registry_search: spec('searchMcpServers', ServerIcon, 'mcp', { chip: chip.query() }),
  mcp_registry_get: spec('checkMcpServers', ServerIcon, 'mcp', {
    chip: chip.text('qualified_name'),
  }),
  mcp_registry_installed_list: spec('checkMcpServers', ServerIcon, 'mcp'),
  mcp_registry_status: spec('checkMcpServers', ServerIcon, 'mcp'),
  mcp_registry_list_tools: spec('checkMcpTools', BlocksIcon, 'mcp', {
    chip: chip.text('server_id'),
  }),
  mcp_registry_connect: spec('connectMcpServer', ServerIcon, 'mcp', {
    chip: chip.text('qualified_name', 'server_id'),
  }),
  mcp_registry_disconnect: spec('disconnectMcpServer', ServerIcon, 'mcp', {
    chip: chip.text('server_id'),
  }),
  mcp_registry_uninstall: spec('removeMcpServer', ServerIcon, 'mcp', {
    chip: chip.text('server_id'),
  }),

  // ── Storage and hosting ────────────────────────────────────────────────
  storage_upload_file: spec('uploadFile', HardDriveIcon, 'storage', { chip: chip.path() }),
  storage_download_file: spec('downloadFile', HardDriveIcon, 'storage', {
    chip: chip.text('key', 'name'),
  }),
  storage_list_files: spec('listStoredFiles', HardDriveIcon, 'storage'),
  storage_get_link: spec('createShareLink', LinkIcon, 'storage', {
    chip: chip.text('key', 'name'),
  }),
  storage_delete_file: spec('deleteFile', HardDriveIcon, 'storage', {
    chip: chip.text('key', 'name'),
  }),
  storage_set_visibility: spec('updateFileAccess', HardDriveIcon, 'storage', {
    chip: chip.text('key', 'name'),
  }),
  hosting_launch_site: spec('deploySite', RocketIcon, 'storage', { chip: chip.text('name') }),
  hosting_rollback: spec('rollBackDeployment', RocketIcon, 'storage'),

  // ── Wallet ──────────────────────────────────────────────────────────────
  wallet_prepare_transfer: spec('prepareTransfer', WalletIcon, 'wallet', { chip: chip.text('to') }),
  web3_swap_quote: spec('getSwapQuote', ArrowLeftRightIcon, 'wallet'),
  web3_swap_routes: spec('getSwapQuote', ArrowLeftRightIcon, 'wallet'),
  web3_swap_execute: spec('swapTokens', ArrowLeftRightIcon, 'wallet'),
  web3_bridge_quote: spec('getBridgeQuote', ArrowLeftRightIcon, 'wallet'),
  web3_bridge_execute: spec('bridgeTokens', ArrowLeftRightIcon, 'wallet'),
  web3_dapp_call: spec('callDapp', CoinsIcon, 'wallet'),
  web3_dapp_execute: spec('callDapp', CoinsIcon, 'wallet'),

  // ── Skills and workflows ───────────────────────────────────────────────
  use_skill: spec('useSkill', SparklesIcon, 'skill', { chip: chip.text('skill') }),
  skill_search: spec('searchSkills', SparklesIcon, 'skill', { chip: chip.query() }),
  create_skill: spec('createSkill', SparklesIcon, 'skill', { chip: chip.text('name') }),
  install_workflow_from_url: spec('installSkill', SparklesIcon, 'skill', { chip: chip.url() }),
  uninstall_workflow: spec('removeSkill', SparklesIcon, 'skill', { chip: chip.text('name', 'id') }),
  run_workflow: spec('runWorkflow', WorkflowIcon, 'skill', { chip: chip.text('workflow_id') }),
  await_workflow: spec('waitForWorkflow', HourglassIcon, 'skill'),
  run_flow: spec('runWorkflow', WorkflowIcon, 'skill', { chip: chip.text('name', 'flow_id') }),
  propose_workflow: spec('designWorkflow', WorkflowIcon, 'skill', { chip: chip.text('name') }),
  revise_workflow: spec('designWorkflow', WorkflowIcon, 'skill'),
  edit_workflow: spec('designWorkflow', WorkflowIcon, 'skill'),
  create_workflow: spec('designWorkflow', WorkflowIcon, 'skill', { chip: chip.text('name') }),
  duplicate_flow: spec('saveWorkflow', WorkflowIcon, 'skill'),
  save_workflow: spec('saveWorkflow', WorkflowIcon, 'skill', { chip: chip.text('name') }),
  validate_workflow: spec('validateWorkflow', WorkflowIcon, 'skill'),
  dry_run_workflow: spec('testWorkflow', WorkflowIcon, 'skill'),
  cancel_flow_run: spec('cancelWorkflow', WorkflowIcon, 'skill'),
  resume_flow_run: spec('runWorkflow', WorkflowIcon, 'skill'),
  suggest_workflows: spec('suggestWorkflows', WorkflowIcon, 'skill'),

  // ── Settings and platform ──────────────────────────────────────────────
  security_policy_info: spec('checkSecurity', ShieldIcon, 'system'),
  credential_list: spec('checkSecurity', ShieldIcon, 'system'),
  session_state: spec('checkSecurity', ShieldIcon, 'system'),
  oauth_connect_url: spec('connectApp', Link2Icon, 'app', { chip: chip.text('provider') }),
  oauth_list: spec('checkConnections', PlugIcon, 'app'),
  dashboard_model_health: spec('runDiagnostics', StethoscopeIcon, 'system'),
  workspace_read_persona: spec('readPersona', UserRoundIcon, 'system'),
  workspace_update_persona: spec('updatePersona', UserRoundIcon, 'system'),
  workspace_reset_persona: spec('updatePersona', UserRoundIcon, 'system'),
  workspace_init: spec('setUpWorkspace', FolderOpenIcon, 'system'),
  artifact_delete: spec('deleteArtifact', PackageIcon, 'system'),
};

/**
 * Collapsed tools that do different things per argument. Keyed by tool name,
 * then by the argument named in `arg`. A value the table does not list falls
 * back to the tool's {@link EXACT_TOOL_SPECS} entry.
 */
export const ACTION_TOOL_SPECS: Record<string, { arg: string; specs: Record<string, ToolSpec> }> = {
  memory: {
    arg: 'action',
    specs: {
      // Memory v2 actions.
      recall: spec('recallMemories', BrainCircuitIcon, 'memory', {
        chip: chip.text('question', 'query'),
      }),
      fetch: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),
      learn: spec('saveToMemory', SaveIcon, 'memory', { chip: chip.text('text') }),
      forget: spec('forgetMemory', EraserIcon, 'memory', { chip: chip.text('key') }),
      // v1 actions, kept so persisted transcripts still render.
      store: spec('saveToMemory', SaveIcon, 'memory', { chip: chip.text('key', 'content') }),
      hybrid_search: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),
      vector_search: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),
      raw_search: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }),
      chunk_context: spec('inspectMemory', BrainIcon, 'memory'),
      raw_chunks: spec('inspectMemory', BrainIcon, 'memory'),
      kinds: spec('inspectMemory', BrainIcon, 'memory'),
      flavour: spec('inspectMemory', BrainIcon, 'memory'),
      doctor: spec('inspectMemory', StethoscopeIcon, 'memory'),
    },
  },
  memory_tree: {
    arg: 'mode',
    specs: {
      ingest_document: spec('saveDocumentToMemory', SaveIcon, 'memory', {
        chip: chip.text('title', 'path'),
      }),
    },
  },
  goals: {
    arg: 'op',
    specs: {
      list: spec('reviewGoals', TargetIcon, 'memory'),
      add: spec('updateGoals', TargetIcon, 'memory', { chip: chip.text('text', 'goal') }),
      edit: spec('updateGoals', TargetIcon, 'memory', { chip: chip.text('text', 'goal') }),
      delete: spec('updateGoals', TargetIcon, 'memory'),
    },
  },
  cron: {
    arg: 'action',
    specs: {
      list: spec('checkSchedules', CalendarClockIcon, 'schedule'),
      add: spec('scheduleTask', CalendarClockIcon, 'schedule', { chip: chip.text('name') }),
      update: spec('updateSchedule', CalendarClockIcon, 'schedule', { chip: chip.text('name') }),
      remove: spec('removeSchedule', CalendarClockIcon, 'schedule'),
      run: spec('runScheduledTask', CalendarClockIcon, 'schedule'),
      runs: spec('checkRunHistory', CalendarClockIcon, 'schedule'),
    },
  },
  schedule: {
    arg: 'action',
    specs: {
      list: spec('checkSchedules', CalendarClockIcon, 'schedule'),
      get: spec('checkSchedules', CalendarClockIcon, 'schedule'),
      cancel: spec('removeSchedule', CalendarClockIcon, 'schedule'),
      remove: spec('removeSchedule', CalendarClockIcon, 'schedule'),
      pause: spec('updateSchedule', CalendarClockIcon, 'schedule'),
      resume: spec('updateSchedule', CalendarClockIcon, 'schedule'),
    },
  },
  browser: {
    arg: 'action',
    specs: {
      open: spec('openPage', AppWindowIcon, 'browser', { chip: chip.url() }),
      snapshot: spec('takeScreenshot', CameraIcon, 'browser'),
      click: spec('click', MousePointerClickIcon, 'browser', { chip: chip.text('selector') }),
      mouse_click: spec('click', MousePointerClickIcon, 'browser'),
      hover: spec('click', MousePointerClickIcon, 'browser', { chip: chip.text('selector') }),
      fill: spec('typeKeys', KeyboardIcon, 'browser', { chip: chip.text('selector') }),
      type: spec('typeKeys', KeyboardIcon, 'browser', { chip: chip.text('selector') }),
      key_type: spec('typeKeys', KeyboardIcon, 'browser'),
      key_press: spec('typeKeys', KeyboardIcon, 'browser', { chip: chip.text('key') }),
      press: spec('typeKeys', KeyboardIcon, 'browser', { chip: chip.text('key') }),
      scroll: spec('scrollPage', AppWindowIcon, 'browser'),
      get_text: spec('readPage', AppWindowIcon, 'browser'),
      get_title: spec('readPage', AppWindowIcon, 'browser'),
      get_url: spec('readPage', AppWindowIcon, 'browser'),
      find: spec('readPage', AppWindowIcon, 'browser', { chip: chip.text('value', 'selector') }),
      is_visible: spec('readPage', AppWindowIcon, 'browser'),
      wait: spec('wait', HourglassIcon, 'browser'),
    },
  },
};

/**
 * Prefix families. Ordered: the first matching rule wins, so a narrower rule
 * must precede a broader one that shares its prefix.
 */
export const FAMILY_TOOL_SPECS: ReadonlyArray<{ test: RegExp; spec: ToolSpec }> = [
  { test: /^memory_/, spec: spec('searchMemory', BrainIcon, 'memory', { chip: chip.query() }) },
  {
    test: /^learning_(update|pin|unpin|forget|rebuild|reset|save|enrich)/,
    spec: spec('updateLearnings', GraduationCapIcon, 'memory'),
  },
  { test: /^learning_/, spec: spec('reviewLearnings', GraduationCapIcon, 'memory') },
  {
    test: /^skill_registry_(install)/,
    spec: spec('installSkill', SparklesIcon, 'skill', { chip: chip.text('name', 'id') }),
  },
  { test: /^skill_registry_uninstall/, spec: spec('removeSkill', SparklesIcon, 'skill') },
  {
    test: /^skill_registry_search/,
    spec: spec('searchSkills', SparklesIcon, 'skill', { chip: chip.query() }),
  },
  { test: /^(skill_|skill_runtime_)/, spec: spec('checkSkills', SparklesIcon, 'skill') },
  {
    test: /^(list_workflows|describe_workflow|read_workflow_|list_workflow_runs|list_flows|get_flow|list_flow_|get_tool_|list_agent_definitions|list_connectable_toolkits|list_node_kinds|get_node_kind_contract)/,
    spec: spec('checkWorkflows', WorkflowIcon, 'skill'),
  },
  {
    test: /^task_source_(add|update|remove)/,
    spec: spec('updateTaskSources', ListChecksIcon, 'app'),
  },
  { test: /^task_source_(fetch|list_tasks)/, spec: spec('fetchTasks', ListChecksIcon, 'app') },
  { test: /^task_source_/, spec: spec('checkTaskSources', ListChecksIcon, 'app') },
  { test: /^hosting_(set_env|add_domain)/, spec: spec('updateHosting', RocketIcon, 'storage') },
  { test: /^hosting_/, spec: spec('checkHosting', RocketIcon, 'storage') },
  { test: /^storage_/, spec: spec('listStoredFiles', HardDriveIcon, 'storage') },
  {
    test: /^stock_/,
    spec: spec('checkMarkets', TrendingUpIcon, 'app', { chip: chip.text('symbol') }),
  },
  {
    test: /^wallet_(tx_|lookup_tx)/,
    spec: spec('checkTransaction', WalletIcon, 'wallet', { chip: chip.text('tx_hash', 'hash') }),
  },
  { test: /^(wallet_|web3_)/, spec: spec('checkWallet', WalletIcon, 'wallet') },
  { test: /^composio_/, spec: spec('runAppAction', PlugIcon, 'app') },
  { test: /^mcp_/, spec: spec('checkMcpServers', ServerIcon, 'mcp') },
  { test: /^config_/, spec: spec('checkSettings', SettingsIcon, 'system') },
  { test: /^(daemon_host_prefs_|service_)/, spec: spec('manageService', PowerIcon, 'system') },
  { test: /^(doctor_|health_)/, spec: spec('runDiagnostics', StethoscopeIcon, 'system') },
  { test: /^cost_/, spec: spec('checkUsageCosts', ReceiptIcon, 'system') },
  { test: /^artifact_/, spec: spec('checkArtifacts', PackageIcon, 'system') },
  { test: /^cron_/, spec: spec('checkSchedules', CalendarClockIcon, 'schedule') },
  { test: /^goal_/, spec: spec('checkGoal', FlagIcon, 'plan') },
];

/**
 * Named agents, reached as `subagent:<id>`, as `spawn_subagent { agent_id }`,
 * as `delegate_<id>`, or as the custom delegate tool names agent TOMLs
 * declare (`delegate_name`).
 *
 * Some entries name specialists the core no longer ships (`code_executor`,
 * `tools_agent`, `do_crypto`, `manage_settings`, `schedule_task`,
 * `use_mcp_server`, `ask_docs`, `run_code`, `review_code`, `plan`): they are
 * now inline skills, but existing transcripts still replay those calls, so the
 * labels stay for history.
 */
export const AGENT_SPECS: Record<string, ToolSpec> = {
  orchestrator: spec('planNextSteps', BotIcon, 'agent'),
  plan: spec('planNextSteps', BotIcon, 'agent'),
  planner: spec('planNextSteps', BotIcon, 'agent'),
  critic: spec('reviewWork', ClipboardCheckIcon, 'agent'),
  review_code: spec('reviewWork', ClipboardCheckIcon, 'agent'),
  tools_agent: spec('useTools', WrenchIcon, 'agent'),
  code_executor: spec('runCode', SquareTerminalIcon, 'agent'),
  run_code: spec('runCode', SquareTerminalIcon, 'agent'),
  ask_docs: spec('searchDocs', BookOpenIcon, 'agent'),
  create_image: spec('generateImage', ImagePlusIcon, 'agent'),
  create_video: spec('generateVideo', ClapperboardIcon, 'agent'),
  analyze_image: spec('analyzeImage', ScanEyeIcon, 'agent'),
  make_presentation: spec('createPresentation', PresentationIcon, 'agent'),
  do_crypto: spec('checkWallet', WalletIcon, 'agent'),
  schedule_task: spec('scheduleTask', CalendarClockIcon, 'agent'),
  manage_tasks: spec('checkTaskSources', ListChecksIcon, 'agent'),
  manage_settings: spec('checkSettings', SettingsIcon, 'agent'),
  use_mcp_server: spec('checkMcpTools', BlocksIcon, 'agent'),
  curate_goals: spec('updateGoals', TargetIcon, 'agent'),
  manage_profile_memory: spec('updateLearnings', GraduationCapIcon, 'agent'),
  archive_session: spec('saveToMemory', SaveIcon, 'agent'),
  retrieve_flow_context: spec('prepareContext', LayersIcon, 'agent'),
};

/** The integrations agent: labelled by the app it works in, when known. */
export const INTEGRATIONS_AGENT_ID = 'integrations_agent';
