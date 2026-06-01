#!/usr/bin/env node

const DEFAULT_BASE_URL = "http://127.0.0.1:7790";
const DEFAULT_FIRST_QUESTION = "主图视觉点击率转化率怎么优化？";
const DEFAULT_SECOND_QUESTION = "那我应该先改哪一块？";
const DEFAULT_EXPECTED_DOCUMENTS = 1779;
const DEFAULT_EXPECTED_CHUNKS = 14597;
const DEFAULT_TIMEOUT_MS = 120_000;
export const FINAL_ACCEPTANCE_SCENARIOS = [
  {
    id: "visual-conversion",
    question: "主图视觉点击率转化率怎么优化？",
    followUp: "那我应该先改哪一块？",
  },
  {
    id: "product-selection",
    question: "新品选品应该如何判断是否值得做？",
  },
  {
    id: "listing-keywords",
    question: "Listing 文案关键词布局收录应该怎么做？",
  },
];
export const TOPIC_SWITCH_ACCEPTANCE_SCENARIO = {
  id: "topic-switch",
  firstQuestion: "主图视觉点击率转化率怎么优化？",
  standaloneQuestions: [
    {
      id: "product-title",
      question: "产品标题怎么写",
      requireSources: true,
      relevancePattern: /标题|关键词|Search Terms|收录|文案|Listing/i,
    },
    {
      id: "listing-prep",
      question: "写 listing 之前，应该要进行哪些准备工作？具体收集哪些资料",
      requireSources: true,
      relevancePattern: /Listing|关键词|标题|五点|Search Terms|资料|竞品|收集|文案/i,
    },
    {
      id: "persona",
      question: "人群画像应该怎么构建？有哪些实操指导建议",
      requireSources: true,
      relevancePattern: /人群|画像|用户|受众|买家|目标客户|竞品信息|搜索词|词库|基本功/i,
    },
    {
      id: "selection-methods",
      question: "列出所有选品实操的可落地执行方法？",
      requireSources: true,
      relevancePattern: /选品|市场|需求|竞争|利润|产品/,
    },
  ],
};

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
    secondDecisionRows: Array.isArray(input.second.sourceDecisionTable?.rows) ? input.second.sourceDecisionTable.rows.length : 0,
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

export function validateAmazonQaFinalAcceptance(input = {}, options = {}) {
  const scenarios = Array.isArray(options.scenarios) && options.scenarios.length
    ? options.scenarios
    : FINAL_ACCEPTANCE_SCENARIOS;
  const status = input.status || {};
  validateAmazonQaSmoke(
    {
      status,
      sourceSelection: input.sourceSelections?.[0],
      first: input.scenarios?.[0]?.first,
      second: input.scenarios?.[0]?.second || input.scenarios?.[0]?.first,
      sourceContext: input.sourceContext,
      notebook: input.notebook,
      studyPack: input.studyPack,
    },
    {
      expectedDocuments: options.expectedDocuments,
      expectedChunks: options.expectedChunks,
      firstQuestion: scenarios[0]?.question || DEFAULT_FIRST_QUESTION,
      secondQuestion: scenarios[0]?.followUp || DEFAULT_SECOND_QUESTION,
      requireSourceContext: options.requireSourceContext,
    },
  );

  assertSmoke(Array.isArray(input.scenarios), "Final acceptance scenarios are missing.");
  assertSmoke(input.scenarios.length >= scenarios.length, `Expected ${scenarios.length} final acceptance scenarios.`);
  const summaries = input.scenarios.slice(0, scenarios.length).map((entry, index) => {
    const scenario = scenarios[index];
    assertSmoke(entry?.id === scenario.id, `Scenario ${index + 1} id mismatch.`);
    assertSourceSelection(input.sourceSelections?.[index]);
    assertAskPayload(entry.first, `${scenario.id} question`);
    assertSmoke(
      entry.first.learningQueue?.items?.length > 0,
      `${scenario.id} did not return a learning queue.`,
    );
    assertSmoke(
      String(entry.first.answerGeneration?.boundary || "").includes("来源"),
      `${scenario.id} answer boundary is missing source language.`,
    );
    if (scenario.followUp) {
      assertAskPayload(entry.second, `${scenario.id} follow-up`);
      assertSmoke(
        Array.isArray(entry.second.messages) && entry.second.messages.length >= 4,
        `${scenario.id} follow-up did not preserve conversation history.`,
      );
    }
    return {
      id: scenario.id,
      question: scenario.question,
      followUp: scenario.followUp || "",
      sources: entry.first.sources.length,
      graphNodes: entry.first.graph.nodes.length,
      learningQueueItems: entry.first.learningQueue.items.length,
      answerMode: entry.first.answerGeneration?.mode || "template",
    };
  });
  const topicSwitch = assertTopicSwitchAcceptance(input.topicSwitch);
  const confirmationLoop = assertConfirmationLoopAcceptance(input.confirmationLoop);

  return {
    ok: true,
    scenarios: summaries,
    topicSwitch,
    confirmationLoop,
    documents: Number(status.health?.documents || 0),
    chunks: Number(status.health?.chunks || 0),
    embeddedChunks: Number(status.health?.embeddedChunks || 0),
    vectorCoveragePercent: Number(status.health?.vectorCoveragePercent || 0),
    sourceTreeQueuedJobs: Number(status.health?.sourceTree?.queuedJobs || 0),
    sourceTreeFailedJobs: Number(status.health?.sourceTree?.failedJobs || 0),
    notebookMessages: Array.isArray(input.notebook?.messages) ? input.notebook.messages.length : 0,
    studyPackSources: Array.isArray(input.studyPack?.sourceLedger) ? input.studyPack.sourceLedger.length : 0,
    studioFlashcards: Array.isArray(input.studyPack?.studio?.flashcards) ? input.studyPack.studio.flashcards.length : 0,
    studioMindMapNodes: Array.isArray(input.studyPack?.studio?.mindMap?.nodes) ? input.studyPack.studio.mindMap.nodes.length : 0,
    studioMasteryItems: Array.isArray(input.studyPack?.studio?.masteryQuiz?.items) ? input.studyPack.studio.masteryQuiz.items.length : 0,
    studioActionSteps: Array.isArray(input.studyPack?.studio?.actionPlan?.steps) ? input.studyPack.studio.actionPlan.steps.length : 0,
  };
}

export async function runAmazonQaFinalAcceptance(options = {}) {
  const baseUrl = String(options.baseUrl || DEFAULT_BASE_URL).replace(/\/+$/, "");
  const timeoutMs = Number(options.timeoutMs || DEFAULT_TIMEOUT_MS);
  const scenarios = Array.isArray(options.scenarios) && options.scenarios.length
    ? options.scenarios
    : FINAL_ACCEPTANCE_SCENARIOS;
  const sessionId = options.sessionId || `amazon-qa-acceptance-${Date.now()}`;
  const status = await fetchJson(`${baseUrl}/api/status`, { timeoutMs });
  const scenarioResults = [];
  const sourceSelections = [];
  let firstSessionId = "";

  for (const scenario of scenarios) {
    const sourceSelectionPayload = await fetchJson(`${baseUrl}/api/source-selection`, {
      method: "POST",
      timeoutMs,
      body: {
        question: scenario.question,
        sessionId,
        history: [],
      },
    });
    const sourceSelection = sourceSelectionPayload.sourceSelection || sourceSelectionPayload;
    sourceSelections.push(sourceSelection);
    const first = await fetchJson(`${baseUrl}/api/ask`, {
      method: "POST",
      timeoutMs,
      body: {
        question: scenario.question,
        sessionId: `${sessionId}-${scenario.id}`,
        history: [],
        intentPreference: sourceSelection.intent?.type || undefined,
      },
    });
    let second;
    if (scenario.followUp) {
      second = await fetchJson(`${baseUrl}/api/ask`, {
        method: "POST",
        timeoutMs,
        body: {
          question: scenario.followUp,
          sessionId: first.sessionId || `${sessionId}-${scenario.id}`,
          history: Array.isArray(first.messages) ? first.messages : [],
        },
      });
    }
    if (!firstSessionId) firstSessionId = second?.sessionId || first.sessionId || "";
    scenarioResults.push({ id: scenario.id, first, second });
  }

  const anchor = scenarioResults[0];
  const sourceContext = options.requireSourceContext === false || !anchor?.second
    ? undefined
    : await fetchSourceContext(baseUrl, {
        timeoutMs,
        sessionId: anchor.second.sessionId || anchor.first.sessionId || firstSessionId,
        second: anchor.second,
      });
  const notebook = firstSessionId
    ? await fetchNotebook(baseUrl, { timeoutMs, sessionId: firstSessionId })
    : undefined;
  const studyPack = firstSessionId
    ? await fetchNotebookStudyPack(baseUrl, { timeoutMs, sessionId: firstSessionId })
    : undefined;
  const topicSwitch = await runTopicSwitchAcceptance(baseUrl, { timeoutMs });
  const confirmationLoop = await runConfirmationLoopAcceptance(baseUrl, {
    timeoutMs,
    sessionId: anchor?.second?.sessionId || anchor?.first?.sessionId || firstSessionId,
    messages: anchor?.second?.messages || anchor?.first?.messages || [],
  });

  return validateAmazonQaFinalAcceptance(
    { status, sourceSelections, scenarios: scenarioResults, sourceContext, notebook, studyPack, topicSwitch, confirmationLoop },
    {
      expectedDocuments: options.expectedDocuments,
      expectedChunks: options.expectedChunks,
      requireSourceContext: options.requireSourceContext,
      scenarios,
    },
  );
}

async function runConfirmationLoopAcceptance(baseUrl, { timeoutMs, sessionId, messages }) {
  assertSmoke(sessionId, "Cannot check result confirmation because the session id is missing.");
  const history = Array.isArray(messages) ? messages : [];
  const messageIndex = findLastAssistantIndex(history);
  assertSmoke(messageIndex >= 0, "Cannot check result confirmation because no assistant answer was found.");
  const answerEffectiveness = {
    status: "needs_source",
    question: previousUserQuestionFromMessages(history, messageIndex),
    updatedAt: new Date().toISOString(),
  };
  const updatePayload = await fetchJson(`${baseUrl}/api/notebooks/${encodeURIComponent(sessionId)}/answer-effectiveness`, {
    method: "POST",
    timeoutMs,
    body: {
      messageIndex,
      answerEffectiveness,
    },
  });
  const confirmedNotebook = await fetchNotebook(baseUrl, { timeoutMs, sessionId });
  const nextQuestion = "继续帮我找更多作者原文来源";
  const next = await fetchJson(`${baseUrl}/api/ask`, {
    method: "POST",
    timeoutMs,
    body: {
      question: nextQuestion,
      sessionId,
      history: Array.isArray(confirmedNotebook.messages) ? confirmedNotebook.messages : [],
    },
  });
  return { sessionId, messageIndex, updatePayload, confirmedNotebook, nextQuestion, next };
}

function assertConfirmationLoopAcceptance(loop = {}) {
  assertSmoke(loop && typeof loop === "object", "Result confirmation acceptance result is missing.");
  assertSmoke(loop.sessionId, "Result confirmation session id is missing.");
  assertSmoke(Number.isInteger(loop.messageIndex) && loop.messageIndex >= 0, "Result confirmation message index is missing.");
  const updatedMessage = loop.updatePayload?.message || loop.confirmedNotebook?.messages?.[loop.messageIndex] || {};
  assertSmoke(
    updatedMessage.answerEffectiveness?.status === "needs_source",
    "Result confirmation did not persist the needs_source status.",
  );
  const notebookMessage = loop.confirmedNotebook?.messages?.[loop.messageIndex] || {};
  assertSmoke(
    notebookMessage.answerEffectiveness?.status === "needs_source",
    "Notebook did not preserve the needs_source result confirmation.",
  );
  assertAskPayload(loop.next, "result confirmation follow-up");
  const nextMessages = Array.isArray(loop.next?.messages) ? loop.next.messages : [];
  assertSmoke(
    nextMessages.some((message) => message?.role === "assistant" && message.answerEffectiveness?.status === "needs_source"),
    "Follow-up history did not carry the user's result confirmation.",
  );
  assertSmoke(
    nextMessages.some((message) => message?.role === "user" && String(message.content || "").includes(loop.nextQuestion)),
    "Result confirmation follow-up question was not preserved in history.",
  );
  return {
    sessionId: loop.sessionId,
    messageIndex: loop.messageIndex,
    status: updatedMessage.answerEffectiveness?.status || "",
    followUpQuestion: loop.nextQuestion || "",
    followUpSources: Array.isArray(loop.next?.sources) ? loop.next.sources.length : 0,
    followUpGraphNodes: Array.isArray(loop.next?.graph?.nodes) ? loop.next.graph.nodes.length : 0,
    followUpLearningQueueItems: Array.isArray(loop.next?.learningQueue?.items) ? loop.next.learningQueue.items.length : 0,
    followUpAnswerMode: loop.next?.answerGeneration?.mode || "template",
  };
}

function findLastAssistantIndex(messages = []) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index]?.role === "assistant") return index;
  }
  return -1;
}

function previousUserQuestionFromMessages(messages = [], messageIndex = messages.length) {
  for (let index = Math.min(messageIndex - 1, messages.length - 1); index >= 0; index -= 1) {
    if (messages[index]?.role === "user") return String(messages[index].content || "").slice(0, 240);
  }
  return "";
}

async function runTopicSwitchAcceptance(baseUrl, { timeoutMs }) {
  const scenario = TOPIC_SWITCH_ACCEPTANCE_SCENARIO;
  const sessionId = `amazon-qa-topic-switch-${Date.now()}`;
  const firstSourceSelectionPayload = await fetchJson(`${baseUrl}/api/source-selection`, {
    method: "POST",
    timeoutMs,
    body: {
      question: scenario.firstQuestion,
      sessionId,
      history: [],
    },
  });
  const firstSourceSelection = firstSourceSelectionPayload.sourceSelection || firstSourceSelectionPayload;
  const first = await fetchJson(`${baseUrl}/api/ask`, {
    method: "POST",
    timeoutMs,
    body: {
      question: scenario.firstQuestion,
      sessionId,
      history: [],
      sourceControls: firstSourceSelection.sourceControls,
      intentPreference: firstSourceSelection.intent?.type || undefined,
    },
  });
  const emptySourceControls = { excludedSourceKeys: [], allowedAuthors: [], allowedSourceKeys: [], selectedSources: [] };
  let prior = Array.isArray(first.messages) ? first.messages : [];
  const standaloneResults = [];
  for (const item of scenario.standaloneQuestions) {
    const response = await fetchJson(`${baseUrl}/api/ask`, {
      method: "POST",
      timeoutMs,
      body: {
        question: item.question,
        sessionId: first.sessionId || sessionId,
        history: prior,
        sourceControls: emptySourceControls,
      },
    });
    standaloneResults.push({ ...item, response });
    prior = Array.isArray(response.messages) ? response.messages : prior;
  }
  return { sessionId: first.sessionId || sessionId, firstSourceSelection, first, standaloneResults };
}

function assertTopicSwitchAcceptance(topicSwitch = {}) {
  assertSmoke(topicSwitch && typeof topicSwitch === "object", "Topic switch acceptance result is missing.");
  assertSourceSelection(topicSwitch.firstSourceSelection);
  assertAskPayload(topicSwitch.first, "topic switch first question");
  const rows = Array.isArray(topicSwitch.standaloneResults) ? topicSwitch.standaloneResults : [];
  assertSmoke(
    rows.length >= TOPIC_SWITCH_ACCEPTANCE_SCENARIO.standaloneQuestions.length,
    "Topic switch acceptance did not run all standalone questions.",
  );
  const results = rows.slice(0, TOPIC_SWITCH_ACCEPTANCE_SCENARIO.standaloneQuestions.length).map((entry) => {
    const expected = TOPIC_SWITCH_ACCEPTANCE_SCENARIO.standaloneQuestions.find((item) => item.id === entry.id) || entry;
    const response = entry.response || {};
    const answer = String(response.answer || "");
    const sources = Array.isArray(response.sources) ? response.sources : [];
    const graphNodes = Array.isArray(response.graph?.nodes) ? response.graph.nodes : [];
    const learningItems = Array.isArray(response.learningQueue?.items) ? response.learningQueue.items : [];
    const combined = `${answer}\n${sources.map((source) => source?.title || "").join("\n")}`;
    const earlyAnswer = answer.slice(0, 360);
    assertSmoke(String(response.question || "") === expected.question, `${expected.id} did not preserve its standalone question.`);
    assertSmoke(graphNodes.length > 0, `${expected.id} did not return an answer graph.`);
    assertSmoke(learningItems.length > 0, `${expected.id} did not return a learning queue.`);
    assertSmoke(
      !/(先把主图|主图点击率|主图差异化|视觉转化)/.test(earlyAnswer),
      `${expected.id} still appears polluted by the previous visual-conversion topic.`,
    );
    assertSmoke(
      expected.relevancePattern.test(combined),
      `${expected.id} answer is not relevant to its standalone question.`,
    );
    if (expected.requireSources) {
      assertSmoke(sources.length > 0, `${expected.id} should have returned source citations.`);
    }
    if (sources.length === 0) {
      assertSmoke(
        expected.allowNoSources && /缺少来源|没有.*资料|没有.*来源/.test(answer),
        `${expected.id} has no sources but did not disclose the source gap.`,
      );
    }
    if (response.sourceScope?.summary) {
      assertSmoke(
        String(response.sourceScope.summary).includes("全部作者"),
        `${expected.id} did not return to the normal all-author source scope.`,
      );
    }
    return {
      id: expected.id,
      question: expected.question,
      sources: sources.length,
      graphNodes: graphNodes.length,
      learningQueueItems: learningItems.length,
      sourceScope: response.sourceScope?.summary || "",
      answerMode: response.answerGeneration?.mode || "template",
    };
  });
  return {
    sessionId: topicSwitch.sessionId || topicSwitch.first?.sessionId || "",
    firstSources: Array.isArray(topicSwitch.first?.sources) ? topicSwitch.first.sources.length : 0,
    standaloneResults: results,
  };
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
  assertSmoke(payload.sourceDecisionTable && typeof payload.sourceDecisionTable === "object", `The ${label} has no source decision table.`);
  assertSmoke(
    String(payload.sourceDecisionTable?.boundary || "").includes("不是新的作者原文证据"),
    `The ${label} source decision table is missing its evidence boundary.`,
  );
  assertSmoke(
    payload.sourceDecisionTable.status === "needs_source" || Array.isArray(payload.sourceDecisionTable.rows),
    `The ${label} source decision table has no rows or no-source state.`,
  );
  if (payload.answerGeneration) {
    assertSmoke(
      ["local_ollama", "template_fallback"].includes(payload.answerGeneration.mode),
      `The ${label} answer generation mode is not a supported local mode: ${payload.answerGeneration.mode}.`,
    );
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
