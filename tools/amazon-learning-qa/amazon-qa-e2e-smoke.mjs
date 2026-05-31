#!/usr/bin/env node

const DEFAULT_BASE_URL = "http://127.0.0.1:7790";
const DEFAULT_FIRST_QUESTION = "主图视觉点击率转化率怎么优化？";
const DEFAULT_SECOND_QUESTION = "那我应该先改哪一块？";
const DEFAULT_EXPECTED_DOCUMENTS = 1779;
const DEFAULT_EXPECTED_CHUNKS = 14597;
const DEFAULT_TIMEOUT_MS = 60_000;

export function validateAmazonQaSmoke(input = {}, options = {}) {
  const expectedDocuments = Number(options.expectedDocuments || DEFAULT_EXPECTED_DOCUMENTS);
  const expectedChunks = Number(options.expectedChunks || DEFAULT_EXPECTED_CHUNKS);
  const firstQuestion = options.firstQuestion || DEFAULT_FIRST_QUESTION;
  const secondQuestion = options.secondQuestion || DEFAULT_SECOND_QUESTION;

  const status = input.status || {};
  const health = status.health || {};
  const readiness = status.readiness || {};
  const sourceTree = health.sourceTree || {};
  const documents = Number(health.documents || 0);
  const chunks = Number(health.chunks || 0);
  const embeddedChunks = Number(health.embeddedChunks || 0);
  const vectorCoveragePercent = Number(health.vectorCoveragePercent || 0);

  assertSmoke(documents === expectedDocuments, `Expected ${expectedDocuments} documents, got ${documents}.`);
  assertSmoke(chunks === expectedChunks, `Expected ${expectedChunks} chunks, got ${chunks}.`);
  assertSmoke(
    embeddedChunks === chunks && vectorCoveragePercent >= 99,
    `Semantic index incomplete: ${embeddedChunks}/${chunks}, coverage ${vectorCoveragePercent}%.`,
  );
  assertSmoke(readiness.searchStatus === "ready", `Search readiness is not ready: ${readiness.searchStatus || "missing"}.`);
  assertSmoke(readiness.citationStatus === "ready", `Citation readiness is not ready: ${readiness.citationStatus || "missing"}.`);
  assertSmoke(Number(sourceTree.failedJobs || 0) === 0, `Source tree has ${Number(sourceTree.failedJobs || 0)} failed jobs.`);

  if (input.sourceSelection) assertSourceSelection(input.sourceSelection);
  assertAskPayload(input.first, "first question");
  assertAskPayload(input.second, "follow-up question");

  const messages = Array.isArray(input.second?.messages) ? input.second.messages : [];
  assertSmoke(messages.length >= 4, `Follow-up conversation did not preserve enough history: ${messages.length} messages.`);
  assertSmoke(
    messages.some((message) => message?.role === "user" && String(message.content || "").includes(firstQuestion.slice(0, 6))),
    "Follow-up history is missing the first user question.",
  );
  assertSmoke(
    messages.some((message) => message?.role === "user" && String(message.content || "").includes(secondQuestion.slice(0, 4))),
    "Follow-up history is missing the second user question.",
  );

  if (options.requireSourceContext !== false) {
    const sourceContext = input.sourceContext || {};
    assertSmoke(
      sourceContext.status === "located",
      `Source context was not located: ${sourceContext.status || "missing"}.`,
    );
  }

  if (input.notebook) {
    const notebook = input.notebook || {};
    const notebookMessages = Array.isArray(notebook.messages) ? notebook.messages : [];
    assertSmoke(notebook.id, "Notebook response is missing an id.");
    assertSmoke(notebookMessages.length >= 4, `Notebook did not preserve the two-turn conversation: ${notebookMessages.length} messages.`);
    assertSmoke(
      String(notebook.boundary || "").includes("不是作者原文证据"),
      "Notebook boundary did not preserve the source-evidence separation.",
    );
  }

  const studyPack = input.studyPack || {};
  assertSmoke(studyPack && typeof studyPack === "object", "Notebook study pack is missing.");
  assertSmoke(Array.isArray(studyPack.takeaways), "Notebook study pack is missing takeaways.");
  assertSmoke(Array.isArray(studyPack.sourceLedger), "Notebook study pack is missing the source ledger.");
  assertSmoke(String(studyPack.boundary || "").includes("不是作者原文证据"), "Notebook study pack boundary is missing.");
  assertSmoke(String(studyPack.markdown || "").includes("## 来源账本"), "Notebook study pack markdown is incomplete.");
  assertSmoke(String(studyPack.exportMarkdown || "").includes("## 本地学习包预览"), "Notebook learning pack export markdown is missing.");
  assertSmoke(String(studyPack.exportMarkdown || "").includes("系统掌握度自测"), "Notebook Studio export markdown is missing the mastery self-test.");
  assertSmoke(Array.isArray(studyPack.studio?.reportSections), "Notebook Studio pack is missing report sections.");
  assertSmoke(Array.isArray(studyPack.studio?.flashcards), "Notebook Studio pack is missing flashcards.");
  assertSmoke(Array.isArray(studyPack.studio?.mindMap?.nodes), "Notebook Studio pack is missing mind map nodes.");
  assertSmoke(Array.isArray(studyPack.studio?.sourceTable), "Notebook Studio pack is missing the source table.");
  assertSmoke(Array.isArray(studyPack.studio?.masteryQuiz?.items), "Notebook Studio pack is missing the mastery self-test.");
  assertSmoke(Array.isArray(studyPack.studio?.actionPlan?.steps), "Notebook Studio pack is missing the Amazon action experiment plan.");
  assertSmoke(
    String(studyPack.studio?.actionPlan?.boundary || "").includes("不会写入作者原文证据"),
    "Notebook Studio action plan boundary is missing.",
  );
  assertSmoke(
    String(studyPack.studio?.masteryQuiz?.boundary || "").includes("不会写入作者原文证据"),
    "Notebook Studio mastery self-test boundary is missing.",
  );
  assertSmoke(String(studyPack.studio?.flashcardsCsv || "").includes("正面"), "Notebook Studio pack is missing flashcard CSV export.");
  assertSmoke(String(studyPack.studio?.sourceTableCsv || "").includes("作者"), "Notebook Studio pack is missing source table CSV export.");
  assertSmoke(
    studyPack.sourceLedger.every((source) => String(source.identity || "").includes("证据来源")),
    "Notebook study pack contains unverified candidate sources.",
  );
  if (studyPack.sourceLedger.length > 0) {
    assertSmoke(
      studyPack.takeaways.some((item) => item.support === "source_backed" || String(item.identity || "").includes("来源支撑")),
      "Notebook study pack has sources but no source-bound learning takeaway.",
    );
  }

  return {
    ok: true,
    documents,
    chunks,
    embeddedChunks,
    vectorCoveragePercent,
    readiness: status.readiness?.level || "",
    searchStatus: readiness.searchStatus || "",
    citationStatus: readiness.citationStatus || "",
    learningStatus: readiness.learningStatus || "",
    sourceTreeQueuedJobs: Number(sourceTree.queuedJobs || 0),
    sourceTreeFailedJobs: Number(sourceTree.failedJobs || 0),
    sourceSelectionStatus: input.sourceSelection?.status || "not_checked",
    sourceSelectionSources: Array.isArray(input.sourceSelection?.recommended) ? input.sourceSelection.recommended.length : 0,
    sourceSelectionIntent: input.sourceSelection?.intent?.type || "",
    firstSources: input.first.sources.length,
    firstGraphNodes: input.first.graph.nodes.length,
    firstRadarStatus: input.first.knowledgeGapRadar?.status || "",
    firstRadarGaps: Array.isArray(input.first.knowledgeGapRadar?.gaps) ? input.first.knowledgeGapRadar.gaps.length : 0,
    firstAnswerMode: input.first.answerGeneration?.mode || "template",
    firstAnswerModel: input.first.answerGeneration?.model || "",
    secondSources: input.second.sources.length,
    secondGraphNodes: input.second.graph.nodes.length,
    secondRadarStatus: input.second.knowledgeGapRadar?.status || "",
    secondRadarGaps: Array.isArray(input.second.knowledgeGapRadar?.gaps) ? input.second.knowledgeGapRadar.gaps.length : 0,
    secondNextBestSourceKind: input.second.nextBestSource?.recommended?.kind || "",
    secondAnswerMode: input.second.answerGeneration?.mode || "template",
    secondAnswerModel: input.second.answerGeneration?.model || "",
    secondTopicSources: input.second.topicSourceTree.sources.length,
    hasSecondNotebookGuide: Boolean(input.second.notebookGuide),
    hasSecondSynthesis: Boolean(input.second.synthesisAnswer),
    sourceContextStatus: input.sourceContext?.status || "not_checked",
    sessionId: input.second.sessionId || input.first.sessionId || "",
    notebookMessages: Array.isArray(input.notebook?.messages) ? input.notebook.messages.length : 0,
    studyPackSources: Array.isArray(input.studyPack?.sourceLedger) ? input.studyPack.sourceLedger.length : 0,
    studyPackTakeaways: Array.isArray(input.studyPack?.takeaways) ? input.studyPack.takeaways.length : 0,
    studioFlashcards: Array.isArray(input.studyPack?.studio?.flashcards) ? input.studyPack.studio.flashcards.length : 0,
    studioMindMapNodes: Array.isArray(input.studyPack?.studio?.mindMap?.nodes) ? input.studyPack.studio.mindMap.nodes.length : 0,
    studioMasteryItems: Array.isArray(input.studyPack?.studio?.masteryQuiz?.items) ? input.studyPack.studio.masteryQuiz.items.length : 0,
    studioActionSteps: Array.isArray(input.studyPack?.studio?.actionPlan?.steps) ? input.studyPack.studio.actionPlan.steps.length : 0,
  };
}

export async function runAmazonQaSmoke(options = {}) {
  const baseUrl = String(options.baseUrl || DEFAULT_BASE_URL).replace(/\/+$/, "");
  const firstQuestion = options.firstQuestion || DEFAULT_FIRST_QUESTION;
  const secondQuestion = options.secondQuestion || DEFAULT_SECOND_QUESTION;
  const timeoutMs = Number(options.timeoutMs || DEFAULT_TIMEOUT_MS);
  const sessionId = options.sessionId || `amazon-qa-smoke-${Date.now()}`;

  const status = await fetchJson(`${baseUrl}/api/status`, { timeoutMs });
  const sourceSelectionPayload = await fetchJson(`${baseUrl}/api/source-selection`, {
    method: "POST",
    timeoutMs,
    body: {
      question: firstQuestion,
      sessionId,
      history: [],
    },
  });
  const sourceSelection = sourceSelectionPayload.sourceSelection || sourceSelectionPayload;
  const first = await fetchJson(`${baseUrl}/api/ask`, {
    method: "POST",
    timeoutMs,
    body: {
      question: firstQuestion,
      sessionId,
      history: [],
    },
  });
  const second = await fetchJson(`${baseUrl}/api/ask`, {
    method: "POST",
    timeoutMs,
    body: {
      question: secondQuestion,
      sessionId: first.sessionId || sessionId,
      history: Array.isArray(first.messages) ? first.messages : [],
    },
  });
  const sourceContext = options.requireSourceContext === false
    ? undefined
    : await fetchSourceContext(baseUrl, {
        timeoutMs,
        sessionId: second.sessionId || first.sessionId || sessionId,
        second,
      });
  const notebook = await fetchNotebook(baseUrl, {
    timeoutMs,
    sessionId: second.sessionId || first.sessionId || sessionId,
  });
  const studyPack = await fetchNotebookStudyPack(baseUrl, {
    timeoutMs,
    sessionId: second.sessionId || first.sessionId || sessionId,
  });

  return validateAmazonQaSmoke(
    { status, sourceSelection, first, second, sourceContext, notebook, studyPack },
    {
      expectedDocuments: options.expectedDocuments,
      expectedChunks: options.expectedChunks,
      firstQuestion,
      secondQuestion,
      requireSourceContext: options.requireSourceContext,
    },
  );
}

function assertSourceSelection(selection) {
  assertSmoke(selection && typeof selection === "object", "Question source selection is missing.");
  const recommended = Array.isArray(selection.recommended) ? selection.recommended : [];
  const intentChoices = Array.isArray(selection.intentChoices) ? selection.intentChoices : [];
  assertSmoke(recommended.length > 0, "Question source selection has no recommended sources.");
  assertSmoke(selection.intent?.type, "Question source selection has no detected intent.");
  assertSmoke(intentChoices.length > 0, "Question source selection has no selectable intents.");
  assertSmoke(selection.sourceControls && typeof selection.sourceControls === "object", "Question source selection has no source controls.");
  assertSmoke(
    String(selection.boundary || "").includes("不是新的作者原文证据"),
    "Question source selection is missing its evidence boundary.",
  );
}

function assertAskPayload(payload, label) {
  assertSmoke(payload && typeof payload === "object", `Missing ${label} response.`);
  const sources = Array.isArray(payload.sources) ? payload.sources : [];
  const graphNodes = Array.isArray(payload.graph?.nodes) ? payload.graph.nodes : [];
  const topicSources = Array.isArray(payload.topicSourceTree?.sources) ? payload.topicSourceTree.sources : [];
  const radarGaps = Array.isArray(payload.knowledgeGapRadar?.gaps) ? payload.knowledgeGapRadar.gaps : [];
  const nextSource = payload.nextBestSource?.recommended || {};
  assertSmoke(sources.length > 0, `The ${label} has no source citations.`);
  assertSmoke(graphNodes.length > 0, `The ${label} has no answer graph.`);
  assertSmoke(topicSources.length > 0, `The ${label} has no topic source tree.`);
  assertSmoke(radarGaps.length > 0, `The ${label} has no knowledge gap radar.`);
  assertSmoke(nextSource.label, `The ${label} has no next source choice.`);
  assertSmoke(
    String(payload.nextBestSource?.boundary || "").includes("不是新的作者原文证据"),
    `The ${label} next source choice is missing its evidence boundary.`,
  );
  assertSmoke(payload.notebookGuide && typeof payload.notebookGuide === "object", `The ${label} has no learning brief.`);
  assertSmoke(payload.synthesisAnswer && typeof payload.synthesisAnswer === "object", `The ${label} has no synthesis answer.`);
  if (payload.answerGeneration) {
    assertSmoke(payload.answerGeneration.mode === "local_ollama", `The ${label} answer generation mode is not local Ollama.`);
    assertSmoke(payload.answerGeneration.model, `The ${label} answer generation has no model.`);
    assertSmoke(
      String(payload.answerGeneration.boundary || "").includes("来源") && String(payload.answerGeneration.boundary || "").includes("核对"),
      `The ${label} answer generation is missing its source boundary.`,
    );
  }
}

async function fetchSourceContext(baseUrl, { timeoutMs, sessionId, second }) {
  const source = Array.isArray(second.sources) ? second.sources[0] : null;
  assertSmoke(source, "Cannot check source context because follow-up returned no sources.");
  const messages = Array.isArray(second.messages) ? second.messages : [];
  const messageIndex = Math.max(0, messages.length - 1);
  return fetchJson(`${baseUrl}/api/source-context`, {
    method: "POST",
    timeoutMs,
    body: {
      sessionId,
      messageIndex,
      sourceIndex: 0,
      source,
      quote: source.excerpt || "",
    },
  }).then((payload) => payload.sourceContext || payload);
}

async function fetchNotebook(baseUrl, { timeoutMs, sessionId }) {
  assertSmoke(sessionId, "Cannot check notebook persistence because the session id is missing.");
  const payload = await fetchJson(`${baseUrl}/api/notebooks/${encodeURIComponent(sessionId)}`, {
    timeoutMs,
  });
  return payload.notebook || payload;
}

async function fetchNotebookStudyPack(baseUrl, { timeoutMs, sessionId }) {
  assertSmoke(sessionId, "Cannot check notebook study pack because the session id is missing.");
  const payload = await fetchJson(`${baseUrl}/api/notebooks/${encodeURIComponent(sessionId)}/study-pack`, {
    timeoutMs,
  });
  return payload.studyPack || payload;
}

async function fetchJson(url, options = {}) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), Number(options.timeoutMs || DEFAULT_TIMEOUT_MS));
  try {
    const response = await fetch(url, {
      method: options.method || "GET",
      headers: options.body ? { "content-type": "application/json" } : undefined,
      body: options.body ? JSON.stringify(options.body) : undefined,
      signal: controller.signal,
    });
    const text = await response.text();
    let payload;
    try {
      payload = text ? JSON.parse(text) : {};
    } catch {
      throw new Error(`Expected JSON from ${url}, got: ${text.slice(0, 180)}`);
    }
    if (!response.ok) {
      throw new Error(payload?.error || `${url} failed with HTTP ${response.status}`);
    }
    return payload;
  } finally {
    clearTimeout(timeout);
  }
}

function assertSmoke(condition, message) {
  if (!condition) throw new Error(message);
}

function parseArgs(argv = []) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--base-url") options.baseUrl = argv[++index];
    else if (arg === "--first") options.firstQuestion = argv[++index];
    else if (arg === "--second") options.secondQuestion = argv[++index];
    else if (arg === "--expected-documents") options.expectedDocuments = Number(argv[++index]);
    else if (arg === "--expected-chunks") options.expectedChunks = Number(argv[++index]);
    else if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++index]);
    else if (arg === "--no-source-context") options.requireSourceContext = false;
    else if (arg === "--help") {
      printHelp();
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return options;
}

function printHelp() {
  console.log(`Usage: node tools/amazon-qa-e2e-smoke.mjs [options]

Options:
  --base-url <url>              Amazon QA server URL. Default: ${DEFAULT_BASE_URL}
  --first <question>            First question.
  --second <question>           Follow-up question.
  --expected-documents <n>      Expected document count. Default: ${DEFAULT_EXPECTED_DOCUMENTS}
  --expected-chunks <n>         Expected semantic chunk count. Default: ${DEFAULT_EXPECTED_CHUNKS}
  --timeout-ms <n>              Request timeout. Default: ${DEFAULT_TIMEOUT_MS}
  --no-source-context           Skip /api/source-context verification.
`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  runAmazonQaSmoke(parseArgs(process.argv.slice(2)))
    .then((report) => {
      console.log(JSON.stringify(report, null, 2));
    })
    .catch((error) => {
      console.error(error instanceof Error ? error.message : String(error));
      process.exit(1);
    });
}
