/**
 * The vocabulary a tool call is described with.
 *
 * Every phrase has two tenses: `active` while the call runs ("Reading file")
 * and `done` once it settled ("Read file"). A finished row used to keep the
 * progressive form next to a check mark, so it read as still running.
 *
 * Phrases are shared between tools on purpose: the managed search and every
 * bring-your-own-key engine all read "Searching the web", which keeps the
 * translation surface to one entry per *meaning* instead of one per tool.
 *
 * The English here is the source; each phrase is served through the i18n
 * keys `conversations.tools.<id>.active` / `.done` (see {@link phraseKey}),
 * so this table and `lib/i18n/en.ts` must agree. `toolPhrases.test.ts`
 * enforces that.
 *
 * Placeholders (`{app}`, `{tool}`) are filled by the caller and must survive
 * translation unchanged.
 */
export const TOOL_PHRASES = {
  // ── Files and code ──────────────────────────────────────────────────────
  readFile: { active: 'Reading file', done: 'Read file' },
  writeFile: { active: 'Writing file', done: 'Wrote file' },
  editFile: { active: 'Editing file', done: 'Edited file' },
  applyEdits: { active: 'Applying edits', done: 'Applied edits' },
  searchCode: { active: 'Searching code', done: 'Searched code' },
  findFiles: { active: 'Finding files', done: 'Found files' },
  listFolder: { active: 'Listing folder', done: 'Listed folder' },
  exportCsv: { active: 'Exporting CSV', done: 'Exported CSV' },
  updateMemoryNotes: { active: 'Updating memory notes', done: 'Updated memory notes' },
  runGit: { active: 'Running git', done: 'Ran git' },
  readChanges: { active: 'Reading changes', done: 'Read changes' },
  runLinter: { active: 'Running linter', done: 'Ran linter' },
  runTests: { active: 'Running tests', done: 'Ran tests' },
  analyzeCode: { active: 'Analyzing code', done: 'Analyzed code' },

  // ── Shell and system ────────────────────────────────────────────────────
  runCommand: { active: 'Running command', done: 'Ran command' },
  runCode: { active: 'Running code', done: 'Ran code' },
  runPackageManager: { active: 'Running npm', done: 'Ran npm' },
  checkInstalledTools: { active: 'Checking installed tools', done: 'Checked installed tools' },
  installTool: { active: 'Installing tool', done: 'Installed tool' },
  checkTime: { active: 'Checking the time', done: 'Checked the time' },
  resolveDate: { active: 'Working out the date', done: 'Worked out the date' },
  retrieveOutput: { active: 'Retrieving full output', done: 'Retrieved full output' },
  reviewWorkspace: { active: 'Reviewing workspace', done: 'Reviewed workspace' },
  configureProxy: { active: 'Configuring proxy', done: 'Configured proxy' },
  checkUpdates: { active: 'Checking for updates', done: 'Checked for updates' },
  installUpdate: { active: 'Installing update', done: 'Installed update' },
  sendNotification: { active: 'Sending notification', done: 'Sent notification' },
  reviewToolUsage: { active: 'Reviewing tool usage', done: 'Reviewed tool usage' },
  typeKeys: { active: 'Typing', done: 'Typed' },
  click: { active: 'Clicking', done: 'Clicked' },

  // ── Web ─────────────────────────────────────────────────────────────────
  searchWeb: { active: 'Searching the web', done: 'Searched the web' },
  searchNews: { active: 'Searching news', done: 'Searched news' },
  searchImages: { active: 'Searching images', done: 'Searched images' },
  searchVideos: { active: 'Searching videos', done: 'Searched videos' },
  findSimilarPages: { active: 'Finding similar pages', done: 'Found similar pages' },
  readPages: { active: 'Reading pages', done: 'Read pages' },
  readWebpage: { active: 'Reading webpage', done: 'Read webpage' },
  research: { active: 'Researching', done: 'Researched' },
  enrichData: { active: 'Enriching data', done: 'Enriched data' },
  buildDataset: { active: 'Building dataset', done: 'Built dataset' },
  askTheWeb: { active: 'Asking the web', done: 'Asked the web' },
  browseForYou: { active: 'Browsing for you', done: 'Browsed for you' },
  callApi: { active: 'Calling API', done: 'Called API' },
  downloadFile: { active: 'Downloading file', done: 'Downloaded file' },
  makePaidRequest: { active: 'Making paid request', done: 'Made paid request' },
  searchDocs: { active: 'Searching docs', done: 'Searched docs' },
  readDocs: { active: 'Reading docs', done: 'Read docs' },

  // ── Browser ─────────────────────────────────────────────────────────────
  useBrowser: { active: 'Using browser', done: 'Used browser' },
  openPage: { active: 'Opening page', done: 'Opened page' },
  navigate: { active: 'Navigating', done: 'Navigated' },
  takeScreenshot: { active: 'Taking screenshot', done: 'Took screenshot' },
  scrollPage: { active: 'Scrolling', done: 'Scrolled' },
  readPage: { active: 'Reading page', done: 'Read page' },

  // ── Native desktop ───────────────────────────────────────────────────────
  inspectDesktop: { active: 'Inspecting desktop', done: 'Inspected desktop' },
  controlDesktop: { active: 'Controlling desktop', done: 'Controlled desktop' },

  // ── Media and documents ────────────────────────────────────────────────
  analyzeImage: { active: 'Analyzing image', done: 'Analyzed image' },
  generateImage: { active: 'Generating image', done: 'Generated image' },
  generateVideo: { active: 'Generating video', done: 'Generated video' },
  checkMediaModels: { active: 'Checking media models', done: 'Checked media models' },
  createDocument: { active: 'Creating document', done: 'Created document' },
  createPresentation: { active: 'Creating presentation', done: 'Created presentation' },
  showView: { active: 'Showing a view', done: 'Showed a view' },
  generatePodcast: { active: 'Generating podcast', done: 'Generated podcast' },
  emailPodcast: { active: 'Emailing podcast', done: 'Emailed podcast' },
  createAndEmailPodcast: {
    active: 'Creating and emailing podcast',
    done: 'Created and emailed podcast',
  },

  // ── Memory ──────────────────────────────────────────────────────────────
  recallMemories: { active: 'Recalling memories', done: 'Recalled memories' },
  saveToMemory: { active: 'Saving to memory', done: 'Saved to memory' },
  forgetMemory: { active: 'Forgetting memory', done: 'Forgot memory' },
  searchMemory: { active: 'Searching memory', done: 'Searched memory' },
  inspectMemory: { active: 'Inspecting memory', done: 'Inspected memory' },
  exploreMemory: { active: 'Exploring memory', done: 'Explored memory' },
  saveDocumentToMemory: { active: 'Saving document to memory', done: 'Saved document to memory' },
  updateGoals: { active: 'Updating goals', done: 'Updated goals' },
  reviewGoals: { active: 'Reviewing goals', done: 'Reviewed goals' },
  savePreference: { active: 'Saving preference', done: 'Saved preference' },
  reviewLearnings: { active: 'Reviewing what I learned', done: 'Reviewed what I learned' },
  updateLearnings: { active: 'Updating what I learned', done: 'Updated what I learned' },

  // ── Agents and delegation ──────────────────────────────────────────────
  delegateTask: { active: 'Delegating task', done: 'Delegated task' },
  runAgentsInParallel: { active: 'Running agents in parallel', done: 'Ran agents in parallel' },
  messageAgent: { active: 'Messaging agent', done: 'Messaged agent' },
  waitForAgent: { active: 'Waiting for agent', done: 'Waited for agent' },
  wait: { active: 'Waiting', done: 'Waited' },
  closeAgent: { active: 'Closing agent', done: 'Closed agent' },
  checkAgents: { active: 'Checking agents', done: 'Checked agents' },
  askQuestion: { active: 'Asking you a question', done: 'Asked you a question' },
  prepareContext: { active: 'Preparing context', done: 'Prepared context' },
  extractDetails: { active: 'Extracting details', done: 'Extracted details' },
  planNextSteps: { active: 'Planning next steps', done: 'Planned next steps' },
  reviewWork: { active: 'Reviewing the work', done: 'Reviewed the work' },
  useTools: { active: 'Using tools', done: 'Used tools' },
  checkConnectedApp: { active: 'Checking your connected app', done: 'Checked your connected app' },

  // ── Planning ────────────────────────────────────────────────────────────
  updateTodos: { active: 'Updating to-do list', done: 'Updated to-do list' },
  requestPlanReview: { active: 'Requesting plan review', done: 'Requested plan review' },
  finishPlan: { active: 'Finishing plan', done: 'Finished plan' },
  setGoal: { active: 'Setting goal', done: 'Set goal' },
  checkGoal: { active: 'Checking goal', done: 'Checked goal' },
  completeGoal: { active: 'Completing goal', done: 'Completed goal' },

  // ── Scheduling ──────────────────────────────────────────────────────────
  scheduleTask: { active: 'Scheduling task', done: 'Scheduled task' },
  checkSchedules: { active: 'Checking schedules', done: 'Checked schedules' },
  updateSchedule: { active: 'Updating scheduled task', done: 'Updated scheduled task' },
  removeSchedule: { active: 'Removing scheduled task', done: 'Removed scheduled task' },
  runScheduledTask: { active: 'Running scheduled task', done: 'Ran scheduled task' },
  checkRunHistory: { active: 'Checking run history', done: 'Checked run history' },

  // ── Connected apps ─────────────────────────────────────────────────────
  useApp: { active: 'Using {app}', done: 'Used {app}' },
  checkAvailableApps: { active: 'Checking available apps', done: 'Checked available apps' },
  checkConnections: { active: 'Checking your connections', done: 'Checked your connections' },
  connectApp: { active: 'Connecting app', done: 'Connected app' },
  authorizeApp: { active: 'Authorizing app', done: 'Authorized app' },
  findAppActions: { active: 'Finding app actions', done: 'Found app actions' },
  runAppAction: { active: 'Running app action', done: 'Ran app action' },
  findTools: { active: 'Finding tools', done: 'Found tools' },
  useTool: { active: 'Using {tool}', done: 'Used {tool}' },
  unsubscribe: { active: 'Unsubscribing', done: 'Unsubscribed' },
  searchPlaces: { active: 'Searching places', done: 'Searched places' },
  lookUpPlace: { active: 'Looking up place', done: 'Looked up place' },
  checkMarkets: { active: 'Checking markets', done: 'Checked markets' },
  placeCall: { active: 'Placing call', done: 'Placed call' },
  checkTaskSources: { active: 'Checking task sources', done: 'Checked task sources' },
  updateTaskSources: { active: 'Updating task sources', done: 'Updated task sources' },
  fetchTasks: { active: 'Fetching tasks', done: 'Fetched tasks' },

  // ── MCP ─────────────────────────────────────────────────────────────────
  checkMcpServers: { active: 'Checking MCP servers', done: 'Checked MCP servers' },
  checkMcpTools: { active: 'Checking MCP tools', done: 'Checked MCP tools' },
  callMcpTool: { active: 'Calling {tool}', done: 'Called {tool}' },
  searchMcpServers: { active: 'Searching MCP servers', done: 'Searched MCP servers' },
  connectMcpServer: { active: 'Connecting MCP server', done: 'Connected MCP server' },
  disconnectMcpServer: { active: 'Disconnecting MCP server', done: 'Disconnected MCP server' },
  removeMcpServer: { active: 'Removing MCP server', done: 'Removed MCP server' },

  // ── Storage and hosting ────────────────────────────────────────────────
  uploadFile: { active: 'Uploading file', done: 'Uploaded file' },
  listStoredFiles: { active: 'Listing stored files', done: 'Listed stored files' },
  createShareLink: { active: 'Creating share link', done: 'Created share link' },
  deleteFile: { active: 'Deleting file', done: 'Deleted file' },
  updateFileAccess: { active: 'Updating file access', done: 'Updated file access' },
  deploySite: { active: 'Deploying site', done: 'Deployed site' },
  checkHosting: { active: 'Checking hosting', done: 'Checked hosting' },
  updateHosting: { active: 'Updating hosting', done: 'Updated hosting' },
  rollBackDeployment: { active: 'Rolling back deployment', done: 'Rolled back deployment' },

  // ── Wallet ──────────────────────────────────────────────────────────────
  checkWallet: { active: 'Checking wallet', done: 'Checked wallet' },
  prepareTransfer: { active: 'Preparing transfer', done: 'Prepared transfer' },
  checkTransaction: { active: 'Checking transaction', done: 'Checked transaction' },
  getSwapQuote: { active: 'Getting swap quote', done: 'Got swap quote' },
  swapTokens: { active: 'Swapping tokens', done: 'Swapped tokens' },
  getBridgeQuote: { active: 'Getting bridge quote', done: 'Got bridge quote' },
  bridgeTokens: { active: 'Bridging tokens', done: 'Bridged tokens' },
  callDapp: { active: 'Calling app contract', done: 'Called app contract' },

  // ── Skills and workflows ───────────────────────────────────────────────
  useSkill: { active: 'Using skill', done: 'Used skill' },
  searchSkills: { active: 'Searching skills', done: 'Searched skills' },
  checkSkills: { active: 'Checking skills', done: 'Checked skills' },
  installSkill: { active: 'Installing skill', done: 'Installed skill' },
  removeSkill: { active: 'Removing skill', done: 'Removed skill' },
  createSkill: { active: 'Creating skill', done: 'Created skill' },
  runWorkflow: { active: 'Running workflow', done: 'Ran workflow' },
  waitForWorkflow: { active: 'Waiting for workflow', done: 'Waited for workflow' },
  designWorkflow: { active: 'Designing workflow', done: 'Designed workflow' },
  saveWorkflow: { active: 'Saving workflow', done: 'Saved workflow' },
  validateWorkflow: { active: 'Validating workflow', done: 'Validated workflow' },
  testWorkflow: { active: 'Testing workflow', done: 'Tested workflow' },
  checkWorkflows: { active: 'Checking workflows', done: 'Checked workflows' },
  cancelWorkflow: { active: 'Cancelling workflow run', done: 'Cancelled workflow run' },
  suggestWorkflows: { active: 'Suggesting workflows', done: 'Suggested workflows' },

  // ── Settings and platform ──────────────────────────────────────────────
  checkSettings: { active: 'Checking settings', done: 'Checked settings' },
  checkSecurity: { active: 'Checking security', done: 'Checked security' },
  runDiagnostics: { active: 'Running diagnostics', done: 'Ran diagnostics' },
  checkUsageCosts: { active: 'Checking usage costs', done: 'Checked usage costs' },
  manageService: { active: 'Managing background service', done: 'Managed background service' },
  readPersona: { active: 'Reading persona', done: 'Read persona' },
  updatePersona: { active: 'Updating persona', done: 'Updated persona' },
  setUpWorkspace: { active: 'Setting up workspace', done: 'Set up workspace' },
  checkArtifacts: { active: 'Checking artifacts', done: 'Checked artifacts' },
  deleteArtifact: { active: 'Deleting artifact', done: 'Deleted artifact' },
} as const satisfies Record<string, { active: string; done: string }>;

export type ToolPhraseId = keyof typeof TOOL_PHRASES;
export type ToolPhraseTense = 'active' | 'done';

/** The i18n key a phrase is served under. */
export function phraseKey(id: ToolPhraseId, tense: ToolPhraseTense): string {
  return `conversations.tools.${id}.${tense}`;
}

/** Substitute `{name}` placeholders. Unknown placeholders are left in place. */
export function fillPlaceholders(template: string, params?: Record<string, string>): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => params[name] ?? match);
}
